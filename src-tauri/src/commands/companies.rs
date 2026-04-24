//! Tauri commands for enumerating companies and promoting local companies to
//! AWS via `hq-sync-runner`.
//!
//! This module mirrors the architecture of [`crate::commands::sync`] (spawn
//! via the pinned `@indigoai-us/hq-cloud` npx shim, stream ndjson, re-emit as
//! typed Tauri events) but for the promote flow introduced in US-004b:
//!
//! * [`list_all_companies`] — one-shot, non-streaming: shells out to
//!   `hq-sync-runner --list-all-companies`, parses the single-line JSON array
//!   on stdout into a [`Vec<CompanyInfo>`], returns it to the frontend.
//! * [`promote_company`] — streaming: shells out to `hq-sync-runner --promote
//!   <slug>`, parses ndjson, emits `promote:start` / `promote:progress` /
//!   `promote:complete` / `promote:error` Tauri events. Singleton per slug
//!   (handle `hq-promote-<slug>`), because promoting different companies in
//!   parallel is legitimate.
//!
//! Binary resolution mirrors `sync.rs` exactly: we invoke `npx -y
//! --package=@indigoai-us/hq-cloud@<ver> hq-sync-runner …`. See the rationale
//! block at the top of `sync.rs` for why we don't require a global install.
//!
//! ## Test seam
//!
//! The public Tauri commands are thin wrappers around `*_impl` functions that
//! take a pre-built [`SpawnArgs`]. Integration tests in `tests/` can then drop
//! in a POSIX shell stub without having to monkey-patch npx.

use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;

use chrono::SecondsFormat;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Runtime};

use crate::commands::config::{HqConfig, MenubarPrefs};
use crate::commands::process::{
    deregister_process, run_process_impl, try_register_handle, ProcessEvent, SpawnArgs,
};
use crate::commands::sync::{HQ_CLOUD_PACKAGE, HQ_CLOUD_VERSION, RUNNER_BIN};
use crate::util::paths;

// ─────────────────────────────────────────────────────────────────────────────
// Public types
// ─────────────────────────────────────────────────────────────────────────────

/// Mirror of hq-cloud's `CompanyEntry`. The frontend renders this as a row.
///
/// * `aws`   — Vault knows about it; no local `company.yaml`.
/// * `local` — local `company.yaml` only; not yet promoted.
/// * `both`  — local folder is linked to a Vault entity via `cloudCompanyUid`.
///
/// `uid` is `None` iff `source == "local"` (Vault-less entries cannot have one
/// yet — that's what the promote flow provisions).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CompanyInfo {
    pub slug: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub uid: Option<String>,
    pub source: String,
}

/// Tauri event payload for `promote:start`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PromoteStartEvent {
    pub slug: String,
    pub started_at: String,
}

/// Tauri event payload for `promote:progress`.
#[derive(Debug, Clone, Serialize)]
pub struct PromoteProgressEvent {
    pub slug: String,
    pub step: String,
}

/// Tauri event payload for `promote:complete`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PromoteCompleteEvent {
    pub slug: String,
    pub uid: String,
    pub bucket_name: String,
}

/// Tauri event payload for `promote:error`.
#[derive(Debug, Clone, Serialize)]
pub struct PromoteErrorEvent {
    pub slug: String,
    pub message: String,
}

/// Tauri event channel names. Mirrors the `EVENT_SYNC_*` constants in
/// `events.rs` — kept here because only this module emits them.
pub const EVENT_PROMOTE_START: &str = "promote:start";
pub const EVENT_PROMOTE_PROGRESS: &str = "promote:progress";
pub const EVENT_PROMOTE_COMPLETE: &str = "promote:complete";
pub const EVENT_PROMOTE_ERROR: &str = "promote:error";

// ─────────────────────────────────────────────────────────────────────────────
// Internal ndjson parse types (mirror the sync-runner protocol)
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
#[allow(dead_code)] // `slug` fields are carried by the protocol but the handler
                    // uses the outer `slug` param (the canonical source of
                    // truth for this subprocess). Keeping the field in the
                    // schema means non-conforming runner output still fails
                    // to parse — a useful integrity check.
