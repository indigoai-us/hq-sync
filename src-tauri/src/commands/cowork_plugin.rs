//! One-click installer for the HQ Cowork plugin.
//!
//! The real plugin build/install contract lives in hq-core's
//! `core/packages/hq-pack-cowork/scripts/install-cowork-plugin.sh`. HQ Sync
//! resolves the user's HQ root, finds that pack, runs the same helper a
//! terminal or `/hq-cowork-install` would run, then mirrors Cowork's upload
//! behavior by unpacking the `.plugin` into Claude Desktop's local RPM plugin
//! store and updating its manifest.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Serialize;
use serde_json::{json, Value};

use crate::commands::config::{read_hq_config_lenient, MenubarPrefs};
use crate::util::paths;

const COWORK_PLUGIN_NAME: &str = "hq-cowork";
const COWORK_UPLOAD_MARKETPLACE_ID: &str = "marketplace_01FZmfsWSt7TtQyKprEiiC6j";
const COWORK_UPLOAD_MARKETPLACE_NAME: &str = "My Uploads";

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CoworkPluginInstallResult {
    pub artifact_path: String,
    pub cowork_install_paths: Vec<String>,
    pub log_tail: String,
}

fn read_menubar_prefs() -> Option<MenubarPrefs> {
    let path = paths::menubar_json_path().ok()?;
    let contents = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&contents).ok()
}

fn resolve_hq_folder_path(explicit: Option<&str>) -> PathBuf {
    if let Some(path) = explicit {
        if !path.trim().is_empty() {
            return PathBuf::from(path);
        }
    }

    let menubar = read_menubar_prefs();
    let config = read_hq_config_lenient().ok().flatten();
    paths::resolve_hq_folder(
        config.as_ref().and_then(|c| c.hq_folder_path.as_deref()),
        menubar.as_ref().and_then(|p| p.hq_path.as_deref()),
    )
}

pub(crate) fn pack_dir_candidates(hq_root: &Path) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Ok(path) = std::env::var("HQ_COWORK_PACK_ROOT") {
        if !path.trim().is_empty() {
            candidates.push(PathBuf::from(path));
        }
    }
    candidates.push(hq_root.join("core/packages/hq-pack-cowork"));
    candidates.push(hq_root.join("repos/private/hq-core-staging/core/packages/hq-pack-cowork"));
    candidates
}

fn find_pack_dir(hq_root: &Path) -> Result<PathBuf, String> {
    pack_dir_candidates(hq_root)
        .into_iter()
        .find(|p| p.join("scripts/install-cowork-plugin.sh").is_file())
        .ok_or_else(|| {
            format!(
                "HQ Cowork pack not found under {}. Update HQ, then try again.",
                hq_root.display()
            )
        })
}

pub(crate) fn install_args(artifact_path: &Path) -> Vec<String> {
    vec![
        "--install".to_string(),
        "--out".to_string(),
        artifact_path.to_string_lossy().to_string(),
    ]
}

fn claude_local_agent_sessions_root() -> Result<PathBuf, String> {
    let data_dir =
        dirs::data_dir().ok_or_else(|| "Could not resolve app data directory.".to_string())?;
    Ok(data_dir.join("Claude/local-agent-mode-sessions"))
}

pub(crate) fn discover_cowork_rpm_dirs_under(root: &Path) -> Vec<PathBuf> {
    let Ok(accounts) = fs::read_dir(root) else {
        return Vec::new();
    };

    let mut dirs = Vec::new();
    for account in accounts.flatten() {
        let Ok(account_type) = account.file_type() else {
            continue;
        };
        if !account_type.is_dir() {
            continue;
        }
        let Ok(workspaces) = fs::read_dir(account.path()) else {
            continue;
        };
        for workspace in workspaces.flatten() {
            let Ok(workspace_type) = workspace.file_type() else {
                continue;
            };
            if !workspace_type.is_dir() {
                continue;
            }
            let rpm_dir = workspace.path().join("rpm");
            if rpm_dir.join("manifest.json").is_file() {
                dirs.push(rpm_dir);
            }
        }
    }
    dirs.sort();
    dirs
}

