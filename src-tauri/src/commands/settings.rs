use crate::commands::config::MenubarPrefs;
use crate::util::feature_gate;
use crate::util::paths;
use crate::util::release_channel::{effective_channel, ReleaseChannel};

/// Resolve the `release_channel` field for the Settings UI. Returns the
/// SAME value the updater's resolver will use, so the toggle the user
/// sees always matches the channel that will actually be polled.
///
/// - `Some("stable" | "beta" | "alpha")` reflects an explicit prior
///   choice (or the indigo-default of `"beta"` for users who haven't
///   touched the picker yet).
/// - Non-indigo users always see `"stable"`, regardless of what's stored
///   — matching the defense-in-depth coercion in `updater.rs`.
async fn resolved_release_channel(stored: Option<&str>) -> String {
    let is_indigo = feature_gate::is_indigo_user().await;
    let resolved: ReleaseChannel = effective_channel(stored, is_indigo);
    resolved.as_str().to_string()
}

/// Read settings from ~/.hq/menubar.json.
/// Returns current prefs with defaults applied for missing fields.
#[tauri::command]
pub async fn get_settings() -> Result<MenubarPrefs, String> {
    let path = paths::menubar_json_path()?;

    if !path.exists() {
        // Fresh install: no menubar.json yet. Resolve the channel
        // against the (possibly cached) indigo gate so the Settings UI
        // shows the right default — `"beta"` for `@getindigo.ai` users
        // (auto-opt-in to dogfood), `"stable"` for everyone else. This
        // value is NOT written back here; save_settings will persist
        // when the user touches a toggle.
        let channel = resolved_release_channel(None).await;
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
            share_notifications: Some(true),
            release_channel: Some(channel),
        });
    }

    let contents = std::fs::read_to_string(&path)
        .map_err(|e| format!("Failed to read menubar.json: {}", e))?;
    let prefs: MenubarPrefs = serde_json::from_str(&contents)
        .map_err(|e| format!("Failed to parse menubar.json: {}", e))?;

    // Resolve channel through the same effective_channel logic the
    // updater uses, so the Settings UI cannot drift from the
    // background-update behavior.
    let resolved_channel = resolved_release_channel(prefs.release_channel.as_deref()).await;

    // Apply defaults for missing fields. `realtime_sync` defaults ON — it
    // mirrors `is_realtime_sync_enabled` in daemon.rs so the Settings toggle
    // and the auto-start logic agree on a fresh install. `personal_sync_enabled`
    // defaults ON to preserve pre-5.25 behavior — only users who explicitly
    // toggle it off see the personal target drop from the fanout.
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
        // Share notifications default ON — re-read on each poll cycle so the
        // toggle takes effect without restart. Only active for @getindigo.ai
        // users (dogfood gate checked separately in share_notify.rs).
        share_notifications: Some(prefs.share_notifications.unwrap_or(true)),
        release_channel: Some(resolved_channel),
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

    /// Builder shorthand — every test below shares the same "blank prefs"
    /// skeleton and overrides only the fields under test. Pre-channel-rollout
    /// tests open-coded full literals; this helper keeps adding a new
    /// `MenubarPrefs` field to a single site.
    fn empty_prefs() -> MenubarPrefs {
        MenubarPrefs {
            hq_path: None,
            sync_on_launch: None,
            notifications: None,
            start_at_login: None,
            autostart_daemon: None,
            realtime_sync: None,
            personal_sync_enabled: None,
            instant_sync: None,
            drift_staging_repo: None,
            share_notifications: None,
            release_channel: None,
        }
    }

    /// The defaults block exercised by `get_settings`'s "file present"
    /// branch — pulled out so each test can apply the same mapping
    /// without re-typing it. `release_channel` is intentionally NOT
    /// resolved here (resolution lives in get_settings itself which is
    /// async and feature-gated); these tests verify the OTHER defaults.
    fn apply_defaults(prefs: MenubarPrefs) -> MenubarPrefs {
        MenubarPrefs {
            hq_path: prefs.hq_path,
            sync_on_launch: Some(prefs.sync_on_launch.unwrap_or(false)),
            notifications: Some(prefs.notifications.unwrap_or(true)),
            start_at_login: Some(prefs.start_at_login.unwrap_or(true)),
            autostart_daemon: Some(prefs.autostart_daemon.unwrap_or(false)),
            realtime_sync: Some(prefs.realtime_sync.unwrap_or(true)),
            personal_sync_enabled: Some(prefs.personal_sync_enabled.unwrap_or(true)),
            instant_sync: Some(prefs.instant_sync.unwrap_or(true)),
            drift_staging_repo: prefs.drift_staging_repo,
            share_notifications: Some(prefs.share_notifications.unwrap_or(true)),
            release_channel: prefs.release_channel,
        }
    }

    #[test]
    fn test_defaults_applied_for_missing_fields() {
        // When all fields are None, defaults should be applied.
        let result = apply_defaults(empty_prefs());

        assert_eq!(result.hq_path, None);
        assert_eq!(result.sync_on_launch, Some(false));
        assert_eq!(result.notifications, Some(true));
        assert_eq!(result.start_at_login, Some(true));
        assert_eq!(result.realtime_sync, Some(true));
        assert_eq!(result.share_notifications, Some(true));
        // release_channel stays None at the apply_defaults boundary; the
        // identity-aware resolution happens inside get_settings itself
        // and is exercised by util::release_channel::tests.
        assert_eq!(result.release_channel, None);
    }

    #[test]
    fn test_explicit_realtime_sync_false_preserved() {
        // A user who explicitly toggled Auto-sync off must NOT be flipped back
        // on by the new default. The `unwrap_or(true)` only fires when the
        // field is absent from menubar.json.
        let prefs = MenubarPrefs {
            realtime_sync: Some(false),
            ..empty_prefs()
        };

        let result = apply_defaults(prefs);

        assert_eq!(result.realtime_sync, Some(false));
        assert_eq!(result.share_notifications, Some(true));
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
            share_notifications: Some(false),
            release_channel: Some("alpha".to_string()),
        };

        let result = apply_defaults(prefs);

        assert_eq!(result.hq_path, Some("/custom/path".to_string()));
        assert_eq!(result.sync_on_launch, Some(true));
        assert_eq!(result.notifications, Some(false));
        assert_eq!(result.start_at_login, Some(false));
        assert_eq!(result.autostart_daemon, Some(true));
        // explicit false must survive the unwrap_or(true)
        assert_eq!(result.share_notifications, Some(false));
        // release_channel passes through apply_defaults untouched; the
        // indigo-gating coercion is verified separately in
        // `util::release_channel::tests::non_indigo_always_coerced_to_stable`.
        assert_eq!(result.release_channel, Some("alpha".to_string()));
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
            share_notifications: Some(true),
            release_channel: Some("beta".to_string()),
        };

        let json = serde_json::to_string_pretty(&prefs).unwrap();
        let parsed: MenubarPrefs = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.hq_path, prefs.hq_path);
        assert_eq!(parsed.sync_on_launch, prefs.sync_on_launch);
        assert_eq!(parsed.notifications, prefs.notifications);
        assert_eq!(parsed.start_at_login, prefs.start_at_login);
        assert_eq!(parsed.share_notifications, prefs.share_notifications);
        // releaseChannel round-trips as a camelCase string (matches the
        // #[serde(rename_all = "camelCase")] on MenubarPrefs).
        assert_eq!(parsed.release_channel, Some("beta".to_string()));
        assert!(
            json.contains("\"releaseChannel\":"),
            "expected camelCase key 'releaseChannel' in serialized output, got: {json}"
        );
    }

    #[test]
    fn test_save_creates_file() {
        let tmp = TempDir::new().unwrap();
        let file_path = tmp.path().join("menubar.json");

        let prefs = MenubarPrefs {
            sync_on_launch: Some(false),
            notifications: Some(true),
            start_at_login: Some(true),
            autostart_daemon: Some(false),
            realtime_sync: Some(false),
            personal_sync_enabled: Some(true),
            instant_sync: Some(true),
            share_notifications: Some(true),
            ..empty_prefs()
        };

        let json = serde_json::to_string_pretty(&prefs).unwrap();
        std::fs::write(&file_path, &json).unwrap();

        let contents = std::fs::read_to_string(&file_path).unwrap();
        let parsed: MenubarPrefs = serde_json::from_str(&contents).unwrap();
        assert_eq!(parsed.sync_on_launch, Some(false));
        assert_eq!(parsed.notifications, Some(true));
        assert_eq!(parsed.share_notifications, Some(true));
    }

    #[test]
    fn test_pretty_print_format() {
        let prefs = MenubarPrefs {
            sync_on_launch: Some(false),
            notifications: Some(true),
            start_at_login: Some(true),
            autostart_daemon: Some(false),
            realtime_sync: Some(false),
            personal_sync_enabled: Some(true),
            instant_sync: Some(true),
            share_notifications: Some(true),
            ..empty_prefs()
        };

        let json = serde_json::to_string_pretty(&prefs).unwrap();
        // Pretty-printed JSON should contain newlines
        assert!(json.contains('\n'));
        // Should use camelCase keys
        assert!(json.contains("syncOnLaunch"));
        assert!(json.contains("startAtLogin"));
        assert!(json.contains("shareNotifications"));
    }

    #[test]
    fn test_release_channel_absent_serializes_skipped() {
        // `release_channel: None` should NOT emit a `releaseChannel: null`
        // key — the field is `skip_serializing_if = "Option::is_none"`,
        // so backwards-compat is preserved (a downgrade to a pre-channel
        // build doesn't see an unknown null key).
        let prefs = MenubarPrefs {
            release_channel: None,
            ..empty_prefs()
        };
        let json = serde_json::to_string(&prefs).unwrap();
        assert!(
            !json.contains("releaseChannel"),
            "None should be skipped, got: {json}"
        );
    }
}
