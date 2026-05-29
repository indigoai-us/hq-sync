//! Direct-message notification client for HQ Sync.
//!
//! A user-to-user "DM via notification" channel layered on the SAME polling
//! infrastructure as `share_notify.rs`. A DM is structurally "a share event
//! minus a file path, plus a reply action".
//!
//! ## Why this mirrors share_notify
//!
//! The 2026-05-28 incident (`workspace/reports/hq-sync-notifications-debug.md`)
//! showed that coupling notification delivery to `sync:all-complete` is fatal:
//! when sync stalls, notifications silently stop. DMs MUST NOT repeat that
//! mistake. `poll_dm_once` is therefore driven by the **independent interval
//! timer** in `share_notify::setup_share_notify_poller` (one timer, two
//! fetches) — never by a sync event.
//!
//! ## Endpoints (hq-cloud, planned — see DM design 2026-05-28)
//!
//!   `GET  /v1/notify/inbox?since=&limit=`  — poll for new DMs (mirrors
//!                                            `/v1/files/shared-with-me`)
//!   `POST /v1/notify/inbox/ack`            — ack delivered DMs
//!   `POST /v1/notify/dm`                    — send a DM to a recipient
//!
//! ## Cursor
//!
//! `~/.hq/dm-cursor.json`, keyed by `machineId` (same scheme as
//! `share-notify-cursor.json`) so each Mac tracks its own inbox position.
//!
//! ## Gating
//!
//! The `dmNotifications` key in `~/.hq/menubar.json` (defaults ON when absent
//! or unreadable). Read directly here rather than via `MenubarPrefs` so adding
//! the DM channel does not force edits to every `MenubarPrefs` literal.
//!
//! ## Log codes (`dm-notify` tag in `~/.hq/logs/hq-sync.log`)
//!
//!   `DM_NOTIFY_POLL_SKIP` / `_START` / `_OK` / `_AUTH_FAIL` /
//!   `_NETWORK_FAIL` / `_ERROR` — mirror the `SHARE_NOTIFY_*` codes.
//!   `DM_NOTIFY_SEND_OK` / `_SEND_FAIL` — outbound send result.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};

use crate::commands::cognito;
use crate::commands::sync::resolve_vault_api_url;
use crate::util::client_info::build_client;
use crate::util::logfile::log;
use crate::util::paths;

const LOG_TAG: &str = "dm-notify";

/// Tauri event emitted to the frontend when the user interacts with a DM
/// notification (currently: inline reply). The frontend invokes `send_dm`.
const EVENT_NOTIFICATION_DM_ACTION: &str = "notification:dm-action";

/// Tauri event emitted when new DMs are found (frontend may surface a badge
/// or inbox view; currently informational, mirrors `share:new-events`).
pub const EVENT_DM_NEW_EVENTS: &str = "dm:new-events";

// ── Wire types ─────────────────────────────────────────────────────────────────

/// A single inbound DM as returned by `GET /v1/notify/inbox`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DmEvent {
    pub event_id: String,
    /// Canonical personUid of the sender (for reply addressing).
    pub from_person_uid: String,
    pub from_email: String,
    pub from_display_name: String,
    pub body: String,
    pub created_at: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct InboxResponse {
    events: Vec<DmEvent>,
    #[allow(dead_code)]
    next_cursor: Option<String>,
}

/// Action dispatched to the frontend when the user replies to a DM banner.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct NotificationDmActionEvent {
    /// Currently always `"reply"`.
    action: String,
    /// personUid to address the reply to.
    to_person_uid: String,
    to_email: String,
    /// The text the user typed into the notification reply field.
    reply_text: String,
}

// ── In-flight guard (separate from share-notify's so they never contend) ────────

static POLL_IN_FLIGHT: OnceLock<Mutex<bool>> = OnceLock::new();

fn poll_lock() -> &'static Mutex<bool> {
    POLL_IN_FLIGHT.get_or_init(|| Mutex::new(false))
}

fn try_set_in_flight() -> bool {
    let mut guard = poll_lock().lock().unwrap_or_else(|p| p.into_inner());
    if *guard {
        false
    } else {
        *guard = true;
        true
    }
}

fn clear_in_flight() {
    let mut guard = poll_lock().lock().unwrap_or_else(|p| p.into_inner());
    *guard = false;
}

// ── Cursor persistence (mirrors share_notify) ───────────────────────────────────

type CursorStore = HashMap<String, String>;

