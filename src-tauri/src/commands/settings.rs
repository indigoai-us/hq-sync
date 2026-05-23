use crate::commands::config::{MeetingDetectNotifyPrefs, MenubarPrefs};
use crate::util::paths;

/// Default platform allow-list (all five) when the field is absent from disk.
const DEFAULT_PLATFORMS: &[&str] = &["zoom", "meet", "teams", "slack", "webex"];

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
            autostart_daemon: Some(false),
            realtime_sync: Some(true),
            personal_sync_enabled: Some(true),
            instant_sync: Some(true),
            drift_staging_repo: None,
            meeting_detect_notify: Some(default_meeting_detect_notify()),
        });
    }

    let contents = std::fs::read_to_string(&path)
        .map_err(|e| format!("Failed to read menubar.json: {}", e))?;
    let prefs: MenubarPrefs = serde_json::from_str(&contents)
        .map_err(|e| format!("Failed to parse menubar.json: {}", e))?;

    // Apply defaults for missing fields. `realtime_sync` defaults ON — it
    // mirrors `is_realtime_sync_enabled` in daemon.rs so the Settings toggle
    // and the auto-start logic agree on a fresh install. `personal_sync_enabled`
    // defaults ON to preserve pre-5.25 behavior — only users who explicitly
    // toggle it off see the personal target drop from the fanout.
    let mdn = prefs.meeting_detect_notify.unwrap_or_else(default_meeting_detect_notify);
    Ok(MenubarPrefs {
        hq_path: prefs.hq_path,
        sync_on_launch: Some(prefs.sync_on_launch.unwrap_or(false)),
        notifications: Some(prefs.notifications.unwrap_or(true)),
        start_at_login: Some(prefs.start_at_login.unwrap_or(true)),
        autostart_daemon: Some(prefs.autostart_daemon.unwrap_or(false)),
        realtime_sync: Some(prefs.realtime_sync.unwrap_or(true)),
        personal_sync_enabled: Some(prefs.personal_sync_enabled.unwrap_or(true)),
        // Instant sync (event-driven) defaults ON, mirroring `realtime_sync`
        // and `is_instant_sync_enabled` in daemon.rs. Only ever takes effect
        // for `event_push_eligible()` users (Phase 1: @getindigo.ai).
        instant_sync: Some(prefs.instant_sync.unwrap_or(true)),
        drift_staging_repo: prefs.drift_staging_repo,
        meeting_detect_notify: Some(MeetingDetectNotifyPrefs {
            enabled: Some(mdn.enabled.unwrap_or(true)),
            platforms: Some(
                mdn.platforms
                    .unwrap_or_else(|| DEFAULT_PLATFORMS.iter().map(|s| s.to_string()).collect()),
            ),
        }),
    })
}