enum RunnerLine {
    #[serde(rename = "promote:start")]
    PromoteStart { slug: String },
    #[serde(rename = "promote:progress")]
    PromoteProgress { slug: String, step: String },
    #[serde(rename = "promote:complete")]
    #[serde(rename_all = "camelCase")]
    PromoteComplete {
        slug: String,
        uid: String,
        bucket_name: String,
    },
    #[serde(rename = "promote:error")]
    PromoteError { slug: String, message: String },
}

// ─────────────────────────────────────────────────────────────────────────────
// Config resolution (same shape as sync.rs — inlined to avoid cross-module
// coupling to an async Tauri command)
// ─────────────────────────────────────────────────────────────────────────────

fn resolve_hq_folder_path() -> Result<String, String> {
    let config_path = paths::config_json_path()?;
    let menubar_path = paths::menubar_json_path()?;

    let menubar_prefs: Option<MenubarPrefs> = if menubar_path.exists() {
        std::fs::read_to_string(&menubar_path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
    } else {
        None
    };

    let config: Option<HqConfig> = if config_path.exists() {
        let contents = std::fs::read_to_string(&config_path)
            .map_err(|e| format!("Failed to read config.json: {}", e))?;
        Some(
            serde_json::from_str(&contents)
                .map_err(|e| format!("Failed to parse config.json: {}", e))?,
        )
    } else {
        None
    };

    let hq_folder = paths::resolve_hq_folder(
        config.as_ref().and_then(|c| c.hq_folder_path.as_deref()),
        menubar_prefs.as_ref().and_then(|p| p.hq_path.as_deref()),
    );

    Ok(hq_folder.to_string_lossy().to_string())
}

// ─────────────────────────────────────────────────────────────────────────────
// SpawnArgs builders (testable)
// ─────────────────────────────────────────────────────────────────────────────

fn base_env(hq_folder_path: &str) -> std::collections::HashMap<String, String> {
    let mut env = std::collections::HashMap::new();
    env.insert("HQ_ROOT".to_string(), hq_folder_path.to_string());
    env.insert("PATH".to_string(), paths::child_path());
    env
}

/// Build spawn args for `npx -y --package=@indigoai-us/hq-cloud@<ver>
/// hq-sync-runner --list-all-companies --hq-root <path>`.
pub fn build_list_all_companies_spawn_args(hq_folder_path: &str) -> SpawnArgs {
    SpawnArgs {
        cmd: paths::resolve_bin("npx"),
        args: vec![
            "-y".to_string(),
            format!("--package={}@{}", HQ_CLOUD_PACKAGE, HQ_CLOUD_VERSION),
            RUNNER_BIN.to_string(),
            "--list-all-companies".to_string(),
            "--hq-root".to_string(),
            hq_folder_path.to_string(),
        ],
        cwd: None,
        env: Some(base_env(hq_folder_path)),
    }
}

/// Build spawn args for `npx -y --package=@indigoai-us/hq-cloud@<ver>
/// hq-sync-runner --promote <slug> --hq-root <path>`.
pub fn build_promote_spawn_args(slug: &str, hq_folder_path: &str) -> SpawnArgs {
    SpawnArgs {
        cmd: paths::resolve_bin("npx"),
        args: vec![
            "-y".to_string(),
            format!("--package={}@{}", HQ_CLOUD_PACKAGE, HQ_CLOUD_VERSION),
            RUNNER_BIN.to_string(),
            "--promote".to_string(),
            slug.to_string(),
            "--hq-root".to_string(),
            hq_folder_path.to_string(),
        ],
        cwd: None,
        env: Some(base_env(hq_folder_path)),
    }
}

/// Singleton handle for an in-flight promotion of `slug`.
pub fn promote_handle(slug: &str) -> String {
    format!("hq-promote-{}", slug)
}

// ─────────────────────────────────────────────────────────────────────────────
// list_all_companies — one-shot, non-streaming
// ─────────────────────────────────────────────────────────────────────────────

/// Shell out to `hq-sync-runner --list-all-companies` and parse its single
/// JSON-array line on stdout into a [`Vec<CompanyInfo>`].
///
/// Separated from the Tauri command so tests can inject a stub
/// [`SpawnArgs`] pointing at a POSIX shell script.
pub fn list_all_companies_impl(spawn: &SpawnArgs) -> Result<Vec<CompanyInfo>, String> {
    let mut cmd = Command::new(&spawn.cmd);
    cmd.args(&spawn.args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(cwd) = &spawn.cwd {
        cmd.current_dir(cwd);
    }
    if let Some(env) = &spawn.env {
        for (k, v) in env {
            cmd.env(k, v);
        }
    }

    let output = cmd
        .output()
        .map_err(|e| format!("spawn '{}': {}", spawn.cmd, e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "hq-sync-runner --list-all-companies exited with code {}: {}",
            output
                .status
                .code()
                .map(|c| c.to_string())
                .unwrap_or_else(|| "signal".to_string()),
            stderr.trim()
        ));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let trimmed = stdout.trim();
    if trimmed.is_empty() {
        return Err("hq-sync-runner --list-all-companies produced no output".to_string());
    }

    // The runner prints one JSON array. We deliberately don't support
    // multi-line output here — if that ever happens, the runner is
    // misbehaving and we want a loud parse failure, not silent truncation.
    serde_json::from_str::<Vec<CompanyInfo>>(trimmed)
        .map_err(|e| format!("Failed to parse --list-all-companies output as JSON: {}", e))
}

/// Tauri command: enumerate every company the caller can see (AWS + local).
///
/// Returns a `Vec<CompanyInfo>` on success, `Err(String)` if the subprocess
/// fails or returns non-JSON. Non-streaming — the frontend gets a single
/// return value, not a Tauri event stream.
#[tauri::command]
pub fn list_all_companies(_app: AppHandle) -> Result<Vec<CompanyInfo>, String> {
    let hq_folder_path = resolve_hq_folder_path()?;
    let spawn_args = build_list_all_companies_spawn_args(&hq_folder_path);
    list_all_companies_impl(&spawn_args)
}

// ─────────────────────────────────────────────────────────────────────────────
// promote_company — streaming, per-slug singleton
// ─────────────────────────────────────────────────────────────────────────────

/// Parse a single ndjson line and emit the corresponding Tauri `promote:*`
/// event. Unknown/malformed lines are silently skipped (logged in debug
/// builds). Returns `true` if the line signalled a terminal state (complete
/// or error) so the caller can decide what to do on exit.
fn handle_promote_line<R: Runtime>(app: &AppHandle<R>, slug: &str, line: &str) {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return;
    }

    let parsed: RunnerLine = match serde_json::from_str(trimmed) {
        Ok(p) => p,
        Err(_e) => {
            #[cfg(debug_assertions)]
            eprintln!(
                "[promote] skipping unparseable line: {} | line: {}",
                _e, trimmed
            );
            return;
        }
    };

    let result = match parsed {
        // We ignore the slug from the runner here — the Tauri command holds
        // the canonical slug for this subprocess. Mismatches would indicate a
        // runner bug; silently trust the caller's slug for event routing.
        RunnerLine::PromoteStart { .. } => {
            let started_at = chrono::Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);
            app.emit(
                EVENT_PROMOTE_START,
                PromoteStartEvent {
                    slug: slug.to_string(),
                    started_at,
                },
            )
        }
        RunnerLine::PromoteProgress { step, .. } => app.emit(
            EVENT_PROMOTE_PROGRESS,
            PromoteProgressEvent {
                slug: slug.to_string(),
                step,
            },
        ),
        RunnerLine::PromoteComplete {
            uid, bucket_name, ..
        } => app.emit(
            EVENT_PROMOTE_COMPLETE,
            PromoteCompleteEvent {
                slug: slug.to_string(),
                uid,
                bucket_name,
            },
        ),
        RunnerLine::PromoteError { message, .. } => app.emit(
            EVENT_PROMOTE_ERROR,
            PromoteErrorEvent {
                slug: slug.to_string(),
                message,
            },
        ),
    };

    if let Err(_e) = result {
        #[cfg(debug_assertions)]
        eprintln!("[promote] failed to emit event: {}", _e);
    }
}

