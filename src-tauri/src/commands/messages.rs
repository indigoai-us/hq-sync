//! Dedicated Messages window + its supporting read commands (US-009).
//!
//! The Messages window (`label = "messages"`) is a resizable master/detail
//! surface opened from the popover header. It is built with the SAME
//! ready-handshake pattern as the DM detail window (`open_dm_detail` /
//! `dm_detail_window_ready` in `dm_notify.rs`): create the window hidden, let
//! the renderer mount its listeners and call `messages_window_ready`, then show
//! the window. There is no per-window payload to stash (the shell self-fetches
//! its data via the commands below), so the handshake here is purely a
//! show-on-ready signal — the window has nothing to render against an
//! `emit_to`-raced payload.
//!
//! ## Commands
//!
//!   `open_messages_window`     — create/focus the window (hidden until ready)
//!   `messages_window_ready`    — renderer→Rust: show + focus the window, reset
//!                                the unread-DM badge
//!   `list_contacts`            — `GET /v1/notify/contacts` (people the caller
//!                                can DM: connections + company teammates)
//!   `list_company_members`     — `GET /v1/notify/contacts?companyUid=…` (the
//!                                teammates in one company)
//!   `get_unread_summary`       — counts for the popover badge: unread DMs
//!                                (managed state, fed by the single DM poll)
//!                                + pending connection requests
//!                                (`GET /v1/notify/connections/requests`)
//!
//! ## Why the unread count lives in dm_notify, not here
//!
//! The unread-DM tally is incremented by the ONE `dm_notify::do_poll` path
//! (no parallel poller — see the PRD hard rule). This module only reads that
//! managed state (`dm_notify::current_unread_dms`) and reconciles it with the
//! pending-request count fetched on demand.
//!
//! ## Log codes (`messages` tag in `~/.hq/logs/hq-sync.log`)
//!
//!   `MESSAGES_WINDOW_OPEN` / `_READY` — window lifecycle.
//!   `MESSAGES_CONTACTS_*` / `MESSAGES_MEMBERS_*` / `MESSAGES_UNREAD_*` —
//!   per-command fetch results, mirroring the `DM_NOTIFY_*` code shape.

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

use crate::commands::cognito;
use crate::commands::dm_notify;
use crate::commands::sync::resolve_vault_api_url;
use crate::util::client_info::build_client;
use crate::util::logfile::log;

/// POST `url` with the bearer + JSON `payload`, parsing the response body into
/// `T`. Centralizes the status-check + server-error-extraction used by the
/// channel write commands below. Mirrors `get_json` for the read side.
async fn post_json<T: serde::de::DeserializeOwned>(
    url: &str,
    token: &str,
    payload: &serde_json::Value,
    code: &str,
) -> Result<T, String> {
    let resp = build_client()
        .post(url)
        .header("authorization", format!("Bearer {token}"))
        .json(payload)
        .send()
        .await
        .map_err(|e| {
            log(LOG_TAG, &format!("{code}_NETWORK_FAIL {e}"));
            format!("Network error: {e}")
        })?;

    let status = resp.status();
    if !status.is_success() {
        let server_msg = resp
            .json::<serde_json::Value>()
            .await
            .ok()
            .and_then(|v| v.get("error").and_then(|e| e.as_str()).map(str::to_string));
        log(LOG_TAG, &format!("{code}_ERROR status={status} msg={server_msg:?}"));
        return Err(server_msg
            .unwrap_or_else(|| format!("Request failed (status {})", status.as_u16())));
    }

    parse_body::<T>(resp, code).await
}

const LOG_TAG: &str = "messages";

/// Label of the dedicated Messages window. Routed in `src/main.ts`.
const MESSAGES_LABEL: &str = "messages";

// ── Wire types ──────────────────────────────────────────────────────────────

/// One person the caller can start (or continue) a DM with. Shape is tolerant
/// of server additions — unknown fields are ignored. `companyUid` is present
/// for company teammates and absent for cross-company connections.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Contact {
    pub person_uid: String,
    #[serde(default)]
    pub email: String,
    #[serde(default)]
    pub display_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub company_uid: Option<String>,
    /// "connection" | "company" — how the caller is allowed to DM this person.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// Connection state relative to the caller: "active" | "pending" | "none" |
    /// "blocked" (US-010). Drives the compose "not-connected" affordance. Absent
    /// on older server payloads → the frontend treats absence as "none".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connection_state: Option<String>,
    /// Optional server-supplied conversation timestamps. Older servers omit
    /// these; the frontend also folds in local notification history.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_message_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_activity_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_dm_at: Option<String>,
    /// Optional server-supplied conversation preview fields. The current server
    /// may omit them; preserving them here keeps the desktop rail from dropping
    /// richer contact payloads as the API evolves.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_message_body: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_message_preview: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_message_text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_message_direction: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContactsResponse {
    #[serde(default)]
    pub contacts: Vec<Contact>,
}

/// Counts surfaced on the popover Messages badge. `unread_dms` comes from the
/// single DM-poll path (managed state); `pending_requests` is fetched live.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UnreadSummary {
    pub unread_dms: u32,
    pub pending_requests: u32,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RequestsResponse {
    #[serde(default)]
    requests: Vec<serde_json::Value>,
}

// ── Window: open + ready handshake ──────────────────────────────────────────

/// Tauri command: open (or focus) the dedicated Messages window.
///
/// Mirrors `dm_notify::open_dm_detail`: the window is created hidden
/// (`visible(false)`) and only shown by `messages_window_ready` once the
/// renderer has mounted. There is no stashed payload — the shell self-fetches.
#[tauri::command]
pub async fn open_messages_window(app: AppHandle) -> Result<(), String> {
    if let Some(window) = app.get_webview_window(MESSAGES_LABEL) {
        window.show().map_err(|e| e.to_string())?;
        window.set_focus().map_err(|e| e.to_string())?;
        log(LOG_TAG, "MESSAGES_WINDOW_OPEN focus-existing");
        return Ok(());
    }

    tauri::WebviewWindowBuilder::new(
        &app,
        MESSAGES_LABEL,
        tauri::WebviewUrl::App("index.html".into()),
    )
    .title("Messages")
    .inner_size(720.0, 560.0)
    .min_inner_size(420.0, 420.0)
    .resizable(true)
    .decorations(true)
    .visible(false)
    .build()
    .map_err(|e| e.to_string())?;

    log(LOG_TAG, "MESSAGES_WINDOW_OPEN create");
    Ok(())
}

