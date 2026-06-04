use std::fs;
use std::io::Write;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use uuid::Uuid;

use crate::util::paths;

/// HQ config.json structure. Lives at `~/.hq/config.json` and is the
/// authoritative source for company / person / bucket / vault wiring.
/// Currently written by the hq-installer onboarding wizard; this app reads
/// it via `read_hq_config_lenient` so a foreign / partial / legacy-shape
/// file never blocks sync — the menubar surfaces SetupNeeded instead.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HqConfig {
    pub company_uid: String,
    pub company_slug: String,
    pub person_uid: String,
    pub role: String,
    pub bucket_name: String,
    pub vault_api_url: String,
    pub hq_folder_path: Option<String>,
}

/// Meeting-detection notification preferences, nested under
/// `meetingDetectNotify` in `~/.hq/menubar.json`.
///
/// Absent when the feature has never been configured; defaults are applied
/// in `get_settings` (enabled=true, platforms=all five).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MeetingDetectNotifyPrefs {
    /// When false, all `meeting:detected` events are suppressed regardless
    /// of the `platforms` list.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    /// Allow-list of platform strings. A detected meeting whose `platform`
    /// is NOT in this list is silently suppressed. An empty vec means
    /// nothing is allowed; `None` means the field was absent (apply default
    /// = all platforms allowed).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub platforms: Option<Vec<String>>,
}

/// Menubar preferences stored in ~/.hq/menubar.json.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MenubarPrefs {
    pub hq_path: Option<String>,
    pub sync_on_launch: Option<bool>,
    pub notifications: Option<bool>,
    pub start_at_login: Option<bool>,
    pub autostart_daemon: Option<bool>,
    /// Auto-sync: when true, the menubar spawns hq-sync-runner in `--watch`
    /// mode at startup so local edits push immediately and remote changes
    /// pull every 10 minutes. Defaults to true when the field is missing
    /// (see `is_realtime_sync_enabled` and `get_settings`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub realtime_sync: Option<bool>,
    /// Sync personal vault: when true (default), the `--companies` fanout
    /// includes the user's personal target (every top-level entry under
    /// hq_root minus PERSONAL_VAULT_EXCLUDED_TOP_LEVEL, see hq-cloud
    /// `personal-vault.ts`). When false, the menubar passes `--no-personal`
    /// to `hq sync` so the spawned sync-runner drops the personal slot
    /// from its fanout plan — only cloud-enabled company memberships sync.
    ///
    /// Useful for devices that joined HQ for company collaboration only,
    /// privacy-by-default postures, or soak/recovery scenarios while we're
    /// cleaning up an already-leaked personal vault. Defaults to true so
    /// existing users see zero behavior change.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub personal_sync_enabled: Option<bool>,
    /// Instant sync (event-driven): when true (default), an *eligible* user
    /// (@getindigo.ai during Phase 1 rollout) gets event-driven push — the
    /// menubar appends `--event-push` to the watch runner so local edits
    /// upload within seconds of the filesystem event rather than waiting for
    /// the next 10-minute poll. When false, the runner stays poll-only.
    ///
    /// This is an ADDITIONAL opt-in layered on top of `event_push_eligible()`:
    /// ineligible users never get `--event-push` regardless of this flag.
    /// Defaults to true (matching the `realtime_sync` default-on convention)
    /// so eligible users get instant push without discovering the toggle; an
    /// explicit `false` written by `save_settings` still wins. Absent in
    /// pre-5.27 menubar.json files → treated as the default at the boundary
    /// (see `is_instant_sync_enabled` in daemon.rs and `get_settings`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instant_sync: Option<bool>,
    /// Staging repo (`owner/name`) for Core-Drift staging classification.
    /// When set, drifted locked-scope files are cross-referenced against this
    /// repo's `main` tree + open PRs and tagged (`staging main` / `PR #n` /
    /// `unaccounted`) in the Core Drift window. Any team adopting a
    /// stage-then-release HQ workflow can point this at their own staging
    /// repo. When absent, classification only runs for `@getindigo.ai` users
    /// (defaulting to `indigoai-us/hq-core-staging`); everyone else sees the
    /// unchanged drift panel. See `commands/hq_core_staging.rs`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub drift_staging_repo: Option<String>,
    /// Share notifications: when true (default), HQ Sync polls
    /// `/v1/files/shared-with-me` on launch and after each sync, fires a
    /// macOS notification per new share event, and opens a ShareDetail window
    /// on click (US-004 / US-005). Only ever active for `@getindigo.ai` users
    /// (dogfood gate in `commands/share_notify.rs`). Absent in pre-share-notify
    /// menubar.json files → treated as true (see `get_settings`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub share_notifications: Option<bool>,
    /// DM notifications: when true (default), HQ Sync polls `/v1/notify/inbox`
    /// on the same independent timer as share-notify, fires a plain
    /// fire-and-forget macOS notification per new direct message, and acks
    /// delivered DMs. RECEIVE-ONLY: there is no in-app reply or send surface —
    /// sending a DM is done by prompting HQ in a session / CLI. Read directly
    /// from menubar.json in `commands/dm_notify.rs::dm_notifications_enabled`
    /// (untyped) so the DM channel never blocks on a typed round-trip; this
    /// typed field exists so the Settings toggle round-trips cleanly through
    /// get/save_settings and isn't wiped on the next save. Absent in pre-DM
    /// menubar.json files → treated as true (see `get_settings`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dm_notifications: Option<bool>,
    /// Auto-update the globally-installed `hq` CLI: when true (default), the
    /// background update checker, on detecting a newer `@indigoai-us/hq-cli`
    /// on the npm registry, runs `npm install -g …@latest` directly instead of
    /// only nagging. The install never prompts for sudo (it fails `EACCES` on a
    /// system prefix), so a failed auto-install just falls back to the clickable
    /// banner. Read directly from menubar.json in
    /// `commands/hq_cli_update.rs::cli_auto_update_enabled` (untyped) so the
    /// checker picks up the toggle without a restart; this typed field exists so
    /// the Settings toggle round-trips cleanly through get/save_settings and
    /// isn't wiped on the next save. Absent in pre-auto-update menubar.json
    /// files → treated as true (see `get_settings`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cli_auto_update: Option<bool>,
    /// Rescue-source channel for @getindigo.ai builders. When `true`
    /// (default), the Settings toggle is ON → the popover's `CoreState`
    /// runs against `indigoai-us/hq-core-staging` (drift vs staging main,
    /// rescue spawned against staging main). When `false`, the toggle is
    /// OFF → the staging code paths short-circuit (feature dark) and the
    /// popover falls through to the prod release channel, comparing
    /// against `indigoai-us/hq-core@v{latest}` like every non-@indigo
    /// user sees.
    ///
    /// Distinct from `release_channel` below: that field controls which
    /// hq-sync release the auto-updater pulls (stable/beta/alpha);
    /// `staging_channel` controls which hq-core source tree the in-app
    /// rescue + drift classifier targets. The two are orthogonal — a
    /// @indigo user can be on the beta hq-sync channel while keeping the
    /// rescue pointed at the prod hq-core release.
    ///
    /// Setting visibility is gated by `staging_channel_setting_visible`
    /// (returns true only for `@getindigo.ai` emails). Non-@indigo users
    /// never see the toggle and always get the prod release channel; their
    /// menubar.json `stagingChannel` field, if set, is read but has no
    /// effect (the staging eligibility gate dominates).
    ///
    /// Defaults to true so existing @indigo builders see no behaviour change
    /// across upgrade — explicit `false` flips them to the prod channel.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub staging_channel: Option<bool>,
    /// Auto-updater release channel: `"stable"`, `"beta"`, or `"alpha"`.
    /// Mapped to a GitHub-tag-suffix filter by
    /// `util::release_channel::ReleaseChannel::from_pref` and gated by
    /// `util::feature_gate::is_indigo_user()` — non-`@getindigo.ai` users
    /// are coerced to `"stable"` at the resolver in `updater.rs`
    /// regardless of what's stored here, so a hand-edited menubar.json
    /// cannot escape stable.
    ///
    /// Absent in pre-channel-rollout menubar.json files → defaulted in
    /// `get_settings` to `"beta"` for indigo users (auto-opt-in to
    /// dogfood the freshest build) and `"stable"` for everyone else.
    /// See `util::release_channel::effective_channel`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub release_channel: Option<String>,
    /// Meeting detect-notify settings (US-007). Absent when not yet
    /// configured; `get_settings` supplies defaults.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meeting_detect_notify: Option<MeetingDetectNotifyPrefs>,
    /// Default company UID for SDK-local recordings (US-010). The popover
    /// active-meetings row presets its company dropdown to this value
    /// when a meeting is detected; the user can override per-recording.
    /// Same shape as the URL-invite picker in MeetingsWindow:
    ///   - `None` (absent) → "Personal" (no company attribution)
    ///   - `Some("co_…")` → that company's vault
    /// Validation that the UID matches an active membership lives in
    /// hq-pro (`/v1/recall/upload-token?companyId=…`); the client doesn't
    /// re-check so stale values just degrade to a "company-access-denied"
    /// 403 the user can resolve by picking a different option.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_recording_company_uid: Option<String>,
}

