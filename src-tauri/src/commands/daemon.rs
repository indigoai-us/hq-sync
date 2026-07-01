//! Feature-flagged daemon lifecycle — V2 prep.
//!
//! Wraps `hq sync start` / `hq sync stop` as Tauri commands.
//! Behind `AUTOSTART_DAEMON` feature flag in ~/.hq/menubar.json (default false).
//! Svelte UI does NOT expose these V1 — invocable only via Tauri devtools.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

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
///   - `--poll-remote-ms 15000` — pulls remote changes every 15 seconds (fixed)
///   - `--event-push` — when the user's Instant-sync setting is ON (Phase 2 GA)
///
/// As of hq-cloud 5.26 the runner's chokidar watcher is real. Phase 2 GA
/// (2026-05-23) opened event-driven push to ALL users: we append `--event-push`
/// (requires `--watch`, always set) whenever the user's Instant-sync setting is
/// ON — which it is by default. Local edits then upload within seconds of the
/// filesystem event. Toggling Instant-sync OFF drops back to poll-only without
/// disabling Auto-sync.
///
/// Instant-sync OFF stays poll-only: the remote→local pull runs on the 15-second
/// cadence and a local push waits for the next pass — there is no second-by-second
/// upload of local edits. (The remote→local pull is poll-driven for most users.
/// The server side shipped in hq-pro US-015/US-016 — `POST /v1/sync/subscribe`
/// mints a per-device SQS queue and vends scoped receive credentials — and as
/// of hq-cloud ≥6.3.1 the runner brings up real event-driven pull INSIDE
/// `--event-push` for accounts enrolled in its Phase 3 rollout gate
/// (`resolveEventSync`, exact-email allowlist + `HQ_SYNC_EVENT_SYNC` override);
/// no new menubar flag is involved. The 15-second poll stays regardless, as
/// the correctness backstop.)
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

    // Remote-pull cadence, fixed at 15 seconds. event-push + event-sync handle
    // real-time propagation; this poll is only the correctness backstop. It is
    // intentionally NOT user-configurable.
    const SYNC_POLL_REMOTE_MS: u64 = 15_000;
    let poll_ms = SYNC_POLL_REMOTE_MS;

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

    // Runner-resolution preflight (HQ-SYNC-E). Before spawning the watcher,
    // confirm the runner's Node/npx interpreter actually resolves. When it
    // doesn't, the old spawn fell through to a bare shell and exited 127
    // (`sh: hq-sync-runner: command not found`), and the supervisor hot-respawned
    // it into a crash-loop. Bail here instead: surface one clear, user-actionable
    // message (rate-limited like the crash path so a persistent miss ships
    // ~log2(N) events, not one per 30s respawn) and DON'T spawn a doomed runner.
    if let Some(msg) = crate::commands::sync::preflight_runner_unresolvable() {
        let consecutive = note_runner_unresolvable();
        if should_capture_crash(consecutive) {
            crate::commands::sync::capture_sync_error(
                None,
                "(auto-sync)",
                &format!(
                    "auto-sync watcher cannot start: {msg} \
                     (consecutive #{consecutive}, further repeats rate-limited)"
                ),
            );
        } else {
            log(
                "daemon",
                &format!("runner unresolvable #{consecutive} — capture rate-limited"),
            );
        }
        deregister_process(DAEMON_HANDLE);
        return Err(msg);
    }

    let spawn_args = build_watch_runner_args(&hq_folder_path);

    log("daemon", "spawn: hq-sync-runner --watch");

    // Stamp the spawn time for crash-loop dampening (HQ-SYNC-4): the Exit
    // handler uses it to tell a fast crash-loop failure from a watcher that ran
    // healthily then died, and the supervisor uses it to reset backoff once a
    // respawn survives.
    note_watcher_spawned();

    // Per-pass totals. Watch mode emits a full Complete/AllComplete cycle on
    // every chokidar tick + every 15-second poll, so we reset on each
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
                    // Accumulate as a Sentry breadcrumb so a crash capture at
                    // the Exit arm below ships with the runner's last words.
                    sentry::add_breadcrumb(sentry::Breadcrumb {
                        category: Some("daemon.stderr".into()),
                        level: sentry::Level::Warning,
                        message: Some(line.clone()),
                        ..Default::default()
                    });
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
                    // Auto-sync runs unattended, so a crashed watcher was
                    // previously invisible (log-only). Capture genuine crashes
                    // to #hq-alerts — but NOT a deliberate stop (SIGTERM from
                    // cancel_process_impl / app-quit teardown / auto-sync-off /
                    // re-spawn). See `is_unexpected_watcher_exit` for why a bare
                    // SIGTERM is never treated as a crash (HQ-SYNC-5).
                    let cancelled = crate::commands::process::is_cancelled(DAEMON_HANDLE);
                    if is_unexpected_watcher_exit(success, signal, cancelled) {
                        // Crash-loop dampening (HQ-SYNC-4): advance the loop
                        // counter (driving the supervisor's respawn backoff) and
                        // rate-limit the capture so an ongoing failure ships
                        // ~log2(N) actionable events, not one per respawn (the
                        // 36,977-event fleet flood). The first crash still alerts.
                        let consecutive = note_watcher_crashed();
                        // Enrich with uptime + last-sampled RSS so a `signal=9`
                        // can be told apart later — jetsam/OOM vs a manual/OS
                        // kill (HQ-SYNC-F). Context only; never gates capture.
                        let (uptime, rss_kb, rss_age) = watcher_exit_diagnostics();
                        let diag = exit_diagnostic_suffix(uptime, rss_kb, rss_age);
                        if should_capture_crash(consecutive) {
                            crate::commands::sync::capture_sync_error(
                                None,
                                "(auto-sync)",
                                &format!(
                                    "auto-sync watcher exited unexpectedly (code={:?} signal={:?}); \
                                     consecutive failure #{consecutive} (further repeats rate-limited){diag}",
                                    code, signal
                                ),
                            );
                        } else {
                            log(
                                "daemon",
                                &format!(
                                    "watcher crash #{consecutive} — capture rate-limited (code={:?} signal={:?}){diag}",
                                    code, signal
                                ),
                            );
                        }
                    }
                }
            }
        });

        if let Err(e) = result {
            log("daemon", &format!("spawn failed: {e}"));
            // The watcher never started — Sync is silently dead until restart.
            crate::commands::sync::capture_sync_error(
                None,
                "(auto-sync)",
                &format!("auto-sync watcher failed to spawn: {e}"),
            );
        }
    });

    Ok(DAEMON_HANDLE.to_string())
}