/// Tauri command: called by MessagesShell.svelte once its listeners are
/// mounted. Shows + focuses the window and resets the unread-DM badge (the user
/// is now looking at their messages). Mirrors `dm_detail_window_ready`.
#[tauri::command]
pub async fn messages_window_ready(app: AppHandle) -> Result<(), String> {
    if let Some(window) = app.get_webview_window(MESSAGES_LABEL) {
        let _ = window.show();
        let _ = window.set_focus();
    }
    // Opening the Messages window clears the unread badge.
    dm_notify::reset_unread_dms(&app);
    log(LOG_TAG, "MESSAGES_WINDOW_READY");
    Ok(())
}

// ── Shared HTTP helper ──────────────────────────────────────────────────────

/// Resolve `(base_url, bearer)` for an authenticated vault call, mapping each
/// failure to a user-facing string. Mirrors the auth+URL preamble used across
/// `dm_notify.rs` commands.
async fn auth_and_base(code: &str) -> Result<(String, String), String> {
    let token = cognito::get_valid_access_token().await.map_err(|e| {
        log(LOG_TAG, &format!("{code}_AUTH_FAIL {e}"));
        format!("Not signed in: {e}")
    })?;
    let base = resolve_vault_api_url()
        .map(|u| u.trim_end_matches('/').to_string())
        .map_err(|e| {
            log(LOG_TAG, &format!("{code}_ERROR vault url: {e}"));
            format!("Could not resolve server URL: {e}")
        })?;
    Ok((base, token))
}

/// GET `url` with the bearer and parse the JSON body into `T`. Centralizes the
/// status-check + server-error-extraction used by every read command here.
async fn get_json<T: serde::de::DeserializeOwned>(
    url: &str,
    token: &str,
    code: &str,
) -> Result<T, String> {
    let resp = build_client()
        .get(url)
        .header("authorization", format!("Bearer {token}"))
        .send()
        .await
        .map_err(|e| {
            log(LOG_TAG, &format!("{code}_NETWORK_FAIL {e}"));
            format!("Network error: {e}")
        })?;

    let status = resp.status();
    if !status.is_success() {
        let server_msg = resp
            .json::<serde_json::Value>()
            .await
            .ok()
            .and_then(|v| v.get("error").and_then(|e| e.as_str()).map(str::to_string));
        log(LOG_TAG, &format!("{code}_ERROR status={status} msg={server_msg:?}"));
        return Err(server_msg
            .unwrap_or_else(|| format!("Request failed (status {})", status.as_u16())));
    }

    parse_body::<T>(resp, code).await
}

/// Like `get_json`, but a `404 Not Found` resolves to `T::default()` instead of
/// an error. Used for optional collections (e.g. a channel roster) where the
/// endpoint may not exist yet server-side — the caller renders an empty list
/// rather than an alarming error banner. Any other non-success status still errors.
async fn get_json_allow_404<T: serde::de::DeserializeOwned + Default>(
    url: &str,
    token: &str,
    code: &str,
) -> Result<T, String> {
    let resp = build_client()
        .get(url)
        .header("authorization", format!("Bearer {token}"))
        .send()
        .await
        .map_err(|e| {
            log(LOG_TAG, &format!("{code}_NETWORK_FAIL {e}"));
            format!("Network error: {e}")
        })?;

    let status = resp.status();
    if status == reqwest::StatusCode::NOT_FOUND {
        log(LOG_TAG, &format!("{code}_EMPTY_404 (treating as empty)"));
        return Ok(T::default());
    }
    if !status.is_success() {
        let server_msg = resp
            .json::<serde_json::Value>()
            .await
            .ok()
            .and_then(|v| v.get("error").and_then(|e| e.as_str()).map(str::to_string));
        log(LOG_TAG, &format!("{code}_ERROR status={status} msg={server_msg:?}"));
        return Err(server_msg
            .unwrap_or_else(|| format!("Request failed (status {})", status.as_u16())));
    }

    parse_body::<T>(resp, code).await
}

/// Read a successful response body as text, then deserialize into `T`. On a
/// decode failure we log a truncated copy of the RAW body (alongside the serde
/// error) so a server↔client contract drift is diagnosable from
/// `~/.hq/logs/hq-sync.log` instead of surfacing only the opaque "error
/// decoding response body". Shared by `get_json` + `post_json`.
async fn parse_body<T: serde::de::DeserializeOwned>(
    resp: reqwest::Response,
    code: &str,
) -> Result<T, String> {
    let body = resp.text().await.map_err(|e| {
        log(LOG_TAG, &format!("{code}_BODY_READ_FAIL {e}"));
        format!("Could not read response: {e}")
    })?;
    serde_json::from_str::<T>(&body).map_err(|e| {
        let snippet: String = body.chars().take(400).collect();
        log(LOG_TAG, &format!("{code}_PARSE_FAIL {e} body={snippet}"));
        format!("Could not parse response: {e}")
    })
}

// ── Read commands ───────────────────────────────────────────────────────────

/// Tauri command: list everyone the caller can DM (active connections + company
/// teammates). `GET /v1/notify/contacts`.
#[tauri::command]
pub async fn list_contacts() -> Result<ContactsResponse, String> {
    let (base, token) = auth_and_base("MESSAGES_CONTACTS").await?;
    let url = format!("{base}/v1/notify/contacts");
    let out: ContactsResponse = get_json(&url, &token, "MESSAGES_CONTACTS").await?;
    log(LOG_TAG, &format!("MESSAGES_CONTACTS_OK count={}", out.contacts.len()));
    Ok(out)
}

/// Tauri command: list the teammates in one company. `GET
/// /v1/notify/contacts?companyUid=…` — the company-scoped slice of the contacts
/// surface, used by the (later) compose flow's company picker.
#[tauri::command]
pub async fn list_company_members(company_uid: String) -> Result<ContactsResponse, String> {
    let target = company_uid.trim();
    if target.is_empty() {
        return Err("companyUid must not be empty".to_string());
    }
    let (base, token) = auth_and_base("MESSAGES_MEMBERS").await?;
    let url = format!("{base}/v1/notify/contacts?companyUid={target}");
    let out: ContactsResponse = get_json(&url, &token, "MESSAGES_MEMBERS").await?;
    log(
        LOG_TAG,
        &format!("MESSAGES_MEMBERS_OK company={target} count={}", out.contacts.len()),
    );
    Ok(out)
}

