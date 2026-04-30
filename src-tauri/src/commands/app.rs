/// Terminate the app from the renderer. Used by the in-popover Quit
/// button — the menubar-app close handler intercepts plain window-close
/// events and only hides, so the renderer needs an explicit exit path
/// to mirror what the tray's Quit menu item does (`app.exit(0)`).
#[tauri::command]
pub fn quit_app(app: tauri::AppHandle) {
    app.exit(0);
}
