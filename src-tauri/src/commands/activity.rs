//! Session activity log — a per-app-session record of every file the sync
//! pipeline uploaded or downloaded, with a timestamp and a direction.
//!
//! Unlike the journal (which keeps each file's *latest* state on disk), this
//! is an in-memory, append-only log scoped to the current app session: it
//! starts empty on launch and accumulates one entry per `progress` event the
//! runner emits (`commands::sync` calls [`record_progress`] from its event
//! dispatch). It clears when the app quits.
//!
//! The log lives in Rust managed state so it can be shared across windows: the
//! main popover triggers [`open_activity_log`] to spawn the detail window, and
//! the window pulls the accumulated list via [`activity_window_ready`] (the
//! same ready-handshake pattern as `new_files`). New entries arriving while the
//! window is open are pushed live via the `activity:append` event.

use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};

use crate::events::SyncProgressEvent;
use crate::util::logfile::log;

/// Window label for the activity-log detail window (routed in `main.ts`).
const ACTIVITY_WINDOW_LABEL: &str = "activity-log";

/// Cap on retained entries so a long-running daemon session can't grow the
/// log unbounded. Oldest entries are dropped first.
const MAX_ENTRIES: usize = 2000;

/// One file change observed during this app session.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ActivityEntry {
    /// Company slug the change belongs to.
    pub company: String,
    /// File path, relative to the company root (as the runner reports it).
    pub path: String,
    /// Size in bytes (0 for deletions).
    pub bytes: u64,
    /// `"up"` (uploaded / synced), `"down"` (downloaded / new-or-updated), or
    /// `"deleted"` (remote delete-marker written). Derived from the runner's
    /// `direction` + `deleted` fields, defaulting to `"down"` for pre-5.29
    /// runners that don't stamp a direction.
    pub direction: String,
    /// Email of the file's author (from the runner's `progress.author`, sourced
    /// from S3 `created-by`). Only present on download rows — a downloaded file
    /// was authored by whoever uploaded it. None on uploads/deletions and on
    /// pre-5.31 runners. The activity log shows it so the user sees who authored
    /// each file they received.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub author: Option<String>,
    /// Epoch milliseconds when the menubar observed the change.
    pub at: u64,
}

/// Managed state: the session's append-only activity log.
pub struct SessionActivity(pub Mutex<Vec<ActivityEntry>>);

impl SessionActivity {
    pub fn new() -> Self {
        SessionActivity(Mutex::new(Vec::new()))
    }

    /// Append an entry, trimming the oldest if over [`MAX_ENTRIES`].
    fn push(&self, entry: ActivityEntry) {
        let mut v = self.0.lock().unwrap_or_else(|e| e.into_inner());
        v.push(entry);
        let len = v.len();
        if len > MAX_ENTRIES {
            v.drain(0..len - MAX_ENTRIES);
        }
    }

