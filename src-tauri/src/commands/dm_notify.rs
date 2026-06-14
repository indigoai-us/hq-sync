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

/// Tauri event emitted by the SINGLE poll path when a new reply lands in the
/// thread the user currently has open (US-022). A "thread" wake on the person
/// topic routes through the same `poll_dm_once` → `do_poll` path as DMs/channels
/// (the MQTT wake is ids-only); `do_poll` re-fetches the active thread and emits
/// this for each reply not previously seen. Payload is `{ rootEventId, reply,
/// replyCount }` — the open ThreadPanel appends `reply` and the root bubble in
/// the main Conversation bumps to `replyCount`. Listened for in ThreadPanel +
/// MessagesShell. There is NO parallel thread poller.
pub const EVENT_THREAD_NEW_REPLY: &str = "thread:new-reply";

/// Tauri event emitted by the SINGLE poll path when reactions on a message in
/// the conversation the user currently has open change (US-025). A "reaction"
/// wake on the person topic routes through the same `poll_dm_once` → `do_poll`
/// path as DMs/channels/threads (the MQTT wake is ids-only); `do_poll`
/// re-fetches the open conversation's reactions and emits this for each message
/// whose aggregate set changed since the last poll. Payload is `MessageReactions`
/// (`{ messageScope, messageId, reactions }` — see messages.rs). The open
/// Conversation host applies it via `applyReactionEvent`, reconciling any
/// optimistic toggle. There is NO parallel reaction poller.
pub const EVENT_MESSAGE_REACTION: &str = "message:reaction";

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

/// Tauri event emitted by the SINGLE poll path when a channel the caller is in
/// has new activity (US-018). Payload is `{ channelId, unread }`. ChannelView
/// (if open on that channel) refreshes its messages; ChannelList bumps the
/// per-channel unread badge; App.svelte folds it into the popover badge accent.
/// The "channel" MQTT wake on the person topic routes here via the same poll
/// path (the wake is ids-only; the client re-derives state by diffing the
/// channels list it re-fetches).
pub const EVENT_CHANNEL_NEW_MESSAGE: &str = "channel:new-message";

/// Tauri event emitted by the SINGLE poll path when a channel's metadata
/// changed (US-018) — a brand-new channel appeared (created/invited), or its
/// name/membership/member-count changed. Payload is the full `Channel` (camel).
/// ChannelList upserts it so a new invite/channel appears live without a manual
/// refresh.
pub const EVENT_CHANNEL_UPDATED: &str = "channel:updated";

/// Managed state for the SINGLE poll path's channel diff (US-018). Tracks, per
/// channel the caller can see, the last-observed unread count so the next poll
/// can detect new activity (unread increased) and emit `channel:new-message`.
/// Also tracks the set of known channelIds so a brand-new channel/invite fires
/// `channel:updated`. `initialized` guards the first poll: we seed the maps
/// without firing events for the backlog the user already had before launch.
#[derive(Default)]
pub struct SeenChannelsInner {
    pub initialized: bool,
    /// channelId → last-observed unread count.
    pub unread_by_id: HashMap<String, u32>,
}

pub struct SeenChannelState(pub Mutex<SeenChannelsInner>);

