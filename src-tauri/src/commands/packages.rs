//! Package management surface — a thin wrapper over the `hq` CLI's pack
//! lifecycle (`hq packs list/update/uninstall`) plus the registry package
//! listing (`hq packages list`).
//!
//! Design: the CLI owns all the real logic (symlink wiring, archive, update
//! probing). This module just resolves the `hq` binary + the user's HQ folder,
//! shells out, and relays results/progress to the caller. Mirrors the
//! resolution pattern in `hq_core_update.rs`.
//!
//! These commands back the unified desktop-alt **Library → Installed** surface
//! (`src/desktop-alt/panels/InstalledPacksPanel.svelte`). US-009 removed the old
//! standalone Packages window (and its `open_packages_window` /
//! `packages_window_ready` lifecycle commands + `PendingPackages` handshake
//! state); the data commands below are unchanged and now feed the in-Library tab.
//!
//! Commands:
//!   * `list_packages`          — read-only snapshot (content packs + registry)
//!   * `check_package_updates`  — slower probe; emits `packages:updates`
//!   * `install_package`        — stream `hq install <source>` progress
//!   * `update_package`         — stream `hq packs update <name>`
//!   * `uninstall_package`      — `hq packs uninstall <name> --yes --json`

use std::path::PathBuf;
use std::time::Duration;

use serde::Serialize;
use serde_json::Value;
use tauri::{AppHandle, Emitter};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

use crate::commands::config::{read_hq_config_lenient, MenubarPrefs};
use crate::util::logfile::log;
use crate::util::paths;

/// Offset from app launch before the first pack-update check fires. 20s keeps
/// it out of lockstep with the app updater (10s) and CLI updater (15s).
const INITIAL_DELAY: Duration = Duration::from_secs(20);

/// Re-check cadence. Pack update probing may hit the network per installed
/// pack, so keep it on the same 6h background rhythm as the other updaters.
const CHECK_INTERVAL: Duration = Duration::from_secs(21600);

/// Payload emitted when installed content packs have newer upstream versions.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PackUpdateInfo {
    pub count: usize,
    pub names: Vec<String>,
}

/// Resolve the user's HQ folder using the standard 4-tier resolver, the same
/// way every other CLI-spawning command in this app does.
fn resolve_hq_folder() -> PathBuf {
    let menubar_prefs: Option<MenubarPrefs> = paths::menubar_json_path()
        .ok()
        .filter(|p| p.exists())
        .and_then(|p| std::fs::read_to_string(&p).ok())
        .and_then(|s| serde_json::from_str(&s).ok());
    let config = read_hq_config_lenient().ok().flatten();
    paths::resolve_hq_folder(
        config.as_ref().and_then(|c| c.hq_folder_path.as_deref()),
        menubar_prefs.as_ref().and_then(|p| p.hq_path.as_deref()),
    )
}

/// Run `hq <args>` in the HQ folder and return parsed stdout JSON.
///
/// `HQ_NO_UPDATE_CHECK=1` is set so the CLI's version gate never tries to
/// auto-update mid-call (which would race the command we asked for).
async fn run_hq_json(args: &[&str]) -> Result<Value, String> {
    let hq = paths::resolve_bin("hq");
    let folder = resolve_hq_folder();
    let output = Command::new(&hq)
        .args(args)
        // `hq` is a `#!/usr/bin/env node` script; a Dock/launchd-spawned app
        // gets a minimal PATH where `env` can't find node (exit 127). Hand it
        // the same enriched PATH the sync runner uses. See util::paths.
        .env("PATH", paths::child_path())
        .current_dir(&folder)
        .env("HQ_NO_UPDATE_CHECK", "1")
        .env("HQ_ROOT", &folder)
        .output()
        .await
        .map_err(|e| format!("spawn `hq {}`: {e}", args.join(" ")))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "`hq {}` exited {}: {}",
            args.join(" "),
            output.status.code().unwrap_or(-1),
            stderr.trim()
        ));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    serde_json::from_str(stdout.trim())
        .map_err(|e| format!("parse `hq {}` JSON: {e}", args.join(" ")))
}