/// Tauri command: counts for the popover Messages badge.
///
/// `unread_dms` is read from managed state fed by the SINGLE DM poll path (no
/// parallel poller). `pending_requests` is fetched live from
/// `GET /v1/notify/connections/requests`. A failed request fetch degrades
/// gracefully to 0 pending so the unread count still surfaces.
#[tauri::command]
pub async fn get_unread_summary(app: AppHandle) -> Result<UnreadSummary, String> {
    let unread_dms = dm_notify::current_unread_dms(&app);

    let pending_requests = match auth_and_base("MESSAGES_UNREAD").await {
        Ok((base, token)) => {
            let url = format!("{base}/v1/notify/connections/requests");
            match get_json::<RequestsResponse>(&url, &token, "MESSAGES_UNREAD").await {
                Ok(r) => r.requests.len() as u32,
                Err(_) => 0, // already logged; degrade to 0 rather than fail the badge
            }
        }
        Err(_) => 0,
    };

    log(
        LOG_TAG,
        &format!("MESSAGES_UNREAD_OK dms={unread_dms} requests={pending_requests}"),
    );
    Ok(UnreadSummary {
        unread_dms,
        pending_requests,
    })
}

// ── Channels (US-018) ────────────────────────────────────────────────────────
//
// Channels are multi-party conversations, personal or company-scoped. The
// commands below are thin clients over the hq-pro `/v1/notify/channels*`
// surface; all HTTP happens here in Rust (the webview never holds the bearer).
// Realtime: a "channel" wake arrives on the person topic and is folded into the
// SINGLE DM poll path (`dm_notify::do_poll` → `poll_channels`), which emits the
// `channel:new-message` / `channel:updated` Tauri events — there is NO parallel
// channel poller.
//
//   `list_channels`         — GET    /v1/notify/channels
//   `fetch_channel`         — GET    /v1/notify/channels/{id}/messages (+ meta)
//   `create_channel`        — POST   /v1/notify/channels
//   `join_channel`          — POST   /v1/notify/channels/{id}/members (self)
//   `invite_to_channel`     — POST   /v1/notify/channels/{id}/members (others)
//   `send_channel_message`  — POST   /v1/notify/channels/{id}/messages
//   `list_channel_members`  — GET    /v1/notify/channels/{id}/members
//   `remove_channel_member` — DELETE /v1/notify/channels/{id}/members/{uid}
//   `mark_channel_read`     — POST   /v1/notify/channels/{id}/read

/// One channel the caller can see. Tolerant of server additions — unknown
/// fields are ignored. `company_uid` is present only for company-scoped
/// channels. Mirrors the TS `Channel` shape in `src/lib/channels.ts`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Channel {
    pub channel_id: String,
    #[serde(default)]
    pub name: String,
    /// "personal" | "company".
    #[serde(default)]
    pub scope: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub company_uid: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub company_name: Option<String>,
    /// "all" | "owner" — who may post.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub post_policy: Option<String>,
    /// "company" | "private".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visibility: Option<String>,
    /// Caller's membership: "joined" | "invited" | "none".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub membership: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unread: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub member_count: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelsResponse {
    #[serde(default)]
    pub channels: Vec<Channel>,
}

/// One member of a channel. `role` is "owner" | "member".
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelMember {
    pub person_uid: String,
    #[serde(default)]
    pub email: String,
    #[serde(default)]
    pub display_name: String,
    #[serde(default)]
    pub role: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelMembersResponse {
    #[serde(default)]
    pub members: Vec<ChannelMember>,
}

/// One channel message, as returned by `/v1/notify/channels/{id}/messages`.
/// `direction` is tagged by the server relative to the caller ("in"/"out") so
/// the shared `<Conversation showAuthors>` renders it identically to a DM.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelMessage {
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

/// The full channel view: its metadata + a page of messages (newest-first).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelDetail {
    /// The channel metadata. Optional because the `/messages` endpoint may
    /// return only the message page (the caller already holds the channel from
    /// the list); a required field here made an otherwise-fine fetch fail to
    /// decode with "error decoding response body". The frontend already treats
    /// it as optional (`if (detail.channel)`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel: Option<Channel>,
    #[serde(default)]
    pub messages: Vec<ChannelMessage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

/// URL-escape a path segment for the channel id / personUid. These are
/// server-issued slugs (URL-safe), but a defensive minimal escape avoids a
/// malformed URL if a future id carries a reserved char. Keeps the dep surface
/// at zero (no `urlencoding` crate) — only `/`, `?`, `#`, and space are escaped.
fn esc_seg(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            '/' => "%2F".to_string(),
            '?' => "%3F".to_string(),
            '#' => "%23".to_string(),
            ' ' => "%20".to_string(),
            other => other.to_string(),
        })
        .collect()
}

/// Tauri command: list every channel the caller can see (personal + company,
/// joined + invited). `GET /v1/notify/channels`.
#[tauri::command]
pub async fn list_channels() -> Result<ChannelsResponse, String> {
    let (base, token) = auth_and_base("MESSAGES_CHANNELS").await?;
    let url = format!("{base}/v1/notify/channels");
    let out: ChannelsResponse = get_json(&url, &token, "MESSAGES_CHANNELS").await?;
    log(LOG_TAG, &format!("MESSAGES_CHANNELS_OK count={}", out.channels.len()));
    Ok(out)
}