impl SeenChannelState {
    pub fn new() -> Self {
        SeenChannelState(Mutex::new(SeenChannelsInner::default()))
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

/// Upper bound on the per-machine `notified` ring. The repeated boundary events
/// (the cause of the re-notify bug) are always the newest, so they never reach
/// the eviction end of the FIFO — 200 is comfortably more than any single
/// `?since=` page (`limit=50`).
const NOTIFIED_CAP: usize = 200;

/// Per-machine cursor state. `cursor` is the ISO8601 `createdAt` of the newest
/// DM seen (the `?since=` value). `notified` is a bounded FIFO of recently
/// banner-fired `eventId`s: the inbox treats `?since=` as **inclusive**, so the
/// boundary DM(s) — and any DM sharing the cursor's exact timestamp — are
/// returned on every subsequent poll. Without an id-level guard that re-fires the
/// same banner each poll/launch (the same class of bug fixed in share_notify on
/// 2026-05-29). Deduping by id makes re-notification impossible regardless of the
/// server's `since` semantics.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct CursorEntry {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    cursor: Option<String>,
    #[serde(default)]
    notified: Vec<String>,
}

/// Back-compat shim: earlier builds stored a bare ISO string per machine. Accept
/// both the new object form and the legacy string form on read so an upgrade
/// doesn't re-notify every historical DM once.
#[derive(Deserialize)]
#[serde(untagged)]
enum CursorEntryCompat {
    Entry(CursorEntry),
    Legacy(String),
}

impl From<CursorEntryCompat> for CursorEntry {
    fn from(c: CursorEntryCompat) -> Self {
        match c {
            CursorEntryCompat::Entry(e) => e,
            CursorEntryCompat::Legacy(s) => CursorEntry {
                cursor: Some(s),
                notified: Vec::new(),
            },
        }
    }
}

type CursorStore = HashMap<String, CursorEntry>;

fn cursor_path() -> Result<std::path::PathBuf, String> {
    paths::hq_config_dir().map(|d| d.join("dm-cursor.json"))
}

/// Read the whole store, normalising any legacy bare-string entries to the
/// current object shape.
fn read_cursor_store() -> CursorStore {
    let Ok(path) = cursor_path() else {
        return CursorStore::default();
    };
    let Ok(contents) = std::fs::read_to_string(&path) else {
        return CursorStore::default();
    };
    match serde_json::from_str::<HashMap<String, CursorEntryCompat>>(&contents) {
        Ok(store) => store.into_iter().map(|(k, v)| (k, v.into())).collect(),
        Err(_) => CursorStore::default(),
    }
}

fn read_cursor_entry(machine_id: &str) -> CursorEntry {
    read_cursor_store().remove(machine_id).unwrap_or_default()
}

fn write_cursor_entry(machine_id: &str, entry: &CursorEntry) {
    let Ok(path) = cursor_path() else { return };
    // Re-read (with normalisation) so we never clobber other machines' entries.
    let mut store = read_cursor_store();
    store.insert(machine_id.to_string(), entry.clone());
    if let Ok(json) = serde_json::to_string_pretty(&store) {
        let _ = std::fs::write(&path, json);
    }
}

/// Split a poll's DMs into the subset to notify (dropping any whose `eventId` is
/// already in `notified`, preserving order) and the updated `notified` ring
/// (bounded to [`NOTIFIED_CAP`], newest at the end). Pure so it is unit-testable
/// without the filesystem or network.
fn partition_unnotified(events: &[DmEvent], notified: &[String]) -> (Vec<DmEvent>, Vec<String>) {
    let seen: HashSet<&str> = notified.iter().map(String::as_str).collect();
    let fresh: Vec<DmEvent> = events
        .iter()
        .filter(|e| !seen.contains(e.event_id.as_str()))
        .cloned()
        .collect();

    let mut updated = notified.to_vec();
    updated.extend(fresh.iter().map(|e| e.event_id.clone()));
    if updated.len() > NOTIFIED_CAP {
        updated.drain(0..updated.len() - NOTIFIED_CAP);
    }
    (fresh, updated)
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

// ── Threads: fetch + reply + fold thread activity into the SINGLE poll (US-022) ──
//
// A thread is a side-conversation hung off a root message (a DM or a channel
// message). The backend (hq-pro, US-021) exposes:
//
//   GET  /v1/notify/threads?rootEventId=&scope=dm|channel[&channelId=|&withPersonUid=]
//        → { root, replies, replyCount }
//   POST /v1/notify/dm                         (+ optional rootEventId) — DM reply
//   POST /v1/notify/channels/{id}/messages     (+ optional rootEventId) — channel reply
//
// Realtime: a "thread" wake ({type:"thread", rootEventId, eventId,...}) lands on
// the person topic and routes through the SAME `poll_dm_once` → `do_poll` path as
// DMs/channels. `do_poll` re-fetches whichever thread the user currently has open
// (tracked in `ActiveThreadState`, set by the frontend when a ThreadPanel opens /
// cleared when it closes) and emits `thread:new-reply` for replies it hasn't seen
// yet. There is NO parallel thread poller.

/// One message in a thread (the pinned root or a reply). Same wire shape as a DM
/// `ThreadMessage` / channel `ChannelMessage` — `direction` is tagged by the
/// server relative to the caller ("in"/"out"). Tolerant of server additions.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadReply {
    pub event_id: String,
    pub from_person_uid: String,
    #[serde(default)]
    pub from_email: String,
    #[serde(default)]
    pub from_display_name: String,
    pub body: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
    pub created_at: String,
    #[serde(default)]
    pub direction: String,
}

/// The full thread view returned by `GET /v1/notify/threads`: the pinned root
/// message, the reply list (newest-first, like the other thread/channel fetches),
/// and the authoritative `replyCount`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadView {
    pub root: ThreadReply,
    #[serde(default)]
    pub replies: Vec<ThreadReply>,
    #[serde(default)]
    pub reply_count: u32,
}

/// Managed state: the thread the user currently has open in the ThreadPanel, if
/// any (US-022). Set by `set_active_thread` when a panel opens and cleared when it
/// closes. The SINGLE poll path reads this to know which thread to re-fetch on a
/// "thread" wake and which reply event-ids it has already surfaced, so it only
/// emits `thread:new-reply` for genuinely new replies.
#[derive(Default)]
pub struct ActiveThreadInner {
    /// The root message id of the open thread (`None` = no panel open).
    pub root_event_id: Option<String>,
    /// "dm" | "channel".
    pub scope: String,
    /// Present for a channel thread.
    pub channel_id: Option<String>,
    /// Present for a DM thread.
    pub with_person_uid: Option<String>,
    /// Reply event-ids already surfaced (seeded on open) so the poll only emits
    /// genuinely-new replies.
    pub seen_reply_ids: HashSet<String>,
}

pub struct ActiveThreadState(pub Mutex<ActiveThreadInner>);

impl ActiveThreadState {
    pub fn new() -> Self {
        ActiveThreadState(Mutex::new(ActiveThreadInner::default()))
    }
}

/// Managed state: the conversation the user currently has open and the message
/// ids visible in it, so the SINGLE poll path can re-fetch reactions on a
/// "reaction" wake (US-025). Set by `set_active_conversation` when a Conversation
/// host opens/changes its message list and cleared when it closes. The poll path
/// reads `scope` + `message_ids` to know what to re-fetch, and `last_seen` (a
/// per-message JSON snapshot of the last-emitted aggregate set) so it only emits
/// `message:reaction` for messages whose reactions actually changed.
#[derive(Default)]
pub struct ActiveConversationInner {
    /// The open conversation's messageScope (`dm:…` | `chan:…`); `None` = none.
    pub scope: Option<String>,
    /// The eventIds currently rendered in the open Conversation.
    pub message_ids: Vec<String>,
    /// messageId → last-emitted aggregate snapshot (serialized) so the poll only
    /// emits genuinely-changed reaction sets.
    pub last_seen: HashMap<String, String>,
}

pub struct ActiveConversationState(pub Mutex<ActiveConversationInner>);

impl ActiveConversationState {
    pub fn new() -> Self {
        ActiveConversationState(Mutex::new(ActiveConversationInner::default()))
    }
}