fn is_safe_plugin_id(id: &str) -> bool {
    id.starts_with("plugin_")
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

fn existing_upload_marketplace_id(plugins: &[Value]) -> String {
    plugins
        .iter()
        .find_map(|plugin| {
            let marketplace_name = plugin.get("marketplaceName")?.as_str()?;
            if marketplace_name == COWORK_UPLOAD_MARKETPLACE_NAME {
                plugin.get("marketplaceId")?.as_str().map(str::to_string)
            } else {
                None
            }
        })
        .unwrap_or_else(|| COWORK_UPLOAD_MARKETPLACE_ID.to_string())
}

fn manifest_plugin_id(manifest: &Value, plugin_name: &str) -> Option<String> {
    manifest
        .get("plugins")?
        .as_array()?
        .iter()
        .find_map(|plugin| {
            let name = plugin.get("name")?.as_str()?;
            let id = plugin.get("id")?.as_str()?;
            if name == plugin_name && is_safe_plugin_id(id) {
                Some(id.to_string())
            } else {
                None
            }
        })
}

fn generated_plugin_id() -> String {
    format!("plugin_{}", ulid::Ulid::new())
}

pub(crate) fn upsert_rpm_manifest(
    manifest: &mut Value,
    plugin_name: &str,
    plugin_id: &str,
    updated_at_millis: i64,
    updated_at_iso: &str,
) {
    if !manifest.is_object() {
        *manifest = json!({});
    }
    manifest["lastUpdated"] = json!(updated_at_millis);

    if !manifest.get("plugins").is_some_and(Value::is_array) {
        manifest["plugins"] = json!([]);
    }

    let plugins = manifest
        .get_mut("plugins")
        .and_then(Value::as_array_mut)
        .expect("plugins array was initialized");
    let marketplace_id = existing_upload_marketplace_id(plugins);
    let replacement = json!({
        "id": plugin_id,
        "name": plugin_name,
        "updatedAt": updated_at_iso,
        "marketplaceId": marketplace_id,
        "marketplaceName": COWORK_UPLOAD_MARKETPLACE_NAME,
        "installedBy": "user"
    });

    if let Some(existing) = plugins
        .iter_mut()
        .find(|plugin| plugin.get("name").and_then(Value::as_str) == Some(plugin_name))
    {
        *existing = replacement;
    } else {
        plugins.push(replacement);
    }
}

fn write_json_atomic(path: &Path, value: &Value) -> Result<(), String> {
    let body =
        serde_json::to_string_pretty(value).map_err(|e| format!("serialize manifest: {e}"))?;
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, body).map_err(|e| format!("write {}: {e}", tmp.display()))?;
    fs::rename(&tmp, path)
        .map_err(|e| format!("rename {} to {}: {e}", tmp.display(), path.display()))
}

fn unpack_plugin_artifact(artifact_path: &Path, destination: &Path) -> Result<(), String> {
    let temp_destination = destination.with_file_name(format!(
        ".tmp-{}-{}",
        destination
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or(COWORK_PLUGIN_NAME),
        ulid::Ulid::new()
    ));
    fs::create_dir_all(&temp_destination)
        .map_err(|e| format!("create {}: {e}", temp_destination.display()))?;

    let output = Command::new("/usr/bin/unzip")
        .arg("-q")
        .arg(artifact_path)
        .arg("-d")
        .arg(&temp_destination)
        .output()
        .map_err(|e| format!("spawn unzip: {e}"))?;

    if !output.status.success() {
        let _ = fs::remove_dir_all(&temp_destination);
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("unzip plugin artifact: {}", stderr.trim()));
    }

    if destination.exists() {
        fs::remove_dir_all(destination)
            .map_err(|e| format!("remove existing {}: {e}", destination.display()))?;
    }
    fs::rename(&temp_destination, destination).map_err(|e| {
        let _ = fs::remove_dir_all(&temp_destination);
        format!(
            "rename {} to {}: {e}",
            temp_destination.display(),
            destination.display()
        )
    })
}