/// Read ~/.hq/menubar.json as an untyped Value map, insert a new v4 UUID under
/// "machineId" if absent or empty, and atomic-rename the file back. All other
/// top-level keys (including unknown future keys) pass through unchanged.
///
/// MenubarPrefs is NOT used here — a typed round-trip would silently drop
/// unknown keys. This mirrors the hq-installer write_menubar_telemetry_pref
/// algorithm so both sides share one canonical merge shape.
pub fn ensure_machine_id() -> Result<String, String> {
    let path: std::path::PathBuf = dirs::home_dir()
        .ok_or("home dir unavailable")?
        .join(".hq/menubar.json");

    // 1. Read existing JSON as untyped Map.
    let mut obj: Map<String, Value> = if path.exists() {
        fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str::<Value>(&s).ok())
            .and_then(|v| v.as_object().cloned())
            .unwrap_or_default()
    } else {
        Map::new()
    };

    // 2. Return existing machineId unchanged if already populated.
    if let Some(Value::String(id)) = obj.get("machineId") {
        if !id.is_empty() {
            return Ok(id.clone());
        }
    }

    // 3. Insert a new v4 UUID; do not touch other keys.
    let id = Uuid::new_v4().to_string();
    obj.insert("machineId".into(), Value::String(id.clone()));

    // 4. Atomic write.
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let tmp = path.with_extension("json.tmp");
    let body = serde_json::to_string_pretty(&Value::Object(obj))
        .map_err(|e| e.to_string())?;
    let mut f = fs::File::create(&tmp).map_err(|e| e.to_string())?;
    f.write_all(body.as_bytes()).map_err(|e| e.to_string())?;
    f.sync_all().ok();
    fs::rename(&tmp, &path).map_err(|e| e.to_string())?;
    Ok(id)
}

