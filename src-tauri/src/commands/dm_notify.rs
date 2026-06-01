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
use tauri::{AppHandle, Emitter, Manager};

use crate::commands::cognito;
use crate::commands::sync::resolve_vault_api_url;
use crate::util::client_info::build_client;
use crate::util::logfile::log;
use crate::util::paths;

const LOG_TAG: &str = "dm-notify";

/// Tauri event emitted when new DMs are found (frontend may surface a badge
/// or inbox view; currently informational, mirrors `share:new-events`).
pub const EVENT_DM_NEW_EVENTS: &str = "dm:new-events";

/// Tauri event emitted when the user actions a DM notification — "copy" (write
/// the agent prompt to the clipboard, only when the DM carries a `prompt`) or
/// "open" (open the DM detail window). Every DM is clickable: a body-click maps
/// to "open". Frontend listener lives in App.svelte.
const EVENT_NOTIFICATION_DM_ACTION: &str = "notification:dm-action";

/// Label of the DM detail window (mirrors share-detail).
const DM_DETAIL_LABEL: &str = "dm-detail";

/// Tauri event the DM detail window listens for to receive its event payload.
const EVENT_DM_DETAIL_EVENT: &str = "dm:detail-event";

// ── Wire types ─────────────────────────────────────────────────────────────────

/// A single inbound DM as returned by `GET /v1/notify/inbox`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DmEvent {
    pub event_id: String,
    /// Canonical personUid of the sender. Used as `toPersonUid` when the
    /// recipient replies from the detail window (see `send_dm`).
    pub from_person_uid: String,
    pub from_email: String,
    pub from_display_name: String,
    pub body: String,
    /// Optional longer-form detail — shown in the DM detail window. Present only
    /// when the sender supplied it; drives whether the "Open details" action shows.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<String>,
    /// Optional agent-context prompt the recipient can copy. Present only when
    /// the sender supplied it; drives whether the "Copy prompt" action shows.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct InboxResponse {
    events: Vec<DmEvent>,
    #[allow(dead_code)]
    next_cursor: Option<String>,
}

/// Action dispatched to the frontend when the user actions a rich DM banner.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct NotificationDmActionEvent {
    /// One of `"copy"` (write prompt to clipboard) or `"open"` (open detail window).
    action: String,
    /// Full DM payload so the frontend can copy the prompt or render details
    /// without re-fetching the inbox.
    event: DmEvent,
}

/// Managed state: the DM event pending for the detail window's ready-handshake.
/// Mirrors `PendingShareEvents` in share_notify.rs.
pub struct PendingDmEvents(pub Mutex<Vec<DmEvent>>);

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

/// Build the `POST /v1/notify/dm` request body for a reply. Matches the server
/// contract in hq-pro `notify-dm.ts` (`handleSendDm`): exactly one recipient key
/// plus a `body` string. Pure + side-effect-free so the wire shape is testable.
fn build_send_payload(to_person_uid: &str, body: &str) -> serde_json::Value {
    serde_json::json!({ "toPersonUid": to_person_uid, "body": body })
}

