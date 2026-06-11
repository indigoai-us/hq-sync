//! Notification history — a unified, re-readable timeline of everything the
//! menubar surfaced and the user may have dismissed.
//!
//! Two server-retained sources are fetched on demand (newest-first, full
//! history rather than the unseen-delta the live pollers request):
//!   - DMs:    `GET /v1/notify/inbox?limit=N`        (DmEvent)
//!   - Shares: `GET /v1/files/shared-with-me?limit=N` (ShareEvent)
//!
//! New-file notifications are intentionally NOT fetched here: the runner emits
//! them per-sync and nothing persists them server-side, so true cross-session
//! history would need a new backend endpoint. The window includes the current
//! session's new-file rows client-side via the existing `get_activity_log`
//! command (entries with `is_new = true`), clearly scoped as session-only.
//!
//! Resilience: each source is fetched independently. A single source failing
//! degrades to its empty list (logged) so the other still renders; only a hard
//! auth/URL failure (can't even build the request) surfaces as an error.

use serde::{Deserialize, Serialize};

use crate::commands::cognito;
use crate::commands::dm_notify::DmEvent;
use crate::commands::share_notify::ShareEvent;
use crate::commands::sync::resolve_vault_api_url;
use crate::util::client_info::build_client;
use crate::util::logfile::log;

const LOG_TAG: &str = "notif-history";

/// Default + maximum number of items fetched per source. The server returns
/// newest-first, so this is the most-recent N; pagination is a future add.
const DEFAULT_LIMIT: u32 = 100;
const MAX_LIMIT: u32 = 200;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct InboxResponse {
    events: Vec<DmEvent>,
    #[allow(dead_code)]
    next_cursor: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SharedWithMeResponse {
    events: Vec<ShareEvent>,
    #[allow(dead_code)]
    next_cursor: Option<String>,
}

/// One new-file event from the server-retained file-history feed
/// (GET /v1/notify/file-history) — populated by the sync runner's report.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileHistoryItem {
    pub event_id: String,
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub added_by: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub company_uid: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub company_slug: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FileHistoryResponse {
    files: Vec<FileHistoryItem>,
    #[allow(dead_code)]
    next_cursor: Option<String>,
}

/// The server-retained notification sources, returned to the history window.
/// `files` is the cross-session new-file history; the window also merges the
/// current session's new files (from `get_activity_log`) and de-dupes.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NotificationHistory {
    pub dms: Vec<DmEvent>,
    pub shares: Vec<ShareEvent>,
    pub files: Vec<FileHistoryItem>,
}

async fn fetch_dms(client: &reqwest::Client, base_url: &str, token: &str, limit: u32) -> Result<Vec<DmEvent>, String> {
    let url = format!("{}/v1/notify/inbox?limit={}", base_url, limit);
    let resp = client
        .get(&url)
        .header("authorization", format!("Bearer {}", token))
        .send()
        .await
        .map_err(|e| format!("network: {e}"))?;
    let status = resp.status();
    if !status.is_success() {
        return Err(format!("status {}", status.as_u16()));
    }
    let parsed = resp
        .json::<InboxResponse>()
        .await
        .map_err(|e| format!("parse: {e}"))?;
    Ok(parsed.events)
}

async fn fetch_shares(client: &reqwest::Client, base_url: &str, token: &str, limit: u32) -> Result<Vec<ShareEvent>, String> {
    let url = format!("{}/v1/files/shared-with-me?limit={}", base_url, limit);
    let resp = client
        .get(&url)
        .header("authorization", format!("Bearer {}", token))
        .send()
        .await
        .map_err(|e| format!("network: {e}"))?;
    let status = resp.status();
    if !status.is_success() {
        return Err(format!("status {}", status.as_u16()));
    }
    let parsed = resp
        .json::<SharedWithMeResponse>()
        .await
        .map_err(|e| format!("parse: {e}"))?;
    Ok(parsed.events)
}

async fn fetch_files(client: &reqwest::Client, base_url: &str, token: &str, limit: u32) -> Result<Vec<FileHistoryItem>, String> {
    let url = format!("{}/v1/notify/file-history?limit={}", base_url, limit);
    let resp = client
        .get(&url)
        .header("authorization", format!("Bearer {}", token))
        .send()
        .await
        .map_err(|e| format!("network: {e}"))?;
    let status = resp.status();
    if !status.is_success() {
        return Err(format!("status {}", status.as_u16()));
    }
    let parsed = resp
        .json::<FileHistoryResponse>()
        .await
        .map_err(|e| format!("parse: {e}"))?;
    Ok(parsed.files)
}

/// Fetch the full (newest-N) DM + share + new-file history for the history window.
#[tauri::command]
pub async fn fetch_notification_history(
    limit: Option<u32>,
) -> Result<NotificationHistory, String> {
    let lim = limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);

    let access_token = cognito::get_valid_access_token().await.map_err(|e| {
        log(LOG_TAG, &format!("NOTIF_HISTORY_AUTH_FAIL {e}"));
        format!("Not signed in: {e}")
    })?;
    let base_url = resolve_vault_api_url()
        .map(|u| u.trim_end_matches('/').to_string())
        .map_err(|e| {
            log(LOG_TAG, &format!("NOTIF_HISTORY_URL_FAIL {e}"));
            format!("Could not resolve server URL: {e}")
        })?;

    let client = build_client();

    let dms_res = fetch_dms(&client, &base_url, &access_token, lim).await;
    let shares_res = fetch_shares(&client, &base_url, &access_token, lim).await;
    // File-history is the newest endpoint; treat its failure as non-fatal so an
    // older backend (or a transient miss) still renders DMs + shares.
    let files = match fetch_files(&client, &base_url, &access_token, lim).await {
        Ok(v) => v,
        Err(e) => {
            log(LOG_TAG, &format!("NOTIF_HISTORY_FILES_FAIL {e}"));
            Vec::new()
        }
    };

    // Capture each source's error (if any) while consuming the Result, so a
    // both-failed check below doesn't need to re-borrow the moved values.
    let mut dm_err: Option<String> = None;
    let dms = match dms_res {
        Ok(v) => v,
        Err(e) => {
            log(LOG_TAG, &format!("NOTIF_HISTORY_DM_FAIL {e}"));
            dm_err = Some(e);
            Vec::new()
        }
    };
    let mut share_err: Option<String> = None;
    let shares = match shares_res {
        Ok(v) => v,
        Err(e) => {
            log(LOG_TAG, &format!("NOTIF_HISTORY_SHARE_FAIL {e}"));
            share_err = Some(e);
            Vec::new()
        }
    };

    // Only fail hard if BOTH sources errored — a one-source outage still renders
    // the other half of the timeline.
    if let (Some(de), Some(se)) = (&dm_err, &share_err) {
        return Err(format!("Could not load notifications (dm: {de}; share: {se})"));
    }

    log(
        LOG_TAG,
        &format!(
            "NOTIF_HISTORY_OK dms={} shares={} files={}",
            dms.len(),
            shares.len(),
            files.len()
        ),
    );
    Ok(NotificationHistory { dms, shares, files })
}
