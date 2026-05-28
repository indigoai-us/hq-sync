/// Terminate the app from the renderer. Used by the in-popover Quit
/// button — the menubar-app close handler intercepts plain window-close
/// events and only hides, so the renderer needs an explicit exit path
/// to mirror what the tray's Quit menu item does (`app.exit(0)`).
#[tauri::command]
pub fn quit_app(app: tauri::AppHandle) {
    app.exit(0);
}

/// Ask the main menubar window to show its existing Settings surface.
///
/// Desktop-alt is a separate webview, but Settings still lives in the main
/// popover App.svelte state. This command keeps the renderer contract simple:
/// desktop UI invokes `open_settings_window`, Rust shows/focuses the main
/// window, then emits the same event path the tray menu already uses.
#[tauri::command]
pub fn open_settings_window(app: tauri::AppHandle) -> Result<(), String> {
    use tauri::{Emitter, Manager};

    if let Some(window) = app.get_webview_window("main") {
        window.show().map_err(|e| e.to_string())?;
        window.set_focus().map_err(|e| e.to_string())?;
    }

    app.emit_to("main", "tray:open-settings", ())
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Open a Claude Code deep link (`claude://code/new?...`). The renderer
/// can't call `@tauri-apps/plugin-shell` `open()` for non-http(s) schemes
/// without widening `shell:allow-open` to the world; this command keeps
/// the surface tight by only forwarding `claude://` URLs to macOS `open`.
#[tauri::command]
pub fn open_claude_code_link(url: String) -> Result<(), String> {
    if !url.starts_with("claude://") {
        return Err(format!("refusing to open non-claude scheme: {}", url));
    }

    let output = std::process::Command::new("open")
        .arg(&url)
        .output()
        .map_err(|e| format!("Failed to run open: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "open exited {}: {}",
            output
                .status
                .code()
                .map(|c| c.to_string())
                .unwrap_or_else(|| "unknown".to_string()),
            stderr.trim()
        ));
    }

    Ok(())
}