fn import_plugin_into_rpm_dir(
    artifact_path: &Path,
    rpm_dir: &Path,
    plugin_name: &str,
    updated_at_millis: i64,
    updated_at_iso: &str,
) -> Result<PathBuf, String> {
    let manifest_path = rpm_dir.join("manifest.json");
    let mut manifest: Value = fs::read_to_string(&manifest_path)
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_else(|| json!({ "plugins": [] }));

    let plugin_id = manifest_plugin_id(&manifest, plugin_name).unwrap_or_else(generated_plugin_id);
    let plugin_dir = rpm_dir.join(&plugin_id);

    unpack_plugin_artifact(artifact_path, &plugin_dir)?;
    upsert_rpm_manifest(
        &mut manifest,
        plugin_name,
        &plugin_id,
        updated_at_millis,
        updated_at_iso,
    );
    write_json_atomic(&manifest_path, &manifest)?;

    Ok(plugin_dir)
}

fn import_plugin_into_cowork(artifact_path: &Path) -> Result<Vec<PathBuf>, String> {
    let root = claude_local_agent_sessions_root()?;
    let rpm_dirs = discover_cowork_rpm_dirs_under(&root);
    if rpm_dirs.is_empty() {
        return Err(format!(
            "No Cowork plugin store found under {}. Open Cowork once, then try again.",
            root.display()
        ));
    }

    let now = chrono::Utc::now();
    let updated_at_millis = now.timestamp_millis();
    let updated_at_iso = now.to_rfc3339_opts(chrono::SecondsFormat::Millis, true);

    rpm_dirs
        .iter()
        .map(|rpm_dir| {
            import_plugin_into_rpm_dir(
                artifact_path,
                rpm_dir,
                COWORK_PLUGIN_NAME,
                updated_at_millis,
                &updated_at_iso,
            )
        })
        .collect()
}

fn tail_for_ui(text: &str, max_chars: usize) -> String {
    let char_count = text.chars().count();
    if char_count <= max_chars {
        return text.trim().to_string();
    }
    let start = char_count.saturating_sub(max_chars);
    format!("…{}", text.chars().skip(start).collect::<String>().trim())
}

