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

use std::collections::{HashMap, HashSet};
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

/// One incoming connection request as returned by
/// `GET /v1/notify/connections/requests` (US-011). The recipient sees these in
/// the Messages "Requests" segment and acts on them (accept/decline/block). The
/// held first message is quoted (muted) on the request card.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DmRequest {
    /// Symmetric pair key identifying the connection — the action POSTs carry it.
    pub pair_key: String,
    /// Canonical personUid of the requester (the person asking to connect).
    pub from_person_uid: String,
    pub from_email: String,
    pub from_display_name: String,
    /// The held first message the requester sent, if any (quoted on the card).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// Optional trust hint surfaced on the card, e.g. a shared company name.
    /// Present only when the server supplies it; the card omits the hint when
    /// absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shared_company: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestsListResponse {
    #[serde(default)]
    pub requests: Vec<DmRequest>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
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

/// Tauri event emitted when the live unread/request counts change so the
/// popover Messages badge stays current without its own poller. Payload is
/// `UnreadSummary` (see messages.rs). Listened for in App.svelte.
pub const EVENT_DM_UNREAD_SUMMARY: &str = "dm:unread-summary";

/// Tauri event emitted by the SINGLE poll path when a brand-new incoming
/// connection request is observed (US-011). Payload is the `DmRequest`. Drives a
/// DISTINCT native banner ("{name} wants to connect") + the popover
/// request-count badge in App.svelte, and the Requests segment in MessagesShell.
pub const EVENT_DM_REQUEST_NEW: &str = "dm:request-new";

/// Tauri event emitted by the SINGLE poll path (and on a respond action) when a
/// pending request changes state (US-011) — e.g. it was accepted and the held
/// message converted to a live thread, or it was declined/blocked. Payload is
/// `{ pairKey, withPersonUid?, state }`. Flips ComposeMessage Pending bubbles
/// and prunes the Requests list. The MQTT `connection_update` wake routes here
/// via the same poll path (the wake is ids-only; the client re-derives state by
/// diffing the requests list it re-fetches).
pub const EVENT_DM_REQUEST_UPDATE: &str = "dm:request-update";

/// Managed state: running count of unread DMs since the user last opened the
/// Messages window. Incremented by the SINGLE `do_poll` path as new DMs land
/// (no parallel poller) and reset to 0 by `mark_messages_read`. The popover
/// badge reads this via `get_unread_summary` and stays live off the
/// `dm:unread-summary` event emitted on every change.
pub struct UnreadDmState(pub Mutex<u32>);

/// Managed state: the set of pending-request pairKeys the SINGLE poll path has
/// already observed (US-011). The poll fetches the current requests list each
/// cycle and diffs it against this set:
///   * a pairKey present now but not before → a NEW request → emit
///     `dm:request-new` + a distinct native banner.
///   * a pairKey present before but gone now → the request left the pending set
///     (accepted/declined/blocked) → emit `dm:request-update` so Pending bubbles
///     flip and the Requests list prunes. The held message (on accept) arrives
///     via the normal DM inbox poll, so no body is carried here.
/// `initialized` guards the first poll: we seed the set without firing a banner
/// for the backlog the user already had before the app launched.
#[derive(Default)]
pub struct SeenRequestsInner {
    pub initialized: bool,
    pub pair_keys: HashSet<String>,
}

pub struct SeenRequestState(pub Mutex<SeenRequestsInner>);

impl SeenRequestState {
    pub fn new() -> Self {
        SeenRequestState(Mutex::new(SeenRequestsInner::default()))
    }
}