/// Tauri command: fetch one channel's metadata + its newest page of messages.
/// `GET /v1/notify/channels/{id}/messages`. Opening a channel also marks it
/// read server-side (the page read advances the caller's cursor), but the
/// caller should still call `mark_channel_read` to zero the local unread.
#[tauri::command]
pub async fn fetch_channel(
    channel_id: String,
    limit: Option<u32>,
    cursor: Option<String>,
) -> Result<ChannelDetail, String> {
    let id = channel_id.trim();
    if id.is_empty() {
        return Err("channelId must not be empty".to_string());
    }
    let (base, token) = auth_and_base("MESSAGES_CHANNEL_FETCH").await?;
    let mut url = format!("{base}/v1/notify/channels/{}/messages", esc_seg(id));
    let mut sep = '?';
    if let Some(n) = limit {
        url.push_str(&format!("{sep}limit={n}"));
        sep = '&';
    }
    if let Some(c) = cursor.as_deref().filter(|c| !c.is_empty()) {
        url.push_str(&format!("{sep}cursor={}", esc_seg(c)));
    }
    let out: ChannelDetail = get_json(&url, &token, "MESSAGES_CHANNEL_FETCH").await?;
    log(
        LOG_TAG,
        &format!("MESSAGES_CHANNEL_FETCH_OK id={id} msgs={}", out.messages.len()),
    );
    Ok(out)
}

/// Build the `POST /v1/notify/channels` create body. Exactly the fields the
/// server contract expects: `name`, `scope`, optional `companyUid` (required
/// only for company scope), optional `invite` (personUids). Pure so the wire
/// shape is unit-testable.
fn build_create_payload(
    name: &str,
    scope: &str,
    company_uid: Option<&str>,
    invite: &[String],
) -> serde_json::Value {
    let mut obj = serde_json::Map::new();
    obj.insert("name".to_string(), serde_json::Value::String(name.to_string()));
    obj.insert("scope".to_string(), serde_json::Value::String(scope.to_string()));
    if let Some(uid) = company_uid.map(str::trim).filter(|s| !s.is_empty()) {
        obj.insert("companyUid".to_string(), serde_json::Value::String(uid.to_string()));
    }
    if !invite.is_empty() {
        obj.insert(
            "invite".to_string(),
            serde_json::Value::Array(
                invite.iter().map(|u| serde_json::Value::String(u.clone())).collect(),
            ),
        );
    }
    serde_json::Value::Object(obj)
}

/// Tauri command: create a channel. `POST /v1/notify/channels`. `scope` is
/// "personal" | "company"; a company channel requires `company_uid`. Optional
/// `invite` seeds initial members by personUid. Returns the created `Channel`.
#[tauri::command]
pub async fn create_channel(
    name: String,
    scope: String,
    company_uid: Option<String>,
    invite: Option<Vec<String>>,
) -> Result<Channel, String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err("Channel name must not be empty".to_string());
    }
    let scope_norm = scope.trim().to_ascii_lowercase();
    if scope_norm != "personal" && scope_norm != "company" {
        return Err("Channel scope must be 'personal' or 'company'".to_string());
    }
    let company = company_uid.as_deref().map(str::trim).filter(|s| !s.is_empty());
    if scope_norm == "company" && company.is_none() {
        return Err("A company channel requires a companyUid".to_string());
    }
    let invites = invite.unwrap_or_default();

    let (base, token) = auth_and_base("MESSAGES_CHANNEL_CREATE").await?;
    let url = format!("{base}/v1/notify/channels");
    let payload = build_create_payload(trimmed, &scope_norm, company, &invites);
    // The server wraps the created channel in an envelope: `{"channel": {…}}`.
    // Decoding into `Channel` directly failed with `missing field channelId` even
    // though the channel WAS created — so the user saw an error, retried, and hit
    // a 409 "name already taken". Decode the envelope (reuse `ChannelDetail`,
    // whose `channel` is optional and other fields default) and unwrap it.
    let detail: ChannelDetail = post_json(&url, &token, &payload, "MESSAGES_CHANNEL_CREATE").await?;
    let out = detail
        .channel
        .ok_or_else(|| "Create response missing channel object".to_string())?;
    log(
        LOG_TAG,
        &format!("MESSAGES_CHANNEL_CREATE_OK id={} scope={scope_norm}", out.channel_id),
    );
    Ok(out)
}

/// Build the `POST /v1/notify/channels` body for a GROUP DM:
/// `{ scope: "group", participants: [...] }` (no name). Pure → unit-testable.
fn build_group_payload(participants: &[String]) -> serde_json::Value {
    let mut obj = serde_json::Map::new();
    obj.insert("scope".to_string(), serde_json::Value::String("group".to_string()));
    obj.insert(
        "participants".to_string(),
        serde_json::Value::Array(
            participants.iter().map(|p| serde_json::Value::String(p.clone())).collect(),
        ),
    );
    serde_json::Value::Object(obj)
}

/// Tauri command: create or reopen a GROUP DM — an unnamed, participant-keyed
/// channel. `POST /v1/notify/channels { scope:"group", participants }`. The
/// caller is added server-side; the server dedupes by member set (idempotent)
/// and requires ≥3 distinct people total. `participants` are personUids or
/// emails (the server resolves emails). Returns the created/reopened `Channel`.
#[tauri::command]
pub async fn create_group_dm(participants: Vec<String>) -> Result<Channel, String> {
    let cleaned: Vec<String> = participants
        .into_iter()
        .map(|p| p.trim().to_string())
        .filter(|p| !p.is_empty())
        .collect();
    if cleaned.len() < 2 {
        return Err("A group DM needs at least 2 other people".to_string());
    }

    let (base, token) = auth_and_base("MESSAGES_GROUP_CREATE").await?;
    let url = format!("{base}/v1/notify/channels");
    let payload = build_group_payload(&cleaned);
    // Same `{"channel": {…}}` envelope as create_channel — decode via ChannelDetail.
    let detail: ChannelDetail = post_json(&url, &token, &payload, "MESSAGES_GROUP_CREATE").await?;
    let out = detail
        .channel
        .ok_or_else(|| "Group create response missing channel object".to_string())?;
    log(
        LOG_TAG,
        &format!("MESSAGES_GROUP_CREATE_OK id={} members={}", out.channel_id, cleaned.len()),
    );
    Ok(out)
}