/// Tauri command: register (or clear) the conversation the open Conversation host
/// currently shows (US-025). Called with the messageScope + the visible message
/// ids when a DM/channel/thread pane opens or its message list changes, so the
/// SINGLE poll path knows which messages to re-fetch reactions for on a
/// "reaction" wake.
///
/// Behavior:
///   * A *new* scope replaces the active conversation and clears the last-seen
///     snapshot (so a switch doesn't suppress the first emit for the new one).
///   * The *same* scope MERGES the message-id sets (deduped). This lets a
///     ThreadPanel (whose replies share the parent conversation's scope) and the
///     main pane coexist over the single active-conversation slot — `poll_reactions`
///     re-fetches the union, and both hosts' `message:reaction` listeners apply
///     the per-message events (each ignoring ids it doesn't render).
///   * A `None` scope clears it (host teardown / close).
#[tauri::command]
pub fn set_active_conversation(
    app: AppHandle,
    scope: Option<String>,
    message_ids: Option<Vec<String>>,
) -> Result<(), String> {
    let Some(state) = app.try_state::<ActiveConversationState>() else {
        return Ok(());
    };
    let mut guard = state.0.lock().unwrap_or_else(|p| p.into_inner());
    match scope.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        Some(s) => {
            let incoming: Vec<String> = message_ids
                .unwrap_or_default()
                .into_iter()
                .map(|m| m.trim().to_string())
                .filter(|m| !m.is_empty())
                .collect();
            if guard.scope.as_deref() == Some(s) {
                // Same conversation — merge the id sets (dedupe, preserve order).
                for id in incoming {
                    if !guard.message_ids.contains(&id) {
                        guard.message_ids.push(id);
                    }
                }
            } else {
                // A scope change invalidates the last-seen snapshot.
                guard.last_seen.clear();
                guard.scope = Some(s.to_string());
                guard.message_ids = incoming;
            }
            log(LOG_TAG, &format!("DM_NOTIFY_ACTIVE_CONV_SET scope={s}"));
        }
        None => {
            *guard = ActiveConversationInner::default();
            log(LOG_TAG, "DM_NOTIFY_ACTIVE_CONV_CLEAR");
        }
    }
    Ok(())
}

/// Re-fetch reactions for the open conversation and emit `message:reaction` for
/// any message whose aggregate set changed since the last poll (US-025). Folded
/// into the SINGLE `do_poll` path (NOT a parallel poller). Best-effort: any
/// failure logs and returns without disturbing the rest of the poll. No-op when
/// no conversation is open.
async fn poll_reactions(app: &AppHandle, base_url: &str, access_token: &str) {
    // Snapshot the descriptor without holding the lock across the network calls.
    let (scope, message_ids) = {
        let Some(state) = app.try_state::<ActiveConversationState>() else {
            return;
        };
        let guard = state.0.lock().unwrap_or_else(|p| p.into_inner());
        match guard.scope.clone() {
            Some(s) if !guard.message_ids.is_empty() => (s, guard.message_ids.clone()),
            _ => return, // nothing open / no messages
        }
    };

    for message_id in &message_ids {
        let url = format!(
            "{}/v1/notify/reactions?messageScope={}&messageId={}",
            base_url,
            esc_thread_seg(&scope),
            esc_thread_seg(message_id),
        );
        let resp = build_client()
            .get(&url)
            .header("authorization", format!("Bearer {}", access_token))
            .send()
            .await;

        let reactions = match resp {
            Err(e) => {
                log(LOG_TAG, &format!("DM_NOTIFY_REACTION_POLL_NETWORK_FAIL {e}"));
                continue;
            }
            Ok(r) => {
                let status = r.status();
                if !status.is_success() {
                    log(LOG_TAG, &format!("DM_NOTIFY_REACTION_POLL_ERROR status={status}"));
                    continue;
                }
                match r.json::<Vec<crate::commands::messages::ReactionAggregate>>().await {
                    Ok(v) => v,
                    Err(e) => {
                        log(LOG_TAG, &format!("DM_NOTIFY_REACTION_POLL_ERROR parse: {e}"));
                        continue;
                    }
                }
            }
        };

        // Compare against the last-emitted snapshot; emit only on a change. Bail
        // if the conversation was closed/swapped while we were fetching.
        let snapshot = serde_json::to_string(&reactions).unwrap_or_default();
        let changed = {
            let Some(state) = app.try_state::<ActiveConversationState>() else {
                return;
            };
            let mut guard = state.0.lock().unwrap_or_else(|p| p.into_inner());
            if guard.scope.as_deref() != Some(scope.as_str()) {
                return; // conversation closed or swapped — stale fetch
            }
            if guard.last_seen.get(message_id) == Some(&snapshot) {
                false
            } else {
                guard.last_seen.insert(message_id.clone(), snapshot);
                true
            }
        };

        if changed {
            let payload = crate::commands::messages::MessageReactions {
                message_scope: scope.clone(),
                message_id: message_id.clone(),
                reactions,
            };
            log(
                LOG_TAG,
                &format!("DM_NOTIFY_REACTION_CHANGED scope={scope} id={message_id}"),
            );
            let _ = app.emit(EVENT_MESSAGE_REACTION, &payload);
        }
    }
}

/// Build the `GET /v1/notify/threads` URL. Pure + side-effect-free so the query
/// shape is unit-testable. Exactly one of `channelId` / `withPersonUid` rides
/// alongside `rootEventId` + `scope`, matching the US-021 contract. Segments are
/// minimally escaped (`esc_thread_seg`) so a reserved char can't break the URL.
fn build_threads_url(
    base_url: &str,
    root_event_id: &str,
    scope: &str,
    channel_id: Option<&str>,
    with_person_uid: Option<&str>,
) -> String {
    let mut url = format!(
        "{}/v1/notify/threads?rootEventId={}&scope={}",
        base_url,
        esc_thread_seg(root_event_id),
        esc_thread_seg(scope),
    );
    match scope {
        "channel" => {
            if let Some(id) = channel_id.map(str::trim).filter(|s| !s.is_empty()) {
                url.push_str(&format!("&channelId={}", esc_thread_seg(id)));
            }
        }
        _ => {
            if let Some(uid) = with_person_uid.map(str::trim).filter(|s| !s.is_empty()) {
                url.push_str(&format!("&withPersonUid={}", esc_thread_seg(uid)));
            }
        }
    }
    url
}

/// Minimal query-value escape for thread URL params (server-issued ids are
/// URL-safe; this is defense-in-depth, mirroring `messages::esc_seg`). Keeps the
/// dep surface at zero — only the reserved chars that would break the query.
fn esc_thread_seg(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            '/' => "%2F".to_string(),
            '?' => "%3F".to_string(),
            '#' => "%23".to_string(),
            '&' => "%26".to_string(),
            '=' => "%3D".to_string(),
            ' ' => "%20".to_string(),
            other => other.to_string(),
        })
        .collect()
}