/// Add `delta` to the running unread-DM count and emit `dm:unread-summary` so
/// the popover badge updates immediately. Called from `do_poll` (the one
/// poller). Best-effort: if the request count can't be fetched here we emit the
/// DM count alone — `get_unread_summary` reconciles requests on next read.
fn bump_unread(app: &AppHandle, delta: u32) {
    let Some(state) = app.try_state::<UnreadDmState>() else {
        return;
    };
    let total = {
        let mut guard = state.0.lock().unwrap_or_else(|p| p.into_inner());
        *guard = guard.saturating_add(delta);
        *guard
    };
    // Emit DM count immediately; pendingRequests is filled in on the next
    // explicit get_unread_summary (which does a network read). Keeping the
    // poll path network-free for requests avoids a second fetch per poll.
    let payload = serde_json::json!({ "unreadDms": total, "pendingRequests": 0u32 });
    let _ = app.emit(EVENT_DM_UNREAD_SUMMARY, &payload);
}

/// Read the current unread-DM count from managed state (0 if unset).
pub fn current_unread_dms(app: &AppHandle) -> u32 {
    app.try_state::<UnreadDmState>()
        .map(|s| *s.0.lock().unwrap_or_else(|p| p.into_inner()))
        .unwrap_or(0)
}

/// Reset the unread-DM count to 0. Called when the Messages window opens.
pub fn reset_unread_dms(app: &AppHandle) {
    if let Some(state) = app.try_state::<UnreadDmState>() {
        *state.0.lock().unwrap_or_else(|p| p.into_inner()) = 0;
    }
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

// ── Compose: send a DM to an email or personUid (US-010) ─────────────────────────
//
// The New Message compose flow (RecipientPicker + ComposeMessage) lets the user
// start a conversation with anyone — a known contact, a company teammate, or any
// valid email. Unlike `send_dm` (which always replies to a known sender by
// `toPersonUid`), this addresses the recipient by EITHER `toPersonUid` (when the
// picker resolved one) OR `toEmail` (free-text email). The backend
// `POST /v1/notify/dm` answers with one of two shapes:
//
//   200 { "delivered": true }                         — recipient is an active
//                                                        connection; the message
//                                                        was delivered.
//   202 { "state": "connection_requested" }           — recipient is not yet
//                                                        connected; the message
//                                                        is held and a connect
//                                                        request was sent.
//
// `send_dm_to_email` returns that discriminant to the frontend so the compose UI
// can render an optimistic Pending bubble (202) or open the normal thread (200).

/// The outcome of a compose send, surfaced to the frontend.
///
/// `delivered` → the message reached an active connection (HTTP 200).
/// `connection_requested` → the recipient isn't connected; the message is held
/// and a connect request was sent (HTTP 202). The compose UI renders a Pending
/// bubble for this case until `dm:request-update` confirms (US-011).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", tag = "state")]
pub enum SendDmOutcome {
    /// HTTP 200 — delivered to an active connection.
    Delivered,
    /// HTTP 202 — held; a connection request was sent alongside the message.
    ConnectionRequested,
}

/// Build the `POST /v1/notify/dm` body for a compose send. Exactly one recipient
/// key is emitted: `toPersonUid` when present (preferred — the picker resolved a
/// canonical id), otherwise `toEmail`. The server rejects a request with both
/// keys, so this never emits both. Pure + side-effect-free so the wire shape is
/// unit-testable.
fn build_compose_payload(
    to_person_uid: Option<&str>,
    to_email: Option<&str>,
    body: &str,
) -> serde_json::Value {
    match to_person_uid.map(str::trim).filter(|s| !s.is_empty()) {
        Some(uid) => serde_json::json!({ "toPersonUid": uid, "body": body }),
        None => {
            let email = to_email.map(str::trim).unwrap_or_default();
            serde_json::json!({ "toEmail": email, "body": body })
        }
    }
}

/// Map a successful `POST /v1/notify/dm` response to a `SendDmOutcome`.
/// A 202 (or an explicit `{"state":"connection_requested"}` body) means the
/// recipient isn't connected yet; anything else 2xx means delivered. Pure so the
/// status→discriminant mapping is unit-testable.
fn classify_send_response(status: u16, body: &serde_json::Value) -> SendDmOutcome {
    let body_says_requested = body
        .get("state")
        .and_then(|v| v.as_str())
        .map(|s| s.eq_ignore_ascii_case("connection_requested"))
        .unwrap_or(false);
    if status == 202 || body_says_requested {
        SendDmOutcome::ConnectionRequested
    } else {
        SendDmOutcome::Delivered
    }
}

/// Tauri command: send a DM from the New Message compose flow (US-010).
///
/// Addresses the recipient by `toPersonUid` (preferred, when the picker resolved
/// one) or `toEmail` (free-text email). Returns a `SendDmOutcome` discriminant so
/// the compose UI can render a Pending bubble (connection requested) or open the
/// normal thread (delivered). Surfaces failures to the caller for delivery
/// feedback. Takes the same guarded blocking-send path as `send_dm`.
#[tauri::command]
pub async fn send_dm_to_email(
    to_email: Option<String>,
    to_person_uid: Option<String>,
    body: String,
) -> Result<SendDmOutcome, String> {
    let body_text = body.trim();
    if body_text.is_empty() {
        return Err("Message body must not be empty".to_string());
    }

    let person_uid = to_person_uid.as_deref().map(str::trim).filter(|s| !s.is_empty());
    let email = to_email.as_deref().map(str::trim).filter(|s| !s.is_empty());
    if person_uid.is_none() && email.is_none() {
        return Err("A recipient (email or personUid) is required".to_string());
    }

    let access_token = cognito::get_valid_access_token().await.map_err(|e| {
        log(LOG_TAG, &format!("DM_NOTIFY_COMPOSE_FAIL auth: {e}"));
        format!("Not signed in: {e}")
    })?;

    let base_url = resolve_vault_api_url()
        .map(|u| u.trim_end_matches('/').to_string())
        .map_err(|e| {
            log(LOG_TAG, &format!("DM_NOTIFY_COMPOSE_FAIL vault url: {e}"));
            format!("Could not resolve server URL: {e}")
        })?;

    let url = format!("{}/v1/notify/dm", base_url);
    let payload = build_compose_payload(person_uid, email, body_text);

    let resp = build_client()
        .post(&url)
        .header("authorization", format!("Bearer {}", access_token))
        .json(&payload)
        .send()
        .await
        .map_err(|e| {
            log(LOG_TAG, &format!("DM_NOTIFY_COMPOSE_FAIL network: {e}"));
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
            &format!("DM_NOTIFY_COMPOSE_FAIL status={status} msg={server_msg:?}"),
        );
        return Err(
            server_msg.unwrap_or_else(|| format!("Send failed (status {})", status.as_u16())),
        );
    }

    let status_code = status.as_u16();
    // The body is optional (a bare 200 with no JSON is treated as delivered).
    let parsed = resp.json::<serde_json::Value>().await.unwrap_or(serde_json::Value::Null);
    let outcome = classify_send_response(status_code, &parsed);
    log(
        LOG_TAG,
        &format!("DM_NOTIFY_COMPOSE_OK status={status_code} outcome={outcome:?}"),
    );
    Ok(outcome)
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

// ── Connection requests: list + respond (US-011) ────────────────────────────────
//
// The recipient of an incoming connection request reviews it in the Messages
// "Requests" segment and acts on it. `list_dm_requests` reads the pending set;
// `respond_dm_request` accepts/declines/blocks it. On accept the backend promotes
// the held first message into a live DM_EVENT, so the conversation pane can swap
// the request card for the standard thread on the next thread load.

/// Tauri command: list the caller's pending incoming connection requests.
/// `GET /v1/notify/connections/requests`. Surfaces failures to the caller so the
/// Requests segment can show a load error.
#[tauri::command]
pub async fn list_dm_requests() -> Result<RequestsListResponse, String> {
    let access_token = cognito::get_valid_access_token().await.map_err(|e| {
        log(LOG_TAG, &format!("DM_NOTIFY_REQUESTS_FAIL auth: {e}"));
        format!("Not signed in: {e}")
    })?;

    let base_url = resolve_vault_api_url()
        .map(|u| u.trim_end_matches('/').to_string())
        .map_err(|e| {
            log(LOG_TAG, &format!("DM_NOTIFY_REQUESTS_FAIL vault url: {e}"));
            format!("Could not resolve server URL: {e}")
        })?;

    let url = format!("{}/v1/notify/connections/requests", base_url);

    let resp = build_client()
        .get(&url)
        .header("authorization", format!("Bearer {}", access_token))
        .send()
        .await
        .map_err(|e| {
            log(LOG_TAG, &format!("DM_NOTIFY_REQUESTS_FAIL network: {e}"));
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
            &format!("DM_NOTIFY_REQUESTS_FAIL status={status} msg={server_msg:?}"),
        );
        return Err(server_msg
            .unwrap_or_else(|| format!("Failed to load requests (status {})", status.as_u16())));
    }

    let out = resp.json::<RequestsListResponse>().await.map_err(|e| {
        log(LOG_TAG, &format!("DM_NOTIFY_REQUESTS_FAIL parse: {e}"));
        format!("Could not parse requests response: {e}")
    })?;

    log(
        LOG_TAG,
        &format!("DM_NOTIFY_REQUESTS_OK count={}", out.requests.len()),
    );
    Ok(out)
}

/// Map a respond action to the backend endpoint path segment. Only the three
/// recipient-side actions are valid here (`unblock` is handled elsewhere). Pure
/// so the action→path mapping is unit-testable. Returns `None` for an unknown
/// action so the command can reject it without an unguarded request.
fn respond_action_path(action: &str) -> Option<&'static str> {
    match action.trim().to_ascii_lowercase().as_str() {
        "accept" => Some("accept"),
        "decline" => Some("decline"),
        "block" => Some("block"),
        _ => None,
    }
}

/// Map a respond action to the resulting connection state surfaced to the UI in
/// the `dm:request-update` flip. Pure so the mapping is unit-testable.
fn respond_action_state(action: &str) -> &'static str {
    match action.trim().to_ascii_lowercase().as_str() {
        "accept" => "active",
        "decline" => "declined",
        "block" => "blocked",
        _ => "unknown",
    }
}