/// Tauri command: join a channel the caller was invited to (or a discoverable
/// company channel). `POST /v1/notify/channels/{id}/members` with no body —
/// the server adds the authenticated caller. Returns the updated `Channel`.
#[tauri::command]
pub async fn join_channel(channel_id: String) -> Result<Channel, String> {
    let id = channel_id.trim();
    if id.is_empty() {
        return Err("channelId must not be empty".to_string());
    }
    let (base, token) = auth_and_base("MESSAGES_CHANNEL_JOIN").await?;
    let url = format!("{base}/v1/notify/channels/{}/members", esc_seg(id));
    // Empty body → join self (no `personUid`). The server distinguishes
    // self-join from invite by the presence of `personUid`.
    let payload = serde_json::json!({});
    let out: Channel = post_json(&url, &token, &payload, "MESSAGES_CHANNEL_JOIN").await?;
    log(LOG_TAG, &format!("MESSAGES_CHANNEL_JOIN_OK id={id}"));
    Ok(out)
}

/// Tauri command: invite people to a channel (owner action). `POST
/// /v1/notify/channels/{id}/members` with `{ personUids: [...] }`. Returns the
/// channel's updated member list so the roster refreshes in place.
#[tauri::command]
pub async fn invite_to_channel(
    channel_id: String,
    person_uids: Vec<String>,
) -> Result<ChannelMembersResponse, String> {
    let id = channel_id.trim();
    if id.is_empty() {
        return Err("channelId must not be empty".to_string());
    }
    let cleaned: Vec<String> = person_uids
        .into_iter()
        .map(|u| u.trim().to_string())
        .filter(|u| !u.is_empty())
        .collect();
    if cleaned.is_empty() {
        return Err("At least one person is required".to_string());
    }
    let (base, token) = auth_and_base("MESSAGES_CHANNEL_INVITE").await?;
    let url = format!("{base}/v1/notify/channels/{}/members", esc_seg(id));
    let payload = serde_json::json!({ "personUids": cleaned });
    let out: ChannelMembersResponse =
        post_json(&url, &token, &payload, "MESSAGES_CHANNEL_INVITE").await?;
    log(
        LOG_TAG,
        &format!("MESSAGES_CHANNEL_INVITE_OK id={id} members={}", out.members.len()),
    );
    Ok(out)
}

/// Tauri command: post a message into a channel. `POST
/// /v1/notify/channels/{id}/messages` with `{ body }`. Surfaces failures to the
/// caller (e.g. an owner-only post policy rejection) for composer feedback.
#[tauri::command]
pub async fn send_channel_message(channel_id: String, body: String) -> Result<(), String> {
    let id = channel_id.trim();
    if id.is_empty() {
        return Err("channelId must not be empty".to_string());
    }
    let text = body.trim();
    if text.is_empty() {
        return Err("Message body must not be empty".to_string());
    }
    let (base, token) = auth_and_base("MESSAGES_CHANNEL_SEND").await?;
    let url = format!("{base}/v1/notify/channels/{}/messages", esc_seg(id));
    let payload = serde_json::json!({ "body": text });
    let _: serde_json::Value =
        post_json(&url, &token, &payload, "MESSAGES_CHANNEL_SEND").await?;
    log(LOG_TAG, &format!("MESSAGES_CHANNEL_SEND_OK id={id}"));
    Ok(())
}

/// Tauri command: list a channel's members. `GET
/// /v1/notify/channels/{id}/members`. Drives the roster (name + role; the owner
/// sees the remove affordance).
#[tauri::command]
pub async fn list_channel_members(channel_id: String) -> Result<ChannelMembersResponse, String> {
    let id = channel_id.trim();
    if id.is_empty() {
        return Err("channelId must not be empty".to_string());
    }
    let (base, token) = auth_and_base("MESSAGES_CHANNEL_MEMBERS").await?;
    let url = format!("{base}/v1/notify/channels/{}/members", esc_seg(id));
    // Tolerate a 404 as an empty roster: the GET endpoint may be absent until the
    // server deploy lands. An empty list renders far better than an error banner.
    let out: ChannelMembersResponse =
        get_json_allow_404(&url, &token, "MESSAGES_CHANNEL_MEMBERS").await?;
    log(
        LOG_TAG,
        &format!("MESSAGES_CHANNEL_MEMBERS_OK id={id} count={}", out.members.len()),
    );
    Ok(out)
}

/// Tauri command: remove a member from a channel (owner action). `DELETE
/// /v1/notify/channels/{id}/members/{personUid}`. Returns the updated member
/// list so the roster refreshes in place.
#[tauri::command]
pub async fn remove_channel_member(
    channel_id: String,
    person_uid: String,
) -> Result<ChannelMembersResponse, String> {
    let id = channel_id.trim();
    let uid = person_uid.trim();
    if id.is_empty() || uid.is_empty() {
        return Err("channelId and personUid must not be empty".to_string());
    }
    let (base, token) = auth_and_base("MESSAGES_CHANNEL_REMOVE").await?;
    let url = format!(
        "{base}/v1/notify/channels/{}/members/{}",
        esc_seg(id),
        esc_seg(uid)
    );
    let resp = build_client()
        .delete(&url)
        .header("authorization", format!("Bearer {token}"))
        .send()
        .await
        .map_err(|e| {
            log(LOG_TAG, &format!("MESSAGES_CHANNEL_REMOVE_NETWORK_FAIL {e}"));
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
            &format!("MESSAGES_CHANNEL_REMOVE_ERROR status={status} msg={server_msg:?}"),
        );
        return Err(server_msg
            .unwrap_or_else(|| format!("Remove failed (status {})", status.as_u16())));
    }
    // The server returns the updated member list; tolerate an empty 204 by
    // re-listing only if the body didn't parse.
    let out: ChannelMembersResponse = resp
        .json::<ChannelMembersResponse>()
        .await
        .unwrap_or(ChannelMembersResponse { members: Vec::new() });
    log(LOG_TAG, &format!("MESSAGES_CHANNEL_REMOVE_OK id={id} uid={uid}"));
    Ok(out)
}

/// Tauri command: mark a channel read (zeroes its server-side unread). `POST
/// /v1/notify/channels/{id}/read`. Called when the user opens a channel; the
/// local unread is cleared in the UI immediately and reconciled here.
#[tauri::command]
pub async fn mark_channel_read(channel_id: String) -> Result<(), String> {
    let id = channel_id.trim();
    if id.is_empty() {
        return Err("channelId must not be empty".to_string());
    }
    let (base, token) = auth_and_base("MESSAGES_CHANNEL_READ").await?;
    let url = format!("{base}/v1/notify/channels/{}/read", esc_seg(id));
    let payload = serde_json::json!({});
    let _: serde_json::Value =
        post_json(&url, &token, &payload, "MESSAGES_CHANNEL_READ").await?;
    log(LOG_TAG, &format!("MESSAGES_CHANNEL_READ_OK id={id}"));
    Ok(())
}