    fn snapshot(&self) -> Vec<ActivityEntry> {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

/// Return the current session activity snapshot. The window pulls this on
/// mount (robust against emit-timing races — the earlier emit-on-ready
/// handshake could fire before the webview's listener registered).
#[tauri::command]
pub fn get_activity_log(app: AppHandle) -> Vec<ActivityEntry> {
    app.try_state::<SessionActivity>()
        .map(|s| s.snapshot())
        .unwrap_or_default()
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Map a runner `progress` event onto an [`ActivityEntry`] direction.
fn direction_for(p: &SyncProgressEvent) -> String {
    if p.deleted == Some(true) {
        return "deleted".to_string();
    }
    match p.direction.as_deref() {
        Some("up") => "up",
        Some("down") => "down",
        // Pre-5.29 runners don't stamp direction; `progress` was historically
        // a download-only event, so default to "down".
        _ => "down",
    }
    .to_string()
}

/// Record one `progress` event into the session log and push it live to the
/// activity window if it's open. Called from `commands::sync`'s event dispatch.
pub fn record_progress(app: &AppHandle, p: &SyncProgressEvent) {
    let Some(state) = app.try_state::<SessionActivity>() else {
        return;
    };
    let entry = ActivityEntry {
        company: p.company.clone(),
        path: p.path.clone(),
        bytes: p.bytes,
        direction: direction_for(p),
        author: p.author.clone(),
        at: now_millis(),
    };
    state.push(entry.clone());

    // Live-append to the window if it's open (best-effort; the window also
    // pulls the full snapshot on ready, so a missed append is recoverable).
    if app.get_webview_window(ACTIVITY_WINDOW_LABEL).is_some() {
        let _ = app.emit_to(ACTIVITY_WINDOW_LABEL, "activity:append", &entry);
    }
}

/// Open (or focus) the activity-log detail window. Mirrors
/// `open_new_files_detail`: the window starts hidden and the renderer calls
/// [`activity_window_ready`] once its listeners are registered.
#[tauri::command]
pub async fn open_activity_log(app: AppHandle) -> Result<(), String> {
    log("activity", "open_activity_log invoked");
    if let Some(window) = app.get_webview_window(ACTIVITY_WINDOW_LABEL) {
        log("activity", "open: window exists -> show + emit list");
        window.show().map_err(|e| e.to_string())?;
        window.set_focus().map_err(|e| e.to_string())?;
        if let Some(state) = app.try_state::<SessionActivity>() {
            let snap = state.snapshot();
            log("activity", &format!("open: emit list len={}", snap.len()));
            let _ = app.emit_to(ACTIVITY_WINDOW_LABEL, "activity:list", snap);
        }
        return Ok(());
    }
    log("activity", "open: building new window");

    // `TitleBarStyle::Overlay` + `transparent(true)` + post-build
    // `apply_vibrancy` give the same dark "Liquid Glass" backdrop-blur as the
    // main popover (and the drift-detail window), instead of an opaque white
    // macOS title bar over a flat gray body. `hidden_title(true)` suppresses
    // the title text (redundant with the in-body header); the traffic lights
    // are inset to clear the body's top padding. See drift_detail.rs for the
    // full rationale on the main-thread vibrancy dispatch.
    tauri::WebviewWindowBuilder::new(
        &app,
        ACTIVITY_WINDOW_LABEL,
        tauri::WebviewUrl::App("index.html".into()),
    )
    .title("Recent Changes")
    .inner_size(560.0, 460.0)
    .resizable(true)
    .decorations(true)
    .title_bar_style(tauri::TitleBarStyle::Overlay)
    .hidden_title(true)
    .traffic_light_position(tauri::LogicalPosition::new(20.0, 18.0))
    .transparent(true)
    .visible(false)
    .build()
    .map_err(|e| e.to_string())?;

    // macOS vibrancy must run on the main thread (AppKit) — command handlers
    // run on the async worker pool, so dispatch via run_on_main_thread.
    #[cfg(target_os = "macos")]
    {
        let app_for_main = app.clone();
        let _ = app.run_on_main_thread(move || {
            use window_vibrancy::{apply_vibrancy, NSVisualEffectMaterial, NSVisualEffectState};
            if let Some(window) = app_for_main.get_webview_window(ACTIVITY_WINDOW_LABEL) {
                let _ = apply_vibrancy(
                    &window,
                    NSVisualEffectMaterial::Popover,
                    Some(NSVisualEffectState::Active),
                    Some(18.0),
                );
            }
        });
    }

    // Show the window now rather than waiting for a ready-handshake from the
    // webview. The component pulls its data via get_activity_log on mount, so
    // there's no emit race to avoid — and showing immediately means the FIRST
    // click always opens the window (the prior handshake-to-show could leave
    // it hidden forever if the webview's invoke didn't fire).
    if let Some(window) = app.get_webview_window(ACTIVITY_WINDOW_LABEL) {
        let _ = window.show();
        let _ = window.set_focus();
        log("activity", "open: shown new window");
    }

    Ok(())
}

/// Called by the activity-log window's Svelte component once its listeners are
/// registered. Emits the current snapshot and shows the window — race-free.
#[tauri::command]
pub async fn activity_window_ready(app: AppHandle) -> Result<(), String> {
    log("activity", "activity_window_ready invoked by webview");
    let entries = app
        .try_state::<SessionActivity>()
        .map(|s| s.snapshot())
        .unwrap_or_default();
    log("activity", &format!("ready: snapshot len={}", entries.len()));

    app.emit_to(ACTIVITY_WINDOW_LABEL, "activity:list", entries)
        .map_err(|e| e.to_string())?;

    if let Some(window) = app.get_webview_window(ACTIVITY_WINDOW_LABEL) {
        let _ = window.show();
        let _ = window.set_focus();
        log("activity", "ready: window shown + focused");
    } else {
        log("activity", "ready: window NOT FOUND");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(direction: Option<&str>, deleted: Option<bool>) -> SyncProgressEvent {
        ev_with_author(direction, deleted, None)
    }

    fn ev_with_author(
        direction: Option<&str>,
        deleted: Option<bool>,
        author: Option<&str>,
    ) -> SyncProgressEvent {
        SyncProgressEvent {
            company: "indigo".to_string(),
            path: "knowledge/x.md".to_string(),
            bytes: 10,
            message: None,
            direction: direction.map(|s| s.to_string()),
            deleted,
            author: author.map(|s| s.to_string()),
        }
    }

    #[test]
    fn direction_maps_up_down_deleted_and_defaults() {
        assert_eq!(direction_for(&ev(Some("up"), None)), "up");
        assert_eq!(direction_for(&ev(Some("down"), None)), "down");
        // deleted wins over direction
        assert_eq!(direction_for(&ev(Some("up"), Some(true))), "deleted");
        // pre-5.29 runner (no direction) defaults to download
        assert_eq!(direction_for(&ev(None, None)), "down");
    }

    #[test]
    fn author_flows_from_progress_event_into_entry() {
        // A download event carrying `author` (from S3 created-by) maps onto the
        // ActivityEntry so the activity log can attribute the file.
        let p = ev_with_author(Some("down"), None, Some("alice@example.com"));
        let entry = ActivityEntry {
            company: p.company.clone(),
            path: p.path.clone(),
            bytes: p.bytes,
            direction: direction_for(&p),
            author: p.author.clone(),
            at: 0,
        };
        assert_eq!(entry.author, Some("alice@example.com".to_string()));

        // An upload event has no author.
        let up = ev_with_author(Some("up"), None, None);
        assert_eq!(up.author, None);
    }

    #[test]
    fn push_trims_to_max_entries() {
        let state = SessionActivity::new();
        for i in 0..(MAX_ENTRIES + 50) {
            state.push(ActivityEntry {
                company: "c".to_string(),
                path: format!("f{i}.md"),
                bytes: 1,
                direction: "down".to_string(),
                author: None,
                at: i as u64,
            });
        }
        let snap = state.snapshot();
        assert_eq!(snap.len(), MAX_ENTRIES);
        // Oldest dropped: first retained entry is f50.md (at=50).
        assert_eq!(snap.first().unwrap().at, 50);
        assert_eq!(snap.last().unwrap().path, format!("f{}.md", MAX_ENTRIES + 49));
    }
}