/// Settle delay before the supervisor's first check (let the launch-time
/// `start_daemon` run first) and the interval between checks thereafter.
const SUPERVISOR_SETTLE: Duration = Duration::from_secs(30);
const SUPERVISOR_INTERVAL: Duration = Duration::from_secs(30);

/// Pure decision for the supervisor: respawn the watch daemon iff auto-sync
/// should be on (the user-facing realtime-sync toggle or the autostart devtools
/// flag) AND it isn't currently alive. Extracted (like `should_event_push`) so
/// the decision stays unit-testable.
fn should_respawn_daemon(realtime_sync: bool, autostart: bool, daemon_alive: bool) -> bool {
    (realtime_sync || autostart) && !daemon_alive
}

/// SIGTERM that the watcher process can receive on a deliberate stop. Extracted
/// as a named constant so the crash-vs-teardown decision reads intentionally.
const SIGTERM: i32 = 15;

/// Pure decision: should this watcher exit be Sentry-captured as an unexpected
/// crash? Extracted (like `should_respawn_daemon`) so the rule stays
/// unit-testable.
///
/// A genuine crash is a non-zero `exit(code)` or a fault signal
/// (SIGSEGV/SIGABRT/SIGBUS = real bug, SIGKILL = OOM/`kill -9`). A bare
/// **SIGTERM is never a crash** — it is the canonical "please stop" request and
/// always originates from a deliberate teardown: our own `cancel_process_impl`,
/// the app-quit `terminate_pids_for_exit` path, the OS on logout/shutdown, or a
/// manual `kill <pid>`. Capturing it as a fatal "watcher exited unexpectedly"
/// was the HQ-SYNC-5 false-positive flood (signal=15, code=None on app quit).
///
/// The `cancelled` flag (from the process registry) is the primary guard for
/// our own stop paths; the explicit `signal != SIGTERM` check is defense in
/// depth that also covers external SIGTERMs and the narrow race where the
/// registry entry is deregistered before this Exit handler observes it.
fn is_unexpected_watcher_exit(success: bool, signal: Option<i32>, cancelled: bool) -> bool {
    if success || cancelled {
        return false;
    }
    signal != Some(SIGTERM)
}

