//! Share-notification poller for HQ Sync (US-004).
//!
//! Polls `/v1/files/shared-with-me` on app launch (5s delay, matching the
//! updater pattern) and after every `sync:all-complete` event. Emits a
//! `share:new-events` Tauri event to the Svelte renderer when new events
//! are found, which US-005 will consume to fire macOS notifications and open
//! the ShareDetail window.
//!
//! ## Feature gating
//!
//! Two independent checks must pass before a poll fires:
//!   1. **Dogfood gate** — `feature_gate::is_indigo_user()` (cached for the
//!      process lifetime; email claim is stable across Cognito rotations).
//!   2. **User preference** — `shareNotifications` in `~/.hq/menubar.json`
//!      (re-read on every poll cycle so the Settings toggle takes effect
//!      immediately after the next sync without an app restart).
//!
//! ## Cursor
//!
//! `~/.hq/share-notify-cursor.json` is a JSON object keyed by `machineId`
//! (from `ensure_machine_id`) so multiple Macs for the same user each track
//! their own polling position independently. The value is an ISO8601 string
//! passed as `since=` in the query. Absent → no `since=` (server returns all
//! events up to the default limit).
//!
//! ## Singleton lock
//!
//! A `std::sync::Mutex<bool>` (`POLL_IN_FLIGHT`) prevents concurrent polls.
//! The guard is dropped before the async HTTP call so it is never held across
//! an `.await` point (MutexGuard is !Send).
//!
//! ## Log codes
//!
//! All log lines to `~/.hq/logs/hq-sync.log` carry a `share-notify` tag and
//! one of these code prefixes so support can grep for them:
//!   `SHARE_NOTIFY_POLL_SKIP`          — gate/preference disabled or in-flight
//!   `SHARE_NOTIFY_POLL_START`         — poll request about to fire
//!   `SHARE_NOTIFY_POLL_OK`            — poll succeeded (may have 0 events)
//!   `SHARE_NOTIFY_POLL_AUTH_FAIL`     — 401/403 or token resolution failure
//!   `SHARE_NOTIFY_POLL_NETWORK_FAIL`  — reqwest transport error
//!   `SHARE_NOTIFY_POLL_ERROR`         — 4xx/5xx other than auth, or parse fail

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_notification::NotificationExt;

use crate::commands::cognito;
use crate::commands::sync::resolve_vault_api_url;
use crate::util::client_info::build_client;
use crate::util::feature_gate;
use crate::util::logfile::log;
use crate::util::paths;

// ── Event name emitted to the Svelte renderer ────────────────────────────────

/// Tauri event emitted when new share events are found. US-005 listens for
/// this to fire macOS notifications and open the ShareDetail window.
pub const EVENT_SHARE_NEW_EVENTS: &str = "share:new-events";

// ── Singleton in-flight guard ─────────────────────────────────────────────────

const LOG_TAG: &str = "share-notify";

static POLL_IN_FLIGHT: OnceLock<Mutex<bool>> = OnceLock::new();

fn poll_lock() -> &'static Mutex<bool> {
    POLL_IN_FLIGHT.get_or_init(|| Mutex::new(false))
}

/// Atomically mark a poll as in-flight. Returns `true` if we successfully
/// took the lock (caller must `clear_in_flight()` when done), `false` if
/// another poll is already running.
fn try_set_in_flight() -> bool {
    let mut guard = poll_lock()
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    if *guard {
        return false;
    }
    *guard = true;
    true
    // guard dropped here — not held across any await point
}

fn clear_in_flight() {
    let mut guard = poll_lock()
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    *guard = false;
}

// ── Wire-format types ─────────────────────────────────────────────────────────

/// A single share event as returned by `GET /v1/files/shared-with-me`.
/// Fields mirror the US-003 response schema; `note` is omitted when absent.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShareEvent {
    pub event_id: String,
    pub issuer_email: String,
    pub issuer_display_name: String,
    pub paths: Vec<String>,
    pub note: Option<String>,
    pub permission: String,
    pub created_at: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SharedWithMeResponse {
    events: Vec<ShareEvent>,
    #[allow(dead_code)]
    next_cursor: Option<String>,
}