/// Lenient reader for `~/.hq/config.json`. Returns `Ok(None)` on any soft
/// failure (file missing, malformed JSON, missing/extra fields, legacy-shape
/// content like `{"defaultOrg":"…"}` written by hq-core's `/deploy` skill
/// before path separation landed). Only filesystem IO errors that aren't
/// `NotFound` propagate as `Err`.
///
/// Callers that only need `hq_folder_path` (the `resolve_hq_folder_path`
/// duplicates in daemon.rs / conflicts.rs / status.rs / sync.rs) can treat
/// `None` as "no path override" and fall through to menubar.json + the
/// 4-tier resolver in `util/paths.rs`. Callers that surface configured
/// state (`get_config`) translate `None` into a SetupNeeded ConfigState.
///
/// Background: see `feedback_3ab4f113-2e7c-4e4e-a171-771b47a2b5fd`.
pub fn read_hq_config_lenient() -> Result<Option<HqConfig>, String> {
    let path = paths::config_json_path()?;
    if !path.exists() {
        return Ok(None);
    }
    let contents = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("Failed to read config.json: {}", e)),
    };
    Ok(serde_json::from_str::<HqConfig>(&contents).ok())
}

/// One-shot migration of a legacy `/deploy`-skill stub at `~/.hq/config.json`.
///
/// The /deploy skill historically wrote `{"defaultOrg":"…"}` (and sometimes
/// `.deploy.preference`) into `~/.hq/config.json`. That path is also where
/// hq-sync reads its strict `HqConfig`. When both apps coexist on the same
/// file the menubar bails on every sync until the user reconstructs the
/// HqConfig by hand. /deploy has been amended to write to
/// `~/.hq/deploy-prefs.json` instead; this function brings existing victims
/// forward by:
///
///   1. No-op if the file already parses as `HqConfig` (the healthy case).
///   2. No-op if the file isn't valid JSON at all (don't mutate garbage).
///   3. Otherwise, lift any `defaultOrg` / `deploy.preference` values out
///      into `~/.hq/deploy-prefs.json`, preserving existing keys there.
///   4. Strip those keys from the original. If the result is `{}`, delete
///      the file so the next read surfaces SetupNeeded cleanly.
///   5. Personal-vault recovery: if `~/.hq/person-entity.json` is present,
///      reconstruct an `HqConfig` (personUid → companyUid, slug "personal",
///      role "owner", bucketName from cache, vaultApiUrl default,
///      hqFolderPath from menubar.json) and atomic-rename it over
///      config.json. Non-personal vaults are left to re-onboarding.
///
/// Idempotent and best-effort. All errors are logged via `util::logfile`
/// and swallowed — launch never fails on migration alone.
pub fn migrate_legacy_config_stub() {
    use crate::util::logfile::log;

    let config_path = match paths::config_json_path() {
        Ok(p) => p,
        Err(_) => return,
    };
    if !config_path.exists() {
        return;
    }
    let contents = match std::fs::read_to_string(&config_path) {
        Ok(s) => s,
        Err(_) => return,
    };
    if serde_json::from_str::<HqConfig>(&contents).is_ok() {
        return;
    }
    let mut value: Value = match serde_json::from_str(&contents) {
        Ok(v) => v,
        Err(_) => return,
    };
    let obj = match value.as_object_mut() {
        Some(o) => o,
        None => return,
    };

    let lifted_default_org = obj
        .remove("defaultOrg")
        .and_then(|v| v.as_str().map(str::to_string));
    let lifted_pref = obj
        .get_mut("deploy")
        .and_then(|d| d.as_object_mut())
        .and_then(|d| d.remove("preference"))
        .and_then(|v| v.as_str().map(str::to_string));
    if let Some(d) = obj.get("deploy").and_then(|v| v.as_object()) {
        if d.is_empty() {
            obj.remove("deploy");
        }
    }

    let lifted_anything = lifted_default_org.is_some() || lifted_pref.is_some();

    // GATE: do nothing destructive unless we positively identified the file
    // as a legacy /deploy stub (i.e. we actually lifted `defaultOrg` or
    // `deploy.preference`). Without this guard, a partially-corrupted but
    // unrelated config (e.g. a company HqConfig missing one new field after
    // a schema bump) would be silently overwritten with a personal-vault
    // config, which is data loss. If nothing was lifted, leave the file
    // alone — the lenient reader/get_config will surface SetupNeeded and
    // the user can repair via hq-installer.
    if !lifted_anything {
        return;
    }

    if let Err(e) =
        write_deploy_prefs(lifted_default_org.as_deref(), lifted_pref.as_deref())
    {
        log("config-migration", &format!("write deploy-prefs failed: {e}"));
        // Don't strip from config.json if we couldn't persist forward —
        // leave the file as-is so /deploy's own backwards-compat read
        // still finds the slug.
        return;
    }
    log(
        "config-migration",
        &format!(
            "lifted deploy keys from ~/.hq/config.json (defaultOrg={}, preference={})",
            lifted_default_org.is_some(),
            lifted_pref.is_some()
        ),
    );

    // Reconstruction only fires for files we've already identified as
    // legacy stubs (see GATE above). For personal vaults, this puts the
    // user back on a working sync without re-onboarding.
    if let Some(reconstructed) = reconstruct_personal_hq_config() {
        match write_hq_config(&reconstructed) {
            Ok(()) => {
                log(
                    "config-migration",
                    "reconstructed personal-vault HqConfig from person-entity.json",
                );
                return;
            }
            Err(e) => log(
                "config-migration",
                &format!("personal HqConfig reconstruction write failed: {e}"),
            ),
        }
    }

    // No reconstruction available. If the stripped object is empty, delete
    // the file so the next read surfaces SetupNeeded cleanly. Otherwise
    // atomically write the stripped version back (still safe to modify
    // since we positively identified the file as a legacy stub above).
    if obj.is_empty() {
        let _ = std::fs::remove_file(&config_path);
        log(
            "config-migration",
            "removed empty stub at ~/.hq/config.json",
        );
        return;
    }
    if let Err(e) = atomic_write_json(&config_path, &Value::Object(obj.clone())) {
        log("config-migration", &format!("strip-rewrite failed: {e}"));
    }
}

