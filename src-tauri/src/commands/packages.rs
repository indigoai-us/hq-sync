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

use serde::Serialize;
use serde_json::Value;
use tauri::{AppHandle, Emitter};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

use crate::commands::config::{read_hq_config_lenient, MenubarPrefs};
use crate::util::logfile::log;
use crate::util::paths;

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
        let msg = format!("`hq {}` exited {}", args.join(" "), status.code().unwrap_or(-1));
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

/// Uninstall a content pack. Returns the structured uninstall result so the
/// window can show what was unlinked / archived. The CLI runs the symlink
/// un-wire + archive + re-scan; the app fires the suggested heavier side
/// effects (a sync/reindex) on its own cadence.
#[tauri::command]
pub async fn uninstall_package(name: String) -> Result<Value, String> {
    run_hq_json(&["packs", "uninstall", &name, "--yes", "--json"]).await
}