/// Tauri command: send a DM (a reply from the detail window). Mirrors the auth +
/// URL plumbing of `post_ack`, but — unlike the best-effort ack — surfaces
/// failures to the caller so the UI can show delivery feedback.
///
/// Addresses the recipient by `toPersonUid` (the original sender's
/// `from_person_uid`). The server requires sender and recipient to share an
/// active company membership and rejects self-DMs; a reply to whoever DM'd you
/// always satisfies that. POSTs to `/v1/notify/dm`.
#[tauri::command]
pub async fn send_dm(to_person_uid: String, body: String) -> Result<(), String> {
    let body_text = body.trim();
    if body_text.is_empty() {
        return Err("Message body must not be empty".to_string());
    }

    let access_token = cognito::get_valid_access_token().await.map_err(|e| {
        log(LOG_TAG, &format!("DM_NOTIFY_SEND_FAIL auth: {e}"));
        format!("Not signed in: {e}")
    })?;

    let base_url = resolve_vault_api_url()
        .map(|u| u.trim_end_matches('/').to_string())
        .map_err(|e| {
            log(LOG_TAG, &format!("DM_NOTIFY_SEND_FAIL vault url: {e}"));
            format!("Could not resolve server URL: {e}")
        })?;

    let url = format!("{}/v1/notify/dm", base_url);
    let payload = build_send_payload(&to_person_uid, body_text);

    let resp = build_client()
        .post(&url)
        .header("authorization", format!("Bearer {}", access_token))
        .json(&payload)
        .send()
        .await
        .map_err(|e| {
            log(LOG_TAG, &format!("DM_NOTIFY_SEND_FAIL network: {e}"));
            format!("Network error: {e}")
        })?;

    let status = resp.status();
    if status.is_success() {
        log(LOG_TAG, "DM_NOTIFY_SEND_OK");
        return Ok(());
    }

    // Surface the server's error message when present so the UI can show it.
    let server_msg = resp
        .json::<serde_json::Value>()
        .await
        .ok()
        .and_then(|v| v.get("error").and_then(|e| e.as_str()).map(str::to_string));
    log(
        LOG_TAG,
        &format!("DM_NOTIFY_SEND_FAIL status={status} msg={server_msg:?}"),
    );
    Err(server_msg.unwrap_or_else(|| format!("Send failed (status {})", status.as_u16())))
}

// ── Conversation thread (history) ───────────────────────────────────────────────
//
// The DM detail window renders a two-way thread, not just the single DM that
// triggered the notification. The backend stores a conversation-keyed mirror of
// every DM (see hq-pro `dm-thread.ts`) and exposes it at
// `GET /v1/notify/thread?withPersonUid=…`. `fetch_dm_thread` pulls that thread
// for whichever person the open DM is with, so the window can show the history
// above the live message + reply box.

