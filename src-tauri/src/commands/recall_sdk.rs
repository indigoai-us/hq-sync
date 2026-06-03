//! Recall Desktop SDK sidecar lifecycle.
//!
//! Spawns the Recall Desktop SDK as a child process (sidecar pattern) and
//! forwards its `meeting:detected` stdout events to the Svelte renderer as
//! typed Tauri `meeting:detected` events.
//!
//! ## Binary discovery
//!
//! The SDK binary (`recall-desktop-sdk`) is resolved in order:
//!   1. Next to the running executable — the Tauri `bundle.externalBin`
//!      placement for release builds. The binary is named
//!      `recall-desktop-sdk` (or `recall-desktop-sdk-aarch64-apple-darwin`
//!      in the Tauri arch-tagged form).
//!   2. `recall-desktop-sdk` on PATH — used during local dev or when the SDK
//!      is installed globally (e.g. `npm install -g @recall-ai/desktop-sdk`).
//!
//! If the binary cannot be found, `start_recall_sdk` logs
//! `RECALL_SDK_UNAVAILABLE` and returns `Ok(())` — the app continues
//! normally. The rest of the MeetingsWindow is unaffected.
//!
//! ## Credentials
//!
//! On startup, the module calls `GET /v1/recall/credentials` on hq-pro to
//! obtain the user's Recall API key. If the endpoint returns 404 (not yet
//! provisioned) or any network error, the SDK is skipped (same
//! `RECALL_SDK_UNAVAILABLE` log). This keeps the credential handshake
//! entirely server-side — no Recall key is ever stored locally in plaintext.
//!
//! ## Protocol
//!
//! The SDK emits ndjson to stdout. Lines whose `type` field equals
//! `"meeting:detected"` are parsed into `MeetingDetectedEvent` and forwarded
//! to the renderer. Unknown / malformed lines are silently skipped.
//!
//! ## Lifecycle
//!
//! - Started once from `main.rs` setup, in a `tauri::async_runtime::spawn`.
//! - The process is registered under the singleton handle `"recall-sdk"` in
//!   the shared `PROCESS_REGISTRY` (from `commands::process`). A second call
//!   to `start_recall_sdk` while the SDK is already running is a no-op.
//! - On app quit the Tauri runtime tears down, which drops the async tasks.
//!   SIGTERM is sent to the process; after `SIGKILL_DELAY` SIGKILL follows.
//!   This mirrors the sync runner and daemon lifecycle.
//!
//! ## Graceful-degradation contract
//!
//! Every error path in `start_recall_sdk` MUST log `RECALL_SDK_UNAVAILABLE`
//! and return `Ok(())` rather than propagating an `Err`. The caller (`main.rs`
//! setup) ignores the return value; an `Err` from setup would abort the
//! Tauri runtime and take the whole menubar app down.

use std::collections::HashMap;
use std::time::Duration;

use chrono::Utc;
use serde::Deserialize;
use tauri::{AppHandle, Emitter};

use std::io::Write;
use std::process::ChildStdin;
use std::sync::{Mutex, OnceLock};

use crate::commands::cognito;
use crate::commands::process::{
    cancel_process_impl, run_process_with_stdin_impl, try_register_handle, ProcessEvent, SpawnArgs,
};
use crate::commands::sync::resolve_vault_api_url;
use crate::events::{
    MeetingClosedEvent, MeetingDetectedEvent, PermissionStatusEvent, RecordingEndedEvent,
    RecordingErrorEvent, RecordingMediaCaptureEvent, RecordingStartedEvent, EVENT_MEETING_CLOSED,
    EVENT_MEETING_DETECTED, EVENT_PERMISSIONS_ALL_GRANTED, EVENT_PERMISSION_STATUS,
    EVENT_RECORDING_ENDED, EVENT_RECORDING_ERROR, EVENT_RECORDING_MEDIA_CAPTURE,
    EVENT_RECORDING_STARTED,
};
use crate::util::client_info::build_client;
use crate::util::logfile::log;
use crate::util::paths;
use crate::util::recordings_ledger::{self, RecordingStatus, ReconcileOutcome};

// ─────────────────────────────────────────────────────────────────────────────
// Constants
// ─────────────────────────────────────────────────────────────────────────────

/// Singleton handle in `PROCESS_REGISTRY`.
const SDK_HANDLE: &str = "recall-sdk";

/// Name of the Recall Desktop SDK binary.
const SDK_BIN: &str = "recall-desktop-sdk";

/// SIGKILL grace period after SIGTERM on app shutdown.
const SIGKILL_DELAY: Duration = Duration::from_secs(5);

/// Log tag used by all `log()` calls in this module.
const LOG_TAG: &str = "recall-sdk";

// ─────────────────────────────────────────────────────────────────────────────
// Bridge stdin (command channel)
// ─────────────────────────────────────────────────────────────────────────────
//
// The SDK bridge accepts ndjson commands on stdin so we can drive
// `RecallAiSdk.startRecording` / `stopRecording` without spawning a new
// process per recording. The `ChildStdin` handle is stored here when
// `run_process_with_stdin_impl` spawns the bridge; cleared when it exits.
//
// All access goes through `write_bridge_command()` which serialises the
// payload to JSON, appends `\n`, and flushes — matching the bridge's
// `readline` parser exactly. Failures are reported as `Err(String)`
// (typically "bridge not running") so the calling Tauri command can
// surface a clean error to the UI instead of panicking.

static BRIDGE_STDIN: OnceLock<Mutex<Option<ChildStdin>>> = OnceLock::new();

fn bridge_stdin_cell() -> &'static Mutex<Option<ChildStdin>> {
    BRIDGE_STDIN.get_or_init(|| Mutex::new(None))
}