// ─────────────────────────────────────────────────────────────────────────────
// Crash-loop dampening (HQ-SYNC-4)
// ─────────────────────────────────────────────────────────────────────────────
//
// A watcher that keeps failing (the runner can't upload — `presign put denied` —
// or its exec target isn't runnable: exit 1/2/126) was respawned by the
// supervisor every `SUPERVISOR_INTERVAL` (30s) AND Sentry-captured on EVERY
// exit. Fleet-wide that turned one per-machine failure into a 36,977-event flood
// plus an endless hot-respawn. We dampen BOTH legs without hiding the signal:
// the first crash still alerts, respawns back off exponentially, and the capture
// is rate-limited to ~log2(N) events.

/// A non-zero exit this soon after spawn is a crash-loop failure — distinct from
/// a watcher that ran healthily for a while and then died (which resets the loop
/// counter and is treated as a fresh, captured failure).
const FAST_FAIL_WINDOW: Duration = Duration::from_secs(60);

/// Ceiling for the respawn backoff. A persistently-failing watcher backs off to
/// at most this between respawns, instead of hammering the 30s supervisor cadence.
const RESPAWN_MAX_BACKOFF: Duration = Duration::from_secs(30 * 60);

/// Exponential respawn backoff after `consecutive` consecutive fast failures.
/// `0` → the base supervisor cadence; then ×2 per failure, capped at `cap`.
/// Pure + unit-testable.
fn respawn_backoff(consecutive: u32, base: Duration, cap: Duration) -> Duration {
    if consecutive == 0 {
        return base;
    }
    // Cap the shift so the multiply can't overflow before the `.min(cap)`.
    let mult = 1u64.checked_shl(consecutive.min(32)).unwrap_or(u64::MAX);
    let secs = base.as_secs().saturating_mul(mult).min(cap.as_secs());
    Duration::from_secs(secs)
}

/// Whether to Sentry-capture this crash. Capture the 1st and then only at
/// exponential milestones (1, 2, 4, 8, 16, …) so a crash-loop ships ~log2(N)
/// actionable events instead of one-per-respawn. Pure + unit-testable.
fn should_capture_crash(consecutive: u32) -> bool {
    consecutive <= 1 || consecutive.is_power_of_two()
}

/// A non-zero exit `run` after spawn — is it a fast (crash-loop) failure?
fn is_fast_failure(run: Duration, window: Duration) -> bool {
    run < window
}