/// Atomically merge `defaultOrg` and `deploy.preference` into
/// `~/.hq/deploy-prefs.json`, preserving any keys already present.
fn write_deploy_prefs(default_org: Option<&str>, preference: Option<&str>) -> Result<(), String> {
    let path = paths::deploy_prefs_json_path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let mut existing: Map<String, Value> = if path.exists() {
        std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str::<Value>(&s).ok())
            .and_then(|v| v.as_object().cloned())
            .unwrap_or_default()
    } else {
        Map::new()
    };
    if let Some(slug) = default_org {
        if !slug.is_empty() {
            existing.insert("defaultOrg".into(), Value::String(slug.to_string()));
        }
    }
    if let Some(pref) = preference {
        if !pref.is_empty() {
            let deploy = existing
                .entry("deploy".to_string())
                .or_insert_with(|| Value::Object(Map::new()));
            if let Some(d) = deploy.as_object_mut() {
                d.insert("preference".into(), Value::String(pref.to_string()));
            }
        }
    }
    atomic_write_json(&path, &Value::Object(existing))
}

/// Attempt to reconstruct a personal-vault `HqConfig` from
/// `~/.hq/person-entity.json` + `~/.hq/menubar.json`. Returns `None` if the
/// person-entity cache isn't present or doesn't deserialize.
fn reconstruct_personal_hq_config() -> Option<HqConfig> {
    let home = dirs::home_dir()?;
    let entity_path = home.join(".hq").join("person-entity.json");
    let entity_contents = std::fs::read_to_string(&entity_path).ok()?;
    let entity: serde_json::Value = serde_json::from_str(&entity_contents).ok()?;
    let person_uid = entity.get("personUid")?.as_str()?.to_string();
    let bucket_name = entity.get("bucketName")?.as_str()?.to_string();
    if person_uid.is_empty() || bucket_name.is_empty() {
        return None;
    }

    let menubar_path = home.join(".hq").join("menubar.json");
    let hq_folder_path = std::fs::read_to_string(&menubar_path)
        .ok()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .and_then(|v| {
            v.get("hqPath")
                .and_then(|p| p.as_str().map(str::to_string))
        });

    Some(HqConfig {
        company_uid: person_uid.clone(),
        company_slug: "personal".to_string(),
        person_uid,
        role: "owner".to_string(),
        bucket_name,
        vault_api_url: "https://hqapi.getindigo.ai".to_string(),
        hq_folder_path,
    })
}

fn write_hq_config(cfg: &HqConfig) -> Result<(), String> {
    let path = paths::config_json_path()?;
    let value = serde_json::to_value(cfg).map_err(|e| e.to_string())?;
    atomic_write_json(&path, &value)
}

fn atomic_write_json(path: &std::path::Path, value: &Value) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let tmp = path.with_extension("json.tmp");
    let body = serde_json::to_string_pretty(value).map_err(|e| e.to_string())?;
    let mut f = std::fs::File::create(&tmp).map_err(|e| e.to_string())?;
    f.write_all(body.as_bytes()).map_err(|e| e.to_string())?;
    f.sync_all().ok();
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())
}

/// Response returned to the frontend from get_config.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigState {
    pub configured: bool,
    pub company_slug: Option<String>,
    pub company_uid: Option<String>,
    pub person_uid: Option<String>,
    pub role: Option<String>,
    pub bucket_name: Option<String>,
    pub vault_api_url: Option<String>,
    pub hq_folder_path: String,
    pub error: Option<String>,
}