/// Serialise a JSON value, append `\n`, and write to the bridge's stdin.
///
/// Returns `Err` when:
/// - The bridge isn't running (`stdin` handle missing — likely never
///   spawned, or already exited)
/// - The lock is poisoned (a previous writer panicked mid-write)
/// - The write or flush itself fails (broken pipe — bridge died)
///
/// The caller is responsible for any retry / recovery semantics; this
/// function intentionally has no implicit retry.
fn write_bridge_command(value: &serde_json::Value) -> Result<(), String> {
    let cell = bridge_stdin_cell();
    let mut guard = cell
        .lock()
        .map_err(|e| format!("bridge stdin lock poisoned: {e}"))?;
    let stdin = guard
        .as_mut()
        .ok_or_else(|| "bridge not running".to_string())?;
    let line = serde_json::to_string(value)
        .map_err(|e| format!("command serialise failed: {e}"))?;
    writeln!(stdin, "{line}").map_err(|e| format!("bridge stdin write failed: {e}"))?;
    stdin
        .flush()
        .map_err(|e| format!("bridge stdin flush failed: {e}"))?;
    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────────
// Active-detection registry
// ─────────────────────────────────────────────────────────────────────────────
//
// The Recall SDK fires `meeting:detected` once when a meeting window appears
// and keeps no state of its own. The classic popover's `main` window is alive
// from launch so it never misses the event, but the Indigo desktop-alt window
// is created on demand (e.g. from a notification click) and therefore misses
// any detection that fired before it existed. We retain the most recent
// detection per meeting window here so a freshly opened desktop-alt window can
// seed `$activeMeetings` via `meetings_list_active_detections` and show the
// in-progress meeting (with its Record control) immediately.
//
// Keyed by `window_id` (always populated by the current bridge; falls back to
// `meeting_url` defensively). Insert on `meeting:detected`, remove on
// `meeting:closed` — mirroring the frontend's upsert/remove lifecycle so the
// retained set and the live event stream converge on the same state.

static ACTIVE_DETECTIONS: OnceLock<Mutex<HashMap<String, MeetingDetectedEvent>>> = OnceLock::new();

fn active_detections_cell() -> &'static Mutex<HashMap<String, MeetingDetectedEvent>> {
    ACTIVE_DETECTIONS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Key a detection by its window id, falling back to the meeting URL when the
/// bridge omitted the window id (defensive — the current bridge always sets it).
fn detection_key(event: &MeetingDetectedEvent) -> String {
    event
        .window_id
        .clone()
        .filter(|id| !id.is_empty())
        .unwrap_or_else(|| event.meeting_url.clone())
}

/// Record (or replace) the retained detection for a meeting window.
fn record_active_detection(event: &MeetingDetectedEvent) {
    if let Ok(mut map) = active_detections_cell().lock() {
        map.insert(detection_key(event), event.clone());
    }
}

/// Drop the retained detection for a closed meeting window. The close event's
/// `window_id` is the same key the detection was inserted under (both come from
/// the bridge's window identity).
fn remove_active_detection(window_id: &str) {
    if let Ok(mut map) = active_detections_cell().lock() {
        map.remove(window_id);
    }
}

/// Snapshot of every retained detection (one per open meeting window). Order is
/// unspecified (HashMap); the frontend upsert is idempotent so order is moot.
fn active_detections_snapshot() -> Vec<MeetingDetectedEvent> {
    active_detections_cell()
        .lock()
        .map(|map| map.values().cloned().collect())
        .unwrap_or_default()
}

/// Return the meeting detections currently retained in-process (one per open
/// meeting window). The Indigo desktop-alt window calls this on mount to seed
/// `$activeMeetings` with meetings detected *before* the window existed — the
/// live `meeting:detected` stream only covers detections that happen while a
/// listener is attached. Empty when the SDK isn't running or no meeting is open.
#[tauri::command]
pub async fn meetings_list_active_detections() -> Result<Vec<MeetingDetectedEvent>, String> {
    Ok(active_detections_snapshot())
}

/// Look up the retained detection for `window_id` and return its meeting URL +
/// source event id — the two inputs to the notify-ledger stable key.
///
/// `start_recording` only receives the SDK `window_id`, but the dedup ledger is
/// keyed by meeting URL / source event id (see `util::meeting_ledger::stable_key`).
/// The active-detection registry retains the full `MeetingDetectedEvent` for each
/// open window (keyed by the same window id), so we can recover the URL/event-id
/// here and derive the identical key the notify path used.
fn detection_url_and_event(window_id: &str) -> Option<(String, Option<String>)> {
    active_detections_cell()
        .lock()
        .ok()
        .and_then(|map| map.get(window_id).map(|e| (e.meeting_url.clone(), e.source_event_id.clone())))
}

/// Authoritatively mark the meeting behind `window_id` as `Recorded` in the
/// notify ledger, so any later `meeting:detected` for the same meeting is
/// suppressed (a `Recorded` entry is honoured for 6 h, like `Notified`).
///
/// Resolves the ledger stable key from the retained detection for this window
/// (the same `meeting_url` / `source_event_id` the notify path used). Best-effort:
/// if no detection is retained for this window (e.g. recorded via a path that
/// bypassed detection) or the key can't be derived, this is a silent no-op — the
/// `claim_notify` lock remains the primary dedup guarantee.
fn mark_recorded_for_window(window_id: &str) {
    use crate::util::meeting_ledger::{record_action, stable_key, LedgerAction};
    let Some((meeting_url, source_event_id)) = detection_url_and_event(window_id) else {
        return;
    };
    if let Some(key) = stable_key(Some(meeting_url.as_str()), source_event_id.as_deref()) {
        record_action(&key, LedgerAction::Recorded, chrono::Utc::now());
        log(
            LOG_TAG,
            &format!("notify-ledger: marked Recorded for windowId={window_id}"),
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Eligibility gate
// ─────────────────────────────────────────────────────────────────────────────

/// Feature flag for the meeting-detect-notify + Desktop SDK recording
/// feature. **`@getindigo.ai` only for v1** — matches the broader
/// `meetings_feature_enabled` gate so the full meeting-pipeline UX
/// (calendar + bot + SDK recording) lights up together for Indigo
/// teammates and stays dark for everyone else.
///
/// Was a single-user allowlist (`stefan@getindigo.ai`) during the
/// 2026-05-26 dogfood; widened once the end-to-end flow shipped (PRs
/// indigoai-us/hq-pro#145, #147, #148, #149, and the menubar feature
/// branch). Universal rollout happens after the SDK webhook handler
/// lands a real transcript pipeline.
const ALLOWED_DOMAIN: &str = "@getindigo.ai";

/// Env-var override for QA: when set to `1`, force-enable the feature
/// regardless of the signed-in email. Lets a tester exercise the SDK on a
/// machine signed in as someone outside the allowlist without flipping the
/// allowlist itself.
const FORCE_ENV: &str = "HQ_SYNC_MEETING_DETECT_FORCE";

/// Cached per-session decision so we don't re-decode the id_token on every
/// callsite (start_recall_sdk + main.rs setup + Tauri command from the
/// renderer). The token is rotated on refresh but the email claim is stable
/// across rotations, so a process-lifetime cache is safe. Same pattern as
/// `meetings::CACHED_FLAG`.
static CACHED_ELIGIBLE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();

/// Returns true iff this user/process should run meeting detection.
///
/// Decision order:
///   1. `HQ_SYNC_MEETING_DETECT_FORCE=1` → true (QA override).
///   2. Signed-in email ends in `@getindigo.ai` → true.
///   3. Otherwise → false.
///
/// Quiet on missing/malformed tokens (returns false rather than erroring) so a
/// signed-out user during launch doesn't crash setup.
pub async fn meeting_detect_eligible() -> bool {
    if let Some(v) = CACHED_ELIGIBLE.get() {
        return *v;
    }
    let enabled = compute_meeting_detect_eligible().await;
    let _ = CACHED_ELIGIBLE.set(enabled);
    enabled
}

async fn compute_meeting_detect_eligible() -> bool {
    // Env override wins first — needed for CI/QA on machines signed in as
    // someone outside the allowlist.
    if matches!(std::env::var(FORCE_ENV).ok().as_deref(), Some("1")) {
        log(LOG_TAG, "meeting_detect_eligible: forced via env override");
        return true;
    }

    let tokens = match cognito::get_tokens().await {
        Ok(Some(t)) => t,
        _ => return false,
    };
    let id_token = match tokens.id_token.as_deref() {
        Some(t) if !t.is_empty() => t,
        _ => return false,
    };
    let claims = match cognito::decode_id_token_claims(id_token) {
        Ok(c) => c,
        Err(_) => return false,
    };
    is_meeting_detect_allowed_email(claims.email.as_deref())
}

/// Pure helper — public for unit testing. Case-insensitive suffix match
/// on the `@getindigo.ai` domain. The leading `@` is what prevents
/// look-alike domains like `forgetindigo.ai` from matching. Empty /
/// `None` / malformed strings are rejected.
pub fn is_meeting_detect_allowed_email(email: Option<&str>) -> bool {
    match email {
        Some(s) if !s.is_empty() => s.to_ascii_lowercase().ends_with(ALLOWED_DOMAIN),
        _ => false,
    }
}

/// Tauri command exposing `meeting_detect_eligible` to the renderer so the
/// frontend can hide the permissions banner / Settings section / meeting
/// detection toggle for users outside the Phase 0 allowlist.
#[tauri::command]
pub async fn meeting_detect_feature_enabled() -> Result<bool, String> {
    Ok(meeting_detect_eligible().await)
}

// ─────────────────────────────────────────────────────────────────────────────
// Credentials
// ─────────────────────────────────────────────────────────────────────────────

/// Response shape for `GET /v1/recall/credentials`.
///
/// hq-pro returns this when the user has an active Recall integration.
/// The `api_key` is a short-lived token or a long-lived key depending on
/// the Recall tier — the SDK handles refresh internally once it has the
/// initial key.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RecallCredentials {
    api_key: String,
}

/// Fetch the user's Recall API key from hq-pro.
///
/// Returns `Ok(Some(key))` when the credentials endpoint responds 200 with
/// a valid `apiKey`. Returns `Ok(None)` when the endpoint responds 404 (the
/// user has no Recall integration yet) or when the credentials are empty.
/// Returns `Err` only on hard network / auth failures.
async fn fetch_recall_credentials() -> Result<Option<String>, String> {
    let base = resolve_vault_api_url()
        .map(|u| u.trim_end_matches('/').to_string())
        .map_err(|e| format!("vault url: {e}"))?;

    let token = cognito::get_valid_access_token()
        .await
        .map_err(|e| format!("auth: {e}"))?;

    let res = build_client()
        .get(format!("{base}/v1/recall/credentials"))
        .header("authorization", format!("Bearer {token}"))
        .send()
        .await
        .map_err(|e| format!("recall/credentials fetch: {e}"))?;

    if res.status().as_u16() == 404 {
        return Ok(None);
    }

    if !res.status().is_success() {
        let status = res.status();
        let body = res.text().await.unwrap_or_default();
        return Err(format!("recall/credentials HTTP {status}: {body}"));
    }

    let text = res
        .text()
        .await
        .map_err(|e| format!("recall/credentials read: {e}"))?;

    let creds: RecallCredentials = serde_json::from_str(&text)
        .map_err(|e| format!("recall/credentials parse: {e}"))?;

    if creds.api_key.is_empty() {
        return Ok(None);
    }

    Ok(Some(creds.api_key))
}

// ─────────────────────────────────────────────────────────────────────────────
// Binary discovery
// ─────────────────────────────────────────────────────────────────────────────

/// Try to find the Recall Desktop SDK binary.
///
/// Search order:
///   1. Adjacent to the running executable (Tauri `externalBin` placement).
///      Also checks the arch-tagged Tauri form: `{bin}-aarch64-apple-darwin`
///      and `{bin}-x86_64-apple-darwin`.
///   2. On PATH via `paths::resolve_bin` (returns bare name when not found on
///      known prefixes — the process manager returns `NotFound` at spawn time,
///      which we catch and log as `RECALL_SDK_UNAVAILABLE`).
///
/// Returns `Some(path)` when the binary exists on disk, `None` otherwise.
fn find_sdk_binary() -> Option<String> {
    // 1. Check next to the running executable (release bundle).
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            // Plain name.
            let plain = dir.join(SDK_BIN);
            if plain.exists() {
                return Some(plain.to_string_lossy().into_owned());
            }
            // Tauri arch-tagged names for macOS universal builds.
            for arch in &["aarch64-apple-darwin", "x86_64-apple-darwin"] {
                let tagged = dir.join(format!("{}-{}", SDK_BIN, arch));
                if tagged.exists() {
                    return Some(tagged.to_string_lossy().into_owned());
                }
            }
        }
    }

    // 2. Try PATH / known install prefixes.
    let resolved = paths::resolve_bin(SDK_BIN);
    // `resolve_bin` returns the bare name when nothing is found on known
    // prefixes. Check whether the result actually exists as an absolute path
    // on disk before accepting it (bare-name entries on PATH will fail at
    // spawn time — that's handled in the caller).
    if std::path::Path::new(&resolved).exists() {
        return Some(resolved);
    }

    // Not found anywhere we can verify statically — return the bare name so
    // the caller gets a clean `NotFound` from the OS rather than a confusing
    // panic. The calling code in `start_recall_sdk` maps spawn failure to
    // `RECALL_SDK_UNAVAILABLE`.
    //
    // Actually: return None so the caller can log RECALL_SDK_UNAVAILABLE
    // before even trying to spawn, giving a cleaner log message.
    None
}

// ─────────────────────────────────────────────────────────────────────────────
// Stdout protocol
// ─────────────────────────────────────────────────────────────────────────────

/// ndjson event shape emitted by the SDK bridge on stdout. The bridge
/// translates real Recall SDK callbacks into the lines we parse here.
///
/// Blank or unknown lines return `None` (handled in `parse_sdk_line`).
#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
enum RecallSdkEvent {
    #[serde(rename = "meeting:detected")]
    MeetingDetected(MeetingDetectedEvent),
    #[serde(rename = "meeting:closed")]
    MeetingClosed(MeetingClosedEvent),
    #[serde(rename = "permission:status")]
    PermissionStatus(PermissionStatusEvent),
    /// Convenience signal — all required perms granted. No payload.
    #[serde(rename = "permissions:all-granted")]
    PermissionsAllGranted {},
    #[serde(rename = "recording:started")]
    RecordingStarted(RecordingStartedEvent),
    #[serde(rename = "recording:ended")]
    RecordingEnded(RecordingEndedEvent),
    #[serde(rename = "recording:media-capture")]
    RecordingMediaCapture(RecordingMediaCaptureEvent),
    #[serde(rename = "recording:error")]
    RecordingError(RecordingErrorEvent),
}

/// Parse a single ndjson line from the SDK bridge. Blank lines and
/// unrecognised types return `None`.
fn parse_sdk_line(line: &str) -> Option<RecallSdkEvent> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }
    serde_json::from_str::<RecallSdkEvent>(trimmed).ok()
}