/// Shared crash-loop state across the spawn (`start_daemon`), the watcher Exit
/// handler, and the supervisor. Small + mutex-guarded; updated on every spawn,
/// exit, and supervisor tick.
#[derive(Default)]
struct WatcherCrashState {
    /// Consecutive fast failures (crash-loop length). Reset to 0 once a watcher
    /// survives `FAST_FAIL_WINDOW`.
    consecutive: u32,
    /// When the current watcher was spawned — drives the fast-failure decision
    /// and the supervisor's "survived long enough to reset" check.
    spawn_at: Option<Instant>,
    /// The supervisor must not respawn before this instant (backoff window).
    backoff_until: Option<Instant>,
    /// Consecutive runner-resolution preflight failures (HQ-SYNC-E). Tracked
    /// separately from `consecutive` because the preflight bails BEFORE a spawn,
    /// so the spawn-timestamp fast-failure logic doesn't apply — this is a plain
    /// per-episode counter, reset to 0 the moment a watcher actually spawns.
    preflight_fails: u32,
    /// Last RSS (KB) sampled from the live watcher by the supervisor, and when.
    /// Enriches an unexpected-exit capture so a `signal=9` can be told apart
    /// after the fact — a footprint near the machine's memory ceiling points at
    /// jetsam/OOM, a small one at a manual `kill -9` / sleep-or-shutdown kill
    /// (HQ-SYNC-F). Best-effort and diagnostic only: `None` until the first
    /// sample, and it never changes whether a crash is captured. Cleared on each
    /// fresh spawn so a new watcher never inherits the prior one's footprint.
    last_rss_kb: Option<u64>,
    last_rss_at: Option<Instant>,
}

static CRASH_STATE: OnceLock<Mutex<WatcherCrashState>> = OnceLock::new();

fn crash_state() -> &'static Mutex<WatcherCrashState> {
    CRASH_STATE.get_or_init(|| Mutex::new(WatcherCrashState::default()))
}

/// Record that a watcher was just spawned (called from `start_daemon`).
fn note_watcher_spawned() {
    let mut st = crash_state().lock().unwrap();
    st.spawn_at = Some(Instant::now());
    // A spawn means the runner resolved — clear the preflight failure streak so
    // its capture rate-limiting resets for the next episode (HQ-SYNC-E).
    st.preflight_fails = 0;
    // Fresh watcher — drop the previous watcher's RSS sample so a crash capture
    // never reports a stale footprint from a process that already died (HQ-SYNC-F).
    st.last_rss_kb = None;
    st.last_rss_at = None;
}

/// Record a runner-resolution preflight failure (HQ-SYNC-E) and return the
/// consecutive count so the caller can rate-limit the capture. Separate from
/// `note_watcher_crashed` because no spawn occurred — this is a plain counter,
/// not the spawn-timestamp fast-failure machinery.
fn note_runner_unresolvable() -> u32 {
    let mut st = crash_state().lock().unwrap();
    st.preflight_fails = st.preflight_fails.saturating_add(1);
    st.preflight_fails
}

/// Update the crash-loop state on an unexpected watcher exit and return the
/// consecutive-failure count so the caller can decide whether to capture.
fn note_watcher_crashed() -> u32 {
    let mut st = crash_state().lock().unwrap();
    let ran = st.spawn_at.map(|t| t.elapsed()).unwrap_or(Duration::ZERO);
    if is_fast_failure(ran, FAST_FAIL_WINDOW) {
        st.consecutive = st.consecutive.saturating_add(1);
    } else {
        // Ran healthily, then died — not a tight loop. Treat as a fresh first
        // failure: reset the counter to 1 so it is captured and backs off lightly.
        st.consecutive = 1;
    }
    let consecutive = st.consecutive;
    st.backoff_until =
        Some(Instant::now() + respawn_backoff(consecutive, SUPERVISOR_INTERVAL, RESPAWN_MAX_BACKOFF));
    consecutive
}

// ─────────────────────────────────────────────────────────────────────────────
// Exit diagnostics (HQ-SYNC-F): uptime + last-sampled RSS on an unexpected exit
// ─────────────────────────────────────────────────────────────────────────────
//
// A one-off `signal=9` watcher kill is ambiguous on its own — it could be macOS
// jetsam reclaiming a memory-hungry process (a real leak worth fixing) or a
// benign `kill -9` / sleep-or-shutdown termination. Sentry carries no memory
// context, so we enrich the crash capture with the watcher's uptime and the last
// RSS the supervisor sampled while it was alive. This is context only — it never
// suppresses a capture; `signal=9` stays fully alertable.

