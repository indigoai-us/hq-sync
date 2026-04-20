use std::path::PathBuf;

const BUNDLE_ID: &str = "ai.indigo.hq-sync-menubar";
const FALLBACK_APP_PATH: &str = "/Applications/HQ Sync.app/Contents/MacOS/HQ Sync";

/// Returns the path to ~/Library/LaunchAgents/{BUNDLE_ID}.plist.
fn plist_path() -> Result<PathBuf, String> {
    let home = dirs::home_dir().ok_or_else(|| "Cannot determine home directory".to_string())?;
    Ok(home
        .join("Library")
        .join("LaunchAgents")
        .join(format!("{}.plist", BUNDLE_ID)))
}

/// Resolve the app executable path by walking up from the current binary
/// to find the .app bundle, then pointing at Contents/MacOS/<name>.
/// Falls back to FALLBACK_APP_PATH if resolution fails.
fn resolve_app_path() -> String {
    if let Ok(exe) = std::env::current_exe() {
        // Walk up looking for a directory ending in .app
        let mut current = exe.as_path();
        while let Some(parent) = current.parent() {
            if let Some(name) = current.file_name() {
                if name.to_string_lossy().ends_with(".app") {
                    // Found the .app bundle — derive the executable path inside it
                    let app_name = name
                        .to_string_lossy()
                        .trim_end_matches(".app")
                        .to_string();
                    let bin_path = current
                        .join("Contents")
                        .join("MacOS")
                        .join(&app_name);
                    return bin_path.to_string_lossy().to_string();
                }
            }
            current = parent;
        }
    }
    FALLBACK_APP_PATH.to_string()
}

/// Generate the LaunchAgent plist XML content for the given app path.
fn generate_plist(app_path: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{}</string>
    <key>ProgramArguments</key>
    <array>
        <string>{}</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
</dict>
</plist>
"#,
        BUNDLE_ID, app_path
    )
}

/// Check whether the LaunchAgent plist exists (i.e. autostart is enabled).
#[tauri::command]
pub async fn get_autostart_enabled() -> Result<bool, String> {
    let path = plist_path()?;
    Ok(path.exists())
}

/// Enable or disable autostart by creating or removing the LaunchAgent plist.
#[tauri::command]
pub async fn set_autostart_enabled(enabled: bool) -> Result<(), String> {
    let path = plist_path()?;

    if enabled {
        // Ensure ~/Library/LaunchAgents/ exists
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create LaunchAgents directory: {}", e))?;
        }

        let app_path = resolve_app_path();
        let plist_content = generate_plist(&app_path);

        std::fs::write(&path, plist_content)
            .map_err(|e| format!("Failed to write LaunchAgent plist: {}", e))?;
    } else {
        // Remove the plist if it exists
        if path.exists() {
            std::fs::remove_file(&path)
                .map_err(|e| format!("Failed to remove LaunchAgent plist: {}", e))?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_plist_path_format() {
        let path = plist_path().unwrap();
        let path_str = path.to_string_lossy();
        assert!(path_str.contains("Library/LaunchAgents"));
        assert!(path_str.ends_with("ai.indigo.hq-sync-menubar.plist"));
    }

    #[test]
    fn test_generate_plist_content() {
        let plist = generate_plist("/Applications/HQ Sync.app/Contents/MacOS/HQ Sync");

        assert!(plist.contains("<?xml version=\"1.0\""));
        assert!(plist.contains("<!DOCTYPE plist"));
        assert!(plist.contains("<key>Label</key>"));
        assert!(plist.contains(&format!("<string>{}</string>", BUNDLE_ID)));
        assert!(plist.contains("<key>ProgramArguments</key>"));
        assert!(plist.contains(
            "<string>/Applications/HQ Sync.app/Contents/MacOS/HQ Sync</string>"
        ));
        assert!(plist.contains("<key>RunAtLoad</key>"));
        assert!(plist.contains("<true/>"));
    }

    #[test]
    fn test_generate_plist_custom_path() {
        let custom = "/usr/local/bin/my-app";
        let plist = generate_plist(custom);
        assert!(plist.contains(&format!("<string>{}</string>", custom)));
    }

    #[test]
    fn test_resolve_app_path_returns_string() {
        // In test context we won't be inside a .app bundle,
        // so this should return the fallback path.
        let path = resolve_app_path();
        assert!(!path.is_empty());
        // In CI/test, expect fallback
        assert_eq!(path, FALLBACK_APP_PATH);
    }

    #[test]
    fn test_plist_write_and_remove() {
        let tmp = TempDir::new().unwrap();
        let plist_file = tmp.path().join("ai.indigo.hq-sync-menubar.plist");

        // Write
        let content = generate_plist(FALLBACK_APP_PATH);
        std::fs::write(&plist_file, &content).unwrap();
        assert!(plist_file.exists());

        // Verify content
        let read_back = std::fs::read_to_string(&plist_file).unwrap();
        assert!(read_back.contains(BUNDLE_ID));

        // Remove
        std::fs::remove_file(&plist_file).unwrap();
        assert!(!plist_file.exists());
    }

    #[test]
    fn test_plist_is_valid_xml() {
        let plist = generate_plist(FALLBACK_APP_PATH);
        // Basic XML validity checks
        assert!(plist.starts_with("<?xml"));
        assert!(plist.contains("<plist version=\"1.0\">"));
        assert!(plist.contains("</plist>"));
        assert!(plist.contains("<dict>"));
        assert!(plist.contains("</dict>"));
        assert!(plist.contains("<array>"));
        assert!(plist.contains("</array>"));
    }
}
