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

use serde::Deserialize;
use tauri::{AppHandle, Emitter};

use crate::commands::cognito;
use crate::commands::process::{
    cancel_process_impl, run_process_impl, try_register_handle, ProcessEvent, SpawnArgs,
};
use crate::commands::sync::resolve_vault_api_url;
use crate::events::{
    MeetingDetectedEvent, PermissionStatusEvent, EVENT_MEETING_DETECTED,
    EVENT_PERMISSIONS_ALL_GRANTED, EVENT_PERMISSION_STATUS,
};
use crate::util::client_info::build_client;
use crate::util::logfile::log;
use crate::util::paths;

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
    #[serde(rename = "permission:status")]
    PermissionStatus(PermissionStatusEvent),
    /// Convenience signal — all required perms granted. No payload.
    #[serde(rename = "permissions:all-granted")]
    PermissionsAllGranted {},
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
        let result = run_process_impl(SDK_HANDLE, &spawn_args, |event| match event {
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
                        if let Err(e) = app_bg.emit(EVENT_MEETING_DETECTED, &payload) {
                            log(
                                LOG_TAG,
                                &format!("emit meeting:detected failed: {e}"),
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
            }
        });

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
}
