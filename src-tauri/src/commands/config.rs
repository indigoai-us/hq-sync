use std::fs;
use std::io::Write;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use uuid::Uuid;

use crate::util::paths;

/// HQ config.json structure (written by hq-installer post-onboarding).
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

    let contents = std::fs::read_to_string(&config_path)
        .map_err(|e| format!("Failed to read config.json: {}", e))?;
    let config: HqConfig = serde_json::from_str(&contents)
        .map_err(|e| format!("Failed to parse config.json: {}", e))?;

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