/// One message in a conversation thread, as returned by `/v1/notify/thread`.
/// `direction` is tagged by the server relative to the signed-in caller:
/// `"out"` = the caller sent it, `"in"` = the counterparty sent it.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadMessage {
    pub event_id: String,
    pub from_person_uid: String,
    pub from_email: String,
    pub from_display_name: String,
    pub body: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
    pub created_at: String,
    pub direction: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadResponse {
    pub messages: Vec<ThreadMessage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

/// Build the `GET /v1/notify/thread` URL. Pure + side-effect-free so the query
/// shape is unit-testable. urlencoding isn't pulled in here; personUids and the
/// base64 cursor are URL-safe, so simple concatenation is sufficient. An
/// empty/blank cursor is omitted so the server returns the first (newest) page.
fn build_thread_url(
    base_url: &str,
    with_person_uid: &str,
    limit: Option<u32>,
    cursor: Option<&str>,
) -> String {
    let mut url = format!(
        "{}/v1/notify/thread?withPersonUid={}",
        base_url, with_person_uid
    );
    if let Some(n) = limit {
        url.push_str(&format!("&limit={}", n));
    }
    if let Some(c) = cursor.filter(|c| !c.is_empty()) {
        url.push_str(&format!("&cursor={}", c));
    }
    url
}

/// Tauri command: fetch the conversation thread with one person. Returns the
/// messages newest-first plus an optional opaque `nextCursor` for loading older
/// pages. Surfaces failures to the caller so the window can show a load error
/// (and still render the single live DM it already has).
#[tauri::command]
pub async fn fetch_dm_thread(
    with_person_uid: String,
    limit: Option<u32>,
    cursor: Option<String>,
) -> Result<ThreadResponse, String> {
    let target = with_person_uid.trim();
    if target.is_empty() {
        return Err("withPersonUid must not be empty".to_string());
    }

    let access_token = cognito::get_valid_access_token().await.map_err(|e| {
        log(LOG_TAG, &format!("DM_NOTIFY_THREAD_FAIL auth: {e}"));
        format!("Not signed in: {e}")
    })?;

    let base_url = resolve_vault_api_url()
        .map(|u| u.trim_end_matches('/').to_string())
        .map_err(|e| {
            log(LOG_TAG, &format!("DM_NOTIFY_THREAD_FAIL vault url: {e}"));
            format!("Could not resolve server URL: {e}")
        })?;

    let url = build_thread_url(&base_url, target, limit, cursor.as_deref());

    let resp = build_client()
        .get(&url)
        .header("authorization", format!("Bearer {}", access_token))
        .send()
        .await
        .map_err(|e| {
            log(LOG_TAG, &format!("DM_NOTIFY_THREAD_FAIL network: {e}"));
            format!("Network error: {e}")
        })?;

    let status = resp.status();
    if !status.is_success() {
        let server_msg = resp
            .json::<serde_json::Value>()
            .await
            .ok()
            .and_then(|v| v.get("error").and_then(|e| e.as_str()).map(str::to_string));
        log(
            LOG_TAG,
            &format!("DM_NOTIFY_THREAD_FAIL status={status} msg={server_msg:?}"),
        );
        return Err(
            server_msg.unwrap_or_else(|| format!("Failed to load thread (status {})", status.as_u16()))
        );
    }

    let thread = resp.json::<ThreadResponse>().await.map_err(|e| {
        log(LOG_TAG, &format!("DM_NOTIFY_THREAD_FAIL parse: {e}"));
        format!("Could not parse thread response: {e}")
    })?;

    log(
        LOG_TAG,
        &format!("DM_NOTIFY_THREAD_OK with={target} count={}", thread.messages.len()),
    );
    Ok(thread)
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

    // SPIKE: when the custom banner is enabled, route every DM through the
    // in-app banner (commands::banner) — event-driven, no blocking Cocoa run
    // loop — and skip the native firing path entirely.
    if crate::commands::banner::custom_banner_enabled() {
        log(LOG_TAG, &format!("DM_NOTIFY_CUSTOM_BANNER {} DM(s)", body.events.len()));
        for dm in &body.events {
            if let Err(e) = crate::commands::banner::show_dm_banner(app.clone(), dm.clone()).await {
                log(LOG_TAG, &format!("DM_NOTIFY_BANNER_FAIL err={e}"));
            }
        }
        let event_ids: Vec<String> = body.events.iter().map(|e| e.event_id.clone()).collect();
        tauri::async_runtime::spawn(async move { post_ack(event_ids).await });
        let _ = app.emit(EVENT_DM_NEW_EVENTS, &body.events);
        return;
    }

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

    // Fire one macOS notification per DM. Every DM is clickable: a body-click
    // opens the DM detail window. Rich DMs additionally expose action buttons.
    //
    //   * Body-click (any DM) → "open" the detail window.
    //   * "Copy prompt" button (when `prompt` present) → copy the agent prompt.
    //   * "Open details" button (when `details` present) → also opens the detail
    //     window; redundant with body-click but kept as an explicit affordance,
    //     since a banner body-click is not discoverable on macOS.
    //
    // Capturing the click requires the blocking `wait_for_click(true)` path,
    // which busy-spins a Cocoa run loop while the banner is on screen unactioned
    // (~1 core; see share_notify::BlockingNotifyGuard + hq-sync-cpu-spin-debug.md).
    // The shared BlockingNotifyGuard caps that to ~1 process-wide ACROSS the
    // share and DM surfaces, so the CPU ceiling is unchanged by making every DM
    // clickable — this only widens which DMs compete for that single capped slot.
    // Concurrent DMs that can't acquire the slot fall back to fire-and-forget
    // `.send()` (lose the click surface for that banner, but never leak a spin).
    for dm in &body.events {
        let title = dm.from_display_name.clone();
        let message = dm.body.clone();
        let has_prompt = dm.prompt.as_deref().map(|s| !s.trim().is_empty()).unwrap_or(false);
        let has_details = dm.details.as_deref().map(|s| !s.trim().is_empty()).unwrap_or(false);
        let app_for_thread = app.clone();
        let event_clone = dm.clone();

        std::thread::spawn(move || {
            let mut notification = mac_notification_sys::Notification::default();
            notification.title(&title).message(&message);

            // Build the dropdown items present for this DM (order = display order).
            // Plain DMs get no buttons — body-click alone opens the detail window.
            let mut actions: Vec<&str> = Vec::new();
            if has_prompt {
                actions.push("Copy prompt");
            }
            if has_details {
                actions.push("Open details");
            }
            if !actions.is_empty() {
                notification.main_button(mac_notification_sys::MainButton::DropdownActions(
                    "Actions",
                    &actions,
                ));
            }

            // Blocking send capped to ~1 process-wide; concurrent DMs fall back to
            // fire-and-forget (lose the click surface for that banner but never
            // leak a spinning thread).
            let response =
                match crate::commands::share_notify::BlockingNotifyGuard::try_acquire() {
                    Some(guard) => {
                        let r = notification.wait_for_click(true).send();
                        drop(guard);
                        r
                    }
                    None => notification.send(),
                };

            match response {
                Ok(resp) => {
                    // Map the response to an action. Body-click ALWAYS opens the
                    // detail window; copying the prompt is only via the explicit
                    // "Copy prompt" button.
                    let action: Option<&'static str> = match resp {
                        mac_notification_sys::NotificationResponse::ActionButton(name)
                            if name.eq_ignore_ascii_case("copy prompt") =>
                        {
                            Some("copy")
                        }
                        mac_notification_sys::NotificationResponse::ActionButton(name)
                            if name.eq_ignore_ascii_case("open details") =>
                        {
                            Some("open")
                        }
                        mac_notification_sys::NotificationResponse::Click => Some("open"),
                        _ => None,
                    };

                    if let Some(action) = action {
                        let payload = NotificationDmActionEvent {
                            action: action.to_string(),
                            event: event_clone,
                        };
                        if let Err(e) =
                            app_for_thread.emit(EVENT_NOTIFICATION_DM_ACTION, &payload)
                        {
                            log(
                                LOG_TAG,
                                &format!("DM_NOTIFY_EMIT_ACTION_FAILED action={action} err={e}"),
                            );
                        }
                    }
                }
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

// ── DM detail window ────────────────────────────────────────────────────────────
//
// Mirrors `open_share_detail` / `share_detail_window_ready` in share_notify.rs:
// stash the event in managed state, create the window hidden, and let the
// renderer's ready-handshake (`dm_detail_window_ready`) pull the payload + show
// the window — avoids the race where emit_to fires before the JS listener mounts.

/// Tauri command: open (or focus) the DM detail window for a single DM event.
/// Invoked by App.svelte's `notification:dm-action` listener on the "open" action.
#[tauri::command]
pub async fn open_dm_detail(app: AppHandle, event: DmEvent) -> Result<(), String> {
    if let Some(state) = app.try_state::<PendingDmEvents>() {
        *state.0.lock().unwrap_or_else(|p| p.into_inner()) = vec![event.clone()];
    }

    if let Some(window) = app.get_webview_window(DM_DETAIL_LABEL) {
        window.show().map_err(|e| e.to_string())?;
        window.set_focus().map_err(|e| e.to_string())?;
        app.emit_to(DM_DETAIL_LABEL, EVENT_DM_DETAIL_EVENT, &event)
            .map_err(|e| e.to_string())?;
        return Ok(());
    }

    tauri::WebviewWindowBuilder::new(
        &app,
        DM_DETAIL_LABEL,
        tauri::WebviewUrl::App("index.html".into()),
    )
    .title("Direct Message")
    .inner_size(560.0, 580.0)
    .resizable(true)
    .decorations(true)
    .visible(false)
    .build()
    .map_err(|e| e.to_string())?;

    Ok(())
}

/// Tauri command: called by DmDetail.svelte once its listener is registered.
/// Emits the pending event, shows the window, and fires a best-effort ack.
#[tauri::command]
pub async fn dm_detail_window_ready(app: AppHandle) -> Result<(), String> {
    let events: Vec<DmEvent> = app
        .try_state::<PendingDmEvents>()
        .map(|s| s.0.lock().unwrap_or_else(|p| p.into_inner()).clone())
        .unwrap_or_default();

    if let Some(event) = events.first() {
        app.emit_to(DM_DETAIL_LABEL, EVENT_DM_DETAIL_EVENT, event)
            .map_err(|e| e.to_string())?;
    }

    if let Some(window) = app.get_webview_window(DM_DETAIL_LABEL) {
        let _ = window.show();
        let _ = window.set_focus();
    }

    // Best-effort ack so the opened DM isn't re-notified next poll.
    if let Some(event) = events.first() {
        let event_id = event.event_id.clone();
        tauri::async_runtime::spawn(async move {
            post_ack(vec![event_id]).await;
        });
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn send_payload_uses_to_person_uid_and_body() {
        let payload = build_send_payload("prs_abc123", "hey there");
        assert_eq!(payload["toPersonUid"], "prs_abc123");
        assert_eq!(payload["body"], "hey there");
        // Exactly two keys — no stray `toEmail` (server rejects both present).
        let obj = payload.as_object().expect("payload is a JSON object");
        assert_eq!(obj.len(), 2);
        assert!(!obj.contains_key("toEmail"));
    }

    #[test]
    fn dm_event_deserializes_camel_case_from_inbox() {
        // The reply target (`fromPersonUid`) must survive the wire round-trip so
        // the detail window can address a reply to the original sender.
        let json = r#"{
            "eventId": "evt_1",
            "fromPersonUid": "prs_sender",
            "fromEmail": "a@b.com",
            "fromDisplayName": "Ada",
            "body": "hi",
            "createdAt": "2026-05-29T00:00:00Z"
        }"#;
        let dm: DmEvent = serde_json::from_str(json).expect("DmEvent parses");
        assert_eq!(dm.from_person_uid, "prs_sender");
        assert_eq!(dm.body, "hi");
        assert!(dm.prompt.is_none());
        assert!(dm.details.is_none());
    }

    #[test]
    fn thread_url_includes_person_and_optional_params() {
        // Base case: just the recipient.
        assert_eq!(
            build_thread_url("https://api.example.com", "prs_x", None, None),
            "https://api.example.com/v1/notify/thread?withPersonUid=prs_x",
        );
        // Limit + cursor appended in order.
        assert_eq!(
            build_thread_url("https://api.example.com", "prs_x", Some(25), Some("Y3Vy")),
            "https://api.example.com/v1/notify/thread?withPersonUid=prs_x&limit=25&cursor=Y3Vy",
        );
        // A blank cursor is omitted (server returns the newest page).
        assert_eq!(
            build_thread_url("https://api.example.com", "prs_x", None, Some("")),
            "https://api.example.com/v1/notify/thread?withPersonUid=prs_x",
        );
    }

    #[test]
    fn thread_response_deserializes_camel_case_with_direction() {
        // Server tags `direction` relative to the caller; the window renders
        // "out" right-aligned and "in" left-aligned. nextCursor is optional.
        let json = r#"{
            "messages": [
                {
                    "eventId": "evt_2",
                    "fromPersonUid": "prs_me",
                    "fromEmail": "me@b.com",
                    "fromDisplayName": "Me",
                    "body": "my reply",
                    "createdAt": "2026-05-29T00:01:00Z",
                    "direction": "out"
                },
                {
                    "eventId": "evt_1",
                    "fromPersonUid": "prs_them",
                    "fromEmail": "them@b.com",
                    "fromDisplayName": "Them",
                    "body": "their msg",
                    "createdAt": "2026-05-29T00:00:00Z",
                    "direction": "in"
                }
            ]
        }"#;
        let thread: ThreadResponse =
            serde_json::from_str(json).expect("ThreadResponse parses");
        assert_eq!(thread.messages.len(), 2);
        assert_eq!(thread.messages[0].direction, "out");
        assert_eq!(thread.messages[0].from_person_uid, "prs_me");
        assert_eq!(thread.messages[1].direction, "in");
        assert!(thread.next_cursor.is_none());
    }
}