// ─────────────────────────────────────────────────────────────────────────────
// Public entry point
// ─────────────────────────────────────────────────────────────────────────────

/// Start the Recall Desktop SDK sidecar.
///
/// Called once from `main.rs` setup inside a `tauri::async_runtime::spawn`.
/// On any failure (binary missing, credentials unavailable, spawn error) the
/// function logs `RECALL_SDK_UNAVAILABLE` and returns `Ok(())` — the menubar
/// app continues running normally.
pub async fn start_recall_sdk(app: AppHandle) -> Result<(), String> {
    log(LOG_TAG, "start_recall_sdk: initialising");

    // ── 0. @getindigo.ai eligibility gate ─────────────────────────────────────
    // Feature is gated to @getindigo.ai users — matches
    // `meetings_feature_enabled` so the full meeting-pipeline UX lights up
    // together. Skip silently for everyone else: no SDK process, no Recall
    // API call, no permission prompts. (Was a single-user Phase-0 allowlist
    // during the 2026-05-26 dogfood; widened once the end-to-end flow
    // landed on hq-prod.)
    if !meeting_detect_eligible().await {
        log(
            LOG_TAG,
            "start_recall_sdk: user not in @getindigo.ai allowlist — skipping (set HQ_SYNC_MEETING_DETECT_FORCE=1 to override)",
        );
        return Ok(());
    }

    // ── 1. Check the singleton — don't double-start ──────────────────────────
    if !try_register_handle(SDK_HANDLE) {
        log(LOG_TAG, "start_recall_sdk: already running (no-op)");
        return Ok(());
    }

    // ── 2. Find the SDK binary ───────────────────────────────────────────────
    let bin_path = match find_sdk_binary() {
        Some(p) => {
            log(LOG_TAG, &format!("start_recall_sdk: binary found at {p}"));
            p
        }
        None => {
            log(
                LOG_TAG,
                "RECALL_SDK_UNAVAILABLE: binary recall-desktop-sdk not found",
            );
            // Deregister so a future attempt (e.g. user installs the SDK and
            // restarts the app) is not blocked by the stale handle.
            crate::commands::process::deregister_process(SDK_HANDLE);
            return Ok(());
        }
    };

    // ── 3. Fetch Recall credentials from hq-pro ──────────────────────────────
    let api_key = match fetch_recall_credentials().await {
        Ok(Some(key)) => {
            log(LOG_TAG, "start_recall_sdk: credentials obtained");
            key
        }
        Ok(None) => {
            log(
                LOG_TAG,
                "RECALL_SDK_UNAVAILABLE: no Recall credentials configured",
            );
            crate::commands::process::deregister_process(SDK_HANDLE);
            return Ok(());
        }
        Err(e) => {
            log(
                LOG_TAG,
                &format!("RECALL_SDK_UNAVAILABLE: credentials fetch failed: {e}"),
            );
            crate::commands::process::deregister_process(SDK_HANDLE);
            return Ok(());
        }
    };

    // ── 4. Build SpawnArgs ───────────────────────────────────────────────────
    let mut env = HashMap::new();
    // Pass the API key via environment variable. The SDK reads RECALL_API_KEY
    // on startup and uses it to authenticate with the Recall cloud service.
    env.insert("RECALL_API_KEY".to_string(), api_key);
    // Include a sane PATH so the SDK binary can find its own dependencies
    // (Node modules, dylibs, etc.) in a Dock-launched context where launchd
    // provides a minimal PATH. Mirrors the sync runner spawn.
    env.insert("PATH".to_string(), paths::child_path());

    let spawn_args = SpawnArgs {
        cmd: bin_path,
        // `--json` tells the SDK to emit ndjson on stdout (Recall SDK CLI
        // convention; the flag name mirrors how hq-sync-runner works).
        args: vec!["--json".to_string()],
        cwd: None,
        env: Some(env),
    };

    // ── 5. Spawn in background ───────────────────────────────────────────────
    log(LOG_TAG, "start_recall_sdk: spawning SDK process");

    let app_bg = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let result = run_process_with_stdin_impl(
            SDK_HANDLE,
            &spawn_args,
            |event| match event {
                ProcessEvent::Stdout(line) => {
                    log("recall-sdk.stdout", &line);
                    match parse_sdk_line(&line) {
                        Some(RecallSdkEvent::MeetingDetected(payload)) => {
                            log(
                                LOG_TAG,
                                &format!(
                                    "meeting:detected — id={} platform={:?} url={}",
                                    payload.detection_id,
                                    payload.platform,
                                    payload.meeting_url
                                ),
                            );
                            // Record into the active-detection registry so an
                            // on-demand desktop-alt window opened *after* this
                            // fired can seed `$activeMeetings` via
                            // `meetings_list_active_detections`.
                            record_active_detection(&payload);
                            if let Err(e) = app_bg.emit(EVENT_MEETING_DETECTED, &payload) {
                                log(
                                    LOG_TAG,
                                    &format!("emit meeting:detected failed: {e}"),
                                );
                            }
                        }
                        Some(RecallSdkEvent::MeetingClosed(payload)) => {
                            log(
                                LOG_TAG,
                                &format!(
                                    "meeting:closed — windowId={} platform={:?}",
                                    payload.window_id, payload.platform
                                ),
                            );
                            // Drop from the active-detection registry so a
                            // freshly opened desktop-alt window won't seed a
                            // meeting that has already ended.
                            remove_active_detection(&payload.window_id);
                            if let Err(e) = app_bg.emit(EVENT_MEETING_CLOSED, &payload) {
                                log(
                                    LOG_TAG,
                                    &format!("emit meeting:closed failed: {e}"),
                                );
                            }
                        }
                        Some(RecallSdkEvent::PermissionStatus(payload)) => {
                            log(
                                LOG_TAG,
                                &format!(
                                    "permission:status — {:?} → {}",
                                    payload.permission, payload.status
                                ),
                            );
                            if let Err(e) = app_bg.emit(EVENT_PERMISSION_STATUS, &payload) {
                                log(
                                    LOG_TAG,
                                    &format!("emit permission:status failed: {e}"),
                                );
                            }
                        }
                        Some(RecallSdkEvent::PermissionsAllGranted {}) => {
                            log(LOG_TAG, "permissions:all-granted");
                            if let Err(e) =
                                app_bg.emit(EVENT_PERMISSIONS_ALL_GRANTED, &())
                            {
                                log(
                                    LOG_TAG,
                                    &format!(
                                        "emit permissions:all-granted failed: {e}"
                                    ),
                                );
                            }
                        }
                        Some(RecallSdkEvent::RecordingStarted(payload)) => {
                            log(
                                LOG_TAG,
                                &format!(
                                    "recording:started — windowId={} platform={:?}",
                                    payload.window_id, payload.platform
                                ),
                            );
                            if let Err(e) = app_bg.emit(EVENT_RECORDING_STARTED, &payload) {
                                log(
                                    LOG_TAG,
                                    &format!("emit recording:started failed: {e}"),
                                );
                            }
                        }
                        Some(RecallSdkEvent::RecordingEnded(payload)) => {
                            log(
                                LOG_TAG,
                                &format!(
                                    "recording:ended — windowId={} platform={:?}",
                                    payload.window_id, payload.platform
                                ),
                            );
                            // Clean terminal event: drop the in-flight ledger
                            // entry so the next launch has nothing to reconcile
                            // for this window. This is the canonical clear path
                            // (covers both explicit Stop and the SDK
                            // auto-stopping when the meeting window closes).
                            if let Err(e) =
                                recordings_ledger::record_ended(&payload.window_id)
                            {
                                log(
                                    LOG_TAG,
                                    &format!(
                                        "recording:ended — failed to clear ledger entry for windowId={}: {e}",
                                        payload.window_id
                                    ),
                                );
                            }
                            if let Err(e) = app_bg.emit(EVENT_RECORDING_ENDED, &payload) {
                                log(
                                    LOG_TAG,
                                    &format!("emit recording:ended failed: {e}"),
                                );
                            }
                        }
                        Some(RecallSdkEvent::RecordingMediaCapture(payload)) => {
                            // High-frequency event during a recording — log
                            // only the binary capturing transition. The
                            // Tauri event is still emitted with full detail
                            // for the UI to render audio/video badges.
                            log(
                                LOG_TAG,
                                &format!(
                                    "recording:media-capture — windowId={} type={} capturing={}",
                                    payload.window_id,
                                    payload.capture_type,
                                    payload.capturing
                                ),
                            );
                            if let Err(e) =
                                app_bg.emit(EVENT_RECORDING_MEDIA_CAPTURE, &payload)
                            {
                                log(
                                    LOG_TAG,
                                    &format!(
                                        "emit recording:media-capture failed: {e}"
                                    ),
                                );
                            }
                        }
                        Some(RecallSdkEvent::RecordingError(payload)) => {
                            log(
                                LOG_TAG,
                                &format!(
                                    "recording:error — cmd={} windowId={} message={}",
                                    payload.cmd, payload.window_id, payload.message
                                ),
                            );
                            if let Err(e) = app_bg.emit(EVENT_RECORDING_ERROR, &payload) {
                                log(
                                    LOG_TAG,
                                    &format!("emit recording:error failed: {e}"),
                                );
                            }
                        }
                        None => {}
                    }
                }
                ProcessEvent::Stderr(line) => {
                    log("recall-sdk.stderr", &line);
                }
                ProcessEvent::Exit {
                    code,
                    signal,
                    success,
                } => {
                    log(
                        LOG_TAG,
                        &format!(
                            "SDK exited: success={} code={:?} signal={:?}",
                            success, code, signal
                        ),
                    );
                    // Clear the stashed stdin handle so a subsequent
                    // start_recording call returns a clean "bridge not
                    // running" error instead of writing into a closed pipe.
                    if let Ok(mut guard) = bridge_stdin_cell().lock() {
                        *guard = None;
                    }

                    // ── Terminal event on unexpected sidecar death (B3) ──────
                    // The bridge synthesizes its own terminal recording:error
                    // when the *SDK* crashes (bridge.mjs::failActiveRecordings).
                    // But if the bridge *process itself* dies — SIGSEGV, OOM,
                    // panic, killed — it can't emit anything, so any row the user
                    // had in `recording`/`stopping` would hang until (at best)
                    // the 12s stop-watchdog, and only if they'd pressed Stop.
                    // Mirror failActiveRecordings here: for every windowId the
                    // durable ledger still has in flight, synthesize a terminal
                    // recording:error so the UI resolves the row immediately.
                    //
                    // Skip a *deliberate* teardown (app quit → stop_recall_sdk →
                    // cancel_process_impl marks the handle cancelled): emitting a
                    // scary error while the app is closing is wrong, and a
                    // genuinely in-flight recording is recovered by the launch
                    // reconcile instead. `success` covers a clean exit(0); the
                    // cancelled flag covers a SIGTERM'd one (non-zero/​signalled).
                    let cancelled =
                        crate::commands::process::is_cancelled(SDK_HANDLE);
                    if success || cancelled {
                        log(
                            LOG_TAG,
                            &format!(
                                "SDK exit treated as orderly (success={success} cancelled={cancelled}) — no terminal recording:error synthesized; any in-flight recording is left for the launch reconcile",
                            ),
                        );
                    } else {
                        fail_active_recordings_on_exit(&app_bg, code, signal);
                    }
                }
            },
            |child| {
                // Stash the bridge's stdin so `start_recording` / `stop_recording`
                // Tauri commands can write commands to it. The handle stays
                // alive for the lifetime of the bridge process; cleared on
                // ProcessEvent::Exit above.
                if let Some(stdin) = child.stdin.take() {
                    match bridge_stdin_cell().lock() {
                        Ok(mut guard) => {
                            *guard = Some(stdin);
                            log(LOG_TAG, "bridge stdin handle registered");
                        }
                        Err(e) => {
                            log(
                                LOG_TAG,
                                &format!("bridge stdin lock poisoned at spawn: {e}"),
                            );
                        }
                    }
                } else {
                    log(LOG_TAG, "bridge spawned without stdin pipe (unexpected)");
                }
            },
        );

        if let Err(e) = result {
            log(
                LOG_TAG,
                &format!("RECALL_SDK_UNAVAILABLE: spawn failed: {e}"),
            );
        }
    });

    Ok(())
}