/// Summarize `hq packs list --json --check-updates` into the tiny popover
/// payload. Only literal JSON `true` counts: `false`, `null`, missing fields,
/// malformed rows, and unnamed rows are ignored so a partial CLI payload never
/// creates a noisy banner.
pub(crate) fn pack_update_summary(packs_view: &serde_json::Value) -> PackUpdateInfo {
    let names: Vec<String> = packs_view
        .get("installed")
        .and_then(|v| v.as_array())
        .into_iter()
        .flatten()
        .filter(|entry| entry.get("updateAvailable").and_then(|v| v.as_bool()) == Some(true))
        .filter_map(|entry| entry.get("name").and_then(|v| v.as_str()))
        .map(str::to_string)
        .collect();
    PackUpdateInfo {
        count: names.len(),
        names,
    }
}

async fn check_pack_updates_once(app: &AppHandle) -> Result<Option<PackUpdateInfo>, String> {
    let packs_view = run_hq_json(&["packs", "list", "--json", "--check-updates"]).await?;
    let info = pack_update_summary(&packs_view);
    if info.count > 0 {
        log(
            "pack-update",
            &format!(
                "{} pack update(s) available: {}",
                info.count,
                info.names.join(", ")
            ),
        );
        let _ = app.emit("pack-update:available", &info);
        Ok(Some(info))
    } else {
        log("pack-update", "no pack updates available");
        let _ = app.emit("pack-update:cleared", ());
        Ok(None)
    }
}

/// Combined view returned to the Packages window: the content-pack lifecycle
/// payload plus the (best-effort) registry payload. Registry is `null` when
/// the user is offline or not entitled — the window renders content packs
/// regardless.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PackagesView {
    packs: Value,
    registry: Option<Value>,
    error: Option<String>,
}

async fn gather_packages(check_updates: bool) -> PackagesView {
    let packs_args: Vec<&str> = if check_updates {
        vec!["packs", "list", "--json", "--check-updates"]
    } else {
        vec!["packs", "list", "--json"]
    };
    let (packs, error) = match run_hq_json(&packs_args).await {
        Ok(v) => (v, None),
        Err(e) => (Value::Null, Some(e)),
    };
    // Registry listing is best-effort: it needs auth + network and may be
    // empty/offline. Never let it fail the whole view.
    let registry = run_hq_json(&["packages", "list", "--json"]).await.ok();
    PackagesView {
        packs,
        registry,
        error,
    }
}

/// Read-only snapshot for the Packages window. Fast path — no update probing.
#[tauri::command]
pub async fn list_packages() -> Result<Value, String> {
    let view = gather_packages(false).await;
    serde_json::to_value(view).map_err(|e| e.to_string())
}

/// Slower probe (network per pack). Emits `packages:updates` with the fresh
/// view so the window can show an "updates available" badge without blocking
/// the initial render.
#[tauri::command]
pub async fn check_package_updates(app: AppHandle) -> Result<(), String> {
    let view = gather_packages(true).await;
    let value = serde_json::to_value(view).map_err(|e| e.to_string())?;
    let _ = app.emit("packages:updates", value);
    Ok(())
}

/// On-demand hydration for the popover's pack-update banner. The background
/// checker emits the same events, but this closes the gap where the popover
/// opened after the last 6h tick or missed the launch-time event.
#[tauri::command]
pub async fn check_pack_update(app: AppHandle) -> Result<Option<PackUpdateInfo>, String> {
    check_pack_updates_once(&app).await
}

/// Stream a long-running `hq` mutation, relaying its output to the window as
/// `packages:progress` lines and a terminal `packages:complete` /
/// `packages:error`. Used by install / update.
async fn stream_hq(app: &AppHandle, op: &str, name: &str, args: Vec<String>) -> Result<(), String> {
    let hq = paths::resolve_bin("hq");
    let folder = resolve_hq_folder();
    log(
        "packages",
        &format!("stream `hq {}` (op={op}, name={name})", args.join(" ")),
    );
    let mut child = Command::new(&hq)
        .args(&args)
        // node-shebang PATH fix — see run_hq_json.
        .env("PATH", paths::child_path())
        .current_dir(&folder)
        .env("HQ_NO_UPDATE_CHECK", "1")
        .env("HQ_ROOT", &folder)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("spawn `hq {}`: {e}", args.join(" ")))?;

    // Relay both streams as progress lines. `hq install` prints human progress
    // to stdout/stderr; we surface every line so the window shows live status.
    if let Some(out) = child.stdout.take() {
        let app = app.clone();
        let op = op.to_string();
        let name = name.to_string();
        tokio::spawn(async move {
            let mut lines = BufReader::new(out).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let _ = app.emit(
                    "packages:progress",
                    serde_json::json!({ "op": op, "name": name, "line": line }),
                );
            }
        });
    }
    if let Some(err) = child.stderr.take() {
        let app = app.clone();
        let op = op.to_string();
        let name = name.to_string();
        tokio::spawn(async move {
            let mut lines = BufReader::new(err).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let _ = app.emit(
                    "packages:progress",
                    serde_json::json!({ "op": op, "name": name, "line": line }),
                );
            }
        });
    }

    let status = child
        .wait()
        .await
        .map_err(|e| format!("await `hq {}`: {e}", args.join(" ")))?;

    if status.success() {
        let _ = app.emit(
            "packages:complete",
            serde_json::json!({ "op": op, "name": name }),
        );
        Ok(())
    } else {
        let msg = format!(
            "`hq {}` exited {}",
            args.join(" "),
            status.code().unwrap_or(-1)
        );
        let _ = app.emit(
            "packages:error",
            serde_json::json!({ "op": op, "name": name, "message": msg }),
        );
        Err(msg)
    }
}

