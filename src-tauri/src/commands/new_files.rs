use std::sync::Mutex;

use tauri::{AppHandle, Emitter, Manager};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewFileEntry {
    pub path: String,
    pub bytes: u64,
    pub added_by: Option<String>,
}

/// Managed state: holds the pending file list so the detail window can
/// retrieve it on ready (race-free handshake instead of a timed delay).
pub struct PendingNewFiles(pub Mutex<Vec<NewFileEntry>>);

#[tauri::command]
pub async fn open_new_files_detail(
    app: AppHandle,
    files: Vec<NewFileEntry>,
) -> Result<(), String> {
    let label = "new-files-detail";

    // Stash the file list in managed state so detail_window_ready can
    // retrieve it after the webview finishes loading.
    if let Some(state) = app.try_state::<PendingNewFiles>() {
        *state.0.lock().unwrap() = files.clone();
    }

    // If window already exists, focus it and re-send data
    if let Some(window) = app.get_webview_window(label) {
        window.show().map_err(|e| e.to_string())?;
        window.set_focus().map_err(|e| e.to_string())?;
        // Re-emit data to update the window contents
        app.emit_to(label, "new-files:list", &files)
            .map_err(|e| e.to_string())?;
        return Ok(());
    }

    // Create new window — starts hidden until the renderer signals ready
    tauri::WebviewWindowBuilder::new(
        &app,
        label,
        tauri::WebviewUrl::App("index.html".into()),
    )
    .title("New Files")
    .inner_size(500.0, 400.0)
    .resizable(true)
    .decorations(true)
    .visible(false)
    .build()
    .map_err(|e| e.to_string())?;

    Ok(())
}

/// Called by the detail window's Svelte component once its event listener
/// is registered. Emits the pending file list and shows the window — no
/// race because the renderer asked for the data, not a timer.
#[tauri::command]
pub async fn detail_window_ready(app: AppHandle) -> Result<(), String> {
    let label = "new-files-detail";

    let files = app
        .try_state::<PendingNewFiles>()
        .map(|s| s.0.lock().unwrap().clone())
        .unwrap_or_default();

    app.emit_to(label, "new-files:list", &files)
        .map_err(|e| e.to_string())?;

    if let Some(window) = app.get_webview_window(label) {
        let _ = window.show();
        let _ = window.set_focus();
    }

    Ok(())
}
