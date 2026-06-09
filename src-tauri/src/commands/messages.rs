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

    resp.json::<T>().await.map_err(|e| {
        log(LOG_TAG, &format!("{code}_PARSE_FAIL {e}"));
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
            "source": "company"
        }"#;
        let c: Contact = serde_json::from_str(json).expect("Contact parses");
        assert_eq!(c.email, "a@b.com");
        assert_eq!(c.company_uid.as_deref(), Some("ent_co"));
        assert_eq!(c.source.as_deref(), Some("company"));
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
}