/// Tauri event emitted to the share-detail window once its listener is ready
/// (ready-handshake; mirrors "new-files:list" in new_files.rs).
const EVENT_SHARE_EVENTS_LIST: &str = "share:events-list";

/// Managed state: pending share events for the detail window ready-handshake.
/// Follows the `PendingNewFiles` pattern in new_files.rs exactly.
pub struct PendingShareEvents(pub Mutex<Vec<ShareEvent>>);

// ── Cursor persistence ────────────────────────────────────────────────────────

/// Cursor store: `{ "<machineId>": "<ISO8601 createdAt of newest seen event>" }`
type CursorStore = HashMap<String, String>;

fn cursor_path() -> Result<std::path::PathBuf, String> {
    paths::hq_config_dir().map(|d| d.join("share-notify-cursor.json"))
}

fn read_cursor(machine_id: &str) -> Option<String> {
    let path = cursor_path().ok()?;
    let contents = std::fs::read_to_string(&path).ok()?;
    let store: CursorStore = serde_json::from_str(&contents).ok()?;
    store.get(machine_id).cloned()
}

fn write_cursor(machine_id: &str, since: &str) {
    let Ok(path) = cursor_path() else { return };

    let mut store: CursorStore = path
        .exists()
        .then(|| {
            std::fs::read_to_string(&path)
                .ok()
                .and_then(|s| serde_json::from_str(&s).ok())
        })
        .flatten()
        .unwrap_or_default();

    store.insert(machine_id.to_string(), since.to_string());

    if let Ok(json) = serde_json::to_string_pretty(&store) {
        let _ = std::fs::write(&path, json);
    }
}

// ── Gate check ───────────────────────────────────────────────────────────────

/// Returns true when both the dogfood gate AND the user preference allow polling.
/// Re-reads menubar.json on every call so toggling the setting takes effect
/// on the next poll without an app restart.
async fn should_poll() -> bool {
    if !feature_gate::is_indigo_user().await {
        return false;
    }
    match crate::commands::settings::get_settings().await {
        Ok(prefs) => prefs.share_notifications.unwrap_or(true),
        Err(_) => true, // default ON when settings unreadable
    }
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Spawn the launch-time poll. Called from `main.rs` setup with a 5-second
/// delay (matches the updater pattern — gives the app time to fully initialize
/// and the Cognito token to be loaded from disk).
pub fn setup_share_notify_poller(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
        poll_once(app).await;
    });
}

/// Fire one poll cycle. Skips if another poll is already in flight (singleton
/// guard). Safe to call from the `sync:all-complete` listener or the Tauri
/// command handler — the guard prevents overlap.
pub async fn poll_once(app: AppHandle) {
    if !try_set_in_flight() {
        log(LOG_TAG, "SHARE_NOTIFY_POLL_SKIP poll already in-flight");
        return;
    }
    do_poll(&app).await;
    clear_in_flight();
}

/// Tauri command: manual poll trigger. Exposed so the frontend (and tests)
/// can force a poll without waiting for a sync event.
#[tauri::command]
pub async fn poll_shared_with_me(app: AppHandle) -> Result<(), String> {
    poll_once(app).await;
    Ok(())
}

// ── Core poll logic ───────────────────────────────────────────────────────────