/// Promotion state shared between the spawner and the stdout reader thread.
/// `saw_error` lets us avoid emitting a bogus "exited with code N" error on
/// top of a real `promote:error` event that already described the failure.
#[derive(Default)]
struct PromoteState {
    saw_error: bool,
}

/// Test-friendly implementation: spawns `spawn`, streams ndjson, emits
/// `promote:*` events, handles the singleton handle lifecycle.
///
/// Blocks until the subprocess exits. In production the Tauri command wraps
/// this in a background thread so the renderer isn't blocked.
pub fn promote_company_impl<R: Runtime>(
    app: AppHandle<R>,
    slug: &str,
    spawn: SpawnArgs,
) -> Result<(), String> {
    let handle = promote_handle(slug);

    // Atomically check-and-register — if this slug is already being promoted,
    // bail without touching the subprocess side.
    if !try_register_handle(&handle) {
        return Err("already running".to_string());
    }

    let state = Arc::new(Mutex::new(PromoteState::default()));
    let state_for_handler = state.clone();
    let app_for_handler = app.clone();
    let slug_owned = slug.to_string();

    let result = run_process_impl(&handle, &spawn, move |event| match event {
        ProcessEvent::Stdout(line) => {
            // Track whether we saw a `promote:error` so the Exit arm doesn't
            // double-emit. Cheap parse — we already parse the line for
            // emission in handle_promote_line, but this module doesn't care
            // about one extra JSON decode per line.
            if let Ok(parsed) = serde_json::from_str::<RunnerLine>(line.trim()) {
                if matches!(parsed, RunnerLine::PromoteError { .. }) {
                    state_for_handler.lock().unwrap().saw_error = true;
                }
            }
            handle_promote_line(&app_for_handler, &slug_owned, &line);
        }
        ProcessEvent::Stderr(_line) => {
            #[cfg(debug_assertions)]
            eprintln!("[promote stderr] {}", _line);
        }
        ProcessEvent::Exit { code, success } => {
            // The runner exits non-zero on promote failures AFTER emitting
            // `promote:error`. Don't double-emit — the frontend already knows.
            // A non-zero exit without a prior `promote:error` means the runner
            // bailed before the protocol stream got useful — surface that as a
            // synthetic error so the UI doesn't hang on "Promoting…".
            if !success && !state_for_handler.lock().unwrap().saw_error {
                let _ = app_for_handler.emit(
                    EVENT_PROMOTE_ERROR,
                    PromoteErrorEvent {
                        slug: slug_owned.clone(),
                        message: format!(
                            "hq-sync-runner exited with code {}",
                            code.map(|c| c.to_string())
                                .unwrap_or_else(|| "unknown".to_string())
                        ),
                    },
                );
            }
        }
    });

    // run_process_impl deregisters on success; on spawn failure we need to
    // undo our try_register_handle so the user can retry.
    if let Err(ref e) = result {
        deregister_process(&handle);
        // Also emit an error event so the UI knows — otherwise the caller
        // would see the Err return value but any subscribed listeners (e.g.
        // a toast toaster that only watches events) would be silent.
        let _ = app.emit(
            EVENT_PROMOTE_ERROR,
            PromoteErrorEvent {
                slug: slug.to_string(),
                message: e.clone(),
            },
        );
    }

    // Final return mirrors sync.rs semantics: Ok(()) if the stream parsed
    // cleanly AND no error was observed; Err(...) otherwise.
    match result {
        Ok(()) => {
            if state.lock().unwrap().saw_error {
                Err("promote failed — see promote:error event".to_string())
            } else {
                Ok(())
            }
        }
        Err(e) => Err(e),
    }
}