/// Normalize a thread scope to "dm" | "channel" (defaults to "dm" for anything
/// unrecognized). Pure so the mapping is unit-testable.
fn normalize_scope(scope: &str) -> String {
    match scope.trim().to_ascii_lowercase().as_str() {
        "channel" => "channel".to_string(),
        _ => "dm".to_string(),
    }
}

/// Tauri command: fetch one thread (its pinned root + reply list + count).
/// `GET /v1/notify/threads`. `scope` is "dm" | "channel"; a channel thread takes
/// `channel_id`, a DM thread takes `with_person_uid`. Surfaces failures to the
/// caller so the ThreadPanel can show a load error.
#[tauri::command]
pub async fn fetch_thread(
    scope: String,
    root_event_id: String,
    channel_id: Option<String>,
    with_person_uid: Option<String>,
) -> Result<ThreadView, String> {
    let root = root_event_id.trim();
    if root.is_empty() {
        return Err("rootEventId must not be empty".to_string());
    }
    let scope_norm = normalize_scope(&scope);

    let access_token = cognito::get_valid_access_token().await.map_err(|e| {
        log(LOG_TAG, &format!("DM_NOTIFY_THREAD_FETCH_FAIL auth: {e}"));
        format!("Not signed in: {e}")
    })?;

    let base_url = resolve_vault_api_url()
        .map(|u| u.trim_end_matches('/').to_string())
        .map_err(|e| {
            log(LOG_TAG, &format!("DM_NOTIFY_THREAD_FETCH_FAIL vault url: {e}"));
            format!("Could not resolve server URL: {e}")
        })?;

    let url = build_threads_url(
        &base_url,
        root,
        &scope_norm,
        channel_id.as_deref(),
        with_person_uid.as_deref(),
    );

    let resp = build_client()
        .get(&url)
        .header("authorization", format!("Bearer {}", access_token))
        .send()
        .await
        .map_err(|e| {
            log(LOG_TAG, &format!("DM_NOTIFY_THREAD_FETCH_FAIL network: {e}"));
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
            &format!("DM_NOTIFY_THREAD_FETCH_FAIL status={status} msg={server_msg:?}"),
        );
        return Err(server_msg
            .unwrap_or_else(|| format!("Failed to load thread (status {})", status.as_u16())));
    }

    let view = resp.json::<ThreadView>().await.map_err(|e| {
        log(LOG_TAG, &format!("DM_NOTIFY_THREAD_FETCH_FAIL parse: {e}"));
        format!("Could not parse thread response: {e}")
    })?;

    log(
        LOG_TAG,
        &format!(
            "DM_NOTIFY_THREAD_FETCH_OK root={root} scope={scope_norm} replies={}",
            view.replies.len()
        ),
    );
    Ok(view)
}

/// Build the reply POST body for a thread reply. Always carries `body` +
/// `rootEventId`; a DM reply also carries `toPersonUid` (channel replies address
/// the channel via the URL path). Pure so the wire shape is unit-testable.
fn build_thread_reply_payload(
    scope: &str,
    root_event_id: &str,
    to_person_uid: Option<&str>,
    body: &str,
) -> serde_json::Value {
    if scope == "channel" {
        serde_json::json!({ "body": body, "rootEventId": root_event_id })
    } else {
        let uid = to_person_uid.unwrap_or_default();
        serde_json::json!({ "toPersonUid": uid, "body": body, "rootEventId": root_event_id })
    }
}

/// Tauri command: post a reply into a thread (US-022). For a DM thread it POSTs
/// `/v1/notify/dm` with `{ toPersonUid, body, rootEventId }`; for a channel
/// thread it POSTs `/v1/notify/channels/{id}/messages` with `{ body, rootEventId }`.
/// Surfaces failures to the caller so the panel composer can show delivery
/// feedback. Takes the same auth + URL plumbing as `send_dm` / `send_channel_message`.
#[tauri::command]
pub async fn send_thread_reply(
    scope: String,
    root_event_id: String,
    body: String,
    channel_id: Option<String>,
    to_person_uid: Option<String>,
) -> Result<(), String> {
    let body_text = body.trim();
    if body_text.is_empty() {
        return Err("Message body must not be empty".to_string());
    }
    let root = root_event_id.trim();
    if root.is_empty() {
        return Err("rootEventId must not be empty".to_string());
    }
    let scope_norm = normalize_scope(&scope);

    let person_uid = to_person_uid.as_deref().map(str::trim).filter(|s| !s.is_empty());
    let channel = channel_id.as_deref().map(str::trim).filter(|s| !s.is_empty());
    if scope_norm == "channel" && channel.is_none() {
        return Err("A channel thread reply requires a channelId".to_string());
    }
    if scope_norm == "dm" && person_uid.is_none() {
        return Err("A DM thread reply requires a toPersonUid".to_string());
    }

    let access_token = cognito::get_valid_access_token().await.map_err(|e| {
        log(LOG_TAG, &format!("DM_NOTIFY_THREAD_REPLY_FAIL auth: {e}"));
        format!("Not signed in: {e}")
    })?;

    let base_url = resolve_vault_api_url()
        .map(|u| u.trim_end_matches('/').to_string())
        .map_err(|e| {
            log(LOG_TAG, &format!("DM_NOTIFY_THREAD_REPLY_FAIL vault url: {e}"));
            format!("Could not resolve server URL: {e}")
        })?;

    let url = if scope_norm == "channel" {
        format!(
            "{}/v1/notify/channels/{}/messages",
            base_url,
            esc_thread_seg(channel.unwrap_or_default())
        )
    } else {
        format!("{}/v1/notify/dm", base_url)
    };
    let payload = build_thread_reply_payload(&scope_norm, root, person_uid, body_text);

    let resp = build_client()
        .post(&url)
        .header("authorization", format!("Bearer {}", access_token))
        .json(&payload)
        .send()
        .await
        .map_err(|e| {
            log(LOG_TAG, &format!("DM_NOTIFY_THREAD_REPLY_FAIL network: {e}"));
            format!("Network error: {e}")
        })?;

    let status = resp.status();
    if status.is_success() {
        log(
            LOG_TAG,
            &format!("DM_NOTIFY_THREAD_REPLY_OK root={root} scope={scope_norm}"),
        );
        return Ok(());
    }

    let server_msg = resp
        .json::<serde_json::Value>()
        .await
        .ok()
        .and_then(|v| v.get("error").and_then(|e| e.as_str()).map(str::to_string));
    log(
        LOG_TAG,
        &format!("DM_NOTIFY_THREAD_REPLY_FAIL status={status} msg={server_msg:?}"),
    );
    Err(server_msg.unwrap_or_else(|| format!("Reply failed (status {})", status.as_u16())))
}