/// Send SIGTERM (then SIGKILL after grace) to the running SDK process.
///
/// Called from the Tauri cleanup hook or the quit command. Safe to call when
/// the SDK is not running — `cancel_process_impl` is a no-op in that case.
pub fn stop_recall_sdk() {
    cancel_process_impl(SDK_HANDLE, SIGKILL_DELAY);
}

// ─────────────────────────────────────────────────────────────────────────────
// Terminal event on unexpected sidecar death (B3 residual)
// ─────────────────────────────────────────────────────────────────────────────

/// Human-readable message stamped on a synthesized terminal `recording:error`
/// when the SDK sidecar process dies unexpectedly. Deliberately distinct from
/// the bridge's own "Recording engine crashed/stopped unexpectedly" text so a
/// log/triage reader can tell a *process death* (this) from an *in-SDK* crash
/// the still-running bridge reported.
const BRIDGE_EXIT_ERROR_MESSAGE: &str =
    "Recording engine exited unexpectedly — the recording may not have been saved.";

/// The `cmd` field used on a synthesized bridge-death `recording:error`. The
/// frontend renders `"{cmd}: {message}"`, and matches no real bridge command,
/// so it's unambiguous in the row/error surface.
const BRIDGE_EXIT_CMD: &str = "bridge-exit";