/// Record the latest RSS (KB) sampled from the live watcher (supervisor tick).
fn note_watcher_rss(kb: u64) {
    let mut st = crash_state().lock().unwrap();
    st.last_rss_kb = Some(kb);
    st.last_rss_at = Some(Instant::now());
}

/// Snapshot for enriching a crash capture: watcher uptime (since spawn), the
/// last RSS sample, and how long before now that sample was taken. Read right
/// after `note_watcher_crashed` (which leaves `spawn_at` untouched). RSS fields
/// are `None` when no sample was ever taken (e.g. a crash within the first
/// supervisor tick).
fn watcher_exit_diagnostics() -> (Option<Duration>, Option<u64>, Option<Duration>) {
    let st = crash_state().lock().unwrap();
    let uptime = st.spawn_at.map(|t| t.elapsed());
    let rss_age = st.last_rss_at.map(|t| t.elapsed());
    (uptime, st.last_rss_kb, rss_age)
}

/// Best-effort RSS (KB) of `pid` via `ps -o rss= -p <pid>`. Both macOS and Linux
/// report RSS here in 1-KB units. Returns `None` on any failure so the sampler
/// can never disrupt the supervisor. Diagnostic only.
fn sample_pid_rss_kb(pid: u32) -> Option<u64> {
    let out = std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &pid.to_string()])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    parse_ps_rss_kb(&String::from_utf8_lossy(&out.stdout))
}

/// Parse `ps -o rss=` output (RSS in KB, possibly whitespace-padded, headerless)
/// into KB. Pure + unit-testable.
fn parse_ps_rss_kb(out: &str) -> Option<u64> {
    out.trim().lines().next()?.trim().parse::<u64>().ok()
}

/// Human-readable RSS from KB (e.g. `182MB`, `1.4GB`). Pure + unit-testable.
fn format_rss_kb(kb: u64) -> String {
    if kb >= 1024 * 1024 {
        format!("{:.1}GB", kb as f64 / (1024.0 * 1024.0))
    } else if kb >= 1024 {
        format!("{}MB", kb / 1024)
    } else {
        format!("{kb}KB")
    }
}

/// Build the ` [uptime=…; rss=…]` suffix appended to an unexpected-exit capture.
/// Pure over its inputs so it is unit-testable; omits pieces that are unknown and
/// returns `""` when nothing is known (so the base message is unchanged).
fn exit_diagnostic_suffix(
    uptime: Option<Duration>,
    rss_kb: Option<u64>,
    rss_age: Option<Duration>,
) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(u) = uptime {
        parts.push(format!("uptime={}", format_duration_secs(u.as_secs())));
    }
    match (rss_kb, rss_age) {
        (Some(kb), Some(age)) => parts.push(format!(
            "last_rss={} (sampled {} before exit)",
            format_rss_kb(kb),
            format_duration_secs(age.as_secs())
        )),
        (Some(kb), None) => parts.push(format!("last_rss={}", format_rss_kb(kb))),
        _ => parts.push("last_rss=unsampled".to_string()),
    }
    if parts.is_empty() {
        String::new()
    } else {
        format!(" [{}]", parts.join("; "))
    }
}

/// Compact `Ns` / `Nm Ns` / `Nh Nm` duration formatter for diagnostics. Pure.
fn format_duration_secs(secs: u64) -> String {
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m{}s", secs / 60, secs % 60)
    } else {
        format!("{}h{}m", secs / 3600, (secs % 3600) / 60)
    }
}

/// Supervisor helper: is the watcher still inside its respawn-backoff window?
fn within_respawn_backoff() -> bool {
    let st = crash_state().lock().unwrap();
    st.backoff_until
        .map(|until| Instant::now() < until)
        .unwrap_or(false)
}