fn cursor_path() -> Result<std::path::PathBuf, String> {
    paths::hq_config_dir().map(|d| d.join("dm-cursor.json"))
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

// ── Gate ────────────────────────────────────────────────────────────────────────

/// True unless the user explicitly set `dmNotifications: false` in
/// `~/.hq/menubar.json`. Read directly (not via `MenubarPrefs`) so the DM
/// channel is additive — see module doc. Missing key / unreadable → ON.
fn dm_notifications_enabled() -> bool {
    let Ok(dir) = paths::hq_config_dir() else {
        return true;
    };
    let path = dir.join("menubar.json");
    let Ok(contents) = std::fs::read_to_string(&path) else {
        return true;
    };
    let Ok(json) = serde_json::from_str::<serde_json::Value>(&contents) else {
        return true;
    };
    json.get("dmNotifications")
        .and_then(|v| v.as_bool())
        .unwrap_or(true)
}

// ── Public API ───────────────────────────────────────────────────────────────────

/// Fire one DM inbox poll. Singleton-guarded; safe to call from the shared
/// interval timer. Called from `share_notify::setup_share_notify_poller`'s
/// loop (one timer, two fetches) — NOT from a sync event.
pub async fn poll_dm_once(app: AppHandle) {
    if !try_set_in_flight() {
        log(LOG_TAG, "DM_NOTIFY_POLL_SKIP poll already in-flight");
        return;
    }
    do_poll(&app).await;
    clear_in_flight();
}

/// Tauri command: manual poll trigger (frontend / tests).
#[tauri::command]
pub async fn poll_dm_inbox(app: AppHandle) -> Result<(), String> {
    poll_dm_once(app).await;
    Ok(())
}

/// Tauri command: send a DM to a recipient. Frontend calls this from the
/// compose box and from the notification reply listener.
#[tauri::command]
pub async fn send_dm(
    to_person_uid: Option<String>,
    to_email: Option<String>,
    body: String,
) -> Result<(), String> {
    if to_person_uid.is_none() && to_email.is_none() {
        return Err("send_dm requires toPersonUid or toEmail".into());
    }
    if body.trim().is_empty() {
        return Err("send_dm requires a non-empty body".into());
    }

    let access_token = cognito::get_valid_access_token()
        .await
        .map_err(|e| format!("auth: {e}"))?;
    let base_url = resolve_vault_api_url()
        .map_err(|e| format!("vault url: {e}"))?
        .trim_end_matches('/')
        .to_string();
    let url = format!("{}/v1/notify/dm", base_url);

    let payload = serde_json::json!({
        "toPersonUid": to_person_uid,
        "toEmail": to_email,
        "body": body,
    });

    let resp = build_client()
        .post(&url)
        .header("authorization", format!("Bearer {}", access_token))
        .json(&payload)
        .send()
        .await
        .map_err(|e| format!("network: {e}"))?;

    if resp.status().is_success() {
        log(LOG_TAG, "DM_NOTIFY_SEND_OK");
        Ok(())
    } else {
        let status = resp.status();
        log(LOG_TAG, &format!("DM_NOTIFY_SEND_FAIL status={status}"));
        Err(format!("send failed: {status}"))
    }
}

// ── Core poll logic (mirrors share_notify::do_poll) ─────────────────────────────

async fn do_poll(app: &AppHandle) {
    if !dm_notifications_enabled() {
        log(LOG_TAG, "DM_NOTIFY_POLL_SKIP dmNotifications disabled");
        return;
    }

    let machine_id = match crate::commands::config::ensure_machine_id() {
        Ok(id) => id,
        Err(e) => {
            log(LOG_TAG, &format!("DM_NOTIFY_POLL_ERROR machineId: {e}"));
            return;
        }
    };

    let access_token = match cognito::get_valid_access_token().await {
        Ok(t) => t,
        Err(e) => {
            log(LOG_TAG, &format!("DM_NOTIFY_POLL_AUTH_FAIL {e}"));
            return;
        }
    };

    let base_url = match resolve_vault_api_url() {
        Ok(u) => u.trim_end_matches('/').to_string(),
        Err(e) => {
            log(LOG_TAG, &format!("DM_NOTIFY_POLL_ERROR vault url: {e}"));
            return;
        }
    };

    let since = read_cursor(&machine_id);
    let url = match since.as_deref() {
        Some(s) => format!("{}/v1/notify/inbox?since={}&limit=50", base_url, s),
        None => format!("{}/v1/notify/inbox?limit=50", base_url),
    };

    log(LOG_TAG, &format!("DM_NOTIFY_POLL_START since={:?}", since));

    let resp = build_client()
        .get(&url)
        .header("authorization", format!("Bearer {}", access_token))
        .send()
        .await;

    let body = match resp {
        Err(e) => {
            log(LOG_TAG, &format!("DM_NOTIFY_POLL_NETWORK_FAIL {e}"));
            return;
        }
        Ok(r) => {
            let status = r.status();
            if status.as_u16() == 401 || status.as_u16() == 403 {
                log(LOG_TAG, &format!("DM_NOTIFY_POLL_AUTH_FAIL status={status}"));
                return;
            }
            if !status.is_success() {
                log(LOG_TAG, &format!("DM_NOTIFY_POLL_ERROR status={status}"));
                return;
            }
            match r.json::<InboxResponse>().await {
                Ok(b) => b,
                Err(e) => {
                    log(LOG_TAG, &format!("DM_NOTIFY_POLL_ERROR parse: {e}"));
                    return;
                }
            }
        }
    };

    if body.events.is_empty() {
        log(LOG_TAG, "DM_NOTIFY_POLL_OK no new DMs");
        return;
    }

    // Advance cursor to the newest DM's createdAt.
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
        &format!("DM_NOTIFY_POLL_OK {} DM(s), cursor→{}", body.events.len(), newest),
    );

    // Lazily register the bundle identifier with mac-notification-sys so the
    // first send doesn't trigger a macOS "Choose Application" picker. Mirrors
    // the guard in share_notify::do_poll.
    static NOTIFICATION_APP_INIT: OnceLock<()> = OnceLock::new();
    NOTIFICATION_APP_INIT.get_or_init(|| {
        const BUNDLE_ID: &str = "ai.indigo.hq-sync-menubar";
        match mac_notification_sys::set_application(BUNDLE_ID) {
            Ok(()) => log(LOG_TAG, &format!("DM_NOTIFY_BUNDLE_SET bundle={BUNDLE_ID}")),
            Err(e) => log(
                LOG_TAG,
                &format!("DM_NOTIFY_BUNDLE_SET_FAILED bundle={BUNDLE_ID} err={e}"),
            ),
        }
    });

    // Fire one macOS notification per DM with an inline Reply field. On reply,
    // emit `notification:dm-action` so the frontend can invoke `send_dm`.
    for dm in &body.events {
        let title = dm.from_display_name.clone();
        let message = dm.body.clone();
        let app_for_thread = app.clone();
        let to_person_uid = dm.from_person_uid.clone();
        let to_email = dm.from_email.clone();

        std::thread::spawn(move || {
            let mut notification = mac_notification_sys::Notification::default();
            notification
                .title(&title)
                .message(&message)
                // Inline text reply field (placeholder shown in the input).
                .main_button(mac_notification_sys::MainButton::Response("Reply…"));

            // CPU cap (Option 1): share the single blocking-send slot with
            // share_notify so at most one busy-spinning `wait_for_click(true)`
            // send is ever outstanding process-wide. Concurrent DMs fall back to
            // a fire-and-forget send (loses the inline-reply affordance for that
            // banner, but avoids leaking a spinning thread). See
            // `BlockingNotifyGuard` + `workspace/reports/hq-sync-cpu-spin-debug.md`.
            let response = match crate::commands::share_notify::BlockingNotifyGuard::try_acquire() {
                Some(guard) => {
                    let r = notification.wait_for_click(true).send();
                    drop(guard);
                    r
                }
                None => notification.send(),
            };

            match response {
                Ok(mac_notification_sys::NotificationResponse::Reply(text))
                    if !text.trim().is_empty() =>
                {
                    let payload = NotificationDmActionEvent {
                        action: "reply".to_string(),
                        to_person_uid,
                        to_email,
                        reply_text: text,
                    };
                    if let Err(e) =
                        app_for_thread.emit(EVENT_NOTIFICATION_DM_ACTION, &payload)
                    {
                        log(LOG_TAG, &format!("DM_NOTIFY_EMIT_ACTION_FAILED err={e}"));
                    }
                }
                Ok(_) => {} // Click / Close / empty reply — no actionable signal.
                Err(e) => log(LOG_TAG, &format!("DM_NOTIFY_SEND_FAILED err={e}")),
            }
        });
    }

    // Best-effort ack so the same DMs aren't re-notified next poll.
    let event_ids: Vec<String> = body.events.iter().map(|e| e.event_id.clone()).collect();
    let app_clone = app.clone();
    tauri::async_runtime::spawn(async move {
        post_ack(event_ids).await;
    });

    let _ = app.emit(EVENT_DM_NEW_EVENTS, &body.events);
    let _ = app_clone; // keep handle alive for the spawned ack
}