/// Tauri command: register (or clear) the thread the ThreadPanel currently has
/// open (US-022). Called with the root id + scope + the reply ids already shown
/// when a panel opens, so the SINGLE poll path knows which thread to re-fetch on a
/// "thread" wake and which replies it has already surfaced. Called with a `None`
/// root (or the panel-close path) to clear it.
#[tauri::command]
pub fn set_active_thread(
    app: AppHandle,
    root_event_id: Option<String>,
    scope: Option<String>,
    channel_id: Option<String>,
    with_person_uid: Option<String>,
    seen_reply_ids: Option<Vec<String>>,
) -> Result<(), String> {
    let Some(state) = app.try_state::<ActiveThreadState>() else {
        return Ok(());
    };
    let mut guard = state.0.lock().unwrap_or_else(|p| p.into_inner());
    match root_event_id.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        Some(root) => {
            guard.root_event_id = Some(root.to_string());
            guard.scope = normalize_scope(scope.as_deref().unwrap_or("dm"));
            guard.channel_id = channel_id.map(|c| c.trim().to_string()).filter(|s| !s.is_empty());
            guard.with_person_uid =
                with_person_uid.map(|c| c.trim().to_string()).filter(|s| !s.is_empty());
            guard.seen_reply_ids = seen_reply_ids.unwrap_or_default().into_iter().collect();
            log(LOG_TAG, &format!("DM_NOTIFY_ACTIVE_THREAD_SET root={root}"));
        }
        None => {
            *guard = ActiveThreadInner::default();
            log(LOG_TAG, "DM_NOTIFY_ACTIVE_THREAD_CLEAR");
        }
    }
    Ok(())
}

/// Poll the active thread (if any) and emit `thread:new-reply` for replies the
/// open panel hasn't seen yet. Folded into the SINGLE `do_poll` path (NOT a
/// parallel poller). Best-effort: any failure logs and returns without disturbing
/// the rest of the poll. No-op when no thread is open.
async fn poll_active_thread(app: &AppHandle, base_url: &str, access_token: &str) {
    // Snapshot the active-thread descriptor without holding the lock across the
    // network call.
    let descriptor = {
        let Some(state) = app.try_state::<ActiveThreadState>() else {
            return;
        };
        let guard = state.0.lock().unwrap_or_else(|p| p.into_inner());
        guard.root_event_id.as_ref().map(|root| {
            (
                root.clone(),
                guard.scope.clone(),
                guard.channel_id.clone(),
                guard.with_person_uid.clone(),
            )
        })
    };
    let Some((root, scope, channel_id, with_person_uid)) = descriptor else {
        return; // no panel open
    };

    let url = build_threads_url(
        base_url,
        &root,
        &normalize_scope(&scope),
        channel_id.as_deref(),
        with_person_uid.as_deref(),
    );
    let resp = build_client()
        .get(&url)
        .header("authorization", format!("Bearer {}", access_token))
        .send()
        .await;

    let view = match resp {
        Err(e) => {
            log(LOG_TAG, &format!("DM_NOTIFY_THREAD_POLL_NETWORK_FAIL {e}"));
            return;
        }
        Ok(r) => {
            let status = r.status();
            if !status.is_success() {
                log(LOG_TAG, &format!("DM_NOTIFY_THREAD_POLL_ERROR status={status}"));
                return;
            }
            match r.json::<ThreadView>().await {
                Ok(v) => v,
                Err(e) => {
                    log(LOG_TAG, &format!("DM_NOTIFY_THREAD_POLL_ERROR parse: {e}"));
                    return;
                }
            }
        }
    };

    // Compute the genuinely-new replies under the lock, then reconcile the
    // seen-set. Bail (without emitting) if the panel was closed or swapped while
    // we were fetching.
    let new_replies: Vec<ThreadReply> = {
        let Some(state) = app.try_state::<ActiveThreadState>() else {
            return;
        };
        let mut guard = state.0.lock().unwrap_or_else(|p| p.into_inner());
        if guard.root_event_id.as_deref() != Some(root.as_str()) {
            return; // panel closed or swapped — stale fetch
        }
        let fresh: Vec<ThreadReply> = view
            .replies
            .iter()
            .filter(|r| !guard.seen_reply_ids.contains(&r.event_id))
            .cloned()
            .collect();
        for r in &fresh {
            guard.seen_reply_ids.insert(r.event_id.clone());
        }
        fresh
    };

    if new_replies.is_empty() {
        return;
    }

    // Emit oldest→newest so the panel appends in chronological order. The server
    // returns newest-first, so reverse.
    for reply in new_replies.iter().rev() {
        let payload = serde_json::json!({
            "rootEventId": root,
            "reply": reply,
            "replyCount": view.reply_count,
        });
        log(
            LOG_TAG,
            &format!("DM_NOTIFY_THREAD_NEW_REPLY root={root} reply={}", reply.event_id),
        );
        let _ = app.emit(EVENT_THREAD_NEW_REPLY, &payload);
    }
}