/// Read ~/.hq/config.json and ~/.hq/menubar.json, resolve HQ folder path,
/// and return a ConfigState for the frontend.
///
/// If config.json is missing, returns configured=false with an error message
/// directing the user to install hq-installer first.
#[tauri::command]
pub async fn get_config() -> Result<ConfigState, String> {
    let config_path = paths::config_json_path()?;
    let menubar_path = paths::menubar_json_path()?;

    // Read menubar.json (optional — may not exist)
    let menubar_prefs: Option<MenubarPrefs> = if menubar_path.exists() {
        let contents = std::fs::read_to_string(&menubar_path)
            .map_err(|e| format!("Failed to read menubar.json: {}", e))?;
        serde_json::from_str(&contents)
            .map_err(|e| format!("Failed to parse menubar.json: {}", e))
            .ok()
    } else {
        None
    };

    // Read config.json (required for configured state)
    if !config_path.exists() {
        let hq_folder = paths::resolve_hq_folder(
            None,
            menubar_prefs.as_ref().and_then(|p| p.hq_path.as_deref()),
        );
        return Ok(ConfigState {
            configured: false,
            company_slug: None,
            company_uid: None,
            person_uid: None,
            role: None,
            bucket_name: None,
            vault_api_url: None,
            hq_folder_path: hq_folder.to_string_lossy().to_string(),
            error: Some(
                "HQ is not configured. Please run hq-installer to complete setup. \
                 Download at https://github.com/indigoai-us/hq-installer/releases"
                    .to_string(),
            ),
        });
    }

    // Lenient parse: a legacy `{"defaultOrg":"…"}` stub (or any
    // non-HqConfig JSON) surfaces as `configured=false` rather than a
    // Rust Err, so the frontend can route the user to SetupNeeded
    // instead of seeing an opaque parse error.
    let config = match read_hq_config_lenient()? {
        Some(c) => c,
        None => {
            let hq_folder = paths::resolve_hq_folder(
                None,
                menubar_prefs.as_ref().and_then(|p| p.hq_path.as_deref()),
            );
            return Ok(ConfigState {
                configured: false,
                company_slug: None,
                company_uid: None,
                person_uid: None,
                role: None,
                bucket_name: None,
                vault_api_url: None,
                hq_folder_path: hq_folder.to_string_lossy().to_string(),
                error: Some(
                    "~/.hq/config.json is present but doesn't match HqConfig. \
                     Re-run hq-installer to repair, or restart HQ Sync — the \
                     launch-time migration recovers personal-vault installs \
                     automatically when ~/.hq/person-entity.json is present."
                        .to_string(),
                ),
            });
        }
    };

    let hq_folder = paths::resolve_hq_folder(
        config.hq_folder_path.as_deref(),
        menubar_prefs.as_ref().and_then(|p| p.hq_path.as_deref()),
    );

    Ok(ConfigState {
        configured: true,
        company_slug: Some(config.company_slug),
        company_uid: Some(config.company_uid),
        person_uid: Some(config.person_uid),
        role: Some(config.role),
        bucket_name: Some(config.bucket_name),
        vault_api_url: Some(config.vault_api_url),
        hq_folder_path: hq_folder.to_string_lossy().to_string(),
        error: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hq_config_deserialize() {
        let json = r#"{
            "companyUid": "abc-123",
            "companySlug": "acme",
            "personUid": "person-456",
            "role": "admin",
            "bucketName": "acme-bucket",
            "vaultApiUrl": "https://vault.example.com",
            "hqFolderPath": "/Users/test/HQ"
        }"#;
        let config: HqConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.company_uid, "abc-123");
        assert_eq!(config.company_slug, "acme");
        assert_eq!(config.person_uid, "person-456");
        assert_eq!(config.role, "admin");
        assert_eq!(config.bucket_name, "acme-bucket");
        assert_eq!(config.vault_api_url, "https://vault.example.com");
        assert_eq!(config.hq_folder_path, Some("/Users/test/HQ".to_string()));
    }

    #[test]
    fn test_hq_config_deserialize_without_hq_folder_path() {
        let json = r#"{
            "companyUid": "abc-123",
            "companySlug": "acme",
            "personUid": "person-456",
            "role": "admin",
            "bucketName": "acme-bucket",
            "vaultApiUrl": "https://vault.example.com"
        }"#;
        let config: HqConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.hq_folder_path, None);
    }

    #[test]
    fn test_menubar_prefs_deserialize() {
        let json = r#"{
            "hqPath": "/custom/HQ",
            "syncOnLaunch": true,
            "notifications": false,
            "startAtLogin": true,
            "autostartDaemon": false
        }"#;
        let prefs: MenubarPrefs = serde_json::from_str(json).unwrap();
        assert_eq!(prefs.hq_path, Some("/custom/HQ".to_string()));
        assert_eq!(prefs.sync_on_launch, Some(true));
        assert_eq!(prefs.notifications, Some(false));
        assert_eq!(prefs.start_at_login, Some(true));
        assert_eq!(prefs.autostart_daemon, Some(false));
    }

    #[test]
    fn test_menubar_prefs_deserialize_empty() {
        let json = r#"{}"#;
        let prefs: MenubarPrefs = serde_json::from_str(json).unwrap();
        assert_eq!(prefs.hq_path, None);
        assert_eq!(prefs.sync_on_launch, None);
        assert_eq!(prefs.autostart_daemon, None);
    }

    #[test]
    fn test_config_state_serialize() {
        let state = ConfigState {
            configured: true,
            company_slug: Some("acme".to_string()),
            company_uid: Some("uid-123".to_string()),
            person_uid: Some("person-456".to_string()),
            role: Some("admin".to_string()),
            bucket_name: Some("bucket".to_string()),
            vault_api_url: Some("https://vault.example.com".to_string()),
            hq_folder_path: "/Users/test/HQ".to_string(),
            error: None,
        };
        let json = serde_json::to_string(&state).unwrap();
        assert!(json.contains("\"configured\":true"));
        assert!(json.contains("\"companySlug\":\"acme\""));
        assert!(json.contains("\"hqFolderPath\":\"/Users/test/HQ\""));
        assert!(json.contains("\"error\":null"));
    }

    #[test]
    fn test_menubar_prefs_realtime_sync_field_round_trip_true() {
        // MenubarPrefs has a `realtime_sync: Option<bool>` field that
        // serializes to the camelCase key `realtimeSync` so the Auto-sync
        // setting persists across app restarts alongside `autostart_daemon`.
        let json = r#"{
            "hqPath": "/custom/HQ",
            "syncOnLaunch": false,
            "notifications": true,
            "startAtLogin": true,
            "autostartDaemon": false,
            "realtimeSync": true
        }"#;
        let prefs: MenubarPrefs = serde_json::from_str(json).unwrap();
        assert_eq!(prefs.realtime_sync, Some(true));

        let out = serde_json::to_string(&prefs).unwrap();
        assert!(
            out.contains("\"realtimeSync\":true"),
            "expected camelCase key 'realtimeSync' in serialized output, got: {out}"
        );
        assert!(!out.contains("realtime_sync"));
    }

    #[test]
    fn test_menubar_prefs_realtime_sync_absent_deserializes_none() {
        // Backwards compatibility: existing menubar.json files predate the
        // field and must continue to load. None is the absent-marker; the
        // settings command applies a `false` default at the boundary.
        let json = r#"{
            "hqPath": "/custom/HQ",
            "syncOnLaunch": true,
            "notifications": false,
            "startAtLogin": true,
            "autostartDaemon": false
        }"#;
        let prefs: MenubarPrefs = serde_json::from_str(json).unwrap();
        assert_eq!(prefs.realtime_sync, None);
    }

    #[test]
    fn test_menubar_prefs_realtime_sync_false_round_trip() {
        let json = r#"{"realtimeSync": false}"#;
        let prefs: MenubarPrefs = serde_json::from_str(json).unwrap();
        assert_eq!(prefs.realtime_sync, Some(false));
        let out = serde_json::to_string(&prefs).unwrap();
        assert!(out.contains("\"realtimeSync\":false"));
    }

    #[test]
    fn test_menubar_prefs_instant_sync_round_trip_true() {
        // `instant_sync` serializes to camelCase `instantSync` so the
        // Instant-sync (event-driven) setting persists across restarts.
        let json = r#"{"instantSync": true}"#;
        let prefs: MenubarPrefs = serde_json::from_str(json).unwrap();
        assert_eq!(prefs.instant_sync, Some(true));
        let out = serde_json::to_string(&prefs).unwrap();
        assert!(
            out.contains("\"instantSync\":true"),
            "expected camelCase key 'instantSync' in serialized output, got: {out}"
        );
        assert!(!out.contains("instant_sync"));
    }

    #[test]
    fn test_menubar_prefs_instant_sync_absent_deserializes_none() {
        // Backwards compatibility: pre-5.27 menubar.json predates the field
        // and must continue to load. None is the absent-marker; the daemon /
        // settings boundary applies the default-on at read time.
        let json = r#"{"hqPath": "/custom/HQ"}"#;
        let prefs: MenubarPrefs = serde_json::from_str(json).unwrap();
        assert_eq!(prefs.instant_sync, None);
    }

    #[test]
    fn test_menubar_prefs_instant_sync_false_round_trip() {
        let json = r#"{"instantSync": false}"#;
        let prefs: MenubarPrefs = serde_json::from_str(json).unwrap();
        assert_eq!(prefs.instant_sync, Some(false));
        let out = serde_json::to_string(&prefs).unwrap();
        assert!(out.contains("\"instantSync\":false"));
    }

    #[test]
    fn test_config_state_unconfigured() {
        let state = ConfigState {
            configured: false,
            company_slug: None,
            company_uid: None,
            person_uid: None,
            role: None,
            bucket_name: None,
            vault_api_url: None,
            hq_folder_path: "/Users/test/HQ".to_string(),
            error: Some("Not configured".to_string()),
        };
        let json = serde_json::to_string(&state).unwrap();
        assert!(json.contains("\"configured\":false"));
        assert!(json.contains("\"error\":\"Not configured\""));
    }
}