/// Pure decision: has a live watcher survived long enough to clear the
/// crash-loop state? Extracted so it is unit-testable without `Instant`.
fn should_reset_after_recovery(spawn_elapsed: Option<Duration>, window: Duration) -> bool {
    spawn_elapsed.map(|e| e >= window).unwrap_or(false)
}

/// Supervisor helper: once a respawned watcher has survived `FAST_FAIL_WINDOW`,
/// clear the crash-loop state so backoff + capture rate-limiting reset for the
/// next failure episode.
fn reset_crash_state_if_recovered() {
    let mut st = crash_state().lock().unwrap();
    if should_reset_after_recovery(st.spawn_at.map(|t| t.elapsed()), FAST_FAIL_WINDOW) {
        st.consecutive = 0;
        st.backoff_until = None;
    }
}

/// Background supervisor: every `SUPERVISOR_INTERVAL`, ensure the watch daemon
/// is running whenever auto-sync is enabled — respawning it if it died (crash,
/// OOM, external kill, or a failed initial spawn). Without this a dead daemon
/// left sync silently quiet until a manual restart; the only tell was a stale
/// "Last synced N minutes ago". `run_process_impl` deregisters `DAEMON_HANDLE`
/// on exit, and `start_daemon`'s live-pid pre-flight makes a respawn a clean
/// no-op when the daemon is already healthy — so this is safe to poll.
pub fn setup_daemon_supervisor(app: &AppHandle) {
    let handle = app.clone();
    thread::spawn(move || {
        thread::sleep(SUPERVISOR_SETTLE);
        loop {
            let watcher_pid = resolve_hq_folder_path()
                .ok()
                .and_then(|p| read_pid_file(&p));
            let daemon_alive = watcher_pid.map(is_pid_alive).unwrap_or(false);
            if daemon_alive {
                // The watcher is up; once it has survived the fast-fail window,
                // clear the crash-loop state so backoff + capture rate-limiting
                // reset for the next failure episode (HQ-SYNC-4).
                reset_crash_state_if_recovered();
                // Sample the live watcher's RSS so if it is later killed by
                // `signal=9`, the crash capture can report the footprint it had
                // shortly before death — the jetsam/OOM vs kill -9 discriminator
                // (HQ-SYNC-F). Best-effort; a failed sample is silently skipped.
                if let Some(pid) = watcher_pid {
                    if let Some(kb) = sample_pid_rss_kb(pid) {
                        note_watcher_rss(kb);
                    }
                }
            } else if should_respawn_daemon(
                is_realtime_sync_enabled(),
                is_autostart_enabled(),
                daemon_alive,
            ) {
                // Crash-loop dampening: hold off respawning a watcher that just
                // crashed until its exponential backoff elapses, instead of
                // hot-respawning every 30s (HQ-SYNC-4).
                if within_respawn_backoff() {
                    log(
                        "daemon.supervisor",
                        "watch daemon down but within crash-loop backoff — holding off respawn",
                    );
                } else {
                    log(
                        "daemon.supervisor",
                        "watch daemon down but auto-sync is on — respawning",
                    );
                    match start_daemon(handle.clone()) {
                        Ok(_) => log("daemon.supervisor", "respawned watch daemon"),
                        Err(e) => log("daemon.supervisor", &format!("respawn skipped: {e}")),
                    }
                }
            }
            thread::sleep(SUPERVISOR_INTERVAL);
        }
    });
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

    // ── Daemon supervisor decision ───────────────────────────────────────

    #[test]
    fn test_should_respawn_daemon() {
        // Auto-sync on (either flag), daemon dead → respawn.
        assert!(should_respawn_daemon(true, false, false));
        assert!(should_respawn_daemon(false, true, false));
        assert!(should_respawn_daemon(true, true, false));
        // Auto-sync on, daemon already alive → no-op.
        assert!(!should_respawn_daemon(true, false, true));
        assert!(!should_respawn_daemon(false, true, true));
        // Auto-sync off (user disabled it), daemon dead → never respawn.
        assert!(!should_respawn_daemon(false, false, false));
        // Auto-sync off, daemon alive → no-op.
        assert!(!should_respawn_daemon(false, false, true));
    }

    // ── Crash-vs-teardown decision (HQ-SYNC-5) ───────────────────────────

    #[test]
    fn sigterm_exit_is_never_an_unexpected_crash() {
        // HQ-SYNC-5: app quit SIGTERMs the watcher (signal=15, code=None).
        // That is a graceful teardown, not a crash — never capture it, whether
        // or not the registry cancelled-flag was observed in time.
        assert!(!is_unexpected_watcher_exit(false, Some(SIGTERM), false));
        assert!(!is_unexpected_watcher_exit(false, Some(SIGTERM), true));
    }

    #[test]
    fn cancelled_exit_is_never_an_unexpected_crash() {
        // Our own stop paths (Stop button, auto-sync-off, re-spawn) mark the
        // handle cancelled — suppress regardless of how the child went down.
        assert!(!is_unexpected_watcher_exit(false, Some(9), true));
        assert!(!is_unexpected_watcher_exit(false, None, true));
    }

    #[test]
    fn successful_exit_is_never_a_crash() {
        assert!(!is_unexpected_watcher_exit(true, None, false));
        assert!(!is_unexpected_watcher_exit(true, Some(SIGTERM), false));
    }

    #[test]
    fn genuine_crash_signatures_are_captured() {
        // A non-zero exit code (None signal) is a real failure.
        assert!(is_unexpected_watcher_exit(false, None, false));
        // Fault signals are real crashes: SIGSEGV(11), SIGABRT(6), SIGBUS(10).
        assert!(is_unexpected_watcher_exit(false, Some(11), false));
        assert!(is_unexpected_watcher_exit(false, Some(6), false));
        // SIGKILL(9) — OOM / `kill -9` — stays loud when not our own cancel.
        assert!(is_unexpected_watcher_exit(false, Some(9), false));
    }

    // ── Crash-loop dampening (HQ-SYNC-4) ─────────────────────────────────

    #[test]
    fn respawn_backoff_is_exponential_and_capped() {
        let base = Duration::from_secs(30);
        let cap = Duration::from_secs(1800); // 30 min
        // No failures yet → the base supervisor cadence.
        assert_eq!(respawn_backoff(0, base, cap), Duration::from_secs(30));
        // Exponential ×2 per consecutive fast failure.
        assert_eq!(respawn_backoff(1, base, cap), Duration::from_secs(60));
        assert_eq!(respawn_backoff(2, base, cap), Duration::from_secs(120));
        assert_eq!(respawn_backoff(3, base, cap), Duration::from_secs(240));
        // Capped — and a large count must NOT overflow.
        assert_eq!(respawn_backoff(6, base, cap), cap);
        assert_eq!(respawn_backoff(1000, base, cap), cap);
    }

    #[test]
    fn capture_is_rate_limited_to_log2_milestones() {
        // The 36,977-event flood collapses to captures only at 1,2,4,8,16,…
        let captured: Vec<u32> = (1..=64).filter(|&n| should_capture_crash(n)).collect();
        assert_eq!(captured, vec![1, 2, 4, 8, 16, 32, 64]);
        // The in-between repeats are suppressed.
        assert!(!should_capture_crash(3));
        assert!(!should_capture_crash(7));
        assert!(!should_capture_crash(1000));
        // The very first crash always alerts (dampening, not masking).
        assert!(should_capture_crash(1));
    }

    #[test]
    fn fast_failure_window_distinguishes_loop_from_ran_then_died() {
        let window = Duration::from_secs(60);
        assert!(is_fast_failure(Duration::from_secs(2), window)); // crash-on-spawn
        assert!(is_fast_failure(Duration::from_secs(59), window));
        assert!(!is_fast_failure(Duration::from_secs(60), window)); // ran a full minute
        assert!(!is_fast_failure(Duration::from_secs(3600), window)); // ran an hour
    }

    #[test]
    fn recovery_reset_decision_needs_a_full_fast_fail_window() {
        let window = Duration::from_secs(60);
        // Survived the window → reset the crash-loop state.
        assert!(should_reset_after_recovery(Some(window), window));
        assert!(should_reset_after_recovery(Some(Duration::from_secs(120)), window));
        // Only briefly alive (a just-spawned watcher) → do NOT reset yet.
        assert!(!should_reset_after_recovery(Some(Duration::from_secs(5)), window));
        // No spawn recorded → nothing to reset.
        assert!(!should_reset_after_recovery(None, window));
    }

    #[test]
    fn parse_ps_rss_handles_padding_and_junk() {
        // `ps -o rss=` is headerless; may be left-padded with spaces.
        assert_eq!(parse_ps_rss_kb("  186234\n"), Some(186234));
        assert_eq!(parse_ps_rss_kb("512"), Some(512));
        // Empty (dead pid) or non-numeric → None, never a bogus 0.
        assert_eq!(parse_ps_rss_kb(""), None);
        assert_eq!(parse_ps_rss_kb("\n"), None);
        assert_eq!(parse_ps_rss_kb("RSS"), None);
    }

    #[test]
    fn format_rss_scales_kb_mb_gb() {
        assert_eq!(format_rss_kb(512), "512KB");
        assert_eq!(format_rss_kb(1024), "1MB");
        assert_eq!(format_rss_kb(186_234), "181MB");
        assert_eq!(format_rss_kb(1024 * 1024), "1.0GB");
        assert_eq!(format_rss_kb(1_572_864), "1.5GB");
    }

    #[test]
    fn format_duration_scales_s_m_h() {
        assert_eq!(format_duration_secs(45), "45s");
        assert_eq!(format_duration_secs(90), "1m30s");
        assert_eq!(format_duration_secs(3600), "1h0m");
        assert_eq!(format_duration_secs(19_082), "5h18m");
    }

    #[test]
    fn exit_diagnostic_suffix_reports_uptime_and_rss() {
        // Healthy-for-hours then killed with a fresh RSS sample — the OOM signal.
        let s = exit_diagnostic_suffix(
            Some(Duration::from_secs(19_082)),
            Some(1_572_864),
            Some(Duration::from_secs(27)),
        );
        assert_eq!(
            s,
            " [uptime=5h18m; last_rss=1.5GB (sampled 27s before exit)]"
        );
    }

    #[test]
    fn exit_diagnostic_suffix_marks_missing_rss_unsampled() {
        // Crash before the first supervisor tick — uptime known, RSS never taken.
        let s = exit_diagnostic_suffix(Some(Duration::from_secs(3)), None, None);
        assert_eq!(s, " [uptime=3s; last_rss=unsampled]");
    }

    #[test]
    fn crash_tracker_counts_fast_failures_and_resets() {
        // Drive the shared tracker through a fast crash-loop, then a manual
        // reset. (Serialized via the global mutex; reset state up front so the
        // test is order-independent.)
        {
            let mut st = crash_state().lock().unwrap();
            *st = WatcherCrashState::default();
        }
        // Spawn → immediate crash, three times: counter climbs 1,2,3.
        note_watcher_spawned();
        assert_eq!(note_watcher_crashed(), 1);
        note_watcher_spawned();
        assert_eq!(note_watcher_crashed(), 2);
        note_watcher_spawned();
        assert_eq!(note_watcher_crashed(), 3);
        // After a crash we are inside the backoff window (no hot-respawn).
        assert!(within_respawn_backoff());

        // Recovery clears the loop so the next failure episode starts fresh.
        {
            let mut st = crash_state().lock().unwrap();
            st.consecutive = 0;
            st.backoff_until = None;
        }
        assert!(!within_respawn_backoff());
    }

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
    //   --poll-remote-ms 15000   — pull from S3 every 15 seconds (fixed)
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
            Some("15000"),
            "expected the fixed 15-second (15000ms) poll interval"
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