async fn do_poll(app: &AppHandle) {
    if !should_poll().await {
        log(LOG_TAG, "SHARE_NOTIFY_POLL_SKIP feature gate or setting disabled");
        return;
    }

    // Cursor key: machineId so each Mac tracks independently.
    let machine_id = match crate::commands::config::ensure_machine_id() {
        Ok(id) => id,
        Err(e) => {
            log(
                LOG_TAG,
                &format!("SHARE_NOTIFY_POLL_ERROR cannot resolve machineId: {e}"),
            );
            return;
        }
    };

    // Resolved + refreshed access token (transparent Cognito refresh).
    let access_token = match cognito::get_valid_access_token().await {
        Ok(t) => t,
        Err(e) => {
            log(
                LOG_TAG,
                &format!("SHARE_NOTIFY_POLL_AUTH_FAIL token error: {e}"),
            );
            return;
        }
    };

    let base_url = match resolve_vault_api_url() {
        Ok(u) => u.trim_end_matches('/').to_string(),
        Err(e) => {
            log(
                LOG_TAG,
                &format!("SHARE_NOTIFY_POLL_ERROR cannot resolve vault URL: {e}"),
            );
            return;
        }
    };

    let since = read_cursor(&machine_id);
    let url = match since.as_deref() {
        Some(s) => format!(
            "{}/v1/files/shared-with-me?since={}&limit=50",
            base_url, s
        ),
        None => format!("{}/v1/files/shared-with-me?limit=50", base_url),
    };

    log(
        LOG_TAG,
        &format!("SHARE_NOTIFY_POLL_START since={:?}", since),
    );

    let resp = build_client()
        .get(&url)
        .header("authorization", format!("Bearer {}", access_token))
        .send()
        .await;

    match resp {
        Err(e) => {
            log(LOG_TAG, &format!("SHARE_NOTIFY_POLL_NETWORK_FAIL {e}"));
        }
        Ok(r) => {
            let status = r.status();
            if status.as_u16() == 401 || status.as_u16() == 403 {
                log(
                    LOG_TAG,
                    &format!("SHARE_NOTIFY_POLL_AUTH_FAIL status={status}"),
                );
                return;
            }
            if !status.is_success() {
                log(
                    LOG_TAG,
                    &format!("SHARE_NOTIFY_POLL_ERROR status={status}"),
                );
                return;
            }
            match r.json::<SharedWithMeResponse>().await {
                Err(e) => {
                    log(
                        LOG_TAG,
                        &format!("SHARE_NOTIFY_POLL_ERROR parse failed: {e}"),
                    );
                }
                Ok(body) => {
                    if body.events.is_empty() {
                        log(LOG_TAG, "SHARE_NOTIFY_POLL_OK no new events");
                        return;
                    }

                    // Advance cursor to the newest event's createdAt.
                    let newest = body
                        .events
                        .iter()
                        .map(|e| e.created_at.as_str())
                        .max()
                        .unwrap_or_default();
                    if !newest.is_empty() {
                        write_cursor(&machine_id, newest);
                    }

                    log(
                        LOG_TAG,
                        &format!(
                            "SHARE_NOTIFY_POLL_OK {} event(s), cursor→{}",
                            body.events.len(),
                            newest
                        ),
                    );

                    // Fire one macOS notification per share event (US-005).
                    // Body: note (truncated to 100 chars) OR comma-joined basenames.
                    for evt in &body.events {
                        let body_text = match &evt.note {
                            Some(n) if !n.is_empty() => {
                                if n.len() > 100 {
                                    format!("{}…", &n[..100])
                                } else {
                                    n.clone()
                                }
                            }
                            _ => {
                                let basenames: Vec<&str> = evt
                                    .paths
                                    .iter()
                                    .map(|p| p.rsplit('/').next().unwrap_or(p.as_str()))
                                    .collect();
                                basenames.join(", ")
                            }
                        };
                        let title =
                            format!("{} shared files with you", evt.issuer_display_name);
                        if let Err(e) = app
                            .notification()
                            .builder()
                            .title(&title)
                            .body(&body_text)
                            .show()
                        {
                            log(
                                LOG_TAG,
                                &format!("NOTIFY_PERMISSION_DENIED or error: {e}"),
                            );
                        }
                    }

                    // Badge the tray icon with the unacknowledged event count.
                    crate::tray::set_share_badge(app, body.events.len());

                    // Emit to frontend — US-005 listens here.
                    let _ = app.emit(EVENT_SHARE_NEW_EVENTS, &body.events);
                }
            }
        }
    }
}