/// Tauri command: respond to a pending connection request (US-011).
///
/// `action` is one of `accept` | `decline` | `block`; it POSTs to the matching
/// `/v1/notify/connections/{action}` endpoint with `{ pairKey }`. On success the
/// caller emits `dm:request-update` so the request leaves the Requests segment
/// and (on accept) the held message converts to a thread. Surfaces failures to
/// the caller so the card can show an error and keep its actions.
#[tauri::command]
pub async fn respond_dm_request(
    app: AppHandle,
    pair_key: String,
    action: String,
) -> Result<(), String> {
    let key = pair_key.trim();
    if key.is_empty() {
        return Err("pairKey must not be empty".to_string());
    }
    let path = respond_action_path(&action)
        .ok_or_else(|| format!("Unsupported action: {action}"))?;

    let access_token = cognito::get_valid_access_token().await.map_err(|e| {
        log(LOG_TAG, &format!("DM_NOTIFY_RESPOND_FAIL auth: {e}"));
        format!("Not signed in: {e}")
    })?;

    let base_url = resolve_vault_api_url()
        .map(|u| u.trim_end_matches('/').to_string())
        .map_err(|e| {
            log(LOG_TAG, &format!("DM_NOTIFY_RESPOND_FAIL vault url: {e}"));
            format!("Could not resolve server URL: {e}")
        })?;

    let url = format!("{}/v1/notify/connections/{}", base_url, path);
    let payload = serde_json::json!({ "pairKey": key });

    let resp = build_client()
        .post(&url)
        .header("authorization", format!("Bearer {}", access_token))
        .json(&payload)
        .send()
        .await
        .map_err(|e| {
            log(LOG_TAG, &format!("DM_NOTIFY_RESPOND_FAIL network: {e}"));
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
            &format!("DM_NOTIFY_RESPOND_FAIL status={status} action={path} msg={server_msg:?}"),
        );
        return Err(server_msg
            .unwrap_or_else(|| format!("Action failed (status {})", status.as_u16())));
    }

    // The request has left the pending set — drop it from the seen-set so a later
    // poll doesn't treat its disappearance as a second state change.
    if let Some(state) = app.try_state::<SeenRequestState>() {
        let mut guard = state.0.lock().unwrap_or_else(|p| p.into_inner());
        guard.pair_keys.remove(key);
    }

    let new_state = respond_action_state(&action);
    let update = serde_json::json!({ "pairKey": key, "state": new_state });
    let _ = app.emit(EVENT_DM_REQUEST_UPDATE, &update);

    log(
        LOG_TAG,
        &format!("DM_NOTIFY_RESPOND_OK action={path} state={new_state}"),
    );
    Ok(())
}

