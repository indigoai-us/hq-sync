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