// ── ShareDetail window ─────────────────────────────────────────────────────────

const SHARE_DETAIL_LABEL: &str = "share-detail";

/// Tauri command: open (or focus) the ShareDetail window with the given events.
///
/// Mirrors `open_new_files_detail` in new_files.rs:
/// 1. Stash events in `PendingShareEvents` managed state.
/// 2. If window exists, show + focus it and re-emit the list directly.
/// 3. Otherwise create it hidden; `share_detail_window_ready` will show it.
#[tauri::command]
pub async fn open_share_detail(
    app: AppHandle,
    events: Vec<ShareEvent>,
) -> Result<(), String> {
    // Stash so the ready-handshake can retrieve them.
    if let Some(state) = app.try_state::<PendingShareEvents>() {
        *state.0.lock().unwrap_or_else(|p| p.into_inner()) = events.clone();
    }

    if let Some(window) = app.get_webview_window(SHARE_DETAIL_LABEL) {
        window.show().map_err(|e| e.to_string())?;
        window.set_focus().map_err(|e| e.to_string())?;
        app.emit_to(SHARE_DETAIL_LABEL, EVENT_SHARE_EVENTS_LIST, &events)
            .map_err(|e| e.to_string())?;
        return Ok(());
    }

    tauri::WebviewWindowBuilder::new(
        &app,
        SHARE_DETAIL_LABEL,
        tauri::WebviewUrl::App("index.html".into()),
    )
    .title("Shared with Me")
    .inner_size(640.0, 560.0)
    .resizable(true)
    .decorations(true)
    .visible(false)
    .build()
    .map_err(|e| e.to_string())?;

    Ok(())
}

/// Tauri command: called by ShareDetail.svelte once its event listener is
/// registered. Emits pending events, shows the window, fires best-effort ack,
/// and clears the tray badge.
#[tauri::command]
pub async fn share_detail_window_ready(app: AppHandle) -> Result<(), String> {
    let events: Vec<ShareEvent> = app
        .try_state::<PendingShareEvents>()
        .map(|s| s.0.lock().unwrap_or_else(|p| p.into_inner()).clone())
        .unwrap_or_default();

    app.emit_to(SHARE_DETAIL_LABEL, EVENT_SHARE_EVENTS_LIST, &events)
        .map_err(|e| e.to_string())?;

    if let Some(window) = app.get_webview_window(SHARE_DETAIL_LABEL) {
        let _ = window.show();
        let _ = window.set_focus();
    }

    // Best-effort ack — fire-and-forget so the window doesn't block on it.
    if !events.is_empty() {
        let event_ids: Vec<String> = events.iter().map(|e| e.event_id.clone()).collect();
        let app_clone = app.clone();
        tauri::async_runtime::spawn(async move {
            post_ack(&app_clone, event_ids).await;
            crate::tray::clear_share_badge(&app_clone);
        });
    }

    Ok(())
}

