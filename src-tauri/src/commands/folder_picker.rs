/// Open a native macOS folder picker dialog.
/// Returns the selected path, or None if the user cancelled.
#[tauri::command]
pub async fn pick_folder() -> Result<Option<String>, String> {
    let result = rfd::AsyncFileDialog::new()
        .set_title("Choose HQ Folder")
        .pick_folder()
        .await;

    Ok(result.map(|handle| handle.path().to_string_lossy().to_string()))
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_dialog_builder_compiles() {
        // Verify rfd API is available and the builder pattern works.
        // We can't actually open a dialog in tests, but we can confirm
        // the builder chain compiles correctly.
        let _builder = rfd::AsyncFileDialog::new().set_title("Choose HQ Folder");
    }
}