// ── Channels: fold channel activity into the SINGLE poll path (US-018) ───────────
//
// A "channel" wake arrives on the caller's person topic and routes through the
// same `poll_dm_once` → `do_poll` path as DMs (the MQTT wake is ids-only). Here
// we list the caller's channels and diff each channel's unread against the
// last-observed value to detect new activity, emitting:
//   * `channel:new-message` { channelId, unread } when a channel's unread grew
//     (or a new channel arrived already carrying unread).
//   * `channel:updated` (full Channel) for a brand-new channel/invite, so the
//     left rail picks it up live.
// There is NO parallel channel poller — this is best-effort and never disturbs
// the DM-inbox poll that follows.

/// The events produced by one channel diff. Pure result type so the diff is
/// unit-testable without an AppHandle.
#[derive(Debug, Default, PartialEq)]
struct ChannelDiff {
    /// (channelId, unread) for channels whose unread increased since last poll.
    new_messages: Vec<(String, u32)>,
    /// channelIds that are brand-new to the caller this poll (fire updated).
    new_channels: Vec<String>,
}

/// Diff the freshly-listed channels against the last-observed unread map.
/// Returns the events to emit. A channel is "new" when its id wasn't seen
/// before; it raises a `new_messages` entry when its unread strictly increased
/// (or it's new AND already carries unread > 0). Pure (operates on the provided
/// map + slice) so the diff is unit-testable.
fn diff_channels(
    seen_unread: &HashMap<String, u32>,
    current: &[crate::commands::messages::Channel],
) -> ChannelDiff {
    let mut diff = ChannelDiff::default();
    for ch in current {
        let unread = ch.unread.unwrap_or(0);
        match seen_unread.get(&ch.channel_id) {
            None => {
                // Brand-new channel/invite this poll.
                diff.new_channels.push(ch.channel_id.clone());
                if unread > 0 {
                    diff.new_messages.push((ch.channel_id.clone(), unread));
                }
            }
            Some(&prev) if unread > prev => {
                diff.new_messages.push((ch.channel_id.clone(), unread));
            }
            _ => {}
        }
    }
    diff
}