/// POST `/v1/files/shared-with-me/ack` with the given event IDs.
/// Best-effort: errors are logged but never surfaced to the caller.
async fn post_ack(_app: &AppHandle, event_ids: Vec<String>) {
    let access_token = match cognito::get_valid_access_token().await {
        Ok(t) => t,
        Err(e) => {
            log(LOG_TAG, &format!("SHARE_NOTIFY_ACK_AUTH_FAIL {e}"));
            return;
        }
    };
    let base_url = match resolve_vault_api_url() {
        Ok(u) => u.trim_end_matches('/').to_string(),
        Err(e) => {
            log(LOG_TAG, &format!("SHARE_NOTIFY_ACK_ERROR cannot resolve vault URL: {e}"));
            return;
        }
    };
    let url = format!("{}/v1/files/shared-with-me/ack", base_url);
    let body = serde_json::json!({ "eventIds": event_ids });

    match build_client()
        .post(&url)
        .header("authorization", format!("Bearer {}", access_token))
        .json(&body)
        .send()
        .await
    {
        Ok(r) if r.status().is_success() => {
            log(
                LOG_TAG,
                &format!("SHARE_NOTIFY_ACK_OK {} event(s)", event_ids.len()),
            );
        }
        Ok(r) => {
            log(LOG_TAG, &format!("SHARE_NOTIFY_ACK_ERROR status={}", r.status()));
        }
        Err(e) => {
            log(LOG_TAG, &format!("SHARE_NOTIFY_ACK_NETWORK_FAIL {e}"));
        }
    }
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_singleton_lock_starts_false() {
        // The OnceLock starts unset; initialised to false on first access.
        let guard = poll_lock().lock().unwrap();
        assert!(!*guard);
    }

    #[test]
    fn test_try_set_and_clear_in_flight() {
        // Force the lock to false first (may already be set from another test
        // calling try_set_in_flight).
        {
            let mut g = poll_lock().lock().unwrap();
            *g = false;
        }
        assert!(try_set_in_flight(), "first attempt should succeed");
        assert!(!try_set_in_flight(), "second attempt while in-flight should fail");
        clear_in_flight();
        assert!(try_set_in_flight(), "after clear, should succeed again");
        clear_in_flight();
    }

    #[test]
    fn test_cursor_store_serde_roundtrip() {
        let mut store = CursorStore::default();
        store.insert(
            "machine-abc".to_string(),
            "2026-05-25T12:00:00.000Z".to_string(),
        );
        let json = serde_json::to_string(&store).unwrap();
        let parsed: CursorStore = serde_json::from_str(&json).unwrap();
        assert_eq!(
            parsed.get("machine-abc").unwrap(),
            "2026-05-25T12:00:00.000Z"
        );
    }

    #[test]
    fn test_share_event_deserializes_without_note() {
        let json = r#"{
            "eventId": "e1",
            "issuerEmail": "a@b.com",
            "issuerDisplayName": "Alice",
            "paths": ["/Foo/bar.md"],
            "permission": "read",
            "createdAt": "2026-05-25T00:00:00.000Z"
        }"#;
        let evt: ShareEvent = serde_json::from_str(json).unwrap();
        assert_eq!(evt.event_id, "e1");
        assert_eq!(evt.paths.len(), 1);
        assert!(evt.note.is_none());
    }

    #[test]
    fn test_share_event_deserializes_with_note() {
        let json = r#"{
            "eventId": "e2",
            "issuerEmail": "s@getindigo.ai",
            "issuerDisplayName": "Stefan",
            "paths": ["/Shared/doc.md", "/Shared/img.png"],
            "note": "Please review before Friday",
            "permission": "read",
            "createdAt": "2026-05-25T10:00:00.000Z"
        }"#;
        let evt: ShareEvent = serde_json::from_str(json).unwrap();
        assert_eq!(evt.note.as_deref(), Some("Please review before Friday"));
        assert_eq!(evt.paths.len(), 2);
    }

    #[test]
    fn test_write_and_read_cursor() {
        // Uses a temp directory to avoid polluting ~/.hq
        let tmp = tempfile::tempdir().unwrap();
        let cursor_file = tmp.path().join("share-notify-cursor.json");

        // Manually exercise the cursor store logic (not through the real path
        // functions, which are hardcoded to ~/.hq).
        let machine_id = "test-machine-001";
        let ts = "2026-05-25T12:34:56.789Z";

        let mut store = CursorStore::default();
        store.insert(machine_id.to_string(), ts.to_string());
        let json = serde_json::to_string_pretty(&store).unwrap();
        std::fs::write(&cursor_file, &json).unwrap();

        let loaded: CursorStore =
            serde_json::from_str(&std::fs::read_to_string(&cursor_file).unwrap()).unwrap();
        assert_eq!(loaded.get(machine_id).unwrap(), ts);
    }

    #[test]
    fn test_cursor_path_under_dot_hq() {
        let path = cursor_path().unwrap();
        assert!(
            path.ends_with(".hq/share-notify-cursor.json"),
            "cursor path must live under ~/.hq, got {path:?}"
        );
    }
}