// ── Reactions (US-025) ────────────────────────────────────────────────────────
//
// Emoji reactions on any message (DM, channel, or thread reply). Thin clients
// over the hq-pro US-024 surface; all HTTP happens here in Rust (the webview
// never holds the bearer). The `messageScope` is opaque to this layer — the
// frontend builds it (`dm:{pairKey-or-peer}` | `chan:{channelId}`) and the
// server keys the reactions partition by it. React authorization reuses the
// message domain's read gate server-side (no new auth here).
//
// Realtime: a "reaction" wake arrives on the person topic and is folded into the
// SINGLE DM poll path (`dm_notify::do_poll` → `poll_reactions`), which re-fetches
// the open conversation's reactions and emits the `message:reaction` Tauri event
// — there is NO parallel reaction poller.
//
//   `toggle_reaction` — POST (add) / DELETE (remove) /v1/notify/reactions
//   `fetch_reactions` — GET /v1/notify/reactions?messageScope=&messageId=

/// One emoji's aggregate on a single message, as returned by
/// `GET /v1/notify/reactions`. `reacted_by_me` drives the highlighted pill +
/// toggle direction in the UI. Mirrors the TS `ReactionAggregate`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ReactionAggregate {
    pub emoji: String,
    #[serde(default)]
    pub count: u32,
    #[serde(default)]
    pub reacted_by_me: bool,
}

/// The aggregate set for one message. The GET endpoint returns THIS object
/// (`{messageScope, messageId, reactions: [...]}`), not a bare array, so
/// `fetch_reactions` deserializes into `MessageReactions` and returns its
/// `reactions`. This shape is also what the `message:reaction` event carries.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MessageReactions {
    pub message_scope: String,
    pub message_id: String,
    pub reactions: Vec<ReactionAggregate>,
}

/// Build the `/v1/notify/reactions` mutation body. Identical shape for add
/// (POST) and remove (DELETE): `{ messageScope, messageId, emoji }`. Pure so the
/// wire shape is unit-testable.
fn build_reaction_payload(message_scope: &str, message_id: &str, emoji: &str) -> serde_json::Value {
    serde_json::json!({
        "messageScope": message_scope,
        "messageId": message_id,
        "emoji": emoji,
    })
}

/// Build the `GET /v1/notify/reactions` query URL. Pure + side-effect-free so
/// the query shape is unit-testable; segments are minimally escaped (`esc_seg`)
/// so a reserved char in the scope/id/emoji can't break the query.
fn build_reactions_url(base_url: &str, message_scope: &str, message_id: &str) -> String {
    format!(
        "{}/v1/notify/reactions?messageScope={}&messageId={}",
        base_url,
        esc_seg(message_scope),
        esc_seg(message_id),
    )
}

/// Tauri command: add or remove the caller's reaction to a message (US-025).
/// `add = true` → POST `/v1/notify/reactions` (idempotent conditional Put);
/// `add = false` → DELETE the exact key. The UI toggles optimistically and
/// reconciles on the `message:reaction` event, so this surfaces failures to the
/// caller for rollback. `message_scope` is opaque (built by the frontend).
#[tauri::command]
pub async fn toggle_reaction(
    message_scope: String,
    message_id: String,
    emoji: String,
    add: bool,
) -> Result<(), String> {
    let scope = message_scope.trim();
    let id = message_id.trim();
    let e = emoji.trim();
    if scope.is_empty() || id.is_empty() || e.is_empty() {
        return Err("messageScope, messageId, and emoji must not be empty".to_string());
    }

    let (base, token) = auth_and_base("MESSAGES_REACTION").await?;
    let url = format!("{base}/v1/notify/reactions");
    let payload = build_reaction_payload(scope, id, e);

    let client = build_client();
    let req = if add {
        client.post(&url)
    } else {
        client.delete(&url)
    };
    let resp = req
        .header("authorization", format!("Bearer {token}"))
        .json(&payload)
        .send()
        .await
        .map_err(|err| {
            log(LOG_TAG, &format!("MESSAGES_REACTION_NETWORK_FAIL {err}"));
            format!("Network error: {err}")
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
            &format!("MESSAGES_REACTION_ERROR status={status} add={add} msg={server_msg:?}"),
        );
        return Err(server_msg
            .unwrap_or_else(|| format!("Reaction failed (status {})", status.as_u16())));
    }

    log(
        LOG_TAG,
        &format!("MESSAGES_REACTION_OK add={add} scope={scope} id={id} emoji={e}"),
    );
    Ok(())
}