/// Tauri command: promote a local company to AWS via `hq-sync-runner --promote
/// <slug>`. Emits `promote:start` → `promote:progress*` →
/// `promote:complete | promote:error`.
///
/// Returns immediately with the handle string once the subprocess is spawned
/// — the actual work happens on a background thread. Second concurrent call
/// for the same slug returns `Err("already running")`.
#[tauri::command]
pub fn promote_company(app: AppHandle, slug: String) -> Result<String, String> {
    let handle = promote_handle(&slug);

    // Do the "already running" check up front so callers get a synchronous
    // error, not an event-via-thread. We register the handle here, then
    // transfer ownership of the handle's lifecycle to the background thread.
    if !try_register_handle(&handle) {
        return Err("already running".to_string());
    }

    let hq_folder_path = match resolve_hq_folder_path() {
        Ok(p) => p,
        Err(e) => {
            deregister_process(&handle);
            return Err(e);
        }
    };
    let spawn_args = build_promote_spawn_args(&slug, &hq_folder_path);

    // Emit promote:start synchronously so the UI's optimistic "Promoting…"
    // state has a confirmed anchor — otherwise the UI would only see it once
    // the runner's own `promote:start` ndjson line arrives (which depends on
    // npx + node startup time). The ndjson `promote:start` is still honored
    // downstream for tests that drive the impl directly without this
    // command wrapper.
    let started_at = chrono::Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);
    let _ = app.emit(
        EVENT_PROMOTE_START,
        PromoteStartEvent {
            slug: slug.clone(),
            started_at,
        },
    );

    // Background thread: own the subprocess lifecycle. We release the handle
    // we just claimed (so the impl's own try_register_handle succeeds) — the
    // gap is microseconds and is protected by the fact that we already
    // returned an explicit already-running error to any racing caller.
    deregister_process(&handle);

    let slug_bg = slug.clone();
    let app_bg = app.clone();
    thread::spawn(move || {
        // Errors from the impl are already surfaced as promote:error events
        // (either from the runner's own ndjson or the synthetic exit-code
        // path). The Result is dropped intentionally.
        let _ = promote_company_impl(app_bg, &slug_bg, spawn_args);
    });

    Ok(handle)
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests (unit — integration lives in src-tauri/tests/)
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_promote_handle_format() {
        assert_eq!(promote_handle("acme"), "hq-promote-acme");
        assert_eq!(promote_handle("weird-slug_1"), "hq-promote-weird-slug_1");
    }

    #[test]
    fn test_build_list_all_companies_spawn_args_flags() {
        let args = build_list_all_companies_spawn_args("/Users/test/HQ");
        assert!(
            args.cmd == "npx" || args.cmd.ends_with("/npx"),
            "expected npx, got `{}`",
            args.cmd,
        );
        assert!(args.args.contains(&"--list-all-companies".to_string()));
        assert!(args.args.contains(&"--hq-root".to_string()));
        assert!(args.args.contains(&"/Users/test/HQ".to_string()));
        assert!(args
            .args
            .contains(&format!("--package={}@{}", HQ_CLOUD_PACKAGE, HQ_CLOUD_VERSION)));
        assert!(args.args.contains(&RUNNER_BIN.to_string()));
    }

    #[test]
    fn test_build_promote_spawn_args_flags() {
        let args = build_promote_spawn_args("acme", "/Users/test/HQ");
        assert!(args.args.contains(&"--promote".to_string()));
        assert!(args.args.contains(&"acme".to_string()));
        assert!(args.args.contains(&"--hq-root".to_string()));
        assert!(args.args.contains(&"/Users/test/HQ".to_string()));
        // --promote <slug> must be adjacent (runner parses positional values).
        let promote_idx = args
            .args
            .iter()
            .position(|a| a == "--promote")
            .expect("--promote present");
        assert_eq!(
            args.args.get(promote_idx + 1),
            Some(&"acme".to_string()),
            "slug must immediately follow --promote",
        );
    }

    #[test]
    fn test_build_spawn_args_env_sets_hq_root_and_path() {
        let args = build_promote_spawn_args("acme", "/Users/test/HQ");
        let env = args.env.unwrap();
        assert_eq!(env.get("HQ_ROOT"), Some(&"/Users/test/HQ".to_string()));
        let path = env.get("PATH").expect("PATH must be set");
        assert!(path.contains("/opt/homebrew/bin"), "PATH missing /opt/homebrew/bin: {}", path);
    }

    // ── ndjson parsing ────────────────────────────────────────────────────

    #[test]
    fn test_parse_promote_start_ndjson() {
        let line = r#"{"type":"promote:start","slug":"acme"}"#;
        let parsed: RunnerLine = serde_json::from_str(line).unwrap();
        assert!(matches!(parsed, RunnerLine::PromoteStart { .. }));
    }

    #[test]
    fn test_parse_promote_progress_ndjson() {
        let line = r#"{"type":"promote:progress","slug":"acme","step":"bucket"}"#;
        let parsed: RunnerLine = serde_json::from_str(line).unwrap();
        match parsed {
            RunnerLine::PromoteProgress { slug, step } => {
                assert_eq!(slug, "acme");
                assert_eq!(step, "bucket");
            }
            _ => panic!("expected PromoteProgress"),
        }
    }

    #[test]
    fn test_parse_promote_complete_ndjson() {
        let line = r#"{"type":"promote:complete","slug":"acme","uid":"cmp_01","bucketName":"b-1"}"#;
        let parsed: RunnerLine = serde_json::from_str(line).unwrap();
        match parsed {
            RunnerLine::PromoteComplete { slug, uid, bucket_name } => {
                assert_eq!(slug, "acme");
                assert_eq!(uid, "cmp_01");
                assert_eq!(bucket_name, "b-1");
            }
            _ => panic!("expected PromoteComplete"),
        }
    }

    #[test]
    fn test_parse_promote_error_ndjson() {
        let line = r#"{"type":"promote:error","slug":"acme","message":"vault down"}"#;
        let parsed: RunnerLine = serde_json::from_str(line).unwrap();
        match parsed {
            RunnerLine::PromoteError { slug, message } => {
                assert_eq!(slug, "acme");
                assert_eq!(message, "vault down");
            }
            _ => panic!("expected PromoteError"),
        }
    }

    #[test]
    fn test_parse_unknown_promote_type_fails() {
        let line = r#"{"type":"promote:partial","slug":"acme"}"#;
        let parsed: Result<RunnerLine, _> = serde_json::from_str(line);
        assert!(parsed.is_err());
    }

    // ── CompanyInfo ───────────────────────────────────────────────────────

    #[test]
    fn test_company_info_aws_deserializes() {
        let json = r#"{"slug":"acme","name":"Acme","uid":"cmp_1","source":"aws"}"#;
        let info: CompanyInfo = serde_json::from_str(json).unwrap();
        assert_eq!(info.slug, "acme");
        assert_eq!(info.name, "Acme");
        assert_eq!(info.uid, Some("cmp_1".to_string()));
        assert_eq!(info.source, "aws");
    }

    #[test]
    fn test_company_info_local_deserializes_without_uid() {
        let json = r#"{"slug":"beta","name":"Beta","source":"local"}"#;
        let info: CompanyInfo = serde_json::from_str(json).unwrap();
        assert!(info.uid.is_none());
        assert_eq!(info.source, "local");
    }

    #[test]
    fn test_company_info_list_round_trip() {
        let list = vec![
            CompanyInfo {
                slug: "acme".to_string(),
                name: "Acme".to_string(),
                uid: None,
                source: "local".to_string(),
            },
            CompanyInfo {
                slug: "beta".to_string(),
                name: "Beta".to_string(),
                uid: Some("U-1".to_string()),
                source: "aws".to_string(),
            },
        ];
        let json = serde_json::to_string(&list).unwrap();
        let parsed: Vec<CompanyInfo> = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, list);
    }

    #[test]
    fn test_company_info_skips_none_uid() {
        let info = CompanyInfo {
            slug: "acme".to_string(),
            name: "Acme".to_string(),
            uid: None,
            source: "local".to_string(),
        };
        let json = serde_json::to_string(&info).unwrap();
        assert!(!json.contains("\"uid\""));
    }

    // ── list_all_companies_impl via a stub POSIX script ──────────────────

    fn make_exec_stub(
        dir: &std::path::Path,
        name: &str,
        body: &str,
    ) -> std::path::PathBuf {
        use std::os::unix::fs::PermissionsExt as _;
        let path = dir.join(name);
        std::fs::write(&path, body).unwrap();
        let mut perms = std::fs::metadata(&path).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&path, perms).unwrap();
        path
    }

    #[test]
    fn test_list_all_companies_impl_parses_stub_output() {
        let tmp = tempfile::tempdir().unwrap();
        let script = make_exec_stub(
            tmp.path(),
            "stub.sh",
            r#"#!/bin/sh
printf '[{"slug":"acme","name":"Acme","source":"local"},{"slug":"beta","name":"Beta","uid":"U-1","source":"aws"}]\n'
"#,
        );
        let spawn = SpawnArgs {
            cmd: script.to_string_lossy().to_string(),
            args: vec![],
            cwd: None,
            env: None,
        };
        let result = list_all_companies_impl(&spawn).unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].slug, "acme");
        assert_eq!(result[0].source, "local");
        assert!(result[0].uid.is_none());
        assert_eq!(result[1].slug, "beta");
        assert_eq!(result[1].uid, Some("U-1".to_string()));
    }

    #[test]
    fn test_list_all_companies_impl_empty_list() {
        let tmp = tempfile::tempdir().unwrap();
        let script = make_exec_stub(
            tmp.path(),
            "stub.sh",
            "#!/bin/sh\nprintf '[]\\n'\n",
        );
        let spawn = SpawnArgs {
            cmd: script.to_string_lossy().to_string(),
            args: vec![],
            cwd: None,
            env: None,
        };
        let result = list_all_companies_impl(&spawn).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_list_all_companies_impl_non_zero_exit_returns_err() {
        let tmp = tempfile::tempdir().unwrap();
        let script = make_exec_stub(
            tmp.path(),
            "stub.sh",
            "#!/bin/sh\nprintf 'boom\\n' >&2\nexit 1\n",
        );
        let spawn = SpawnArgs {
            cmd: script.to_string_lossy().to_string(),
            args: vec![],
            cwd: None,
            env: None,
        };
        let err = list_all_companies_impl(&spawn).unwrap_err();
        assert!(err.contains("exited"), "expected exit error, got: {}", err);
        assert!(err.contains("boom"), "expected stderr passed through, got: {}", err);
    }

    #[test]
    fn test_list_all_companies_impl_non_json_returns_err() {
        let tmp = tempfile::tempdir().unwrap();
        let script = make_exec_stub(
            tmp.path(),
            "stub.sh",
            "#!/bin/sh\nprintf 'not json\\n'\n",
        );
        let spawn = SpawnArgs {
            cmd: script.to_string_lossy().to_string(),
            args: vec![],
            cwd: None,
            env: None,
        };
        let err = list_all_companies_impl(&spawn).unwrap_err();
        assert!(err.contains("parse"), "expected parse error, got: {}", err);
    }

    #[test]
    fn test_list_all_companies_impl_missing_binary_returns_err() {
        let spawn = SpawnArgs {
            cmd: "/definitely/does/not/exist/hq-sync-runner-xyz".to_string(),
            args: vec![],
            cwd: None,
            env: None,
        };
        let err = list_all_companies_impl(&spawn).unwrap_err();
        assert!(err.contains("spawn"), "expected spawn error, got: {}", err);
    }
}