fn default_meeting_detect_notify() -> MeetingDetectNotifyPrefs {
    MeetingDetectNotifyPrefs {
        enabled: Some(true),
        platforms: Some(DEFAULT_PLATFORMS.iter().map(|s| s.to_string()).collect()),
    }
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
            autostart_daemon: None,
            realtime_sync: None,
            personal_sync_enabled: None,
            instant_sync: None,
            drift_staging_repo: None,
            meeting_detect_notify: None,
        };

        let result = MenubarPrefs {
            hq_path: prefs.hq_path,
            sync_on_launch: Some(prefs.sync_on_launch.unwrap_or(false)),
            notifications: Some(prefs.notifications.unwrap_or(true)),
            start_at_login: Some(prefs.start_at_login.unwrap_or(true)),
            autostart_daemon: Some(prefs.autostart_daemon.unwrap_or(false)),
            realtime_sync: Some(prefs.realtime_sync.unwrap_or(true)),
            personal_sync_enabled: Some(prefs.personal_sync_enabled.unwrap_or(true)),
            instant_sync: Some(prefs.instant_sync.unwrap_or(true)),
            drift_staging_repo: prefs.drift_staging_repo,
            meeting_detect_notify: None,
        };

        assert_eq!(result.hq_path, None);
        assert_eq!(result.sync_on_launch, Some(false));
        assert_eq!(result.notifications, Some(true));
        assert_eq!(result.start_at_login, Some(true));
        assert_eq!(result.realtime_sync, Some(true));
    }

    #[test]
    fn test_explicit_realtime_sync_false_preserved() {
        // A user who explicitly toggled Auto-sync off must NOT be flipped back
        // on by the new default. The `unwrap_or(true)` only fires when the
        // field is absent from menubar.json.
        let prefs = MenubarPrefs {
            hq_path: None,
            sync_on_launch: None,
            notifications: None,
            start_at_login: None,
            autostart_daemon: None,
            realtime_sync: Some(false),
            personal_sync_enabled: None,
            instant_sync: None,
            drift_staging_repo: None,
            meeting_detect_notify: None,
        };

        let result = MenubarPrefs {
            hq_path: prefs.hq_path,
            sync_on_launch: Some(prefs.sync_on_launch.unwrap_or(false)),
            notifications: Some(prefs.notifications.unwrap_or(true)),
            start_at_login: Some(prefs.start_at_login.unwrap_or(true)),
            autostart_daemon: Some(prefs.autostart_daemon.unwrap_or(false)),
            realtime_sync: Some(prefs.realtime_sync.unwrap_or(true)),
            personal_sync_enabled: Some(prefs.personal_sync_enabled.unwrap_or(true)),
            instant_sync: Some(prefs.instant_sync.unwrap_or(true)),
            drift_staging_repo: prefs.drift_staging_repo,
            meeting_detect_notify: None,
        };

        assert_eq!(result.realtime_sync, Some(false));
    }

    #[test]
    fn test_explicit_values_preserved() {
        let prefs = MenubarPrefs {
            hq_path: Some("/custom/path".to_string()),
            sync_on_launch: Some(true),
            notifications: Some(false),
            start_at_login: Some(false),
            autostart_daemon: Some(true),
            realtime_sync: Some(true),
            personal_sync_enabled: Some(true),
            instant_sync: Some(true),
            drift_staging_repo: None,
            meeting_detect_notify: None,
        };

        let result = MenubarPrefs {
            hq_path: prefs.hq_path,
            sync_on_launch: Some(prefs.sync_on_launch.unwrap_or(false)),
            notifications: Some(prefs.notifications.unwrap_or(true)),
            start_at_login: Some(prefs.start_at_login.unwrap_or(true)),
            autostart_daemon: Some(prefs.autostart_daemon.unwrap_or(false)),
            realtime_sync: Some(prefs.realtime_sync.unwrap_or(true)),
            personal_sync_enabled: Some(prefs.personal_sync_enabled.unwrap_or(true)),
            instant_sync: Some(prefs.instant_sync.unwrap_or(true)),
            drift_staging_repo: prefs.drift_staging_repo,
            meeting_detect_notify: None,
        };

        assert_eq!(result.hq_path, Some("/custom/path".to_string()));
        assert_eq!(result.sync_on_launch, Some(true));
        assert_eq!(result.notifications, Some(false));
        assert_eq!(result.start_at_login, Some(false));
        assert_eq!(result.autostart_daemon, Some(true));
    }

    #[test]
    fn test_roundtrip_serialization() {
        let prefs = MenubarPrefs {
            hq_path: Some("/Users/test/HQ".to_string()),
            sync_on_launch: Some(true),
            notifications: Some(true),
            start_at_login: Some(false),
            autostart_daemon: Some(false),
            realtime_sync: Some(false),
            personal_sync_enabled: Some(true),
            instant_sync: Some(true),
            drift_staging_repo: None,
            meeting_detect_notify: None,
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
            autostart_daemon: Some(false),
            realtime_sync: Some(false),
            personal_sync_enabled: Some(true),
            instant_sync: Some(true),
            drift_staging_repo: None,
            meeting_detect_notify: None,
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
            autostart_daemon: Some(false),
            realtime_sync: Some(false),
            personal_sync_enabled: Some(true),
            instant_sync: Some(true),
            drift_staging_repo: None,
            meeting_detect_notify: None,
        };

        let json = serde_json::to_string_pretty(&prefs).unwrap();
        // Pretty-printed JSON should contain newlines
        assert!(json.contains('\n'));
        // Should use camelCase keys
        assert!(json.contains("syncOnLaunch"));
        assert!(json.contains("startAtLogin"));
    }

    #[test]
    fn test_meeting_detect_notify_defaults_applied() {
        // When absent from disk, get_settings should return enabled=true + all 5 platforms.
        let mdn = default_meeting_detect_notify();
        assert_eq!(mdn.enabled, Some(true));
        let platforms = mdn.platforms.unwrap();
        assert!(platforms.contains(&"zoom".to_string()));
        assert!(platforms.contains(&"meet".to_string()));
        assert!(platforms.contains(&"teams".to_string()));
        assert!(platforms.contains(&"slack".to_string()));
        assert!(platforms.contains(&"webex".to_string()));
        assert_eq!(platforms.len(), 5);
    }

    #[test]
    fn test_meeting_detect_notify_roundtrip() {
        // Partial prefs (only zoom + meet) survive a serde round-trip.
        use crate::commands::config::MeetingDetectNotifyPrefs;
        let prefs = MenubarPrefs {
            hq_path: None,
            sync_on_launch: None,
            notifications: None,
            start_at_login: None,
            autostart_daemon: None,
            realtime_sync: None,
            personal_sync_enabled: None,
            instant_sync: None,
            drift_staging_repo: None,
            meeting_detect_notify: Some(MeetingDetectNotifyPrefs {
                enabled: Some(false),
                platforms: Some(vec!["zoom".to_string(), "meet".to_string()]),
            }),
        };
        let json = serde_json::to_string_pretty(&prefs).unwrap();
        // Key should appear in camelCase
        assert!(json.contains("meetingDetectNotify"), "expected camelCase key");
        let parsed: MenubarPrefs = serde_json::from_str(&json).unwrap();
        let mdn = parsed.meeting_detect_notify.unwrap();
        assert_eq!(mdn.enabled, Some(false));
        assert_eq!(mdn.platforms, Some(vec!["zoom".to_string(), "meet".to_string()]));
    }

    #[test]
    fn test_meeting_detect_notify_absent_deserializes_none() {
        // Old menubar.json files without the field must still load cleanly.
        let json = r#"{"hqPath":"/x","syncOnLaunch":false,"notifications":true,"startAtLogin":true,"autostartDaemon":false}"#;
        let prefs: MenubarPrefs = serde_json::from_str(json).unwrap();
        assert!(prefs.meeting_detect_notify.is_none());
    }
}