/// Tauri command: fetch the aggregated reactions for one message (US-025).
/// `GET /v1/notify/reactions?messageScope=&messageId=` → the per-emoji counts
/// with `reactedByMe`. Used for the initial load of a conversation's reactions
/// and as the truth source the `message:reaction` event re-fetches. Surfaces
/// failures so the caller can keep its optimistic state.
#[tauri::command]
pub async fn fetch_reactions(
    message_scope: String,
    message_id: String,
) -> Result<Vec<ReactionAggregate>, String> {
    let scope = message_scope.trim();
    let id = message_id.trim();
    if scope.is_empty() || id.is_empty() {
        return Err("messageScope and messageId must not be empty".to_string());
    }
    let (base, token) = auth_and_base("MESSAGES_REACTIONS_GET").await?;
    let url = build_reactions_url(&base, scope, id);
    // Server returns the `MessageReactions` envelope, not a bare array — decoding
    // into `Vec<ReactionAggregate>` threw `invalid type: map, expected a sequence`
    // on every message load. Decode the object and return its `reactions`.
    let out: MessageReactions = get_json(&url, &token, "MESSAGES_REACTIONS_GET").await?;
    log(
        LOG_TAG,
        &format!("MESSAGES_REACTIONS_GET_OK scope={scope} id={id} count={}", out.reactions.len()),
    );
    Ok(out.reactions)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contact_deserializes_camel_case_minimal() {
        // Only personUid is required on the wire; the rest default.
        let json = r#"{ "personUid": "prs_x" }"#;
        let c: Contact = serde_json::from_str(json).expect("Contact parses");
        assert_eq!(c.person_uid, "prs_x");
        assert_eq!(c.email, "");
        assert!(c.company_uid.is_none());
    }

    #[test]
    fn contact_deserializes_full_row() {
        let json = r#"{
            "personUid": "prs_y",
            "email": "a@b.com",
            "displayName": "Ada",
            "companyUid": "ent_co",
            "source": "company",
            "lastMessageAt": "2026-06-12T01:02:03Z",
            "lastActivityAt": "2026-06-11T01:02:03Z",
            "lastDmAt": "2026-06-10T01:02:03Z",
            "lastMessageBody": "latest text",
            "lastMessageDirection": "out"
        }"#;
        let c: Contact = serde_json::from_str(json).expect("Contact parses");
        assert_eq!(c.email, "a@b.com");
        assert_eq!(c.company_uid.as_deref(), Some("ent_co"));
        assert_eq!(c.source.as_deref(), Some("company"));
        assert_eq!(c.last_message_at.as_deref(), Some("2026-06-12T01:02:03Z"));
        assert_eq!(c.last_activity_at.as_deref(), Some("2026-06-11T01:02:03Z"));
        assert_eq!(c.last_dm_at.as_deref(), Some("2026-06-10T01:02:03Z"));
        assert_eq!(c.last_message_body.as_deref(), Some("latest text"));
        assert_eq!(c.last_message_direction.as_deref(), Some("out"));
    }

    #[test]
    fn channel_detail_decodes_without_channel_key() {
        // Regression: the `/v1/notify/channels/{id}/messages` endpoint returns
        // only the message page (no nested `channel`). A required `channel`
        // field made this fail to decode ("error decoding response body") and
        // broke opening a freshly-created/empty channel. `channel` is optional.
        let json = r#"{ "messages": [], "nextCursor": null }"#;
        let detail: ChannelDetail = serde_json::from_str(json).expect("ChannelDetail parses");
        assert!(detail.channel.is_none());
        assert!(detail.messages.is_empty());
        assert!(detail.next_cursor.is_none());
    }

    #[test]
    fn channel_detail_decodes_with_channel_and_messages() {
        let json = r#"{
            "channel": { "channelId": "chn_1", "name": "crew", "scope": "company" },
            "messages": [
                {
                    "eventId": "evt_1",
                    "fromPersonUid": "prs_a",
                    "body": "hi",
                    "createdAt": "2026-06-10T16:00:00Z",
                    "direction": "in"
                }
            ]
        }"#;
        let detail: ChannelDetail = serde_json::from_str(json).expect("ChannelDetail parses");
        let channel = detail.channel.expect("channel present");
        assert_eq!(channel.channel_id, "chn_1");
        assert_eq!(detail.messages.len(), 1);
        assert_eq!(detail.messages[0].body, "hi");
    }

    #[test]
    fn unread_summary_serializes_camel_case() {
        let s = UnreadSummary {
            unread_dms: 3,
            pending_requests: 1,
        };
        let v = serde_json::to_value(&s).unwrap();
        assert_eq!(v["unreadDms"], 3);
        assert_eq!(v["pendingRequests"], 1);
    }

    #[test]
    fn requests_response_counts_rows() {
        let json = r#"{ "requests": [ {"a":1}, {"b":2} ] }"#;
        let r: RequestsResponse = serde_json::from_str(json).expect("parses");
        assert_eq!(r.requests.len(), 2);
        // Missing key → empty.
        let empty: RequestsResponse = serde_json::from_str("{}").unwrap();
        assert_eq!(empty.requests.len(), 0);
    }

    // ── Channels (US-018) ────────────────────────────────────────────────────

    #[test]
    fn channel_deserializes_minimal() {
        // Only channelId is strictly required; the rest default.
        let json = r#"{ "channelId": "chn_1", "name": "general", "scope": "company" }"#;
        let c: Channel = serde_json::from_str(json).expect("Channel parses");
        assert_eq!(c.channel_id, "chn_1");
        assert_eq!(c.name, "general");
        assert_eq!(c.scope, "company");
        assert!(c.company_uid.is_none());
        assert!(c.unread.is_none());
    }

    #[test]
    fn channel_deserializes_full_row() {
        let json = r#"{
            "channelId": "chn_2",
            "name": "eng",
            "scope": "company",
            "companyUid": "ent_co",
            "companyName": "Acme",
            "postPolicy": "all",
            "visibility": "company",
            "membership": "invited",
            "unread": 3,
            "memberCount": 12
        }"#;
        let c: Channel = serde_json::from_str(json).expect("Channel parses");
        assert_eq!(c.company_uid.as_deref(), Some("ent_co"));
        assert_eq!(c.company_name.as_deref(), Some("Acme"));
        assert_eq!(c.membership.as_deref(), Some("invited"));
        assert_eq!(c.unread, Some(3));
        assert_eq!(c.member_count, Some(12));
    }

    #[test]
    fn channel_member_and_detail_deserialize() {
        let members_json = r#"{ "members": [
            { "personUid": "prs_o", "email": "o@x.com", "displayName": "Owner", "role": "owner" },
            { "personUid": "prs_m", "email": "m@x.com", "displayName": "Member", "role": "member" }
        ] }"#;
        let m: ChannelMembersResponse =
            serde_json::from_str(members_json).expect("members parse");
        assert_eq!(m.members.len(), 2);
        assert_eq!(m.members[0].role, "owner");

        let detail_json = r#"{
            "channel": { "channelId": "chn_1", "name": "g", "scope": "personal" },
            "messages": [
                { "eventId": "e1", "fromPersonUid": "prs_a", "body": "hi",
                  "createdAt": "2026-06-05T00:00:00Z", "direction": "in" }
            ]
        }"#;
        let d: ChannelDetail = serde_json::from_str(detail_json).expect("detail parses");
        assert_eq!(d.channel.expect("channel present").channel_id, "chn_1");
        assert_eq!(d.messages.len(), 1);
        assert_eq!(d.messages[0].direction, "in");
    }

    #[test]
    fn create_channel_response_envelope_unwraps() {
        // The create endpoint wraps the channel: `{"channel": {...}}` with no
        // `messages`. `create_channel` decodes into `ChannelDetail` and unwraps
        // `.channel`. A bare `Channel` decode here was the original bug (the
        // server's `channelId` lives one level down), surfacing as
        // "missing field channelId" even though the channel was created.
        let json = r#"{ "channel": { "channelId": "chn_1", "name": "general", "scope": "company" } }"#;
        let detail: ChannelDetail = serde_json::from_str(json).expect("envelope parses");
        let channel = detail.channel.expect("channel present in create envelope");
        assert_eq!(channel.channel_id, "chn_1");
        assert!(detail.messages.is_empty());
    }

    #[test]
    fn reactions_response_envelope_unwraps() {
        // The GET reactions endpoint returns the `MessageReactions` object, not a
        // bare array — decoding into `Vec<ReactionAggregate>` threw
        // "invalid type: map, expected a sequence" on every message load.
        let empty = r#"{ "messageScope": "chan:chn_1", "messageId": "m1", "reactions": [] }"#;
        let r: MessageReactions = serde_json::from_str(empty).expect("empty envelope parses");
        assert!(r.reactions.is_empty());

        let one = r#"{ "messageScope": "chan:chn_1", "messageId": "m1",
            "reactions": [ { "emoji": "👍", "count": 2, "reactedByMe": true } ] }"#;
        let r: MessageReactions = serde_json::from_str(one).expect("one-emoji envelope parses");
        assert_eq!(r.reactions.len(), 1);
        assert_eq!(r.reactions[0].emoji, "👍");
        assert_eq!(r.reactions[0].count, 2);
        assert!(r.reactions[0].reacted_by_me);
    }

    #[test]
    fn group_payload_carries_scope_and_participants_no_name() {
        let payload = build_group_payload(&["prs_a".to_string(), "prs_b".to_string()]);
        assert_eq!(payload["scope"], "group");
        assert_eq!(payload["participants"][0], "prs_a");
        assert_eq!(payload["participants"][1], "prs_b");
        // A group DM has no name field.
        assert!(payload.get("name").is_none());
    }

    #[test]
    fn create_payload_personal_omits_company_and_empty_invite() {
        let payload = build_create_payload("diary", "personal", None, &[]);
        let obj = payload.as_object().expect("object");
        assert_eq!(payload["name"], "diary");
        assert_eq!(payload["scope"], "personal");
        assert!(!obj.contains_key("companyUid"));
        assert!(!obj.contains_key("invite"));
    }

    #[test]
    fn create_payload_company_with_invites() {
        let invites = vec!["prs_a".to_string(), "prs_b".to_string()];
        let payload = build_create_payload("eng", "company", Some("ent_co"), &invites);
        assert_eq!(payload["companyUid"], "ent_co");
        assert_eq!(payload["invite"][0], "prs_a");
        assert_eq!(payload["invite"][1], "prs_b");
        // A blank companyUid is treated as absent.
        let blank = build_create_payload("x", "company", Some("   "), &[]);
        assert!(!blank.as_object().unwrap().contains_key("companyUid"));
    }

    #[test]
    fn esc_seg_escapes_path_reserved_chars_only() {
        assert_eq!(esc_seg("chn_abc123"), "chn_abc123");
        assert_eq!(esc_seg("a/b c"), "a%2Fb%20c");
        assert_eq!(esc_seg("q?x#y"), "q%3Fx%23y");
    }

    // ── Reactions (US-025) ────────────────────────────────────────────────────

    #[test]
    fn reaction_payload_carries_scope_id_emoji_only() {
        let payload = build_reaction_payload("dm:prs_peer", "evt_1", "👍");
        assert_eq!(payload["messageScope"], "dm:prs_peer");
        assert_eq!(payload["messageId"], "evt_1");
        assert_eq!(payload["emoji"], "👍");
        // Exactly the three contract keys — add (POST) and remove (DELETE) share
        // this body.
        assert_eq!(payload.as_object().expect("object").len(), 3);
    }

    #[test]
    fn reactions_url_escapes_scope_and_id() {
        // Channel scope is URL-safe; a stray reserved char must still be escaped.
        assert_eq!(
            build_reactions_url("https://api.example.com", "chan:chn_1", "evt_9"),
            "https://api.example.com/v1/notify/reactions?messageScope=chan:chn_1&messageId=evt_9"
        );
        assert_eq!(
            build_reactions_url("https://api.example.com", "dm:a/b", "e?1"),
            "https://api.example.com/v1/notify/reactions?messageScope=dm:a%2Fb&messageId=e%3F1"
        );
    }

    #[test]
    fn reaction_aggregate_deserializes_camel_case() {
        // The GET endpoint returns a bare array of aggregates.
        let json = r#"[
            { "emoji": "👍", "count": 3, "reactedByMe": true },
            { "emoji": "🎉", "count": 1, "reactedByMe": false }
        ]"#;
        let out: Vec<ReactionAggregate> = serde_json::from_str(json).expect("aggregates parse");
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].emoji, "👍");
        assert_eq!(out[0].count, 3);
        assert!(out[0].reacted_by_me);
        assert!(!out[1].reacted_by_me);
    }

    #[test]
    fn reaction_aggregate_tolerates_missing_fields() {
        // count/reactedByMe default so a sparse server row still parses.
        let json = r#"{ "emoji": "🔥" }"#;
        let a: ReactionAggregate = serde_json::from_str(json).expect("parses");
        assert_eq!(a.emoji, "🔥");
        assert_eq!(a.count, 0);
        assert!(!a.reacted_by_me);
    }

    #[test]
    fn message_reactions_serializes_camel_case_for_event() {
        // The `message:reaction` event payload shape the frontend listens for.
        let mr = MessageReactions {
            message_scope: "dm:prs_x".to_string(),
            message_id: "evt_1".to_string(),
            reactions: vec![ReactionAggregate {
                emoji: "👍".to_string(),
                count: 2,
                reacted_by_me: true,
            }],
        };
        let v = serde_json::to_value(&mr).unwrap();
        assert_eq!(v["messageScope"], "dm:prs_x");
        assert_eq!(v["messageId"], "evt_1");
        assert_eq!(v["reactions"][0]["reactedByMe"], true);
    }
}
