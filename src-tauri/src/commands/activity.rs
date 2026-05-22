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
    if let Some(window) = app.get_webview_window(ACTIVITY_WINDOW_LABEL) {
        window.show().map_err(|e| e.to_string())?;
        window.set_focus().map_err(|e| e.to_string())?;
        if let Some(state) = app.try_state::<SessionActivity>() {
            let _ = app.emit_to(ACTIVITY_WINDOW_LABEL, "activity:list", state.snapshot());
        }
        return Ok(());
    }

    tauri::WebviewWindowBuilder::new(
        &app,
        ACTIVITY_WINDOW_LABEL,
        tauri::WebviewUrl::App("index.html".into()),
    )
    .title("Recent Changes")
    .inner_size(560.0, 460.0)
    .resizable(true)
    .decorations(true)
    .visible(false)
    .build()
    .map_err(|e| e.to_string())?;

    Ok(())
}

/// Called by the activity-log window's Svelte component once its listeners are
/// registered. Emits the current snapshot and shows the window — race-free.
#[tauri::command]
pub async fn activity_window_ready(app: AppHandle) -> Result<(), String> {
    let entries = app
        .try_state::<SessionActivity>()
        .map(|s| s.snapshot())
        .unwrap_or_default();

    app.emit_to(ACTIVITY_WINDOW_LABEL, "activity:list", entries)
        .map_err(|e| e.to_string())?;

    if let Some(window) = app.get_webview_window(ACTIVITY_WINDOW_LABEL) {
        let _ = window.show();
        let _ = window.set_focus();
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(direction: Option<&str>, deleted: Option<bool>) -> SyncProgressEvent {
        SyncProgressEvent {
            company: "indigo".to_string(),
            path: "knowledge/x.md".to_string(),
            bytes: 10,
            message: None,
            direction: direction.map(|s| s.to_string()),
            deleted,
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
    fn push_trims_to_max_entries() {
        let state = SessionActivity::new();
        for i in 0..(MAX_ENTRIES + 50) {
            state.push(ActivityEntry {
                company: "c".to_string(),
                path: format!("f{i}.md"),
                bytes: 1,
                direction: "down".to_string(),
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