/// Poll the channels list and emit channel events off the diff. Folded into the
/// SINGLE `do_poll` path (NOT a parallel poller). Best-effort: any failure logs
/// and returns without disturbing the DM-inbox poll. The first poll seeds the
/// unread map silently (no events for the pre-launch backlog).
async fn poll_channels(app: &AppHandle, base_url: &str, access_token: &str) {
    let url = format!("{}/v1/notify/channels", base_url);
    let resp = build_client()
        .get(&url)
        .header("authorization", format!("Bearer {}", access_token))
        .send()
        .await;

    let list = match resp {
        Err(e) => {
            log(LOG_TAG, &format!("DM_NOTIFY_CHAN_POLL_NETWORK_FAIL {e}"));
            return;
        }
        Ok(r) => {
            let status = r.status();
            if !status.is_success() {
                log(LOG_TAG, &format!("DM_NOTIFY_CHAN_POLL_ERROR status={status}"));
                return;
            }
            match r.json::<crate::commands::messages::ChannelsResponse>().await {
                Ok(b) => b,
                Err(e) => {
                    log(LOG_TAG, &format!("DM_NOTIFY_CHAN_POLL_ERROR parse: {e}"));
                    return;
                }
            }
        }
    };

    let Some(state) = app.try_state::<SeenChannelState>() else {
        return;
    };

    let (diff, channels, first_run) = {
        let mut guard = state.0.lock().unwrap_or_else(|p| p.into_inner());
        let first_run = !guard.initialized;
        let diff = diff_channels(&guard.unread_by_id, &list.channels);
        // Reconcile the unread map to exactly the current channels.
        guard.unread_by_id = list
            .channels
            .iter()
            .map(|c| (c.channel_id.clone(), c.unread.unwrap_or(0)))
            .collect();
        guard.initialized = true;
        (diff, list.channels, first_run)
    };

    if first_run {
        log(LOG_TAG, &format!("DM_NOTIFY_CHAN_POLL_SEED count={}", channels.len()));
        return;
    }

    // Emit `channel:updated` for brand-new channels/invites (full payload so the
    // rail can render the row without a separate fetch).
    for channel_id in &diff.new_channels {
        if let Some(ch) = channels.iter().find(|c| &c.channel_id == channel_id) {
            log(LOG_TAG, &format!("DM_NOTIFY_CHAN_UPDATED id={channel_id}"));
            let _ = app.emit(EVENT_CHANNEL_UPDATED, ch);
        }
    }
    // Emit `channel:new-message` for channels whose unread grew.
    for (channel_id, unread) in &diff.new_messages {
        log(
            LOG_TAG,
            &format!("DM_NOTIFY_CHAN_NEW_MESSAGE id={channel_id} unread={unread}"),
        );
        let payload = serde_json::json!({ "channelId": channel_id, "unread": unread });
        let _ = app.emit(EVENT_CHANNEL_NEW_MESSAGE, &payload);
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

    // Fold channel-activity polling into the SAME single path (US-018) — a
    // "channel" wake on the person topic routes here. Best-effort; emits
    // `channel:new-message` / `channel:updated`. NOT a parallel poller.
    poll_channels(app, &base_url, &access_token).await;

    // Fold thread-activity polling into the SAME single path (US-022) — a
    // "thread" wake on the person topic routes here. Re-fetches whichever thread
    // the ThreadPanel currently has open and emits `thread:new-reply` for replies
    // it hasn't surfaced yet. No-op when no panel is open. NOT a parallel poller.
    poll_active_thread(app, &base_url, &access_token).await;

    // Fold reaction-activity polling into the SAME single path (US-025) — a
    // "reaction" wake on the person topic routes here. Re-fetches reactions for
    // whichever conversation is open and emits `message:reaction` for messages
    // whose aggregate set changed. No-op when no conversation is open. NOT a
    // parallel poller.
    poll_reactions(app, &base_url, &access_token).await;

    let entry = read_cursor_entry(&machine_id);
    let since = entry.cursor.clone();
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

    // Advance the cursor to the newest DM's createdAt across ALL returned events
    // (so it moves forward even when every event was a re-delivered boundary
    // dupe), and dedupe by eventId against the notified ring. Only `fresh` DMs —
    // ones never banner-fired before — drive unread, banners, ack, and the
    // live `dm:new-events` emit. Persist the advanced cursor + grown ring before
    // returning, even on the all-dupes path, so the ring keeps converging.
    let newest = body
        .events
        .iter()
        .map(|e| e.created_at.as_str())
        .max()
        .unwrap_or_default();
    let (fresh, updated_notified) = partition_unnotified(&body.events, &entry.notified);
    write_cursor_entry(
        &machine_id,
        &CursorEntry {
            cursor: (!newest.is_empty()).then(|| newest.to_string()),
            notified: updated_notified,
        },
    );

    if fresh.is_empty() {
        log(
            LOG_TAG,
            &format!(
                "DM_NOTIFY_POLL_OK {} DM(s) all already notified, cursor→{}",
                body.events.len(),
                newest
            ),
        );
        return;
    }

    log(
        LOG_TAG,
        &format!(
            "DM_NOTIFY_POLL_OK {} new DM(s) ({} returned), cursor→{}",
            fresh.len(),
            body.events.len(),
            newest
        ),
    );

    // Extend the SINGLE poll path with unread accounting (US-009) — NOT a
    // parallel poller. Every freshly-polled DM increments the running unread
    // count and emits `dm:unread-summary` so the popover Messages badge stays
    // live. The count is reset when the Messages window opens.
    bump_unread(app, fresh.len() as u32);

    // SPIKE: when the custom banner is enabled, route every DM through the
    // in-app banner (commands::banner) — event-driven, no blocking Cocoa run
    // loop — and skip the native firing path entirely.
    if crate::commands::banner::custom_banner_enabled() {
        log(LOG_TAG, &format!("DM_NOTIFY_CUSTOM_BANNER {} DM(s)", fresh.len()));
        for dm in &fresh {
            if let Err(e) = crate::commands::banner::show_dm_banner(app.clone(), dm.clone()).await {
                log(LOG_TAG, &format!("DM_NOTIFY_BANNER_FAIL err={e}"));
            }
        }
        let event_ids: Vec<String> = fresh.iter().map(|e| e.event_id.clone()).collect();
        // Await (don't detach) so the server-side unread decrement lands within
        // the poll's lifetime. Detaching risked the runtime dropping the task on
        // a quick app quit, leaving the web/other-device unread badge stuck even
        // though this Mac already showed + dismissed the DM. post_ack is
        // best-effort + uses a timed client, so awaiting can't hang the poll.
        post_ack(event_ids).await;
        let _ = app.emit(EVENT_DM_NEW_EVENTS, &fresh);
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
    for dm in &fresh {
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

    // Ack only the fresh DMs — boundary dupes were acked on the poll where they
    // were first fresh, so each event is acked exactly once. Await (don't detach)
    // so the server-side unread decrement reliably lands within the poll's
    // lifetime; post_ack is best-effort + uses a timed client, so it can't hang.
    let event_ids: Vec<String> = fresh.iter().map(|e| e.event_id.clone()).collect();
    post_ack(event_ids).await;

    let _ = app.emit(EVENT_DM_NEW_EVENTS, &fresh);
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

    fn mk_dm(event_id: &str, created_at: &str) -> DmEvent {
        DmEvent {
            event_id: event_id.to_string(),
            from_person_uid: "prs_sender".to_string(),
            from_email: "a@b.com".to_string(),
            from_display_name: "Ada".to_string(),
            body: "hi".to_string(),
            details: None,
            prompt: None,
            created_at: created_at.to_string(),
        }
    }

    #[test]
    fn partition_unnotified_drops_already_notified_boundary_dupes() {
        // The inbox treats `?since=` as inclusive, so the boundary DM (e1) comes
        // back on the next poll. With e1 already in the ring, only e2 is fresh —
        // no duplicate banner. The ring grows to include the newly-fired e2.
        let events = vec![mk_dm("e1", "2026-06-05T00:00:00Z"), mk_dm("e2", "2026-06-05T00:01:00Z")];
        let (fresh, updated) = partition_unnotified(&events, &["e1".to_string()]);
        assert_eq!(fresh.iter().map(|e| e.event_id.as_str()).collect::<Vec<_>>(), ["e2"]);
        assert_eq!(updated, ["e1", "e2"]);
    }

    #[test]
    fn partition_unnotified_all_seen_returns_empty() {
        // Every returned DM already notified → nothing fresh, ring unchanged.
        let events = vec![mk_dm("e1", "t1"), mk_dm("e2", "t2")];
        let (fresh, updated) = partition_unnotified(&events, &["e1".to_string(), "e2".to_string()]);
        assert!(fresh.is_empty());
        assert_eq!(updated, ["e1", "e2"]);
    }

    #[test]
    fn partition_unnotified_ring_is_bounded_newest_kept() {
        // The FIFO never exceeds NOTIFIED_CAP; the oldest ids evict first and the
        // just-fired (newest) id is always retained.
        let prior: Vec<String> = (0..NOTIFIED_CAP).map(|i| format!("old{i}")).collect();
        let events = vec![mk_dm("newest", "t")];
        let (fresh, updated) = partition_unnotified(&events, &prior);
        assert_eq!(fresh.len(), 1);
        assert_eq!(updated.len(), NOTIFIED_CAP);
        assert_eq!(updated.last().unwrap(), "newest");
        assert_eq!(updated.first().unwrap(), "old1"); // old0 evicted
    }

    #[test]
    fn cursor_entry_compat_upgrades_legacy_bare_string() {
        // Earlier builds stored `{"machineX":"2026-06-05T00:00:00Z"}`. Reading it
        // must yield a cursor with an empty ring — not re-notify history.
        let legacy = r#"{"machineX":"2026-06-05T00:00:00Z"}"#;
        let store: HashMap<String, CursorEntryCompat> = serde_json::from_str(legacy).unwrap();
        let entry: CursorEntry = store.into_iter().next().unwrap().1.into();
        assert_eq!(entry.cursor.as_deref(), Some("2026-06-05T00:00:00Z"));
        assert!(entry.notified.is_empty());

        // And the new object form round-trips with its ring intact.
        let modern = r#"{"machineX":{"cursor":"t","notified":["e1","e2"]}}"#;
        let store: HashMap<String, CursorEntryCompat> = serde_json::from_str(modern).unwrap();
        let entry: CursorEntry = store.into_iter().next().unwrap().1.into();
        assert_eq!(entry.cursor.as_deref(), Some("t"));
        assert_eq!(entry.notified, ["e1", "e2"]);
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

    fn mk_channel(id: &str, unread: u32) -> crate::commands::messages::Channel {
        crate::commands::messages::Channel {
            channel_id: id.to_string(),
            name: format!("#{id}"),
            scope: "company".to_string(),
            company_uid: Some("ent_co".to_string()),
            company_name: Some("Acme".to_string()),
            post_policy: None,
            visibility: None,
            membership: Some("joined".to_string()),
            unread: Some(unread),
            member_count: None,
        }
    }

    #[test]
    fn diff_channels_first_seed_marks_all_new() {
        // Empty seen map → every channel is "new"; channels with unread>0 also
        // raise a new-message entry. (The seed guard in poll_channels suppresses
        // emission on the very first poll; the diff itself is pure.)
        let seen: HashMap<String, u32> = HashMap::new();
        let current = vec![mk_channel("a", 0), mk_channel("b", 4)];
        let diff = diff_channels(&seen, &current);
        assert_eq!(diff.new_channels, vec!["a".to_string(), "b".to_string()]);
        assert_eq!(diff.new_messages, vec![("b".to_string(), 4)]);
    }

    #[test]
    fn diff_channels_detects_unread_increase_only() {
        // a stayed flat, b grew, c shrank (read elsewhere) → only b fires.
        let mut seen: HashMap<String, u32> = HashMap::new();
        seen.insert("a".to_string(), 2);
        seen.insert("b".to_string(), 1);
        seen.insert("c".to_string(), 5);
        let current = vec![mk_channel("a", 2), mk_channel("b", 3), mk_channel("c", 0)];
        let diff = diff_channels(&seen, &current);
        assert!(diff.new_channels.is_empty());
        assert_eq!(diff.new_messages, vec![("b".to_string(), 3)]);
    }

    #[test]
    fn diff_channels_new_invite_fires_updated() {
        // A brand-new channel with zero unread (a fresh invite) fires updated but
        // no new-message.
        let mut seen: HashMap<String, u32> = HashMap::new();
        seen.insert("a".to_string(), 0);
        let current = vec![mk_channel("a", 0), mk_channel("new", 0)];
        let diff = diff_channels(&seen, &current);
        assert_eq!(diff.new_channels, vec!["new".to_string()]);
        assert!(diff.new_messages.is_empty());
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

    // ── Threads (US-022) ──────────────────────────────────────────────────────

    #[test]
    fn threads_url_dm_scope_carries_with_person_uid() {
        let url = build_threads_url(
            "https://api.example.com",
            "evt_root",
            "dm",
            None,
            Some("prs_peer"),
        );
        assert_eq!(
            url,
            "https://api.example.com/v1/notify/threads?rootEventId=evt_root&scope=dm&withPersonUid=prs_peer"
        );
    }

    #[test]
    fn threads_url_channel_scope_carries_channel_id() {
        let url = build_threads_url(
            "https://api.example.com",
            "evt_root",
            "channel",
            Some("chn_1"),
            // A stray withPersonUid is ignored for a channel-scoped thread.
            Some("prs_ignored"),
        );
        assert_eq!(
            url,
            "https://api.example.com/v1/notify/threads?rootEventId=evt_root&scope=channel&channelId=chn_1"
        );
    }

    #[test]
    fn normalize_scope_defaults_to_dm() {
        assert_eq!(normalize_scope("channel"), "channel");
        assert_eq!(normalize_scope("CHANNEL"), "channel");
        assert_eq!(normalize_scope("dm"), "dm");
        assert_eq!(normalize_scope("anything"), "dm");
        assert_eq!(normalize_scope(""), "dm");
    }

    #[test]
    fn thread_reply_payload_dm_carries_recipient_and_root() {
        let payload = build_thread_reply_payload("dm", "evt_root", Some("prs_peer"), "hi there");
        assert_eq!(payload["toPersonUid"], "prs_peer");
        assert_eq!(payload["rootEventId"], "evt_root");
        assert_eq!(payload["body"], "hi there");
        let obj = payload.as_object().expect("object");
        assert_eq!(obj.len(), 3);
    }

    #[test]
    fn thread_reply_payload_channel_omits_recipient() {
        // A channel reply addresses the channel via the URL path, so the body
        // carries only body + rootEventId — never a toPersonUid.
        let payload = build_thread_reply_payload("channel", "evt_root", Some("prs_x"), "yo");
        assert_eq!(payload["rootEventId"], "evt_root");
        assert_eq!(payload["body"], "yo");
        let obj = payload.as_object().expect("object");
        assert_eq!(obj.len(), 2);
        assert!(!obj.contains_key("toPersonUid"));
    }

    #[test]
    fn thread_view_deserializes_root_replies_and_count() {
        let json = r#"{
            "root": {
                "eventId": "evt_root",
                "fromPersonUid": "prs_a",
                "body": "the root message",
                "createdAt": "2026-06-05T00:00:00Z",
                "direction": "in"
            },
            "replies": [
                {
                    "eventId": "evt_r2",
                    "fromPersonUid": "prs_me",
                    "body": "second reply",
                    "createdAt": "2026-06-05T00:02:00Z",
                    "direction": "out"
                },
                {
                    "eventId": "evt_r1",
                    "fromPersonUid": "prs_a",
                    "body": "first reply",
                    "createdAt": "2026-06-05T00:01:00Z",
                    "direction": "in"
                }
            ],
            "replyCount": 2
        }"#;
        let view: ThreadView = serde_json::from_str(json).expect("ThreadView parses");
        assert_eq!(view.root.event_id, "evt_root");
        assert_eq!(view.replies.len(), 2);
        assert_eq!(view.reply_count, 2);
        assert_eq!(view.replies[0].direction, "out");
    }
}
