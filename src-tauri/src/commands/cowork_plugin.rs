//! One-click installer for the HQ Cowork plugin.
//!
//! The real plugin build/install contract lives in hq-core's
//! `core/packages/hq-pack-cowork/scripts/install-cowork-plugin.sh`. HQ Sync
//! only resolves the user's HQ root, finds that pack, and runs the same helper
//! a terminal or `/hq-cowork-install` would run.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Serialize;

use crate::commands::config::{read_hq_config_lenient, MenubarPrefs};
use crate::util::paths;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CoworkPluginInstallResult {
    pub artifact_path: String,
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

fn tail_for_ui(text: &str, max_chars: usize) -> String {
    let char_count = text.chars().count();
    if char_count <= max_chars {
        return text.trim().to_string();
    }
    let start = char_count.saturating_sub(max_chars);
    format!("…{}", text.chars().skip(start).collect::<String>().trim())
}

/// Build, register, install, and enable `hq-cowork@hq`.
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

    Ok(CoworkPluginInstallResult {
        artifact_path: artifact_path.to_string_lossy().to_string(),
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
    fn tail_for_ui_preserves_short_text_and_truncates_long_text() {
        assert_eq!(tail_for_ui("ok\n", 10), "ok");
        let tail = tail_for_ui("abcdefghijklmnopqrstuvwxyz", 5);
        assert_eq!(tail, "…vwxyz");
    }
}