/// Build one terminal [`RecordingErrorEvent`] per still-open windowId after an
/// unexpected sidecar exit. Pure (no I/O, no Tauri) so it is unit-testable: the
/// caller supplies the open windowIds (from the durable ledger) and the exit
/// code/signal, this maps them to the events the renderer already consumes
/// (`recording:error` → row state `error`, watchdog cleared). Returns an empty
/// Vec when nothing was in flight (the common case — a death with no active
/// recording needs no UI action).
fn synthesize_bridge_exit_errors(
    open_window_ids: &[String],
    code: Option<i32>,
    signal: Option<i32>,
) -> Vec<RecordingErrorEvent> {
    let detail = match (code, signal) {
        (_, Some(sig)) => format!(" (signal {sig})"),
        (Some(c), None) => format!(" (exit code {c})"),
        (None, None) => String::new(),
    };
    open_window_ids
        .iter()
        .map(|window_id| RecordingErrorEvent {
            cmd: BRIDGE_EXIT_CMD.to_string(),
            window_id: window_id.clone(),
            message: format!("{BRIDGE_EXIT_ERROR_MESSAGE}{detail}"),
        })
        .collect()
}

/// Mirror the bridge's `failActiveRecordings` from the Rust side when the
/// sidecar *process* dies: read every in-flight windowId from the durable
/// ledger, synthesize a terminal `recording:error` for each, emit it to the
/// renderer, and clear the ledger so the launch reconcile doesn't double-report
/// the same death.
///
/// Best-effort: a ledger read/clear failure is logged, not propagated (the SDK
/// task is already unwinding). Emitting the terminal events is the user-facing
/// priority; ledger hygiene is secondary.
fn fail_active_recordings_on_exit(app: &AppHandle, code: Option<i32>, signal: Option<i32>) {
    // Clear-and-take the open windowIds in one shot so the entries can't also
    // re-surface through the launch reconcile (the terminal event below IS the
    // resolution). On a read/clear failure fall back to a plain read so we can
    // still emit — losing the clear is acceptable (reconcile would re-report,
    // not lose data); losing the emit is the actual hang we're fixing.
    let window_ids = match recordings_ledger::record_bridge_died() {
        Ok(ids) => ids,
        Err(e) => {
            log(
                LOG_TAG,
                &format!("bridge-exit: failed to clear recordings ledger: {e}"),
            );
            recordings_ledger::open_window_ids().unwrap_or_default()
        }
    };

    if window_ids.is_empty() {
        log(
            LOG_TAG,
            "bridge-exit: no in-flight recordings to fail (nothing to surface)",
        );
        return;
    }

    let events = synthesize_bridge_exit_errors(&window_ids, code, signal);
    log(
        LOG_TAG,
        &format!(
            "bridge-exit: synthesizing terminal recording:error for {} in-flight recording(s)",
            events.len()
        ),
    );
    for ev in &events {
        if let Err(e) = app.emit(EVENT_RECORDING_ERROR, ev) {
            log(
                LOG_TAG,
                &format!(
                    "bridge-exit: emit recording:error failed for windowId={}: {e}",
                    ev.window_id
                ),
            );
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Recording control (start / stop)
// ─────────────────────────────────────────────────────────────────────────────

/// Response shape for `POST /v1/recall/upload-token` on hq-pro.
///
/// hq-pro mints a one-shot Recall.ai SDK upload token via Recall's
/// `/api/v2/sdk-upload/` endpoint and returns the token + the durable
/// Recording id. The token is consumed by `RecallAiSdk.startRecording`
/// inside the bridge.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SdkUploadTokenResponse {
    /// Recall.ai Recording UUID — stable handle for the recording.
    id: String,
    /// One-shot token consumed by `RecallAiSdk.startRecording({ uploadToken })`.
    upload_token: String,
}

/// Fetch a fresh SDK upload token from hq-pro.
///
/// Returns `(recordingId, uploadToken)` on success. Errors when hq-pro
/// rejects (`recall-not-provisioned`, upstream Recall failure, network) —
/// caller surfaces the message to the UI.
async fn fetch_sdk_upload_token(
    company_uid: Option<&str>,
) -> Result<(String, String), String> {
    let base = resolve_vault_api_url()
        .map(|u| u.trim_end_matches('/').to_string())
        .map_err(|e| format!("vault url: {e}"))?;

    let token = cognito::get_valid_access_token()
        .await
        .map_err(|e| format!("auth: {e}"))?;

    // The bot-* routes use ?companyId=… as the canonical company-context
    // query param (see `bot.controller.ts` router). Mirror that here so
    // the SDK-recording path's attribution slot lines up with the rest of
    // the recording surface — hq-pro reads it from the same place, validates
    // the membership, and bakes companyUid+slug+name into the Recall
    // recording's `metadata` so the webhook handler can route it later.
    //
    // Empty body is preserved — `recording_config` belongs there if/when we
    // expose transcript-provider knobs to the UI. Company context goes on
    // the URL because it's a request-scope parameter, not a Recall API
    // payload field.
    let mut url = format!("{base}/v1/recall/upload-token");
    if let Some(uid) = company_uid.filter(|u| !u.is_empty()) {
        url = format!("{url}?companyId={uid}");
    }

    let res = build_client()
        .post(url)
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "application/json")
        .body("{}")
        .send()
        .await
        .map_err(|e| format!("upload-token fetch: {e}"))?;

    let status = res.status();
    let text = res
        .text()
        .await
        .map_err(|e| format!("upload-token read: {e}"))?;

    if !status.is_success() {
        return Err(format!("upload-token HTTP {status}: {text}"));
    }

    let parsed: SdkUploadTokenResponse = serde_json::from_str(&text)
        .map_err(|e| format!("upload-token parse: {e} — body: {text}"))?;

    if parsed.id.is_empty() || parsed.upload_token.is_empty() {
        return Err(format!(
            "upload-token response missing id or token — body: {text}"
        ));
    }

    Ok((parsed.id, parsed.upload_token))
}

/// Start a local recording for the given SDK window.
///
/// Pre-conditions checked before the bridge command is sent:
/// - The user is in the `@getindigo.ai` allowlist (same gate as detection)
/// - The bridge is running (stdin handle present)
///
/// Side effects on success:
/// - hq-pro mints a fresh `uploadToken`
/// - The bridge starts the SDK recording, which streams audio + metadata
///   to Recall.ai. A `recording:started` event will follow asynchronously
///   when the SDK confirms.
///
/// Returns the Recall.ai `recordingId` so the caller can stash it
/// alongside the windowId for later transcript fetch.
#[tauri::command]
pub async fn start_recording(
    window_id: String,
    company_uid: Option<String>,
) -> Result<String, String> {
    if !meeting_detect_eligible().await {
        return Err("user not in @getindigo.ai allowlist".to_string());
    }
    if window_id.trim().is_empty() {
        return Err("window_id is required".to_string());
    }

    // Normalise empty / whitespace-only values to None so the URL query
    // doesn't get an empty `?companyId=`. The frontend defaults to
    // `null` for Personal and the user-picked dropdown can also produce
    // `""` if the membership list races empty on first render.
    let company_uid = company_uid
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    log(
        LOG_TAG,
        &format!(
            "start_recording: requested for windowId={window_id}, company={}",
            company_uid.as_deref().unwrap_or("(personal)"),
        ),
    );

    let (recording_id, upload_token) =
        match fetch_sdk_upload_token(company_uid.as_deref()).await {
            Ok(v) => v,
            Err(e) => {
                // Without this log line the Rust-side failure was opaque to
                // debugging — the user saw an "Error" badge but no visible
                // reason. Surface the upstream HTTP status + body verbatim so
                // post-deploy issues (route 404, auth 401, Recall 5xx) are
                // diagnosable from `~/.hq/logs/hq-sync.log` alone.
                log(
                    LOG_TAG,
                    &format!(
                        "start_recording: upload-token fetch failed for windowId={window_id}: {e}"
                    ),
                );
                return Err(e);
            }
        };
    log(
        LOG_TAG,
        &format!(
            "start_recording: minted token (recordingId={recording_id}) for windowId={window_id}"
        ),
    );

    // Persist the in-flight mapping to the durable ledger BEFORE we tell the
    // bridge to start. If the app is force-quit (or crashes) mid-recording, the
    // next launch reconciles this entry — without it the windowId→recordingId
    // mapping lives only in the in-memory Svelte store and is lost on restart,
    // silently orphaning a recording Recall still holds server-side. Best-effort:
    // a ledger-write failure is logged but must not block the recording from
    // starting (the recording itself is the user's primary intent).
    if let Err(e) = recordings_ledger::record_started(
        window_id.clone(),
        recording_id.clone(),
        company_uid.clone(),
        Utc::now(),
    ) {
        log(
            LOG_TAG,
            &format!(
                "start_recording: failed to persist recordings ledger entry for windowId={window_id} (recording continues): {e}"
            ),
        );
    }

    let cmd = serde_json::json!({
        "cmd": "start-recording",
        "windowId": window_id,
        "uploadToken": upload_token,
    });
    write_bridge_command(&cmd)?;

    // The user explicitly recorded this meeting — authoritatively mark it
    // `Recorded` in the notify ledger so a later `meeting:detected` for the same
    // meeting (e.g. an SDK re-fire) doesn't re-notify. Best-effort; the
    // `claim_notify` lock is the primary dedup guard.
    mark_recorded_for_window(&window_id);

    Ok(recording_id)
}

/// Stop the active recording for the given SDK window.
///
/// Idempotent — issuing stop against a window that isn't recording is a
/// bridge-side no-op (the SDK silently ignores). Always returns `Ok(())`
/// unless the bridge isn't running.
#[tauri::command]
pub async fn stop_recording(window_id: String) -> Result<(), String> {
    if !meeting_detect_eligible().await {
        return Err("user not in @getindigo.ai allowlist".to_string());
    }
    if window_id.trim().is_empty() {
        return Err("window_id is required".to_string());
    }

    log(
        LOG_TAG,
        &format!("stop_recording: requested for windowId={window_id}"),
    );

    let cmd = serde_json::json!({
        "cmd": "stop-recording",
        "windowId": window_id,
    });
    write_bridge_command(&cmd)
}

// ─────────────────────────────────────────────────────────────────────────────
// Launch reconcile (in-flight recordings ledger)
// ─────────────────────────────────────────────────────────────────────────────

/// Tauri event channel for a reconciled in-flight recording. The renderer
/// listens for this on launch and surfaces a "still processing" / "ingest
/// failed" thread so a recording that was in flight across a restart is
/// recovered instead of silently lost.
pub const EVENT_RECORDING_RECONCILED: &str = "recording:reconciled";

/// Subset of hq-pro `GET /v1/bot/{botId}/status` the reconcile needs. The SDK
/// recording's durable `recordingId` is the `recallBotId` hq-pro keys the bot
/// record under, so this status route is the recording's server-side state.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BotStatusResponse {
    #[serde(default)]
    status: String,
    /// US-010 source-landed signal. The status route may omit it on a
    /// pre-US-010 server; default false so a missing field never reads as
    /// "saved" prematurely.
    #[serde(default)]
    source_landed: bool,
}

