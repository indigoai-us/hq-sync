//! Drift detail window — secondary Tauri window listing the modified /
//! missing / added files from a `DriftReport`. Mirrors `new_files.rs` for
//! the multi-window-handshake pattern (see CLAUDE.md gotchas: managed
//! state + `*_window_ready` command instead of timed `emit_to` to avoid
//! the listener-registration race).

use std::sync::Mutex;

use tauri::{AppHandle, Emitter, Manager};

use crate::commands::hq_core_drift::DriftReport;
use crate::util::logfile::log;

/// Window label used by both this command and the `main.ts` window-router
/// branch. Kept as a constant so the two stay in sync — drifting either
/// side breaks the handshake.
pub const WINDOW_LABEL: &str = "drift-detail";

/// Managed state: holds the pending drift report so the detail window
/// can fetch it on ready (race-free handshake instead of a timed delay).
pub struct PendingDrift(pub Mutex<Option<DriftReport>>);

/// Tauri command — open (or focus + re-emit to) the drift detail window
/// with the given report.
#[tauri::command]
pub async fn open_drift_detail(app: AppHandle, report: DriftReport) -> Result<(), String> {
    log(
        "drift-detail",
        &format!(
            "open: count={} (modified={}, missing={}, added={})",
            report.count,
            report.modified.len(),
            report.missing.len(),
            report.added.len()
        ),
    );
    let mut stashed = false;
    if let Some(state) = app.try_state::<PendingDrift>() {
        *state.0.lock().unwrap() = Some(report.clone());
        stashed = true;
    }
    log(
        "drift-detail",
        &format!("open: PendingDrift stashed={}", stashed),
    );

    if let Some(window) = app.get_webview_window(WINDOW_LABEL) {
        window.show().map_err(|e| e.to_string())?;
        window.set_focus().map_err(|e| e.to_string())?;
        app.emit_to(WINDOW_LABEL, "drift:report", &report)
            .map_err(|e| e.to_string())?;
        return Ok(());
    }

    // `TitleBarStyle::Overlay` keeps the macOS traffic-light buttons
    // (close / minimise / zoom) but removes the opaque title-bar surface
    // — the body extends underneath the buttons so our dark glass bg is
    // continuous instead of butting up against a white macOS title bar.
    // Matches the chrome treatment of VS Code, Xcode, Things, etc. Body
    // CSS reserves the top ~28px with padding so the heading clears the
    // traffic lights. `hidden_title(true)` suppresses the "Core Drift"
    // text in the title bar — redundant with the in-body header.
    // `transparent(true)` + post-build `apply_vibrancy` below combine to
    // give the same backdrop-blur "Liquid Glass" treatment as the main
    // popover. Without vibrancy, transparent: true just makes the window
    // see-through (content behind shows through), which looks broken.
    // Bind builder result to `_window` — we don't need a direct handle
    // here because the vibrancy dispatch below re-fetches via
    // `get_webview_window` (it has to, since it runs in a different
    // closure on the main thread and can't capture the builder's value
    // across the thread boundary cleanly).
    let _window = tauri::WebviewWindowBuilder::new(
        &app,
        WINDOW_LABEL,
        tauri::WebviewUrl::App("index.html".into()),
    )
    .title("Core Drift")
    .inner_size(560.0, 480.0)
    .resizable(true)
    .decorations(true)
    .title_bar_style(tauri::TitleBarStyle::Overlay)
    .hidden_title(true)
    // Native macOS inset for the traffic-light buttons. With
    // TitleBarStyle::Overlay + hidden_title, Tauri otherwise places
    // them at (0, 0) flush against the top-left corner. (20, 14)
    // mirrors what AppKit picks for a standard window title bar.
    .traffic_light_position(tauri::LogicalPosition::new(20.0, 14.0))
    .transparent(true)
    .visible(false)
    .build()
    .map_err(|e| e.to_string())?;

    // macOS-only: apply the same NSVisualEffectView material the popover
    // uses so the drift window inherits the system's backdrop-blur look.
    // window-vibrancy's `apply_vibrancy()` calls into AppKit which is
    // main-thread-only — Tauri command handlers run on the async runtime
    // worker pool, so calling it directly here panics with
    // "can only be used on the main thread". Dispatch via
    // `run_on_main_thread` so AppKit gets its required thread. Without
    // this the window is just `transparent: true` with no blur — content
    // behind shows through as if the window were unset, which looks
    // broken rather than glassy. Failures log but don't propagate: a
    // window without vibrancy is still functional, just transparent.
    #[cfg(target_os = "macos")]
    {
        let app_for_main = app.clone();
        let _ = app.run_on_main_thread(move || {
            use window_vibrancy::{apply_vibrancy, NSVisualEffectMaterial, NSVisualEffectState};
            let Some(window) = app_for_main.get_webview_window(WINDOW_LABEL) else {
                log("drift-detail", "apply_vibrancy: window vanished before main-thread dispatch");
                return;
            };
            if let Err(e) = apply_vibrancy(
                &window,
                NSVisualEffectMaterial::Popover,
                Some(NSVisualEffectState::Active),
                Some(18.0),
            ) {
                log("drift-detail", &format!("apply_vibrancy FAILED: {e}"));
            } else {
                log("drift-detail", "apply_vibrancy: success (Popover material, blur 18)");
            }
        });
    }

    Ok(())
}

/// Called by the detail window's Svelte component once its listener
/// is registered. Emits the pending report and shows the window.
#[tauri::command]
pub async fn drift_window_ready(app: AppHandle) -> Result<(), String> {
    log("drift-detail", "ready: invoked by webview");
    let report = app
        .try_state::<PendingDrift>()
        .and_then(|s| s.0.lock().unwrap().clone());

    log(
        "drift-detail",
        &format!("ready: PendingDrift has report = {}", report.is_some()),
    );

    if let Some(report) = report {
        match app.emit_to(WINDOW_LABEL, "drift:report", &report) {
            Ok(_) => log(
                "drift-detail",
                &format!("ready: emit_to({}) ok, count={}", WINDOW_LABEL, report.count),
            ),
            Err(e) => log("drift-detail", &format!("ready: emit_to failed: {e}")),
        }
    }

    if let Some(window) = app.get_webview_window(WINDOW_LABEL) {
        let _ = window.show();
        let _ = window.set_focus();
        log("drift-detail", "ready: window shown + focused");
    } else {
        log("drift-detail", "ready: get_webview_window returned None");
    }

    Ok(())
}