#[cfg(test)]
mod ensure_machine_id_tests {
    use super::*;
    use crate::util::test_support::ENV_MUTEX;
    use serde_json::{json, Value};
    use std::fs;
    use tempfile::TempDir;

    fn fixture() -> TempDir {
        let tmp = TempDir::new().unwrap();
        fs::create_dir_all(tmp.path().join(".hq")).unwrap();
        tmp
    }

    fn read_menubar_value(home: &std::path::Path) -> Value {
        let body = fs::read_to_string(home.join(".hq/menubar.json")).unwrap();
        serde_json::from_str(&body).unwrap()
    }

    // (a) Missing file — created with valid v4 UUID.
    #[test]
    fn ensure_machine_id_creates_file_when_missing() {
        let _g = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = TempDir::new().unwrap();
        std::env::set_var("HOME", tmp.path());
        let id = ensure_machine_id().unwrap();
        assert!(uuid::Uuid::parse_str(&id).is_ok());
        let v = read_menubar_value(tmp.path());
        assert_eq!(v["machineId"], Value::String(id));
    }

    // (b) File without `machineId` — field added, UUID is valid v4.
    #[test]
    fn ensure_machine_id_adds_field_when_missing() {
        let _g = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = fixture();
        std::env::set_var("HOME", tmp.path());
        fs::write(
            tmp.path().join(".hq/menubar.json"),
            r#"{"hqPath":"/foo"}"#,
        )
        .unwrap();
        let id = ensure_machine_id().unwrap();
        assert!(uuid::Uuid::parse_str(&id).is_ok());
        let v = read_menubar_value(tmp.path());
        assert_eq!(v["machineId"], Value::String(id));
        assert_eq!(v["hqPath"], Value::String("/foo".into()));
    }