/// Build, register, install, and enable `hq-cowork@hq`, then import the
/// plugin package into Cowork's local plugin store.
#[tauri::command]
pub async fn install_cowork_plugin(
    hq_folder_path: Option<String>,
) -> Result<CoworkPluginInstallResult, String> {
    let hq_root = resolve_hq_folder_path(hq_folder_path.as_deref());
    let pack_dir = find_pack_dir(&hq_root)?;
    let script = pack_dir.join("scripts/install-cowork-plugin.sh");
    let artifact_path = paths::hq_config_dir()?.join("plugins/hq-pack-cowork.plugin");
    let args = install_args(&artifact_path);
    let path = paths::child_path();

    let output = tauri::async_runtime::spawn_blocking(move || {
        Command::new("bash")
            .arg(&script)
            .args(&args)
            .current_dir(&pack_dir)
            .env("PATH", path)
            .output()
    })
    .await
    .map_err(|e| format!("join plugin install task: {e}"))?
    .map_err(|e| format!("spawn plugin installer: {e}"))?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = if stderr.trim().is_empty() {
        stdout.to_string()
    } else {
        format!("{stdout}\n{stderr}")
    };

    if !output.status.success() {
        return Err(tail_for_ui(&combined, 4000));
    }

    let cowork_install_paths = import_plugin_into_cowork(&artifact_path)
        .map_err(|e| format!("{e}\n\n{}", tail_for_ui(&combined, 2000)))?;

    Ok(CoworkPluginInstallResult {
        artifact_path: artifact_path.to_string_lossy().to_string(),
        cowork_install_paths: cowork_install_paths
            .iter()
            .map(|path| path.to_string_lossy().to_string())
            .collect(),
        log_tail: tail_for_ui(&combined, 2000),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pack_dir_candidates_include_release_and_staging_locations() {
        let root = PathBuf::from("/tmp/HQ");
        let candidates = pack_dir_candidates(&root);
        assert!(candidates.contains(&PathBuf::from("/tmp/HQ/core/packages/hq-pack-cowork")));
        assert!(candidates.contains(&PathBuf::from(
            "/tmp/HQ/repos/private/hq-core-staging/core/packages/hq-pack-cowork"
        )));
    }

    #[test]
    fn install_args_request_one_click_install_to_artifact_path() {
        let args = install_args(Path::new("/tmp/hq-pack-cowork.plugin"));
        assert_eq!(args, ["--install", "--out", "/tmp/hq-pack-cowork.plugin"]);
    }

    #[test]
    fn discover_cowork_rpm_dirs_under_finds_manifest_dirs_two_levels_down() {
        let tmp = tempfile::tempdir().unwrap();
        let rpm = tmp.path().join("account/workspace/rpm");
        std::fs::create_dir_all(&rpm).unwrap();
        std::fs::write(rpm.join("manifest.json"), r#"{"plugins":[]}"#).unwrap();
        std::fs::create_dir_all(tmp.path().join("account/workspace/nope")).unwrap();

        assert_eq!(discover_cowork_rpm_dirs_under(tmp.path()), vec![rpm]);
    }

    #[test]
    fn upsert_rpm_manifest_replaces_existing_plugin_and_preserves_upload_marketplace() {
        let mut manifest = json!({
            "lastUpdated": 1,
            "plugins": [
                {
                    "id": "plugin_existing",
                    "name": "hq-cowork",
                    "updatedAt": "old",
                    "marketplaceId": "marketplace_upload",
                    "marketplaceName": "My Uploads",
                    "installedBy": "user"
                },
                {
                    "id": "plugin_other",
                    "name": "other",
                    "marketplaceId": "marketplace_org",
                    "marketplaceName": "Org",
                    "installedBy": "user"
                }
            ]
        });

        upsert_rpm_manifest(&mut manifest, "hq-cowork", "plugin_existing", 42, "now");

        let plugins = manifest["plugins"].as_array().unwrap();
        assert_eq!(plugins.len(), 2);
        assert_eq!(manifest["lastUpdated"], json!(42));
        assert_eq!(plugins[0]["id"], json!("plugin_existing"));
        assert_eq!(plugins[0]["updatedAt"], json!("now"));
        assert_eq!(plugins[0]["marketplaceId"], json!("marketplace_upload"));
    }

    #[test]
    fn manifest_plugin_id_rejects_path_like_ids() {
        let manifest = json!({
            "plugins": [
                { "id": "../bad", "name": "hq-cowork" }
            ]
        });

        assert_eq!(manifest_plugin_id(&manifest, "hq-cowork"), None);
    }

    #[test]
    fn import_plugin_into_rpm_dir_unpacks_artifact_and_updates_manifest() {
        let tmp = tempfile::tempdir().unwrap();
        let source = tmp.path().join("source");
        std::fs::create_dir_all(source.join(".claude-plugin")).unwrap();
        std::fs::write(
            source.join(".claude-plugin/plugin.json"),
            r#"{"name":"hq-cowork"}"#,
        )
        .unwrap();
        std::fs::write(source.join("README.md"), "plugin").unwrap();

        let artifact = tmp.path().join("hq-pack-cowork.plugin");
        let output = Command::new("/usr/bin/zip")
            .arg("-qr")
            .arg(&artifact)
            .arg(".")
            .current_dir(&source)
            .output()
            .unwrap();
        assert!(output.status.success());

        let rpm = tmp.path().join("rpm");
        std::fs::create_dir_all(&rpm).unwrap();
        std::fs::write(rpm.join("manifest.json"), r#"{"plugins":[]}"#).unwrap();

        let plugin_dir = import_plugin_into_rpm_dir(
            &artifact,
            &rpm,
            "hq-cowork",
            99,
            "2026-05-31T20:00:00.000Z",
        )
        .unwrap();

        assert!(plugin_dir.join(".claude-plugin/plugin.json").is_file());
        let manifest: Value =
            serde_json::from_str(&std::fs::read_to_string(rpm.join("manifest.json")).unwrap())
                .unwrap();
        assert_eq!(manifest["lastUpdated"], json!(99));
        assert_eq!(manifest["plugins"][0]["name"], json!("hq-cowork"));
        assert_eq!(
            manifest["plugins"][0]["marketplaceName"],
            json!("My Uploads")
        );
    }

    #[test]
    fn tail_for_ui_preserves_short_text_and_truncates_long_text() {
        assert_eq!(tail_for_ui("ok\n", 10), "ok");
        let tail = tail_for_ui("abcdefghijklmnopqrstuvwxyz", 5);
        assert_eq!(tail, "…vwxyz");
    }
}
