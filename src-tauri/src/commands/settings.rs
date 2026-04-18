use crate::commands::config::MenubarPrefs;
use crate::util::paths;

/// Read settings from ~/.hq/menubar.json.
/// Returns current prefs with defaults applied for missing fields.
#[tauri::command]
pub async fn get_settings() -> Result<MenubarPrefs, String> {
    let path = paths::menubar_json_path()?;

    if !path.exists() {
        return Ok(MenubarPrefs {
            hq_path: None,
            sync_on_launch: Some(false),
            notifications: Some(true),
            start_at_login: Some(true),
        });
    }

    let contents = std::fs::read_to_string(&path)
        .map_err(|e| format!("Failed to read menubar.json: {}", e))?;
    let prefs: MenubarPrefs = serde_json::from_str(&contents)
        .map_err(|e| format!("Failed to parse menubar.json: {}", e))?;

    // Apply defaults for missing fields
    Ok(MenubarPrefs {
        hq_path: prefs.hq_path,
        sync_on_launch: Some(prefs.sync_on_launch.unwrap_or(false)),
        notifications: Some(prefs.notifications.unwrap_or(true)),
        start_at_login: Some(prefs.start_at_login.unwrap_or(true)),
    })
}

/// Write settings to ~/.hq/menubar.json (pretty-printed JSON).
#[tauri::command]
pub async fn save_settings(prefs: MenubarPrefs) -> Result<(), String> {
    let path = paths::menubar_json_path()?;

    // Ensure ~/.hq/ directory exists
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create config directory: {}", e))?;
    }

    let json = serde_json::to_string_pretty(&prefs)
        .map_err(|e| format!("Failed to serialize settings: {}", e))?;

    std::fs::write(&path, json)
        .map_err(|e| format!("Failed to write menubar.json: {}", e))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_defaults_applied_for_missing_fields() {
        // When all fields are None, defaults should be applied
        let prefs = MenubarPrefs {
            hq_path: None,
            sync_on_launch: None,
            notifications: None,
            start_at_login: None,
        };

        let result = MenubarPrefs {
            hq_path: prefs.hq_path,
            sync_on_launch: Some(prefs.sync_on_launch.unwrap_or(false)),
            notifications: Some(prefs.notifications.unwrap_or(true)),
            start_at_login: Some(prefs.start_at_login.unwrap_or(true)),
        };

        assert_eq!(result.hq_path, None);
        assert_eq!(result.sync_on_launch, Some(false));
        assert_eq!(result.notifications, Some(true));
        assert_eq!(result.start_at_login, Some(true));
    }

    #[test]
    fn test_explicit_values_preserved() {
        let prefs = MenubarPrefs {
            hq_path: Some("/custom/path".to_string()),
            sync_on_launch: Some(true),
            notifications: Some(false),
            start_at_login: Some(false),
        };

        let result = MenubarPrefs {
            hq_path: prefs.hq_path,
            sync_on_launch: Some(prefs.sync_on_launch.unwrap_or(false)),
            notifications: Some(prefs.notifications.unwrap_or(true)),
            start_at_login: Some(prefs.start_at_login.unwrap_or(true)),
        };

        assert_eq!(result.hq_path, Some("/custom/path".to_string()));
        assert_eq!(result.sync_on_launch, Some(true));
        assert_eq!(result.notifications, Some(false));
        assert_eq!(result.start_at_login, Some(false));
    }

    #[test]
    fn test_roundtrip_serialization() {
        let prefs = MenubarPrefs {
            hq_path: Some("/Users/test/HQ".to_string()),
            sync_on_launch: Some(true),
            notifications: Some(true),
            start_at_login: Some(false),
        };

        let json = serde_json::to_string_pretty(&prefs).unwrap();
        let parsed: MenubarPrefs = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.hq_path, prefs.hq_path);
        assert_eq!(parsed.sync_on_launch, prefs.sync_on_launch);
        assert_eq!(parsed.notifications, prefs.notifications);
        assert_eq!(parsed.start_at_login, prefs.start_at_login);
    }

    #[test]
    fn test_save_creates_file() {
        let tmp = TempDir::new().unwrap();
        let file_path = tmp.path().join("menubar.json");

        let prefs = MenubarPrefs {
            hq_path: None,
            sync_on_launch: Some(false),
            notifications: Some(true),
            start_at_login: Some(true),
        };

        let json = serde_json::to_string_pretty(&prefs).unwrap();
        std::fs::write(&file_path, &json).unwrap();

        let contents = std::fs::read_to_string(&file_path).unwrap();
        let parsed: MenubarPrefs = serde_json::from_str(&contents).unwrap();
        assert_eq!(parsed.sync_on_launch, Some(false));
        assert_eq!(parsed.notifications, Some(true));
    }

    #[test]
    fn test_pretty_print_format() {
        let prefs = MenubarPrefs {
            hq_path: None,
            sync_on_launch: Some(false),
            notifications: Some(true),
            start_at_login: Some(true),
        };

        let json = serde_json::to_string_pretty(&prefs).unwrap();
        // Pretty-printed JSON should contain newlines
        assert!(json.contains('\n'));
        // Should use camelCase keys
        assert!(json.contains("syncOnLaunch"));
        assert!(json.contains("startAtLogin"));
    }
}