    // (c) Existing `machineId` — unchanged.
    #[test]
    fn ensure_machine_id_returns_existing() {
        let _g = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = fixture();
        std::env::set_var("HOME", tmp.path());
        let pre = "00000000-0000-4000-8000-000000000000";
        fs::write(
            tmp.path().join(".hq/menubar.json"),
            format!(r#"{{"machineId":"{pre}","hqPath":"/foo"}}"#),
        )
        .unwrap();
        let id = ensure_machine_id().unwrap();
        assert_eq!(id, pre);
        let v = read_menubar_value(tmp.path());
        assert_eq!(v["machineId"], Value::String(pre.into()));
    }

    // (d) Atomic write — verify temp-file-rename pattern.
    #[test]
    fn ensure_machine_id_writes_atomically() {
        let _g = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = fixture();
        std::env::set_var("HOME", tmp.path());
        ensure_machine_id().unwrap();
        assert!(!tmp.path().join(".hq/menubar.json.tmp").exists());
        assert!(tmp.path().join(".hq/menubar.json").exists());
    }

    // (e) All-keys-preserved via untyped merge.
    #[test]
    fn ensure_machine_id_preserves_all_pre_existing_keys() {
        let _g = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = fixture();
        std::env::set_var("HOME", tmp.path());
        let seed = json!({
            "hqPath": "/custom",
            "syncOnLaunch": true,
            "notifications": false,
            "startAtLogin": true,
            "autostartDaemon": null,
            "telemetryEnabled": true,
            "some_unknown_future_key": "x",
        });
        fs::write(
            tmp.path().join(".hq/menubar.json"),
            serde_json::to_string(&seed).unwrap(),
        )
        .unwrap();
        ensure_machine_id().unwrap();
        let v = read_menubar_value(tmp.path());
        assert_eq!(v["hqPath"], Value::String("/custom".into()));
        assert_eq!(v["syncOnLaunch"], Value::Bool(true));
        assert_eq!(v["notifications"], Value::Bool(false));
        assert_eq!(v["startAtLogin"], Value::Bool(true));
        assert_eq!(v["autostartDaemon"], Value::Null);
        assert_eq!(v["telemetryEnabled"], Value::Bool(true));
        assert_eq!(v["some_unknown_future_key"], Value::String("x".into()));
        assert!(v["machineId"].is_string());
        assert!(uuid::Uuid::parse_str(v["machineId"].as_str().unwrap()).is_ok());
    }
}

#[cfg(test)]
mod lenient_reader_and_migration_tests {
    use super::*;
    use crate::util::test_support::ENV_MUTEX;
    use serde_json::json;
    use std::fs;
    use tempfile::TempDir;

    fn fresh_home() -> TempDir {
        let tmp = TempDir::new().unwrap();
        fs::create_dir_all(tmp.path().join(".hq")).unwrap();
        tmp
    }

    fn write(path: std::path::PathBuf, contents: &str) {
        fs::write(path, contents).unwrap();
    }

    fn valid_hq_config_json() -> String {
        serde_json::to_string(&json!({
            "companyUid": "uid-1",
            "companySlug": "indigo",
            "personUid": "prs-1",
            "role": "owner",
            "bucketName": "bkt",
            "vaultApiUrl": "https://example.invalid",
            "hqFolderPath": "/tmp/HQ"
        })).unwrap()
    }

    // (a) lenient reader: file missing → Ok(None), never Err.
    #[test]
    fn lenient_returns_none_when_missing() {
        let _g = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = fresh_home();
        std::env::set_var("HOME", tmp.path());
        let got = read_hq_config_lenient().unwrap();
        assert!(got.is_none());
    }

    // (b) lenient reader: legacy `{"defaultOrg":"…"}` stub → Ok(None) (was previously Err).
    #[test]
    fn lenient_returns_none_on_legacy_stub() {
        let _g = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = fresh_home();
        std::env::set_var("HOME", tmp.path());
        write(
            tmp.path().join(".hq/config.json"),
            r#"{"defaultOrg":"personal"}"#,
        );
        let got = read_hq_config_lenient().unwrap();
        assert!(got.is_none(), "legacy stub must surface as None, not Err");
    }

    // (c) lenient reader: malformed JSON → Ok(None) (no panic, no Err).
    #[test]
    fn lenient_returns_none_on_garbage() {
        let _g = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = fresh_home();
        std::env::set_var("HOME", tmp.path());
        write(
            tmp.path().join(".hq/config.json"),
            "not even json {{{",
        );
        let got = read_hq_config_lenient().unwrap();
        assert!(got.is_none());
    }

    // (d) lenient reader: valid HqConfig → Ok(Some(cfg)).
    #[test]
    fn lenient_returns_some_on_valid_hq_config() {
        let _g = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = fresh_home();
        std::env::set_var("HOME", tmp.path());
        write(tmp.path().join(".hq/config.json"), &valid_hq_config_json());
        let got = read_hq_config_lenient().unwrap().unwrap();
        assert_eq!(got.company_slug, "indigo");
        assert_eq!(got.role, "owner");
    }