/// Diff the freshly-polled request set against the previously-seen pairKeys.
/// Returns `(new_requests, removed_pair_keys)`:
///   * `new_requests` — requests whose pairKey wasn't seen before (fire
///     `dm:request-new` + banner).
///   * `removed_pair_keys` — pairKeys seen before but absent now (fire
///     `dm:request-update`; the connection left the pending set).
/// Pure (operates on the provided set + slice) so the diff is unit-testable.
fn diff_requests(
    seen: &HashSet<String>,
    current: &[DmRequest],
) -> (Vec<DmRequest>, Vec<String>) {
    let current_keys: HashSet<&str> = current.iter().map(|r| r.pair_key.as_str()).collect();
    let new_requests: Vec<DmRequest> = current
        .iter()
        .filter(|r| !seen.contains(&r.pair_key))
        .cloned()
        .collect();
    let removed: Vec<String> = seen
        .iter()
        .filter(|k| !current_keys.contains(k.as_str()))
        .cloned()
        .collect();
    (new_requests, removed)
}

/// Poll the connection-requests list and emit request events off the diff.
/// Folded into the SINGLE `do_poll` path (NOT a parallel poller). Best-effort:
/// any failure logs and returns without disturbing the DM-inbox poll. The first
/// poll seeds the seen-set silently (no banner for the pre-launch backlog).
async fn poll_requests(app: &AppHandle, base_url: &str, access_token: &str) {
    let url = format!("{}/v1/notify/connections/requests", base_url);
    let resp = build_client()
        .get(&url)
        .header("authorization", format!("Bearer {}", access_token))
        .send()
        .await;

    let list = match resp {
        Err(e) => {
            log(LOG_TAG, &format!("DM_NOTIFY_REQ_POLL_NETWORK_FAIL {e}"));
            return;
        }
        Ok(r) => {
            let status = r.status();
            if !status.is_success() {
                log(LOG_TAG, &format!("DM_NOTIFY_REQ_POLL_ERROR status={status}"));
                return;
            }
            match r.json::<RequestsListResponse>().await {
                Ok(b) => b,
                Err(e) => {
                    log(LOG_TAG, &format!("DM_NOTIFY_REQ_POLL_ERROR parse: {e}"));
                    return;
                }
            }
        }
    };

    let Some(state) = app.try_state::<SeenRequestState>() else {
        return;
    };

    let (new_requests, removed, first_run) = {
        let mut guard = state.0.lock().unwrap_or_else(|p| p.into_inner());
        let first_run = !guard.initialized;
        let (new_requests, removed) = diff_requests(&guard.pair_keys, &list.requests);
        // Reconcile the seen-set to exactly the current pending pairKeys.
        guard.pair_keys = list.requests.iter().map(|r| r.pair_key.clone()).collect();
        guard.initialized = true;
        (new_requests, removed, first_run)
    };

    if first_run {
        // Seed silently — the user already had these before launch.
        log(
            LOG_TAG,
            &format!("DM_NOTIFY_REQ_POLL_SEED count={}", list.requests.len()),
        );
        return;
    }

    for req in &new_requests {
        log(
            LOG_TAG,
            &format!("DM_NOTIFY_REQ_NEW from={} pair={}", req.from_email, req.pair_key),
        );
        let _ = app.emit(EVENT_DM_REQUEST_NEW, req);
    }
    for pair_key in &removed {
        // The request left the pending set. We can't tell accept vs decline from
        // its disappearance alone, so report a neutral "resolved" flip; on accept
        // the held message arrives via the DM inbox poll and renders the thread.
        let update = serde_json::json!({ "pairKey": pair_key, "state": "resolved" });
        log(LOG_TAG, &format!("DM_NOTIFY_REQ_RESOLVED pair={pair_key}"));
        let _ = app.emit(EVENT_DM_REQUEST_UPDATE, &update);
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

    // Fold connection-request polling into the SINGLE poll path (US-011) — NOT a
    // parallel poller. Runs every cycle before the inbox fetch so request events
    // fire even when the DM inbox is empty (the inbox path returns early on an
    // empty body). Best-effort: any failure logs and returns without disturbing
    // the DM-inbox poll below.
    poll_requests(app, &base_url, &access_token).await;

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

    // Extend the SINGLE poll path with unread accounting (US-009) — NOT a
    // parallel poller. Every freshly-polled DM increments the running unread
    // count and emits `dm:unread-summary` so the popover Messages badge stays
    // live. The count is reset when the Messages window opens.
    bump_unread(app, body.events.len() as u32);

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
    fn compose_payload_prefers_person_uid_single_key() {
        // When a personUid is resolved, address by it — never emit toEmail too
        // (the server rejects both present).
        let payload = build_compose_payload(Some("prs_x"), Some("a@b.com"), "hi");
        let obj = payload.as_object().expect("object");
        assert_eq!(obj.len(), 2);
        assert_eq!(payload["toPersonUid"], "prs_x");
        assert_eq!(payload["body"], "hi");
        assert!(!obj.contains_key("toEmail"));
    }

    #[test]
    fn compose_payload_falls_back_to_email_single_key() {
        // Free-text email with no resolved personUid → address by toEmail only.
        let payload = build_compose_payload(None, Some("new@person.com"), "hello");
        let obj = payload.as_object().expect("object");
        assert_eq!(obj.len(), 2);
        assert_eq!(payload["toEmail"], "new@person.com");
        assert!(!obj.contains_key("toPersonUid"));
        // A blank personUid is treated as absent.
        let blank = build_compose_payload(Some("   "), Some("x@y.com"), "h");
        assert_eq!(blank["toEmail"], "x@y.com");
        assert!(!blank.as_object().unwrap().contains_key("toPersonUid"));
    }

    #[test]
    fn classify_send_response_maps_status_and_body() {
        let null = serde_json::Value::Null;
        // 200 → delivered.
        assert_eq!(
            classify_send_response(200, &null),
            SendDmOutcome::Delivered
        );
        // 202 → connection requested even with an empty body.
        assert_eq!(
            classify_send_response(202, &null),
            SendDmOutcome::ConnectionRequested
        );
        // An explicit body state wins even on a 200 (defensive).
        let body = serde_json::json!({ "state": "connection_requested" });
        assert_eq!(
            classify_send_response(200, &body),
            SendDmOutcome::ConnectionRequested
        );
        // A delivered body on 200 stays delivered.
        let delivered = serde_json::json!({ "delivered": true });
        assert_eq!(
            classify_send_response(200, &delivered),
            SendDmOutcome::Delivered
        );
    }

    #[test]
    fn send_dm_outcome_serializes_to_state_tag() {
        // The frontend discriminates on `state` — lock the wire shape.
        let requested = serde_json::to_value(SendDmOutcome::ConnectionRequested).unwrap();
        assert_eq!(requested["state"], "connectionRequested");
        let delivered = serde_json::to_value(SendDmOutcome::Delivered).unwrap();
        assert_eq!(delivered["state"], "delivered");
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

    fn mk_request(pair_key: &str) -> DmRequest {
        DmRequest {
            pair_key: pair_key.to_string(),
            from_person_uid: "prs_x".to_string(),
            from_email: "x@y.com".to_string(),
            from_display_name: "Ex".to_string(),
            message: None,
            shared_company: None,
            created_at: "2026-06-05T00:00:00Z".to_string(),
        }
    }

    #[test]
    fn respond_action_path_maps_known_actions_only() {
        assert_eq!(respond_action_path("accept"), Some("accept"));
        assert_eq!(respond_action_path("Decline"), Some("decline"));
        assert_eq!(respond_action_path("  BLOCK "), Some("block"));
        // Unknown / unsupported actions are rejected (no unguarded request).
        assert_eq!(respond_action_path("unblock"), None);
        assert_eq!(respond_action_path(""), None);
        assert_eq!(respond_action_path("delete"), None);
    }

    #[test]
    fn respond_action_state_maps_to_ui_states() {
        assert_eq!(respond_action_state("accept"), "active");
        assert_eq!(respond_action_state("decline"), "declined");
        assert_eq!(respond_action_state("block"), "blocked");
        assert_eq!(respond_action_state("nope"), "unknown");
    }

    #[test]
    fn diff_requests_detects_new_and_removed() {
        // Seen had {a, b}; current has {b, c}. → new = [c], removed = [a].
        let seen: HashSet<String> =
            ["a", "b"].iter().map(|s| s.to_string()).collect();
        let current = vec![mk_request("b"), mk_request("c")];
        let (new_requests, removed) = diff_requests(&seen, &current);
        assert_eq!(new_requests.len(), 1);
        assert_eq!(new_requests[0].pair_key, "c");
        assert_eq!(removed, vec!["a".to_string()]);
    }

    #[test]
    fn diff_requests_first_run_all_new_none_removed() {
        // Empty seen-set (first poll before seeding) → every current request is
        // "new" and nothing is removed. The seed guard in poll_requests is what
        // suppresses the banner on the very first run; the diff itself is pure.
        let seen: HashSet<String> = HashSet::new();
        let current = vec![mk_request("a"), mk_request("b")];
        let (new_requests, removed) = diff_requests(&seen, &current);
        assert_eq!(new_requests.len(), 2);
        assert!(removed.is_empty());
    }

    #[test]
    fn diff_requests_stable_set_no_events() {
        // Unchanged pending set → no new, no removed (no spurious events/banners).
        let seen: HashSet<String> =
            ["a", "b"].iter().map(|s| s.to_string()).collect();
        let current = vec![mk_request("a"), mk_request("b")];
        let (new_requests, removed) = diff_requests(&seen, &current);
        assert!(new_requests.is_empty());
        assert!(removed.is_empty());
    }

    #[test]
    fn dm_request_deserializes_camel_case_from_requests_endpoint() {
        // The card needs pairKey (for the action POST) + the held message to
        // survive the wire round-trip; the trust hint is optional.
        let json = r#"{
            "pairKey": "pair_ab",
            "fromPersonUid": "prs_req",
            "fromEmail": "req@b.com",
            "fromDisplayName": "Reqer",
            "message": "hey, can we connect?",
            "sharedCompany": "Indigo",
            "createdAt": "2026-06-05T00:00:00Z"
        }"#;
        let req: DmRequest = serde_json::from_str(json).expect("DmRequest parses");
        assert_eq!(req.pair_key, "pair_ab");
        assert_eq!(req.from_person_uid, "prs_req");
        assert_eq!(req.message.as_deref(), Some("hey, can we connect?"));
        assert_eq!(req.shared_company.as_deref(), Some("Indigo"));
    }

    #[test]
    fn requests_list_response_tolerates_missing_fields() {
        // An empty/absent requests array and absent cursor must not break parsing
        // (the Requests segment renders the empty state).
        let empty: RequestsListResponse =
            serde_json::from_str("{}").expect("empty requests parses");
        assert!(empty.requests.is_empty());
        assert!(empty.next_cursor.is_none());
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