/// POST `/v1/notify/inbox/ack`. Best-effort: errors logged, never surfaced.
async fn post_ack(event_ids: Vec<String>) {
    let access_token = match cognito::get_valid_access_token().await {
        Ok(t) => t,
        Err(e) => {
            log(LOG_TAG, &format!("DM_NOTIFY_ACK_AUTH_FAIL {e}"));
            return;
        }
    };
    let base_url = match resolve_vault_api_url() {
        Ok(u) => u.trim_end_matches('/').to_string(),
        Err(e) => {
            log(LOG_TAG, &format!("DM_NOTIFY_ACK_ERROR vault url: {e}"));
            return;
        }
    };
    let url = format!("{}/v1/notify/inbox/ack", base_url);
    let body = serde_json::json!({ "eventIds": event_ids });

    match build_client()
        .post(&url)
        .header("authorization", format!("Bearer {}", access_token))
        .json(&body)
        .send()
        .await
    {
        Ok(r) if r.status().is_success() => {
            log(LOG_TAG, &format!("DM_NOTIFY_ACK_OK {} DM(s)", event_ids.len()));
        }
        Ok(r) => log(LOG_TAG, &format!("DM_NOTIFY_ACK_ERROR status={}", r.status())),
        Err(e) => log(LOG_TAG, &format!("DM_NOTIFY_ACK_ERROR {e}")),
    }
}
