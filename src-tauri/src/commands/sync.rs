//! Tauri commands for spawning and cancelling `hq-sync-runner --companies`.
//!
//! Uses [`crate::commands::process`] for subprocess lifecycle (spawn, stream,
//! SIGTERM→SIGKILL). Emits typed sync events to the Svelte renderer.
//!
//! Phase 7 (ADR-0001, 2026-04-19): switched from `hq sync --json` (never
//! shipped) to `hq-sync-runner --companies`. The runner is the canonical
//! machine-targeted entrypoint from `@indigoai-us/hq-cloud` ≥5.1.0 — ndjson is
//! the default and only output mode. See:
//!   packages/hq-cloud/src/bin/sync-runner.ts
//!
//! Binary resolution: `hq-sync-runner` must be on PATH. It's installed
//! globally via `npm install -g @indigoai-us/hq-cloud` or through `hq-cli`'s
//! transitive dep. For DMG distribution, this binary will need to be bundled
//! (tracked as a follow-up; out of scope for Phase 7).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use chrono::SecondsFormat;
use tauri::{AppHandle, Emitter};

use crate::commands::config::{HqConfig, MenubarPrefs};
use crate::commands::process::{
    cancel_process_impl, deregister_process, is_registered, run_process_impl, try_register_handle,
    ProcessEvent, SpawnArgs,
};
use crate::commands::status::{journal_for_sync_complete, write_journal};
use crate::events::{
    SyncEvent, EVENT_SYNC_ALL_COMPLETE, EVENT_SYNC_AUTH_ERROR, EVENT_SYNC_COMPLETE,
    EVENT_SYNC_ERROR, EVENT_SYNC_FANOUT_PLAN, EVENT_SYNC_PROGRESS, EVENT_SYNC_SETUP_NEEDED,
};
use crate::util::paths;

// ─────────────────────────────────────────────────────────────────────────────
// Per-run aggregated counters
// ─────────────────────────────────────────────────────────────────────────────

/// Aggregated counters across a single sync run.
///
/// A fresh instance is created per `start_sync` invocation, so totals are
/// scoped to the run — no reset needed between runs. Per-company `Complete`
/// events contribute via `accumulate`; the `AllComplete` handler reads the
/// final totals to build the journal.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RunTotals {
    pub conflicts: u32,
}

impl RunTotals {
    /// Update totals from a single event. Only `Complete` events contribute;
    /// others are ignored. Saturates at `u32::MAX` to avoid panics.
    pub fn accumulate(&mut self, event: &SyncEvent) {
        if let SyncEvent::Complete(c) = event {
            self.conflicts = self.conflicts.saturating_add(c.conflicts);
        }
    }
}

/// Singleton handle — only one sync at a time.
const SYNC_HANDLE: &str = "hq-sync";

/// Hard timeout for a sync run (10 minutes).
const SYNC_TIMEOUT: Duration = Duration::from_secs(600);

/// SIGKILL delay after SIGTERM on cancel.
const SIGKILL_DELAY: Duration = Duration::from_secs(5);

/// Binary name. Must be on PATH — installed globally via
/// `npm install -g @indigoai-us/hq-cloud` (or bundled with the DMG in V2).
const RUNNER_BIN: &str = "hq-sync-runner";

// ─────────────────────────────────────────────────────────────────────────────
// Config resolution (inline — avoids calling async Tauri command)
// ─────────────────────────────────────────────────────────────────────────────

/// Resolve the HQ folder path by reading config.json and menubar.json directly.
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
        config
            .as_ref()
            .and_then(|c| c.hq_folder_path.as_deref()),
        menubar_prefs
            .as_ref()
            .and_then(|p| p.hq_path.as_deref()),
    );

    Ok(hq_folder.to_string_lossy().to_string())
}

// ─────────────────────────────────────────────────────────────────────────────
// SpawnArgs builder (testable)
// ─────────────────────────────────────────────────────────────────────────────

