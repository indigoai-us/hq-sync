//! Feature-flagged daemon lifecycle — V2 prep.
//!
//! Wraps `hq sync start` / `hq sync stop` as Tauri commands.
//! Behind `AUTOSTART_DAEMON` feature flag in ~/.hq/menubar.json (default false).
//! Svelte UI does NOT expose these V1 — invocable only via Tauri devtools.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};

use crate::commands::config::MenubarPrefs;
use crate::commands::process::{
    cancel_process_impl, deregister_process, run_process_impl, try_register_handle, ProcessEvent,
    SpawnArgs,
};
use crate::commands::status::{journal_for_sync_complete, write_journal};
use crate::commands::sync::RunTotals;
use crate::events::{SyncEvent, EVENT_SYNC_ALL_COMPLETE};
use crate::util::logfile::log;
use crate::util::paths;

// ─────────────────────────────────────────────────────────────────────────────
// Constants
// ─────────────────────────────────────────────────────────────────────────────

/// Singleton handle for daemon process.
const DAEMON_HANDLE: &str = "hq-sync-daemon";

/// SIGKILL delay after SIGTERM when stopping daemon.
const SIGKILL_DELAY: Duration = Duration::from_secs(5);

// ─────────────────────────────────────────────────────────────────────────────
// Types
// ─────────────────────────────────────────────────────────────────────────────

/// Daemon status response for the frontend.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DaemonStatus {
    pub running: bool,
    pub pid: Option<u32>,
    pub started_at: Option<String>,
    pub watch_path: Option<String>,
    pub source: String, // "pid_file", "daemon_json", or "none"
}

/// Structure of .hq-sync-daemon.json written by `hq sync start`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DaemonJson {
    pub pid: Option<u32>,
    pub started_at: Option<String>,
    pub watch_path: Option<String>,
}

// ─────────────────────────────────────────────────────────────────────────────
// Config resolution (same pattern as sync.rs and status.rs)
// ─────────────────────────────────────────────────────────────────────────────