/// Fetch one recording's server-side status from hq-pro, mapped onto the
/// ledger's normalised [`RecordingStatus`].
///
/// HTTP 404 → `not_found` (the recording never finalised server-side — a lost
/// ingest, not "still processing"). Any other non-2xx or a network error is an
/// `Err` so the reconcile classifies it as transient (`Unknown`) and retries on
/// a later launch rather than dropping the recording.
async fn fetch_recording_status(
    recording_id: &str,
    _company_uid: Option<&str>,
) -> Result<RecordingStatus, String> {
    let base = resolve_vault_api_url()
        .map(|u| u.trim_end_matches('/').to_string())
        .map_err(|e| format!("vault url: {e}"))?;
    let token = cognito::get_valid_access_token()
        .await
        .map_err(|e| format!("auth: {e}"))?;

    let url = format!("{base}/v1/bot/{recording_id}/status");
    let res = build_client()
        .get(url)
        .header("authorization", format!("Bearer {token}"))
        .send()
        .await
        .map_err(|e| format!("status fetch: {e}"))?;

    let http = res.status();
    if http.as_u16() == 404 {
        return Ok(RecordingStatus {
            status: "not-found".to_string(),
            source_landed: false,
            not_found: true,
        });
    }

    let text = res
        .text()
        .await
        .map_err(|e| format!("status read: {e}"))?;
    if !http.is_success() {
        return Err(format!("status HTTP {http}: {text}"));
    }

    let parsed: BotStatusResponse =
        serde_json::from_str(&text).map_err(|e| format!("status parse: {e} — body: {text}"))?;
    Ok(RecordingStatus {
        status: parsed.status,
        source_landed: parsed.source_landed,
        not_found: false,
    })
}