/// Install a pack. `registry=true` routes to the entitlement-gated registry
/// flow (`hq packages install <slug>`); otherwise the content-pack flow
/// (`hq install <source> --allow-hooks`). `--allow-hooks` avoids a blocking
/// prompt — the window warns the user when a pack contributes hooks.
#[tauri::command]
pub async fn install_package(
    app: AppHandle,
    source: String,
    registry: Option<bool>,
) -> Result<(), String> {
    let args: Vec<String> = if registry.unwrap_or(false) {
        vec!["packages".into(), "install".into(), source.clone()]
    } else {
        vec!["install".into(), source.clone(), "--allow-hooks".into()]
    };
    stream_hq(&app, "install", &source, args).await
}

/// Update an installed content pack (re-install latest).
#[tauri::command]
pub async fn update_package(app: AppHandle, name: String) -> Result<(), String> {
    let args = vec![
        "packs".into(),
        "update".into(),
        name.clone(),
        "--yes".into(),
    ];
    stream_hq(&app, "update", &name, args).await
}

/// Update every selected installed content pack sequentially. The named form
/// (`hq packs update <name> --yes`) forces a clean re-sync for that pack, which
/// is more reliable than the bare aggregate command when quarantine messaging
/// tells the user to repair a specific stale pack.
#[tauri::command]
pub async fn update_packs(app: AppHandle, names: Vec<String>) -> Result<(), String> {
    for name in names {
        let args = vec![
            "packs".into(),
            "update".into(),
            name.clone(),
            "--yes".into(),
        ];
        stream_hq(&app, "update", &name, args).await?;
    }
    Ok(())
}

/// Uninstall a content pack. Returns the structured uninstall result so the
/// window can show what was unlinked / archived. The CLI runs the symlink
/// un-wire + archive + re-scan; the app fires the suggested heavier side
/// effects (a sync/reindex) on its own cadence.
#[tauri::command]
pub async fn uninstall_package(name: String) -> Result<Value, String> {
    run_hq_json(&["packs", "uninstall", &name, "--yes", "--json"]).await
}

/// Background loop: first check 20s after launch, then every 6h. Unlike the
/// CLI updater, this never auto-runs the update — pack updates can be heavier
/// and should stay user-initiated from the banner.
pub fn setup_pack_update_checker(app: &AppHandle) {
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(INITIAL_DELAY).await;
        loop {
            match check_pack_updates_once(&handle).await {
                Ok(_) => {}
                Err(e) => log("pack-update", &format!("background check failed: {e}")),
            }
            tokio::time::sleep(CHECK_INTERVAL).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pack_update_summary_counts_true_update_flags_only() {
        let view = serde_json::json!({
            "installed": [
                { "name": "a", "updateAvailable": true },
                { "name": "b", "updateAvailable": false },
                { "name": "c", "updateAvailable": null },
                { "name": "d", "updateAvailable": true }
            ]
        });

        let info = pack_update_summary(&view);

        assert_eq!(info.count, 2);
        assert_eq!(info.names, vec!["a".to_string(), "d".to_string()]);
    }

    #[test]
    fn pack_update_summary_tolerates_missing_or_empty_installed() {
        assert_eq!(pack_update_summary(&serde_json::json!({})).count, 0);
        assert_eq!(
            pack_update_summary(&serde_json::json!({ "installed": [] })).count,
            0
        );
    }
}