/// Resolve the HQ folder path by reading config.json and menubar.json directly.
fn resolve_hq_folder_path() -> Result<String, String> {
    let menubar_path = paths::menubar_json_path()?;

    let menubar_prefs: Option<MenubarPrefs> = if menubar_path.exists() {
        std::fs::read_to_string(&menubar_path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
    } else {
        None
    };

    // Use the shared lenient reader so the policy is uniform across all
    // four `resolve_hq_folder_path` duplicates: parse failures fall
    // through to menubar.json + the 4-tier resolver, but real IO errors
    // (permission denied, transient FS failure) still propagate as Err.
    // Without this, silently swallowing read errors could route sync at
    // the wrong HQ folder when config.json is the only source of
    // `hqFolderPath`.
    let config = crate::commands::config::read_hq_config_lenient()?;

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
// SpawnArgs builders (testable)
// ─────────────────────────────────────────────────────────────────────────────

/// Build SpawnArgs for the Auto-sync watcher: hq-sync-runner in watch mode,
/// fanned out across every membership the caller has.
///
/// Mirrors `build_sync_spawn_args` (manual Sync Now) and adds:
///   - `--watch` — runner stays alive after the first pass
///   - `--poll-remote-ms 600000` — pulls remote changes every 10 minutes
///   - `--event-push` — when the user's Instant-sync setting is ON (Phase 2 GA)
///
/// As of hq-cloud 5.26 the runner's chokidar watcher is real. Phase 2 GA
/// (2026-05-23) opened event-driven push to ALL users: we append `--event-push`
/// (requires `--watch`, always set) whenever the user's Instant-sync setting is
/// ON — which it is by default. Local edits then upload within seconds of the
/// filesystem event. Toggling Instant-sync OFF drops back to poll-only without
/// disabling Auto-sync.
///
/// Instant-sync OFF stays poll-only: the remote→local pull runs on the 10-minute
/// cadence and a local push waits for the next pass — there is no second-by-second
/// upload of local edits. (The remote→local pull is ALWAYS poll-driven for now —
/// the real-time pull-on-event receiver is dormant until the server-side per-
/// client queue is provisioned — so the 10-minute poll stays regardless.)
/// Conflict policy is `keep` (skip-and-surface) — local
/// edits win and the conflict store routes them through the existing modal so
/// auto-pull never clobbers an in-progress resolution.

/// Pure decision: should the watch runner get `--event-push`?
///
/// As of Phase 2 GA (2026-05-23) eligibility is universal, so this effectively
/// reduces to "is the user's Instant-sync setting ON?". Kept as a pure
/// `(eligible, instant_sync) -> bool` so the decision stays unit-testable and a
/// future targeted re-gate (flip `event_push_eligible`) works without touching
/// this logic.
fn should_event_push(eligible: bool, instant_sync: bool) -> bool {
    eligible && instant_sync
}

/// Resolve whether the signed-in user is eligible for event-driven push.
///
/// Phase 2 (2026-05-23): event-driven push is GA — every signed-in user is
/// eligible. The per-user Instant-sync setting (`is_instant_sync_enabled`,
/// default-on) is now the sole gate. Kept as a function (rather than inlining
/// `true` at the call site) so the `should_event_push` seam stays intact and a
/// future targeted re-gate is a one-line change here.
fn event_push_eligible() -> bool {
    true
}

pub fn build_watch_runner_args(hq_folder_path: &str) -> SpawnArgs {
    use crate::commands::sync::{HQ_CLOUD_PACKAGE, HQ_CLOUD_VERSION, RUNNER_BIN};

    let mut env = HashMap::new();
    env.insert("HQ_ROOT".to_string(), hq_folder_path.to_string());
    // GUI-launched Tauri apps inherit a minimal launchd PATH and otherwise
    // can't find node/npx. See paths::child_path.
    env.insert("PATH".to_string(), paths::child_path());

    // Dev override: HQ_CLOUD_DEV_POLL_MS lets us tighten the 10-minute
    // production cadence for hands-on testing. Defaults to 600_000ms.
    let poll_ms = std::env::var("HQ_CLOUD_DEV_POLL_MS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .filter(|&n| n > 0)
        .unwrap_or(600_000);

    let mut runner_args = vec![
        "--companies".to_string(),
        "--direction".to_string(),
        "both".to_string(),
        "--on-conflict".to_string(),
        "keep".to_string(),
        "--hq-root".to_string(),
        hq_folder_path.to_string(),
        "--watch".to_string(),
        "--poll-remote-ms".to_string(),
        poll_ms.to_string(),
    ];

    // Phase 2 GA: event-driven push is gated solely by the user's Instant-sync
    // setting (eligibility is now universal — see `event_push_eligible`). The
    // hq-cloud runner requires --watch for --event-push (already set above), so
    // appending here is safe for both spawn paths below.
    if should_event_push(event_push_eligible(), is_instant_sync_enabled()) {
        runner_args.push("--event-push".to_string());
    }

    // Dev override: HQ_CLOUD_LOCAL_RUNNER points at a built sync-runner.js
    // (e.g. /…/hq/packages/hq-cloud/dist/bin/sync-runner.js). Lets us
    // exercise unreleased runner changes before the version is published
    // to npm; production falls through to the npx-pinned path below.
    if let Ok(local_runner) = std::env::var("HQ_CLOUD_LOCAL_RUNNER") {
        if !local_runner.is_empty() {
            let mut args = vec![local_runner];
            args.extend(runner_args);
            return SpawnArgs {
                cmd: paths::resolve_bin("node"),
                args,
                cwd: None,
                env: Some(env),
            };
        }
    }

    let mut args = vec![
        "-y".to_string(),
        format!("--package={}@{}", HQ_CLOUD_PACKAGE, HQ_CLOUD_VERSION),
        RUNNER_BIN.to_string(),
    ];
    args.extend(runner_args);

    SpawnArgs {
        cmd: paths::resolve_bin("npx"),
        args,
        cwd: None,
        env: Some(env),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────────────────────

/// Check if a PID is alive using kill(0).
///
/// Note: kill(0) checks if the calling user has permission to signal the PID.
/// If the original process died and a different process reused the PID, this
/// may return a false positive. Acceptable for V2 prep — daemon.json cross-check
/// can be added in V2 if PID reuse becomes an issue.
fn is_pid_alive(pid: u32) -> bool {
    use nix::sys::signal;
    use nix::unistd::Pid;
    signal::kill(Pid::from_raw(pid as i32), None).is_ok()
}

/// Read .hq-sync.pid file from the HQ folder.
fn read_pid_file(hq_folder_path: &str) -> Option<u32> {
    let pid_path = PathBuf::from(hq_folder_path).join(".hq-sync.pid");
    std::fs::read_to_string(&pid_path)
        .ok()
        .and_then(|s| s.trim().parse::<u32>().ok())
}

/// Read .hq-sync-daemon.json from the HQ folder.
fn read_daemon_json(hq_folder_path: &str) -> Option<DaemonJson> {
    let json_path = PathBuf::from(hq_folder_path).join(".hq-sync-daemon.json");
    std::fs::read_to_string(&json_path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
}

/// Check if autostart_daemon flag is enabled in menubar.json.
pub fn is_autostart_enabled() -> bool {
    read_menubar_bool(|p| p.autostart_daemon, false)
}

/// Check if the user-facing Auto-sync flag is enabled in menubar.json.
/// Both flags trigger the same daemon — `autostart_daemon` is the V2-prep
/// devtools flag and `realtime_sync` is the user-facing Settings toggle —
/// but they're kept separate so each can evolve independently.
///
/// Defaults to true when the field is missing so fresh installs auto-sync
/// without the user having to discover the Settings toggle. An explicit
/// `false` written by `save_settings` still wins.
pub fn is_realtime_sync_enabled() -> bool {
    read_menubar_bool(|p| p.realtime_sync, true)
}

/// Check if the user-facing Instant-sync (event-driven) flag is enabled in
/// menubar.json.
///
/// Defaults to true when the field is missing so eligible (@getindigo.ai)
/// users get instant push on a fresh install without discovering the toggle,
/// matching the `realtime_sync` default-on convention. An explicit `false`
/// written by `save_settings` still wins. Note this is only consulted for
/// `event_push_eligible()` users — see `should_event_push`.
pub fn is_instant_sync_enabled() -> bool {
    read_menubar_bool(|p| p.instant_sync, true)
}

fn read_menubar_bool<F: FnOnce(&MenubarPrefs) -> Option<bool>>(field: F, default: bool) -> bool {
    let menubar_path = match paths::menubar_json_path() {
        Ok(p) => p,
        Err(_) => return default,
    };
    if !menubar_path.exists() {
        return default;
    }
    let prefs: Option<MenubarPrefs> = std::fs::read_to_string(&menubar_path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok());
    prefs.and_then(|p| field(&p)).unwrap_or(default)
}

// ─────────────────────────────────────────────────────────────────────────────
// Watch-mode ndjson handler
// ─────────────────────────────────────────────────────────────────────────────

/// Process a single stdout line from `hq-sync-runner --watch`.
///
/// The watcher emits the same ndjson protocol as a manual sync (one full
/// fanout-plan → plan/progress/complete → all-complete cycle per pass).
/// `handle_sync_line` in `sync.rs` owns the rich manual-sync handling
/// (per-file progress events, reconcile, telemetry, sentry captures);
/// here we only do what the popover needs to surface auto-sync to the
/// user — keep the conflict tally up-to-date and, on each pass's
/// AllComplete, write the journal and emit the same `sync:all-complete`
/// event the frontend already listens for.
///
/// Failing to parse a line is non-fatal: blank lines arrive at runner
/// teardown, and any unknown variant the runner adds in the future
/// should not kill the watcher.
fn handle_watch_stdout_line(
    app: &AppHandle,
    hq_folder: &str,
    totals: &Mutex<RunTotals>,
    line: &str,
) {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return;
    }
    let event: SyncEvent = match serde_json::from_str(trimmed) {
        Ok(e) => e,
        Err(_) => return,
    };
    {
        let mut t = totals.lock().unwrap_or_else(|e| e.into_inner());
        t.accumulate(&event);
    }
    // Record each per-file transfer into the session activity log (Recent
    // Changes window). The watch daemon is the primary instant-sync path, so
    // without this the activity log would only ever capture foreground
    // "Sync Now" runs (handle_sync_line) and stay empty in normal use.
    if let SyncEvent::Progress(payload) = &event {
        crate::commands::activity::record_progress(app, payload);
    }
    if let SyncEvent::AllComplete(payload) = &event {
        let conflicts = {
            let t = totals.lock().unwrap_or_else(|e| e.into_inner());
            t.conflicts
        };
        let now_iso = chrono::Utc::now()
            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        let journal = journal_for_sync_complete(&now_iso, conflicts);
        if let Err(e) = write_journal(hq_folder, &journal) {
            log("daemon", &format!("failed to write journal: {e}"));
        }
        log("daemon", &format!("all-complete (conflicts={conflicts})"));
        // Mirror to a git repo at the HQ root (if any). Fire-and-forget so
        // a slow `git push` can't stall the next watch pass; the mirror's
        // in-flight guard skips overlapping runs.
        crate::commands::git_mirror::spawn_mirror_after_sync(hq_folder);
        let _ = app.emit(EVENT_SYNC_ALL_COMPLETE, payload.clone());
        // Reset for the next pass — watch mode loops indefinitely.
        *totals.lock().unwrap_or_else(|e| e.into_inner()) = RunTotals::default();
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tauri commands
// ─────────────────────────────────────────────────────────────────────────────

/// Start the sync daemon via `hq sync start`.
///
/// Pre-flight: checks PID file to see if a daemon is already running from a
/// previous app session. If alive, returns an error without spawning.
///
/// Spawns the daemon subprocess in the background. The daemon writes its own
/// .hq-sync.pid and .hq-sync-daemon.json files. This command returns immediately
/// after spawning.
///
/// Returns the handle string on success.
#[tauri::command]
pub fn start_daemon(app: AppHandle) -> Result<String, String> {
    if !try_register_handle(DAEMON_HANDLE) {
        return Err("Daemon is already starting".to_string());
    }

    let hq_folder_path = match resolve_hq_folder_path() {
        Ok(p) => p,
        Err(e) => {
            deregister_process(DAEMON_HANDLE);
            return Err(e);
        }
    };

    // Pre-flight: check if daemon is already running from a previous session
    if let Some(pid) = read_pid_file(&hq_folder_path) {
        if is_pid_alive(pid) {
            deregister_process(DAEMON_HANDLE);
            return Err(format!(
                "Daemon is already running (PID {})",
                pid
            ));
        }
    }

    let spawn_args = build_watch_runner_args(&hq_folder_path);

    log("daemon", "spawn: hq-sync-runner --watch");

    // Per-pass totals. Watch mode emits a full Complete/AllComplete cycle on
    // every chokidar tick + every 10-minute poll, so we reset on each
    // AllComplete instead of accumulating forever.
    let totals: Arc<Mutex<RunTotals>> = Arc::new(Mutex::new(RunTotals::default()));
    let hq_folder = hq_folder_path.clone();

    thread::spawn(move || {
        let result = run_process_impl(DAEMON_HANDLE, &spawn_args, move |event| {
            // Surface stderr and non-success exits unconditionally — they
            // are the only signals the user has when the watcher dies
            // (e.g. "Unknown argument: --watch" on a stale runner pin).
            // Stdout is parsed for ndjson SyncEvents so each watcher pass
            // updates `.hq-sync-journal.json` and refreshes the popover's
            // "Last synced" stat — without that, the UI only ever showed
            // the timestamp of the last manual `Sync Now` click.
            match event {
                ProcessEvent::Stdout(line) => {
                    handle_watch_stdout_line(&app, &hq_folder, &totals, &line);
                }
                ProcessEvent::Stderr(line) => {
                    log("daemon.stderr", &line);
                }
                ProcessEvent::Exit {
                    code,
                    signal,
                    success,
                } => {
                    log(
                        "daemon",
                        &format!(
                            "exited: code={:?} signal={:?} success={}",
                            code, signal, success
                        ),
                    );
                }
            }
        });

        if let Err(e) = result {
            log("daemon", &format!("spawn failed: {e}"));
        }
    });

    Ok(DAEMON_HANDLE.to_string())
}

/// Stop the sync daemon via SIGTERM (graceful) → SIGKILL (timeout fallback).
///
/// Returns `true` if a stop was initiated. The watcher process owns its own
/// pid-file lifecycle; we don't shell out to a separate stop CLI here.
#[tauri::command]
pub fn stop_daemon() -> Result<bool, String> {
    let hq_folder_path = resolve_hq_folder_path()?;

    // Cancel via the process registry first — this signals the spawned
    // runner from `start_daemon` and cleans up the handle.
    let cancelled = cancel_process_impl(DAEMON_HANDLE, SIGKILL_DELAY);
    if cancelled {
        return Ok(true);
    }

    // Daemon from a previous app session — registry has no handle, but the
    // pid-file may point at a still-alive runner. SIGTERM directly so the
    // user can re-toggle Auto-sync without a process zombie.
    if let Some(pid) = read_pid_file(&hq_folder_path) {
        if is_pid_alive(pid) {
            use nix::sys::signal::{self, Signal};
            use nix::unistd::Pid;
            let _ = signal::kill(Pid::from_raw(pid as i32), Signal::SIGTERM);
            return Ok(true);
        }
    }

    Ok(false)
}

/// Get daemon status by reading .hq-sync.pid and .hq-sync-daemon.json.
///
/// Does NOT shell out to `hq` — reads filesystem state directly for speed.
#[tauri::command]
pub fn daemon_status() -> Result<DaemonStatus, String> {
    let hq_folder_path = resolve_hq_folder_path()?;

    // Try .hq-sync-daemon.json first (richer info)
    if let Some(daemon) = read_daemon_json(&hq_folder_path) {
        let pid = daemon.pid.or_else(|| read_pid_file(&hq_folder_path));
        let running = pid.map(is_pid_alive).unwrap_or(false);
        return Ok(DaemonStatus {
            running,
            pid,
            started_at: daemon.started_at,
            watch_path: daemon.watch_path,
            source: "daemon_json".to_string(),
        });
    }

    // Fallback to .hq-sync.pid
    if let Some(pid) = read_pid_file(&hq_folder_path) {
        let running = is_pid_alive(pid);
        return Ok(DaemonStatus {
            running,
            pid: Some(pid),
            started_at: None,
            watch_path: None,
            source: "pid_file".to_string(),
        });
    }

    // No daemon state files found
    Ok(DaemonStatus {
        running: false,
        pid: None,
        started_at: None,
        watch_path: None,
        source: "none".to_string(),
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── DaemonStatus serialization ───────────────────────────────────────

    #[test]
    fn test_daemon_status_serializes_camel_case() {
        let status = DaemonStatus {
            running: true,
            pid: Some(12345),
            started_at: Some("2026-04-18T12:00:00Z".to_string()),
            watch_path: Some("/Users/test/HQ".to_string()),
            source: "daemon_json".to_string(),
        };
        let json = serde_json::to_string(&status).unwrap();
        assert!(json.contains("\"startedAt\""));
        assert!(json.contains("\"watchPath\""));
        assert!(!json.contains("\"started_at\""));
        assert!(!json.contains("\"watch_path\""));
    }

    #[test]
    fn test_daemon_status_roundtrip() {
        let status = DaemonStatus {
            running: true,
            pid: Some(12345),
            started_at: Some("2026-04-18T12:00:00Z".to_string()),
            watch_path: Some("/Users/test/HQ".to_string()),
            source: "daemon_json".to_string(),
        };
        let json = serde_json::to_string(&status).unwrap();
        let parsed: DaemonStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(status, parsed);
    }

    #[test]
    fn test_daemon_status_default_none() {
        let status = DaemonStatus {
            running: false,
            pid: None,
            started_at: None,
            watch_path: None,
            source: "none".to_string(),
        };
        let json = serde_json::to_string(&status).unwrap();
        assert!(json.contains("\"running\":false"));
        assert!(json.contains("\"pid\":null"));
        assert!(json.contains("\"startedAt\":null"));
        assert!(json.contains("\"watchPath\":null"));
        assert!(json.contains("\"source\":\"none\""));
    }

    // ── DaemonJson deserialization ───────────────────────────────────────

    #[test]
    fn test_daemon_json_deserialize_full() {
        let json = r#"{
            "pid": 42,
            "startedAt": "2026-04-18T10:30:00Z",
            "watchPath": "/Users/test/HQ"
        }"#;
        let daemon: DaemonJson = serde_json::from_str(json).unwrap();
        assert_eq!(daemon.pid, Some(42));
        assert_eq!(daemon.started_at, Some("2026-04-18T10:30:00Z".to_string()));
        assert_eq!(daemon.watch_path, Some("/Users/test/HQ".to_string()));
    }

    #[test]
    fn test_daemon_json_deserialize_minimal() {
        let json = r#"{}"#;
        let daemon: DaemonJson = serde_json::from_str(json).unwrap();
        assert_eq!(daemon.pid, None);
        assert_eq!(daemon.started_at, None);
        assert_eq!(daemon.watch_path, None);
    }

    #[test]
    fn test_daemon_json_deserialize_partial() {
        let json = r#"{"pid": 99}"#;
        let daemon: DaemonJson = serde_json::from_str(json).unwrap();
        assert_eq!(daemon.pid, Some(99));
        assert_eq!(daemon.started_at, None);
        assert_eq!(daemon.watch_path, None);
    }

    // ── is_pid_alive ──────────────────────────────────────────────────────

    #[test]
    fn test_is_pid_alive_current_process() {
        // Current process should always be alive
        let pid = std::process::id();
        assert!(is_pid_alive(pid));
    }

    #[test]
    fn test_is_pid_alive_invalid_pid() {
        // PID 0 is the kernel — kill(0) should fail for a regular user process
        // PID 4_000_000 is unlikely to exist on any system
        assert!(!is_pid_alive(4_000_000));
    }

    // ── is_autostart_enabled ─────────────────────────────────────────────

    #[test]
    fn test_is_autostart_enabled_does_not_panic() {
        // This test relies on the real menubar.json path. If the file
        // doesn't exist or doesn't have autostartDaemon=true, it returns false.
        // On CI / clean machines this will always be false.
        let _result = is_autostart_enabled();
        // Function should not panic regardless of filesystem state
    }

    // ── Double-start prevention ──────────────────────────────────────────

    #[test]
    fn test_double_register_prevented() {
        use crate::commands::process::{try_register_handle, deregister_process};
        let handle = "test-daemon-double-start";
        // First register succeeds
        assert!(try_register_handle(handle));
        // Second register fails (already registered)
        assert!(!try_register_handle(handle));
        // Cleanup
        deregister_process(handle);
        // After cleanup, register succeeds again
        assert!(try_register_handle(handle));
        deregister_process(handle);
    }

    // ── Constants ────────────────────────────────────────────────────────

    #[test]
    fn test_daemon_handle_constant() {
        assert_eq!(DAEMON_HANDLE, "hq-sync-daemon");
    }

    #[test]
    fn test_sigkill_delay_constant() {
        assert_eq!(SIGKILL_DELAY, Duration::from_secs(5));
    }

    // ── build_watch_runner_args (Auto-sync) ───────────────────────────────
    //
    // Auto-sync reuses the same hq-sync-runner binary as the manual Sync Now
    // button (see commands/sync.rs::build_sync_spawn_args), but adds:
    //   --watch                  — keep the runner alive after the first pass
    //   --poll-remote-ms 600000  — pull from S3 every 10 minutes
    //
    // Conflict policy stays `keep` (skip-and-surface) — local edits win and
    // the conflict store routes them through the existing modal. Direction
    // stays `both`. Companies stays fanned out (`--companies`).

    #[test]
    fn test_build_watch_runner_args_uses_npx_runner() {
        let args = build_watch_runner_args("/Users/test/HQ");
        // Resolved npx path; varies by machine. Asserting it ends with "npx"
        // avoids hard-coding /opt/homebrew/bin vs ~/.npm-global/bin.
        assert!(
            args.cmd.ends_with("npx"),
            "expected resolved npx path, got: {}",
            args.cmd
        );
    }

    #[test]
    fn test_build_watch_runner_args_pins_hq_cloud_package() {
        use crate::commands::sync::{HQ_CLOUD_PACKAGE, HQ_CLOUD_VERSION};
        let args = build_watch_runner_args("/any");
        let expected_pin = format!("--package={}@{}", HQ_CLOUD_PACKAGE, HQ_CLOUD_VERSION);
        assert!(
            args.args.contains(&expected_pin),
            "expected pinned --package= flag, got: {:?}",
            args.args
        );
        assert!(args.args.contains(&"-y".to_string()));
        assert!(args.args.contains(&"hq-sync-runner".to_string()));
    }

    #[test]
    fn test_build_watch_runner_args_includes_watch_and_poll_interval() {
        let args = build_watch_runner_args("/any");
        assert!(args.args.contains(&"--watch".to_string()));
        let poll_idx = args
            .args
            .iter()
            .position(|a| a == "--poll-remote-ms")
            .expect("--poll-remote-ms flag missing");
        assert_eq!(
            args.args.get(poll_idx + 1).map(|s| s.as_str()),
            Some("600000"),
            "expected 10-minute (600000ms) poll interval"
        );
    }

    #[test]
    fn test_build_watch_runner_args_fans_out_to_all_companies() {
        // Auto-sync mirrors the manual Sync Now button: --companies, not a
        // single --company. Bidirectional, conflict-keep.
        let args = build_watch_runner_args("/any");
        assert!(args.args.contains(&"--companies".to_string()));
        assert!(!args.args.iter().any(|a| a == "--company"));

        let dir_idx = args
            .args
            .iter()
            .position(|a| a == "--direction")
            .expect("--direction flag missing");
        assert_eq!(args.args.get(dir_idx + 1).map(|s| s.as_str()), Some("both"));

        let conflict_idx = args
            .args
            .iter()
            .position(|a| a == "--on-conflict")
            .expect("--on-conflict flag missing");
        assert_eq!(
            args.args.get(conflict_idx + 1).map(|s| s.as_str()),
            Some("keep")
        );
    }

    #[test]
    fn test_build_watch_runner_args_passes_hq_root() {
        let args = build_watch_runner_args("/Users/test/HQ");
        let root_idx = args
            .args
            .iter()
            .position(|a| a == "--hq-root")
            .expect("--hq-root flag missing");
        assert_eq!(
            args.args.get(root_idx + 1).map(|s| s.as_str()),
            Some("/Users/test/HQ")
        );
    }

    #[test]
    fn test_build_watch_runner_args_env_carries_hq_root_and_path() {
        // Mirrors build_sync_spawn_args: HQ_ROOT for defense-in-depth and
        // PATH so Dock-launched apps can resolve node/npx (see paths::child_path).
        let args = build_watch_runner_args("/Users/test/HQ");
        let env = args.env.expect("env should be populated");
        assert_eq!(
            env.get("HQ_ROOT").map(String::as_str),
            Some("/Users/test/HQ")
        );
        assert!(
            env.get("PATH").map(|p| !p.is_empty()).unwrap_or(false),
            "PATH must be set so Dock-launched Tauri apps can find node/npx"
        );
    }

    // ── event-push gating (Phase 2 GA) ─────────────────────────────────────
    //
    // Phase 2 GA (2026-05-23): eligibility is universal (`event_push_eligible`
    // => true), so --event-push is appended whenever the user's Instant-sync
    // setting is ON. The pure `should_event_push` still models the
    // (eligible × setting) AND, so a future targeted re-gate is a one-liner.

    #[test]
    fn test_event_push_eligible_is_universal_phase2_ga() {
        // GA: every signed-in user is eligible — no token/email required.
        assert!(event_push_eligible());
    }

    #[test]
    fn test_should_event_push_eligible_and_instant_on_pushes() {
        // (i) Instant-sync ON + eligible => event-driven push.
        assert!(should_event_push(true, true));
    }

    #[test]
    fn test_should_event_push_eligible_but_instant_off_is_poll_only() {
        // (ii) Instant-sync OFF => poll-only, no --event-push.
        assert!(!should_event_push(true, false));
    }

    #[test]
    fn test_should_event_push_ineligible_never_pushes_regardless_of_setting() {
        // (iii) The seam still holds: were eligibility ever re-gated to false,
        // the Instant-sync setting could not override it.
        assert!(!should_event_push(false, true));
        assert!(!should_event_push(false, false));
    }
}