/// Reconcile any in-flight recordings left over from a previous run.
///
/// Called once from `main.rs` setup (gated on the same meeting-detect
/// eligibility as the SDK boot). Reads the durable ledger, asks hq-pro for each
/// still-open recording's status, classifies it, persists the trimmed ledger,
/// and emits one [`EVENT_RECORDING_RECONCILED`] per non-transient outcome so
/// the UI surfaces the thread.
///
/// Best-effort: every failure is logged and swallowed. A corrupt / unreadable
/// ledger is treated as empty so a bad file never blocks launch.
pub async fn reconcile_recordings_on_launch(app: AppHandle) {
    let mut ledger = match recordings_ledger::read_ledger() {
        Ok(l) => l,
        Err(e) => {
            // Treat a corrupt ledger as empty (don't block launch). Logged so a
            // recurring corruption is visible in the diagnostic log.
            log(
                LOG_TAG,
                &format!("reconcile: ledger unreadable, treating as empty: {e}"),
            );
            return;
        }
    };

    if ledger.is_empty() {
        return;
    }

    log(
        LOG_TAG,
        &format!(
            "reconcile: {} in-flight recording(s) left over from a previous run — reconciling",
            ledger.len()
        ),
    );

    // The ledger's `reconcile` takes a synchronous fetcher, but the real status
    // call is async. Pre-fetch every recording's status into a map first, then
    // hand `reconcile` a closure that just looks the result up. This keeps the
    // (heavily unit-tested) classify/prune logic synchronous while the I/O
    // stays async.
    let mut statuses: HashMap<String, Result<RecordingStatus, String>> = HashMap::new();
    for entry in ledger.values() {
        if statuses.contains_key(&entry.recording_id) {
            continue;
        }
        let result =
            fetch_recording_status(&entry.recording_id, entry.company_uid.as_deref()).await;
        statuses.insert(entry.recording_id.clone(), result);
    }

    let outcomes = recordings_ledger::reconcile(&mut ledger, Utc::now(), |recording_id, _co| {
        statuses
            .get(recording_id)
            .cloned()
            .unwrap_or_else(|| Err("status not pre-fetched".to_string()))
    });

    // Persist the trimmed ledger (terminal entries removed, in-flight retained).
    if let Err(e) = recordings_ledger::write_ledger(&ledger) {
        log(
            LOG_TAG,
            &format!("reconcile: failed to persist trimmed ledger: {e}"),
        );
    }

    // Surface each outcome. Transient `Unknown` outcomes are retained in the
    // ledger and not shown to the user (they retry on a later launch).
    for outcome in &outcomes {
        match outcome {
            ReconcileOutcome::Unknown { recording_id, reason, .. } => {
                log(
                    LOG_TAG,
                    &format!(
                        "reconcile: recordingId={recording_id} status unknown (will retry next launch): {reason}"
                    ),
                );
            }
            other => {
                log(LOG_TAG, &format!("reconcile: {other:?}"));
                if let Err(e) = app.emit(EVENT_RECORDING_RECONCILED, other) {
                    log(LOG_TAG, &format!("emit recording:reconciled failed: {e}"));
                }
            }
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::{DetectionSource, MeetingPlatform, RecallPermission};

    fn meeting(line: &str) -> MeetingDetectedEvent {
        match parse_sdk_line(line).expect("should parse") {
            RecallSdkEvent::MeetingDetected(m) => m,
            other => panic!("expected MeetingDetected, got {:?}", other),
        }
    }

    fn permission(line: &str) -> PermissionStatusEvent {
        match parse_sdk_line(line).expect("should parse") {
            RecallSdkEvent::PermissionStatus(p) => p,
            other => panic!("expected PermissionStatus, got {:?}", other),
        }
    }

    #[test]
    fn parse_sdk_line_returns_none_for_empty() {
        assert!(parse_sdk_line("").is_none());
        assert!(parse_sdk_line("   ").is_none());
    }

    #[test]
    fn parse_sdk_line_returns_none_for_unknown_type() {
        let line = r#"{"type":"health-check","status":"ok"}"#;
        assert!(parse_sdk_line(line).is_none());
    }

    #[test]
    fn parse_sdk_line_returns_none_for_malformed_json() {
        assert!(parse_sdk_line("not json at all").is_none());
        assert!(parse_sdk_line("{unclosed").is_none());
    }

    #[test]
    fn parse_sdk_line_parses_meeting_detected_zoom() {
        let line = r#"{"type":"meeting:detected","detectionId":"det_1","meetingUrl":"https://zoom.us/j/999","platform":"zoom","detectedAt":"2026-05-20T10:00:00Z","source":"sdk-calendar","sourceEventId":"evt_abc"}"#;
        let payload = meeting(line);
        assert_eq!(payload.detection_id, "det_1");
        assert_eq!(payload.meeting_url, "https://zoom.us/j/999");
        assert_eq!(payload.platform, MeetingPlatform::Zoom);
        assert_eq!(payload.source, DetectionSource::SdkCalendar);
        assert_eq!(payload.source_event_id.as_deref(), Some("evt_abc"));
    }

    #[test]
    fn parse_sdk_line_parses_meeting_detected_active_app() {
        let line = r#"{"type":"meeting:detected","detectionId":"det_2","meetingUrl":"https://meet.google.com/abc-def","platform":"meet","detectedAt":"2026-05-20T11:00:00Z","source":"sdk-active-app"}"#;
        let payload = meeting(line);
        assert_eq!(payload.platform, MeetingPlatform::Meet);
        assert_eq!(payload.source, DetectionSource::SdkActiveApp);
        assert!(payload.source_event_id.is_none());
    }

    #[test]
    fn parse_sdk_line_handles_leading_whitespace() {
        let line = r#"  {"type":"meeting:detected","detectionId":"det_3","meetingUrl":"https://zoom.us/j/1","platform":"zoom","detectedAt":"2026-05-20T12:00:00Z","source":"sdk-active-app"}  "#;
        let payload = meeting(line);
        assert_eq!(payload.detection_id, "det_3");
    }

    #[test]
    fn parse_sdk_line_parses_other_platform() {
        let line = r#"{"type":"meeting:detected","detectionId":"det_4","meetingUrl":"https://webex.com/meet/abc","platform":"webex","detectedAt":"2026-05-20T13:00:00Z","source":"sdk-calendar"}"#;
        let payload = meeting(line);
        assert_eq!(payload.platform, MeetingPlatform::Webex);
    }

    #[test]
    fn parse_sdk_line_parses_permission_status() {
        let line = r#"{"type":"permission:status","permission":"screen-capture","status":"denied"}"#;
        let payload = permission(line);
        assert_eq!(payload.permission, RecallPermission::ScreenCapture);
        assert_eq!(payload.status, "denied");
    }

    #[test]
    fn parse_sdk_line_parses_all_granted() {
        let line = r#"{"type":"permissions:all-granted"}"#;
        assert!(matches!(
            parse_sdk_line(line),
            Some(RecallSdkEvent::PermissionsAllGranted {})
        ));
    }

    #[test]
    fn find_sdk_binary_returns_none_when_not_installed() {
        // In CI / dev environments without the Recall Desktop SDK installed,
        // find_sdk_binary() must return None (not panic). This is the
        // RECALL_SDK_UNAVAILABLE path exercised by the E2E test "binary missing".
        //
        // We can't assert None always (a dev may have installed the SDK), but we
        // can assert the function doesn't panic.
        let _ = find_sdk_binary(); // must not panic
    }

    // ── Eligibility gate (@getindigo.ai feature flag) ─────────────────────────
    //
    // Gate widened from `stefan@getindigo.ai` exact-match to the
    // `@getindigo.ai` suffix on 2026-05-26 once the end-to-end SDK
    // recording flow shipped to hq-prod. Tests below cover the suffix
    // semantics + look-alike defence; the leading `@` in ALLOWED_DOMAIN
    // is what blocks `forgetindigo.ai` and friends.

    #[test]
    fn meeting_detect_allowlist_accepts_any_getindigo_user() {
        assert!(is_meeting_detect_allowed_email(Some("stefan@getindigo.ai")));
        assert!(is_meeting_detect_allowed_email(Some("teammate@getindigo.ai")));
        assert!(is_meeting_detect_allowed_email(Some("anyone@getindigo.ai")));
    }

    #[test]
    fn meeting_detect_allowlist_case_insensitive() {
        // Cognito sometimes returns emails with non-canonical casing.
        assert!(is_meeting_detect_allowed_email(Some("Stefan@GetIndigo.ai")));
        assert!(is_meeting_detect_allowed_email(Some("STEFAN@GETINDIGO.AI")));
        assert!(is_meeting_detect_allowed_email(Some("Teammate@GetIndigo.AI")));
    }

    #[test]
    fn meeting_detect_allowlist_accepts_plus_addressing() {
        // Common `+tag` pattern used for filtering — still a real
        // `@getindigo.ai` mailbox, should be allowed.
        assert!(is_meeting_detect_allowed_email(Some("stefan+test@getindigo.ai")));
    }

    #[test]
    fn meeting_detect_allowlist_rejects_lookalike_domains() {
        // The leading `@` in ALLOWED_DOMAIN is the load-bearing piece —
        // it blocks any domain that ends in `getindigo.ai` without the
        // explicit `@` boundary.
        assert!(!is_meeting_detect_allowed_email(Some("stefan@forgetindigo.ai")));
        assert!(!is_meeting_detect_allowed_email(Some("stefan@notgetindigo.ai")));
        assert!(!is_meeting_detect_allowed_email(Some("stefan@evil-getindigo.ai")));
    }

    #[test]
    fn meeting_detect_allowlist_rejects_missing_and_empty() {
        assert!(!is_meeting_detect_allowed_email(None));
        assert!(!is_meeting_detect_allowed_email(Some("")));
    }

    #[test]
    fn meeting_detect_allowlist_rejects_other_domains() {
        assert!(!is_meeting_detect_allowed_email(Some("stefan@example.com")));
        assert!(!is_meeting_detect_allowed_email(Some("stefan@gmail.com")));
        assert!(!is_meeting_detect_allowed_email(Some("admin@indigo.ai")));
    }

    #[test]
    fn meeting_detect_allowlist_matches_meetings_feature_enabled() {
        // The two gates should agree — they're parallel checks of the
        // same `@getindigo.ai` flag, just from different sites in the
        // codebase. If the broader `meetings_feature_enabled` ever
        // diverges from this one, the menubar UI surfaces and the SDK
        // boot will disagree about who's an Indigo user.
        use crate::util::feature_gate::is_allowed_email;
        for email in [
            "stefan@getindigo.ai",
            "Anyone@GetIndigo.AI",
            "stefan@gmail.com",
            "stefan@forgetindigo.ai",
            "",
        ] {
            assert_eq!(
                is_meeting_detect_allowed_email(Some(email)),
                is_allowed_email(Some(email)),
                "gate disagreement for {email}",
            );
        }
        assert_eq!(
            is_meeting_detect_allowed_email(None),
            is_allowed_email(None),
        );
    }

    // ── Recording event parsing ────────────────────────────────────────────
    //
    // The bridge emits these on stdout after the SDK responds to a
    // startRecording/stopRecording command, or when a meeting window
    // closes and the SDK auto-ends the recording. Parser is the bottleneck
    // — if these break, recording state on the UI side desyncs from reality.

    #[test]
    fn parse_sdk_line_parses_recording_started() {
        let line = r#"{"type":"recording:started","windowId":"win-1","platform":"zoom","startedAt":"2026-05-25T17:00:00Z"}"#;
        match parse_sdk_line(line).expect("should parse") {
            RecallSdkEvent::RecordingStarted(p) => {
                assert_eq!(p.window_id, "win-1");
                assert_eq!(p.platform, MeetingPlatform::Zoom);
                assert_eq!(p.started_at, "2026-05-25T17:00:00Z");
            }
            other => panic!("expected RecordingStarted, got {:?}", other),
        }
    }

    #[test]
    fn parse_sdk_line_parses_recording_ended() {
        let line = r#"{"type":"recording:ended","windowId":"win-1","platform":"meet","endedAt":"2026-05-25T17:30:00Z"}"#;
        match parse_sdk_line(line).expect("should parse") {
            RecallSdkEvent::RecordingEnded(p) => {
                assert_eq!(p.window_id, "win-1");
                assert_eq!(p.platform, MeetingPlatform::Meet);
                assert_eq!(p.ended_at, "2026-05-25T17:30:00Z");
            }
            other => panic!("expected RecordingEnded, got {:?}", other),
        }
    }

    #[test]
    fn parse_sdk_line_parses_recording_media_capture() {
        let line = r#"{"type":"recording:media-capture","windowId":"win-1","captureType":"audio","capturing":true}"#;
        match parse_sdk_line(line).expect("should parse") {
            RecallSdkEvent::RecordingMediaCapture(p) => {
                assert_eq!(p.window_id, "win-1");
                assert_eq!(p.capture_type, "audio");
                assert!(p.capturing);
            }
            other => panic!("expected RecordingMediaCapture, got {:?}", other),
        }
    }

    #[test]
    fn parse_sdk_line_parses_recording_error() {
        let line = r#"{"type":"recording:error","cmd":"start-recording","windowId":"win-1","message":"upload token rejected"}"#;
        match parse_sdk_line(line).expect("should parse") {
            RecallSdkEvent::RecordingError(p) => {
                assert_eq!(p.cmd, "start-recording");
                assert_eq!(p.window_id, "win-1");
                assert_eq!(p.message, "upload token rejected");
            }
            other => panic!("expected RecordingError, got {:?}", other),
        }
    }

    // ── Terminal event on unexpected sidecar death (B3 residual) ───────────────
    //
    // When the bridge *process* dies it cannot run its own
    // failActiveRecordings, so `ProcessEvent::Exit` synthesizes the terminal
    // recording:error for every in-flight windowId. These cover the pure
    // mapping; the ledger read/clear + emit glue is exercised by the
    // recordings_ledger tests (record_bridge_died / open_window_ids) and, in the
    // real app, by the wired ProcessEvent::Exit handler.

    #[test]
    fn bridge_exit_errors_one_per_open_window_with_terminal_cmd() {
        let ids = vec!["win-1".to_string(), "win-2".to_string()];
        let events = synthesize_bridge_exit_errors(&ids, None, Some(11));
        assert_eq!(events.len(), 2);
        // Every synthesized event uses the dedicated bridge-exit cmd (so the UI
        // and logs can tell it from a real start/stop-recording failure) and a
        // message that tells the user the recording may not have been saved.
        for ev in &events {
            assert_eq!(ev.cmd, BRIDGE_EXIT_CMD);
            assert!(
                ev.message.contains("exited unexpectedly"),
                "message should explain the engine died: {}",
                ev.message
            );
            assert!(
                ev.message.contains("may not have been saved"),
                "message should warn about lost recording: {}",
                ev.message
            );
        }
        let windows: Vec<&str> = events.iter().map(|e| e.window_id.as_str()).collect();
        assert!(windows.contains(&"win-1"));
        assert!(windows.contains(&"win-2"));
    }

    #[test]
    fn bridge_exit_errors_empty_when_nothing_in_flight() {
        // The common case: the sidecar dies with no active recording — there is
        // no row to resolve, so no terminal event is produced.
        let events = synthesize_bridge_exit_errors(&[], Some(1), None);
        assert!(events.is_empty());
    }

    #[test]
    fn bridge_exit_error_message_includes_signal_detail() {
        // A SIGKILL/SIGSEGV death stamps the signal so triage can see *how* the
        // sidecar died straight from the row's error text.
        let ids = vec!["win-1".to_string()];
        let events = synthesize_bridge_exit_errors(&ids, None, Some(9));
        assert!(
            events[0].message.contains("signal 9"),
            "expected signal detail in: {}",
            events[0].message
        );
    }

    #[test]
    fn bridge_exit_error_message_includes_exit_code_detail() {
        // A non-zero plain exit (no signal) stamps the code instead.
        let ids = vec!["win-1".to_string()];
        let events = synthesize_bridge_exit_errors(&ids, Some(2), None);
        assert!(
            events[0].message.contains("exit code 2"),
            "expected exit-code detail in: {}",
            events[0].message
        );
    }

    #[test]
    fn bridge_exit_error_parses_back_as_recording_error_event() {
        // The synthesized event must round-trip through the same
        // serde shape the renderer consumes on the `recording:error` channel
        // (serde camelCase): cmd / windowId / message.
        let ids = vec!["win-xyz".to_string()];
        let event = &synthesize_bridge_exit_errors(&ids, None, Some(15))[0];
        let json = serde_json::to_string(event).expect("serialize");
        assert!(json.contains("\"windowId\":\"win-xyz\""));
        assert!(json.contains("\"cmd\":\"bridge-exit\""));
        let parsed: RecordingErrorEvent = serde_json::from_str(&json).expect("round-trip");
        assert_eq!(parsed, *event);
    }
}