    // (e) migration: legacy stub → defaultOrg lifted to deploy-prefs.json,
    // stub config.json removed (so SetupNeeded surfaces on next read).
    #[test]
    fn migration_lifts_default_org_and_deletes_empty_stub() {
        let _g = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = fresh_home();
        std::env::set_var("HOME", tmp.path());
        write(
            tmp.path().join(".hq/config.json"),
            r#"{"defaultOrg":"personal"}"#,
        );

        migrate_legacy_config_stub();

        let prefs: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(tmp.path().join(".hq/deploy-prefs.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(prefs["defaultOrg"], "personal");
        assert!(
            !tmp.path().join(".hq/config.json").exists(),
            "empty stub should be removed so SetupNeeded surfaces cleanly"
        );
    }

    // (f) migration: legacy stub + person-entity → reconstruct HqConfig
    // for the personal vault. Config.json now parses as HqConfig.
    #[test]
    fn migration_reconstructs_personal_hq_config_when_entity_present() {
        let _g = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = fresh_home();
        std::env::set_var("HOME", tmp.path());
        write(
            tmp.path().join(".hq/config.json"),
            r#"{"defaultOrg":"personal"}"#,
        );
        write(
            tmp.path().join(".hq/person-entity.json"),
            r#"{"personUid":"prs_x","bucketName":"hq-vault-prs-x","createdAt":"2026-01-01T00:00:00Z"}"#,
        );
        write(
            tmp.path().join(".hq/menubar.json"),
            r#"{"hqPath":"/Users/me/HQ"}"#,
        );

        migrate_legacy_config_stub();

        let cfg = read_hq_config_lenient().unwrap().unwrap();
        assert_eq!(cfg.company_uid, "prs_x");
        assert_eq!(cfg.company_slug, "personal");
        assert_eq!(cfg.person_uid, "prs_x");
        assert_eq!(cfg.bucket_name, "hq-vault-prs-x");
        assert_eq!(cfg.role, "owner");
        assert_eq!(cfg.hq_folder_path.as_deref(), Some("/Users/me/HQ"));

        // defaultOrg also lifted forward so /deploy keeps its persisted choice.
        let prefs: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(tmp.path().join(".hq/deploy-prefs.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(prefs["defaultOrg"], "personal");
    }

    // (g) migration: valid HqConfig → no-op. File unchanged, no
    // deploy-prefs.json created.
    #[test]
    fn migration_noop_on_valid_hq_config() {
        let _g = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = fresh_home();
        std::env::set_var("HOME", tmp.path());
        let original = valid_hq_config_json();
        write(tmp.path().join(".hq/config.json"), &original);

        migrate_legacy_config_stub();

        let after = fs::read_to_string(tmp.path().join(".hq/config.json")).unwrap();
        assert_eq!(after, original);
        assert!(!tmp.path().join(".hq/deploy-prefs.json").exists());
    }

    // (h) migration: deploy.preference lift preserved.
    #[test]
    fn migration_lifts_deploy_preference() {
        let _g = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = fresh_home();
        std::env::set_var("HOME", tmp.path());
        write(
            tmp.path().join(".hq/config.json"),
            r#"{"deploy":{"preference":"vercel"}}"#,
        );

        migrate_legacy_config_stub();

        let prefs: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(tmp.path().join(".hq/deploy-prefs.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(prefs["deploy"]["preference"], "vercel");
        assert!(!tmp.path().join(".hq/config.json").exists());
    }

    // (j) migration: foreign content without deploy keys is NOT overwritten,
    // even when person-entity.json is present. Prevents data loss when a
    // partially-corrupted but unrelated config file ends up at ~/.hq/config.json.
    // This is the Codex P2 #2 case.
    #[test]
    fn migration_does_not_overwrite_unrelated_foreign_content() {
        let _g = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = fresh_home();
        std::env::set_var("HOME", tmp.path());
        // Looks like a partial company-config — valid JSON, parses as object,
        // but doesn't deserialize as HqConfig (missing several required fields)
        // and has none of the legacy /deploy keys.
        let foreign = r#"{"companyUid":"co-1","companySlug":"acme"}"#;
        write(tmp.path().join(".hq/config.json"), foreign);
        // person-entity.json present — under the old code this would trigger
        // a personal-vault reconstruction that overwrites the foreign content.
        write(
            tmp.path().join(".hq/person-entity.json"),
            r#"{"personUid":"prs_x","bucketName":"hq-vault-prs-x","createdAt":"2026-01-01T00:00:00Z"}"#,
        );

        migrate_legacy_config_stub();

        let after = fs::read_to_string(tmp.path().join(".hq/config.json")).unwrap();
        assert_eq!(after, foreign, "foreign config.json must NOT be overwritten when no deploy keys are present");
        assert!(
            !tmp.path().join(".hq/deploy-prefs.json").exists(),
            "deploy-prefs.json must NOT be created from a non-stub file"
        );
    }

    // (i) migration: idempotent — running twice is a no-op after the first pass.
    #[test]
    fn migration_is_idempotent() {
        let _g = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = fresh_home();
        std::env::set_var("HOME", tmp.path());
        write(
            tmp.path().join(".hq/config.json"),
            r#"{"defaultOrg":"personal"}"#,
        );
        write(
            tmp.path().join(".hq/person-entity.json"),
            r#"{"personUid":"prs_x","bucketName":"hq-vault-prs-x","createdAt":"2026-01-01T00:00:00Z"}"#,
        );

        migrate_legacy_config_stub();
        let first = fs::read_to_string(tmp.path().join(".hq/config.json")).unwrap();
        migrate_legacy_config_stub();
        let second = fs::read_to_string(tmp.path().join(".hq/config.json")).unwrap();
        assert_eq!(first, second, "second migrate must be a no-op");
    }
}
