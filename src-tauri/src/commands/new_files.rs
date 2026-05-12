use tauri::{AppHandle, Emitter, Manager};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewFileEntry {
    pub path: String,
    pub bytes: u64,
    pub added_by: Option<String>,
}

#[tauri::command]
pub async fn open_new_files_detail(
    app: AppHandle,
    files: Vec<NewFileEntry>,
) -> Result<(), String> {
    let label = "new-files-detail";

    // If window already exists, focus it and re-send data
    if let Some(window) = app.get_webview_window(label) {
        window.show().map_err(|e| e.to_string())?;
        window.set_focus().map_err(|e| e.to_string())?;
        // Re-emit data to update the window contents
        app.emit_to(label, "new-files:list", &files)
            .map_err(|e| e.to_string())?;
        return Ok(());
    }

    // Create new window
    let window = tauri::WebviewWindowBuilder::new(
        &app,
        label,
        tauri::WebviewUrl::App("index.html".into()),
    )
    .title("New Files")
    .inner_size(500.0, 400.0)
    .resizable(true)
    .decorations(true)
    .visible(false) // Show after data is sent
    .build()
    .map_err(|e| e.to_string())?;

    // Wait for the webview to initialize, then emit data and show the window.
    let files_clone = files.clone();
    let app_clone = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        let _ = app_clone.emit_to(label, "new-files:list", &files_clone);
        let _ = window.show();
        let _ = window.set_focus();
    });

    Ok(())
}