/// Build the SpawnArgs for `hq-sync-runner --companies`.
///
/// Flags:
/// - `--companies` — fan out to every membership the caller has
/// - `--on-conflict abort` — V1 policy; conflicts surface as `aborted: true` on
///   the per-company `complete` event. Interactive resolution is a follow-up
///   (the runner protocol doesn't emit per-file conflict events).
/// - `--hq-root <path>` — local HQ directory
///
/// `HQ_ROOT` is also set in the child env as defense-in-depth (matches the
/// pre-Phase-7 pattern).
pub fn build_sync_spawn_args(hq_folder_path: &str) -> SpawnArgs {
    let mut env = HashMap::new();
    env.insert("HQ_ROOT".to_string(), hq_folder_path.to_string());

    SpawnArgs {
        // Resolve via known install prefixes + login-shell PATH fallback.
        // See `paths::resolve_bin` — GUI-launched Tauri apps get a minimal
        // launchd PATH and would otherwise fail with os error 2 on any
        // binary installed via homebrew or user-level npm.
        cmd: paths::resolve_bin(RUNNER_BIN),
        args: vec![
            "--companies".to_string(),
            "--on-conflict".to_string(),
            "abort".to_string(),
            "--hq-root".to_string(),
            hq_folder_path.to_string(),
        ],
        cwd: None,
        env: Some(env),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// ndjson line handler (testable)
// ─────────────────────────────────────────────────────────────────────────────

/// Parse a single ndjson line and emit the corresponding Tauri event.
/// Unknown/malformed lines are silently skipped (logged in debug builds).
///
/// Per-company `Complete` events also accumulate into `totals`. On
/// `all-complete`, the aggregated totals are persisted to
/// `{hq_folder}/.hq-sync-journal.json` so `get_sync_status` surfaces a real
/// `lastSyncAt` and conflict count instead of "never" / zero.
fn handle_sync_line(app: &AppHandle, hq_folder: &str, totals: &Mutex<RunTotals>, line: &str) {
    // The runner can emit blank lines at process teardown. Skip those cheaply
    // rather than logging a parse error.
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return;
    }

    let event: SyncEvent = match serde_json::from_str(trimmed) {
        Ok(e) => e,
        Err(_e) => {
            #[cfg(debug_assertions)]
            eprintln!("[sync] skipping unparseable line: {} | line: {}", _e, trimmed);
            return;
        }
    };

    // Accumulate per-run counters before emitting. Poisoned locks shouldn't
    // happen in practice (no panics while the mutex is held), but we recover
    // by using the inner value rather than crashing the sync thread.
    {
        let mut t = totals.lock().unwrap_or_else(|e| e.into_inner());
        t.accumulate(&event);
    }

    // Unit struct variants (SetupNeeded) serialize to `()` when emitted via
    // Tauri's `emit(...)` — the frontend gets the event name and an empty
    // payload, which is exactly what we want for a "caller has no person
    // entity" signal.
    let result = match &event {
        SyncEvent::SetupNeeded => app.emit(EVENT_SYNC_SETUP_NEEDED, ()),
        SyncEvent::AuthError(payload) => app.emit(EVENT_SYNC_AUTH_ERROR, payload.clone()),
        SyncEvent::FanoutPlan(payload) => app.emit(EVENT_SYNC_FANOUT_PLAN, payload.clone()),
        SyncEvent::Progress(payload) => app.emit(EVENT_SYNC_PROGRESS, payload.clone()),
        SyncEvent::Error(payload) => app.emit(EVENT_SYNC_ERROR, payload.clone()),
        SyncEvent::Complete(payload) => app.emit(EVENT_SYNC_COMPLETE, payload.clone()),
        SyncEvent::AllComplete(payload) => {
            // Persist summary journal before emitting — the frontend's
            // SyncStats refresh reads this file on popover mount.
            let conflicts = totals.lock().unwrap_or_else(|e| e.into_inner()).conflicts;
            let now_iso = chrono::Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);
            let journal = journal_for_sync_complete(&now_iso, conflicts);
            if let Err(_e) = write_journal(hq_folder, &journal) {
                #[cfg(debug_assertions)]
                eprintln!("[sync] failed to write journal: {}", _e);
            }
            app.emit(EVENT_SYNC_ALL_COMPLETE, payload.clone())
        }
    };

    if let Err(_e) = result {
        #[cfg(debug_assertions)]
        eprintln!("[sync] failed to emit event: {}", _e);
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tauri commands
// ─────────────────────────────────────────────────────────────────────────────

/// Spawn `hq-sync-runner --companies` as a child process.
///
/// - Only one sync can run at a time (singleton handle).
/// - Emits typed sync events (see `events.rs`) to the Svelte renderer as
///   ndjson lines arrive.
/// - Hard timeout of 10 minutes; the sync is cancelled if it exceeds this.
///
/// Returns the handle string on success (always `"hq-sync"`).
#[tauri::command]
pub fn start_sync(app: AppHandle) -> Result<String, String> {
    #[cfg(debug_assertions)]
    eprintln!("[sync] start_sync invoked");

    // Atomically check-and-register to prevent concurrent syncs (TOCTOU-safe)
    if !try_register_handle(SYNC_HANDLE) {
        #[cfg(debug_assertions)]
        eprintln!("[sync] BAIL: already running");
        return Err("Sync is already running".to_string());
    }

    // Resolve config — deregister on failure so future syncs aren't blocked
    let hq_folder_path = match resolve_hq_folder_path() {
        Ok(p) => p,
        Err(e) => {
            #[cfg(debug_assertions)]
            eprintln!("[sync] BAIL: resolve_hq_folder_path failed: {}", e);
            deregister_process(SYNC_HANDLE);
            return Err(e);
        }
    };
    let spawn_args = build_sync_spawn_args(&hq_folder_path);
    #[cfg(debug_assertions)]
    eprintln!(
        "[sync] about to spawn: cmd={} args={:?} hq_root={}",
        spawn_args.cmd, spawn_args.args, hq_folder_path
    );

    // Timeout watchdog — cancels sync after SYNC_TIMEOUT
    thread::spawn(move || {
        thread::sleep(SYNC_TIMEOUT);
        if is_registered(SYNC_HANDLE) {
            #[cfg(debug_assertions)]
            eprintln!("[sync] timeout reached, cancelling");
            cancel_process_impl(SYNC_HANDLE, SIGKILL_DELAY);
        }
    });

    // Background thread: run the subprocess and stream events
    let app_bg = app.clone();
    let hq_folder_for_handler = hq_folder_path.clone();
    // Fresh totals per run — no reset needed between runs.
    let totals: Arc<Mutex<RunTotals>> = Arc::new(Mutex::new(RunTotals::default()));
    thread::spawn(move || {
        #[cfg(debug_assertions)]
        eprintln!("[sync] bg thread: entering run_process_impl");
        let result = run_process_impl(SYNC_HANDLE, &spawn_args, |event| match event {
            ProcessEvent::Stdout(line) => {
                #[cfg(debug_assertions)]
                eprintln!("[sync stdout] {}", line);
                handle_sync_line(&app_bg, &hq_folder_for_handler, &totals, &line);
            }
            ProcessEvent::Stderr(_line) => {
                #[cfg(debug_assertions)]
                eprintln!("[sync stderr] {}", _line);
            }
            ProcessEvent::Exit { code, success } => {
                // The runner exits 0 for recoverable conditions (setup-needed,
                // auth-error) — those surface as ndjson events before exit, so
                // the frontend already knows. A non-zero exit means the runner
                // bailed before emitting a useful protocol stream.
                if !success {
                    let _ = app_bg.emit(
                        EVENT_SYNC_ERROR,
                        crate::events::SyncErrorEvent {
                            company: None,
                            path: "(runner)".to_string(),
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

        if let Err(e) = result {
            let _ = app_bg.emit(
                EVENT_SYNC_ERROR,
                crate::events::SyncErrorEvent {
                    company: None,
                    path: "(spawn)".to_string(),
                    message: e,
                },
            );
        }
    });

    Ok(SYNC_HANDLE.to_string())
}

/// Cancel a running sync.
///
/// Sends SIGTERM to the process group. If the process doesn't exit within 5
/// seconds, SIGKILL is sent.
///
/// Returns `true` if a sync was running and cancellation was initiated.
#[tauri::command]
pub fn cancel_sync() -> bool {
    cancel_process_impl(SYNC_HANDLE, SIGKILL_DELAY)
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_sync_spawn_args_cmd() {
        let args = build_sync_spawn_args("/Users/test/HQ");
        // `resolve_bin` may return an absolute path (e.g.
        // `/opt/homebrew/bin/hq-sync-runner`) on a dev box with the CLI
        // installed, or the bare name on a CI box without it. Either way,
        // the trailing file component must be `hq-sync-runner`.
        assert!(
            args.cmd == RUNNER_BIN || args.cmd.ends_with(&format!("/{}", RUNNER_BIN)),
            "expected cmd to be `{}` or `*/{}`, got `{}`",
            RUNNER_BIN,
            RUNNER_BIN,
            args.cmd
        );
    }

    #[test]
    fn test_build_sync_spawn_args_flags() {
        let args = build_sync_spawn_args("/Users/test/HQ");
        assert_eq!(
            args.args,
            vec![
                "--companies",
                "--on-conflict",
                "abort",
                "--hq-root",
                "/Users/test/HQ",
            ]
        );
    }

    #[test]
    fn test_build_sync_spawn_args_env_sets_hq_root() {
        let args = build_sync_spawn_args("/Users/test/HQ");
        let env = args.env.unwrap();
        assert_eq!(env.get("HQ_ROOT"), Some(&"/Users/test/HQ".to_string()));
        assert_eq!(env.len(), 1);
    }

    #[test]
    fn test_build_sync_spawn_args_no_cwd() {
        let args = build_sync_spawn_args("/any/path");
        assert!(args.cwd.is_none());
    }

    #[test]
    fn test_parse_progress_ndjson() {
        let line = r#"{"type":"progress","company":"indigo","path":"docs/a.md","bytes":42}"#;
        let event: SyncEvent = serde_json::from_str(line).unwrap();
        match event {
            SyncEvent::Progress(p) => {
                assert_eq!(p.company, "indigo");
                assert_eq!(p.path, "docs/a.md");
                assert_eq!(p.bytes, 42);
                assert_eq!(p.message, None);
            }
            _ => panic!("Expected Progress event"),
        }
    }

    #[test]
    fn test_parse_setup_needed_ndjson() {
        let line = r#"{"type":"setup-needed"}"#;
        let event: SyncEvent = serde_json::from_str(line).unwrap();
        assert_eq!(event, SyncEvent::SetupNeeded);
    }

    #[test]
    fn test_parse_auth_error_ndjson() {
        let line = r#"{"type":"auth-error","message":"Token expired"}"#;
        let event: SyncEvent = serde_json::from_str(line).unwrap();
        match event {
            SyncEvent::AuthError(e) => assert_eq!(e.message, "Token expired"),
            _ => panic!("Expected AuthError event"),
        }
    }

    #[test]
    fn test_parse_fanout_plan_ndjson() {
        let line = r#"{"type":"fanout-plan","companies":[{"uid":"cmp_1","slug":"indigo"}]}"#;
        let event: SyncEvent = serde_json::from_str(line).unwrap();
        match event {
            SyncEvent::FanoutPlan(p) => {
                assert_eq!(p.companies.len(), 1);
                assert_eq!(p.companies[0].slug, "indigo");
            }
            _ => panic!("Expected FanoutPlan event"),
        }
    }

    #[test]
    fn test_parse_error_ndjson() {
        let line = r#"{"type":"error","company":"indigo","path":"docs/x.md","message":"Access denied"}"#;
        let event: SyncEvent = serde_json::from_str(line).unwrap();
        match event {
            SyncEvent::Error(e) => {
                assert_eq!(e.company, Some("indigo".to_string()));
                assert_eq!(e.path, "docs/x.md");
                assert_eq!(e.message, "Access denied");
            }
            _ => panic!("Expected Error event"),
        }
    }

    #[test]
    fn test_parse_complete_ndjson() {
        let line = r#"{"type":"complete","company":"indigo","filesDownloaded":7,"bytesDownloaded":204800,"filesSkipped":1,"conflicts":0,"aborted":false}"#;
        let event: SyncEvent = serde_json::from_str(line).unwrap();
        match event {
            SyncEvent::Complete(c) => {
                assert_eq!(c.company, "indigo");
                assert_eq!(c.files_downloaded, 7);
                assert_eq!(c.bytes_downloaded, 204800);
                assert!(!c.aborted);
            }
            _ => panic!("Expected Complete event"),
        }
    }

    #[test]
    fn test_parse_all_complete_ndjson() {
        let line = r#"{"type":"all-complete","companiesAttempted":2,"filesDownloaded":10,"bytesDownloaded":999,"errors":[]}"#;
        let event: SyncEvent = serde_json::from_str(line).unwrap();
        match event {
            SyncEvent::AllComplete(a) => {
                assert_eq!(a.companies_attempted, 2);
                assert!(a.errors.is_empty());
            }
            _ => panic!("Expected AllComplete event"),
        }
    }

    #[test]
    fn test_unknown_event_type_skipped() {
        let line = r#"{"type":"metrics","cpu":50}"#;
        let result: Result<SyncEvent, _> = serde_json::from_str(line);
        assert!(result.is_err(), "Unknown type should fail to parse");
    }

    #[test]
    fn test_malformed_json_skipped() {
        let line = "not json at all";
        let result: Result<SyncEvent, _> = serde_json::from_str(line);
        assert!(result.is_err());
    }

    #[test]
    fn test_empty_line_skipped() {
        let line = "";
        let result: Result<SyncEvent, _> = serde_json::from_str(line);
        assert!(result.is_err());
    }

    #[test]
    fn test_sync_handle_constant() {
        assert_eq!(SYNC_HANDLE, "hq-sync");
    }

    #[test]
    fn test_runner_bin_constant() {
        assert_eq!(RUNNER_BIN, "hq-sync-runner");
    }

    // ── RunTotals ────────────────────────────────────────────────────────

    use crate::events::{SyncAllCompleteEvent, SyncCompleteEvent, SyncProgressEvent};

    fn complete(company: &str, conflicts: u32, aborted: bool) -> SyncEvent {
        SyncEvent::Complete(SyncCompleteEvent {
            company: company.to_string(),
            files_downloaded: 0,
            bytes_downloaded: 0,
            files_skipped: 0,
            conflicts,
            aborted,
        })
    }

    #[test]
    fn test_run_totals_default_is_zero() {
        let t = RunTotals::default();
        assert_eq!(t.conflicts, 0);
    }

    #[test]
    fn test_accumulate_ignores_setup_needed() {
        let mut t = RunTotals::default();
        t.accumulate(&SyncEvent::SetupNeeded);
        assert_eq!(t.conflicts, 0);
    }

    #[test]
    fn test_accumulate_ignores_progress() {
        let mut t = RunTotals::default();
        t.accumulate(&SyncEvent::Progress(SyncProgressEvent {
            company: "x".to_string(),
            path: "y".to_string(),
            bytes: 0,
            message: None,
        }));
        assert_eq!(t.conflicts, 0);
    }

    #[test]
    fn test_accumulate_ignores_all_complete() {
        let mut t = RunTotals { conflicts: 4 };
        t.accumulate(&SyncEvent::AllComplete(SyncAllCompleteEvent {
            companies_attempted: 1,
            files_downloaded: 0,
            bytes_downloaded: 0,
            errors: vec![],
        }));
        // AllComplete is the signal to read, not accumulate — totals unchanged.
        assert_eq!(t.conflicts, 4);
    }

    #[test]
    fn test_accumulate_sums_conflicts_across_completes() {
        let mut t = RunTotals::default();
        t.accumulate(&complete("a", 3, false));
        t.accumulate(&complete("b", 2, true)); // aborted companies still contribute
        assert_eq!(t.conflicts, 5);
    }

    #[test]
    fn test_accumulate_zero_conflicts_is_noop() {
        let mut t = RunTotals { conflicts: 10 };
        t.accumulate(&complete("a", 0, false));
        assert_eq!(t.conflicts, 10);
    }

    #[test]
    fn test_accumulate_saturates_on_overflow() {
        let mut t = RunTotals {
            conflicts: u32::MAX,
        };
        t.accumulate(&complete("a", 1, false));
        assert_eq!(t.conflicts, u32::MAX);
    }
}
