use crate::commands::config::{MeetingDetectNotifyPrefs, MenubarPrefs};
use crate::util::paths;

/// Default platform allow-list (all five) when the field is absent from disk.
const DEFAULT_PLATFORMS: &[&str] = &["zoom", "meet", "teams", "slack", "webex"];

/// Read settings from ~/.hq/menubar.json.
/// Returns current prefs with defaults applied for missing fields.
///
/// `release_channel` is returned RAW (the value as stored on disk; `None`
/// when the user has never explicitly chosen a channel). The Settings UI
/// is responsible for resolving `None` into a displayed default via
/// `available_channels`; resolution-for-the-updater lives in
/// `updater::read_stored_release_channel` + `effective_channel` so this
/// boundary stays a pure pass-through. Persisting the resolved value
/// here would lock indigo users into "beta" the first time they touch
/// any unrelated toggle, defeating the "no preference" state the
/// effective_channel gate is designed to honor (Codex P1 review on #120).
#[tauri::command]
pub async fn get_settings() -> Result<MenubarPrefs, String> {
    let path = paths::menubar_json_path()?;

    if !path.exists() {
        return Ok(MenubarPrefs {
            hq_path: None,
            // Sync-on-launch defaults ON so a fresh install syncs as soon as it
            // opens, matching the always-on auto-sync (realtime_sync) default.
            sync_on_launch: Some(true),
            notifications: Some(true),
            start_at_login: Some(true),
            autostart_daemon: Some(false),
            realtime_sync: Some(true),
            personal_sync_enabled: Some(true),
            instant_sync: Some(true),
            drift_staging_repo: None,
            share_notifications: Some(true),
            dm_notifications: Some(true),
            cli_auto_update: Some(true),
            staging_channel: Some(true),
            release_channel: None,
            meeting_detect_notify: Some(default_meeting_detect_notify()),
            default_recording_company_uid: None,
            // Telemetry is opt-in; absent → off (mirrors
            // telemetry.rs::read_local_telemetry_enabled's unwrap_or(false)).
            telemetry_enabled: Some(false),
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
    let mdn = prefs
        .meeting_detect_notify
        .unwrap_or_else(default_meeting_detect_notify);
    Ok(MenubarPrefs {
        hq_path: prefs.hq_path,
        // Default ON (see the no-file branch above) — absent key syncs on launch.
        sync_on_launch: Some(prefs.sync_on_launch.unwrap_or(true)),
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
        // DM notifications default ON — re-read directly from menubar.json on
        // each poll cycle in dm_notify.rs so the toggle takes effect without
        // restart. Mirrors `share_notifications`.
        dm_notifications: Some(prefs.dm_notifications.unwrap_or(true)),
        // CLI auto-update defaults ON — re-read untyped from menubar.json by
        // hq_cli_update.rs on each background check so the toggle takes effect
        // without restart. Mirrors `dm_notifications`.
        cli_auto_update: Some(prefs.cli_auto_update.unwrap_or(true)),
        // Staging channel (@getindigo.ai-only): defaults ON so existing
        // builders' "Update to Staging" pill keeps rendering across the
        // upgrade. An explicit `false` flips them to the prod release
        // channel — the same surface non-@indigo users see. See
        // `MenubarPrefs::staging_channel` for the full gating contract.
        staging_channel: Some(prefs.staging_channel.unwrap_or(true)),
        // Pass-through (NOT resolved) — see fn-level comment.
        release_channel: prefs.release_channel,
        // Meeting detect-notify: defaults to enabled + all five platforms
        // when absent on disk. Only ever fires for `@getindigo.ai` users
        // (gate in `commands/recall_sdk.rs::is_meeting_detect_allowed_email`),
        // and the platform allowlist is applied in `meetings.rs` notify path.
        meeting_detect_notify: Some(MeetingDetectNotifyPrefs {
            enabled: Some(mdn.enabled.unwrap_or(true)),
            platforms: Some(
                mdn.platforms
                    .unwrap_or_else(|| DEFAULT_PLATFORMS.iter().map(|s| s.to_string()).collect()),
            ),
        }),
        // Pass-through — `None` means Personal; the Settings dropdown
        // surfaces this as the "Personal" option (same shape as the
        // URL-invite picker in MeetingsWindow).
        default_recording_company_uid: prefs.default_recording_company_uid,
        // Telemetry defaults OFF (opt-in). Re-read untyped from menubar.json by
        // the collector each sync, so the toggle takes effect without restart.
        telemetry_enabled: Some(prefs.telemetry_enabled.unwrap_or(false)),
    })
}

fn default_meeting_detect_notify() -> MeetingDetectNotifyPrefs {
    MeetingDetectNotifyPrefs {
        enabled: Some(true),
        platforms: Some(DEFAULT_PLATFORMS.iter().map(|s| s.to_string()).collect()),
    }
}

/// Write settings to ~/.hq/menubar.json, preserving keys the typed
/// `MenubarPrefs` doesn't model.
///
/// IMPORTANT: a plain `to_string_pretty(&prefs)` overwrite WIPED every
/// top-level key absent from the struct — `machineId`, `firstRunCompleted`,
/// `autoSyncNoticeShown`, `cliUpdateDismissedVersion` — on every settings save.
/// That regenerated machine identity (telemetry double-counting) and could
/// re-trigger first-run onboarding on the next launch. So we merge: emit the
/// typed fields, then carry forward any on-disk keys the typed output lacks,
/// and atomic-rename — the same untyped-merge contract as
/// `config::ensure_machine_id` / first_run.rs.
#[tauri::command]
pub async fn save_settings(prefs: MenubarPrefs) -> Result<(), String> {
    let path = paths::menubar_json_path()?;

    // Ensure ~/.hq/ directory exists
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create config directory: {}", e))?;
    }

    let existing = if path.exists() {
        std::fs::read_to_string(&path).ok()
    } else {
        None
    };
    let json = merge_prefs_over_existing(&prefs, existing.as_deref())?;

    // Atomic write: stage to a temp file, fsync, rename into place.
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, json.as_bytes())
        .map_err(|e| format!("Failed to write menubar.json: {}", e))?;
    std::fs::rename(&tmp, &path)
        .map_err(|e| format!("Failed to write menubar.json: {}", e))?;

    Ok(())
}

/// Serialize the typed `prefs`, then carry forward any top-level keys present
/// in `existing_json` (the current on-disk menubar.json) that the typed struct
/// doesn't model — `machineId`, `firstRunCompleted`, `autoSyncNoticeShown`,
/// `cliUpdateDismissedVersion`, and any future/unknown keys. Typed fields
/// always win on collision; on-disk-only keys are preserved verbatim.
///
/// Pulled out of `save_settings` so the data-loss-prevention contract is unit
/// testable without touching the real `~/.hq/menubar.json` (the command itself
/// resolves an absolute path via `paths::menubar_json_path`).
fn merge_prefs_over_existing(
    prefs: &MenubarPrefs,
    existing_json: Option<&str>,
) -> Result<String, String> {
    let incoming = serde_json::to_value(prefs)
        .map_err(|e| format!("Failed to serialize settings: {}", e))?;
    let mut obj = incoming.as_object().cloned().unwrap_or_default();
    if let Some(existing) = existing_json
        .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
        .and_then(|v| v.as_object().cloned())
    {
        for (k, v) in existing {
            obj.entry(k).or_insert(v);
        }
    }
    serde_json::to_string_pretty(&serde_json::Value::Object(obj))
        .map_err(|e| format!("Failed to serialize settings: {}", e))
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
            dm_notifications: None,
            cli_auto_update: None,
            staging_channel: None,
            release_channel: None,
            meeting_detect_notify: None,
            default_recording_company_uid: None,
            telemetry_enabled: None,
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
            sync_on_launch: Some(prefs.sync_on_launch.unwrap_or(true)),
            notifications: Some(prefs.notifications.unwrap_or(true)),
            start_at_login: Some(prefs.start_at_login.unwrap_or(true)),
            autostart_daemon: Some(prefs.autostart_daemon.unwrap_or(false)),
            realtime_sync: Some(prefs.realtime_sync.unwrap_or(true)),
            personal_sync_enabled: Some(prefs.personal_sync_enabled.unwrap_or(true)),
            instant_sync: Some(prefs.instant_sync.unwrap_or(true)),
            drift_staging_repo: prefs.drift_staging_repo,
            share_notifications: Some(prefs.share_notifications.unwrap_or(true)),
            dm_notifications: Some(prefs.dm_notifications.unwrap_or(true)),
            cli_auto_update: Some(prefs.cli_auto_update.unwrap_or(true)),
            staging_channel: Some(prefs.staging_channel.unwrap_or(true)),
            release_channel: prefs.release_channel,
            meeting_detect_notify: prefs.meeting_detect_notify,
            default_recording_company_uid: prefs.default_recording_company_uid,
            telemetry_enabled: Some(prefs.telemetry_enabled.unwrap_or(false)),
        }
    }

    #[test]
    fn test_defaults_applied_for_missing_fields() {
        // When all fields are None, defaults should be applied.
        let result = apply_defaults(empty_prefs());

        assert_eq!(result.hq_path, None);
        assert_eq!(result.sync_on_launch, Some(true));
        assert_eq!(result.notifications, Some(true));
        assert_eq!(result.start_at_login, Some(true));
        assert_eq!(result.realtime_sync, Some(true));
        assert_eq!(result.share_notifications, Some(true));
        assert_eq!(result.dm_notifications, Some(true));
        // staging_channel defaults ON (@indigo users keep "Update to Staging"
        // across the upgrade until they explicitly toggle off).
        assert_eq!(result.staging_channel, Some(true));
        // CLI auto-update defaults ON — the app keeps the CLI current unless
        // the user opts out.
        assert_eq!(result.cli_auto_update, Some(true));
        // Telemetry is opt-in — defaults OFF when absent from disk.
        assert_eq!(result.telemetry_enabled, Some(false));
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
            dm_notifications: Some(false),
            cli_auto_update: Some(false),
            staging_channel: Some(false),
            release_channel: Some("alpha".to_string()),
            meeting_detect_notify: None,
            default_recording_company_uid: Some("co_xyz".to_string()),
            telemetry_enabled: Some(true),
        };

        let result = apply_defaults(prefs);

        assert_eq!(result.hq_path, Some("/custom/path".to_string()));
        assert_eq!(result.sync_on_launch, Some(true));
        assert_eq!(result.notifications, Some(false));
        assert_eq!(result.start_at_login, Some(false));
        assert_eq!(result.autostart_daemon, Some(true));
        // explicit false must survive the unwrap_or(true)
        assert_eq!(result.share_notifications, Some(false));
        assert_eq!(result.dm_notifications, Some(false));
        assert_eq!(result.cli_auto_update, Some(false));
        assert_eq!(result.staging_channel, Some(false));
        // explicit telemetry opt-in survives the default-off coercion
        assert_eq!(result.telemetry_enabled, Some(true));
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
            dm_notifications: Some(true),
            cli_auto_update: Some(true),
            staging_channel: Some(true),
            release_channel: Some("beta".to_string()),
            meeting_detect_notify: None,
            default_recording_company_uid: None,
            telemetry_enabled: Some(true),
        };

        let json = serde_json::to_string_pretty(&prefs).unwrap();
        let parsed: MenubarPrefs = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.hq_path, prefs.hq_path);
        assert_eq!(parsed.sync_on_launch, prefs.sync_on_launch);
        assert_eq!(parsed.notifications, prefs.notifications);
        assert_eq!(parsed.start_at_login, prefs.start_at_login);
        assert_eq!(parsed.share_notifications, prefs.share_notifications);
        assert_eq!(parsed.staging_channel, Some(true));
        assert_eq!(parsed.telemetry_enabled, Some(true));
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
            staging_channel: Some(true),
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
            staging_channel: Some(true),
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
        let prefs = MenubarPrefs {
            meeting_detect_notify: Some(MeetingDetectNotifyPrefs {
                enabled: Some(false),
                platforms: Some(vec!["zoom".to_string(), "meet".to_string()]),
            }),
            ..empty_prefs()
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
    fn test_save_preserves_unmodeled_top_level_keys() {
        // REGRESSION: a plain `to_string_pretty(&prefs)` overwrite wiped every
        // top-level key the typed struct doesn't model — machineId,
        // firstRunCompleted, autoSyncNoticeShown, cliUpdateDismissedVersion,
        // and any future key. That regenerated machine identity (telemetry
        // double-counting) and could re-trigger first-run onboarding. The
        // untyped merge must carry those forward untouched.
        let existing = r#"{
            "machineId": "mid-keepme",
            "firstRunCompleted": true,
            "autoSyncNoticeShown": true,
            "cliUpdateDismissedVersion": "1.2.3",
            "someFutureKey": {"nested": 42},
            "telemetryEnabled": false
        }"#;

        // User flips telemetry ON in Settings; everything else is whatever the
        // Settings UI round-tripped back.
        let prefs = MenubarPrefs {
            telemetry_enabled: Some(true),
            ..empty_prefs()
        };

        let merged = merge_prefs_over_existing(&prefs, Some(existing)).unwrap();
        let v: serde_json::Value = serde_json::from_str(&merged).unwrap();

        // On-disk-only keys survive verbatim.
        assert_eq!(v["machineId"], "mid-keepme");
        assert_eq!(v["firstRunCompleted"], true);
        assert_eq!(v["autoSyncNoticeShown"], true);
        assert_eq!(v["cliUpdateDismissedVersion"], "1.2.3");
        assert_eq!(v["someFutureKey"]["nested"], 42);

        // The typed field the user actually changed wins over the on-disk value.
        assert_eq!(v["telemetryEnabled"], true);
    }

    #[test]
    fn test_save_with_no_existing_file_emits_typed_only() {
        // First-ever save (no menubar.json yet): output is just the typed
        // fields, no spurious keys, telemetry opt-in respected.
        let prefs = MenubarPrefs {
            telemetry_enabled: Some(true),
            ..empty_prefs()
        };
        let merged = merge_prefs_over_existing(&prefs, None).unwrap();
        let v: serde_json::Value = serde_json::from_str(&merged).unwrap();
        assert_eq!(v["telemetryEnabled"], true);
        assert!(v.get("machineId").is_none());
    }

    #[test]
    fn test_meeting_detect_notify_absent_deserializes_none() {
        // Old menubar.json files without the field must still load cleanly.
        let json = r#"{"hqPath":"/x","syncOnLaunch":false,"notifications":true,"startAtLogin":true,"autostartDaemon":false}"#;
        let prefs: MenubarPrefs = serde_json::from_str(json).unwrap();
        assert!(prefs.meeting_detect_notify.is_none());
    }
}
