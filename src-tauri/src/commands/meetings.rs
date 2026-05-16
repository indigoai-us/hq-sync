//! Meeting invite UX — Tauri commands for the discreet meeting icon + modal
//! in the Popover (gated to @getindigo.ai for v1).
//!
//! The icon opens a modal that lists upcoming meetings (from the user's
//! connected Google calendars) plus an input field for inviting the bot to
//! an ad-hoc meeting URL. Per-row Invite/Uninvite toggles the Recall.ai
//! bot scheduled for that meeting; when the calendar has a company mapping,
//! the bot's transcript lands in the mapped company's vault (hq-pro routes
//! based on `companyId`).
//!
//! Feature gate: `meetings_feature_enabled()` decodes the locally-cached
//! id_token claims and returns true iff `email` ends in @getindigo.ai. Same
//! allowlist as hq-console's `isCalendarFeatureEnabled`. No signature
//! verification — the token came from Cognito via our own OAuth flow and
//! lives on local disk; we trust it for the duration of the session.
//!
//! HTTP surface: thin reqwest wrapper around the hq-pro routes shipped by
//! the meeting-pipeline project:
//!   GET    /v1/calendar/events                       — upcoming events
//!   GET    /v1/bot/list?calendarEventIds=...         — bots for given events
//!   POST   /v1/bot/invite                            — schedule a new bot
//!   POST   /v1/bot/{botId}/cancel                    — cancel scheduled bot

use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

use crate::commands::cognito;
use crate::commands::sync::resolve_vault_api_url;
use crate::util::client_info::build_client;

// ── Feature flag ─────────────────────────────────────────────────────────────

/// `@getindigo.ai` only for v1. Mirrors the hq-console gate. Lifted to a
/// shared allowlist once the rollout widens.
const ALLOWED_DOMAIN: &str = "@getindigo.ai";

/// Cached per-session decision so we don't re-decode the id_token on every
/// popover open. The token is rotated on refresh but the email claim is
/// stable across rotations (Cognito sub stays the same), so a process-
/// lifetime cache is safe.
static CACHED_FLAG: OnceLock<bool> = OnceLock::new();

/// Returns true iff the signed-in user's email ends in `@getindigo.ai`.
/// Quiet on missing/malformed tokens (returns false rather than erroring) so
/// the popover never breaks just because the user is signed out.
#[tauri::command]
pub async fn meetings_feature_enabled() -> Result<bool, String> {
    if let Some(v) = CACHED_FLAG.get() {
        return Ok(*v);
    }
    let enabled = compute_enabled().await;
    let _ = CACHED_FLAG.set(enabled);
    Ok(enabled)
}

async fn compute_enabled() -> bool {
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
    is_allowed_email(claims.email.as_deref())
}

/// Pure helper — public for unit testing. Same logic as the hq-console
/// `isCalendarFeatureEnabled`. Case-insensitive suffix match on the
/// `@getindigo.ai` domain (the leading `@` prevents look-alike domains
/// like `forgetindigo.ai` from matching).
pub fn is_allowed_email(email: Option<&str>) -> bool {
    match email {
        Some(s) if !s.is_empty() => s.to_ascii_lowercase().ends_with(ALLOWED_DOMAIN),
        _ => false,
    }
}

// ── Data types (mirror hq-pro response shapes) ────────────────────────────────

/// Google calendar event as returned by hq-pro `GET /v1/calendar/events`.
/// Only the fields we render in the modal — the full shape lives in hq-pro's
/// CalendarEvent type.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MeetingEvent {
    pub id: String,
    #[serde(default)]
    pub summary: Option<String>,
    pub start: EventTime,
    pub end: EventTime,
    /// "confirmed" | "tentative" | "cancelled"
    pub status: String,
    #[serde(default, rename = "hangoutLink")]
    pub hangout_link: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventTime {
    /// ISO 8601 with TZ. Set for timed events.
    #[serde(default, rename = "dateTime")]
    pub date_time: Option<String>,
    /// YYYY-MM-DD for all-day events.
    #[serde(default)]
    pub date: Option<String>,
    #[serde(default, rename = "timeZone")]
    pub time_zone: Option<String>,
}

/// Subset of hq-pro `BotRecord` that the modal renders. Field names mirror
/// the JSON shape from `GET /v1/bot/list`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScheduledBot {
    pub bot_id: String,
    pub meeting_url: String,
    pub platform: String,
    pub status: String,
    pub calendar_event_id: Option<String>,
    pub meeting_title: Option<String>,
    pub scheduled_start_time: Option<String>,
    pub auto_scheduled: bool,
    pub error_message: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct InviteBotBody {
    meeting_url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    calendar_event_id: Option<String>,
}

// ── HTTP wrappers ─────────────────────────────────────────────────────────────

async fn auth_header() -> Result<String, String> {
    let tokens = cognito::get_tokens()
        .await
        .map_err(|e| format!("auth: {e}"))?;
    let tokens = tokens.ok_or_else(|| "auth: not signed in".to_string())?;
    Ok(format!("Bearer {}", tokens.access_token))
}

async fn vault_base() -> Result<String, String> {
    resolve_vault_api_url().map(|u| u.trim_end_matches('/').to_string())
}

/// `GET /v1/calendar/events` — upcoming events from the caller's selected
/// calendars (within hq-pro's configured sync window).
#[tauri::command]
pub async fn meetings_list_upcoming() -> Result<Vec<MeetingEvent>, String> {
    let base = vault_base().await?;
    let auth = auth_header().await?;
    let res = build_client()
        .get(format!("{base}/v1/calendar/events"))
        .header("authorization", &auth)
        .send()
        .await
        .map_err(|e| format!("events fetch: {e}"))?;
    let status = res.status();
    let text = res.text().await.map_err(|e| format!("events read: {e}"))?;
    if !status.is_success() {
        return Err(format!("events HTTP {status}: {text}"));
    }
    let parsed: EventsResponse = serde_json::from_str(&text)
        .map_err(|e| format!("events parse: {e} — body: {text}"))?;
    Ok(parsed.events)
}

#[derive(Deserialize)]
struct EventsResponse {
    #[serde(default)]
    events: Vec<MeetingEvent>,
}

/// `GET /v1/bot/list` (optionally `?calendarEventIds=a,b,c`) — bots for the
/// caller. Filter param lets the UI ask only about the events it's rendering.
#[tauri::command]
pub async fn meetings_list_scheduled_bots(
    calendar_event_ids: Option<Vec<String>>,
) -> Result<Vec<ScheduledBot>, String> {
    let base = vault_base().await?;
    let auth = auth_header().await?;
    let mut url = format!("{base}/v1/bot/list");
    if let Some(ids) = calendar_event_ids.as_ref() {
        if !ids.is_empty() {
            let joined = ids.join(",");
            url.push_str(&format!("?calendarEventIds={joined}"));
        }
    }
    let res = build_client()
        .get(url)
        .header("authorization", &auth)
        .send()
        .await
        .map_err(|e| format!("bot/list fetch: {e}"))?;
    let status = res.status();
    let text = res.text().await.map_err(|e| format!("bot/list read: {e}"))?;
    if !status.is_success() {
        return Err(format!("bot/list HTTP {status}: {text}"));
    }
    let parsed: BotsResponse = serde_json::from_str(&text)
        .map_err(|e| format!("bot/list parse: {e} — body: {text}"))?;
    Ok(parsed.bots)
}

#[derive(Deserialize)]
struct BotsResponse {
    #[serde(default)]
    bots: Vec<ScheduledBot>,
}

/// `POST /v1/bot/invite` — schedule a Recall.ai bot for a meeting. Pass
/// `company_id` as a query param so hq-pro routes the transcript to that
/// company's vault (it validates the caller is a member). Omit to land
/// the meeting in the user's personal vault.
#[tauri::command]
pub async fn meetings_invite_bot(
    meeting_url: String,
    calendar_event_id: Option<String>,
    company_id: Option<String>,
) -> Result<ScheduledBot, String> {
    let base = vault_base().await?;
    let auth = auth_header().await?;
    let mut url = format!("{base}/v1/bot/invite");
    if let Some(cid) = company_id.as_ref() {
        if !cid.is_empty() {
            url.push_str(&format!("?companyId={cid}"));
        }
    }
    let body = InviteBotBody {
        meeting_url,
        calendar_event_id,
    };
    let res = build_client()
        .post(url)
        .header("authorization", &auth)
        .header("content-type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("bot/invite fetch: {e}"))?;
    let status = res.status();
    let text = res.text().await.map_err(|e| format!("bot/invite read: {e}"))?;
    if !status.is_success() {
        return Err(format!("bot/invite HTTP {status}: {text}"));
    }
    serde_json::from_str(&text).map_err(|e| format!("bot/invite parse: {e} — body: {text}"))
}

/// `POST /v1/bot/{botId}/cancel` — uninvite a scheduled bot. hq-pro validates
/// caller ownership before calling Recall.ai bot-leave.
///
/// `bot_id` must be a Recall.ai bot id (UUID-style — `[a-zA-Z0-9_-]+`). We
/// validate the shape before concatenating into the path to keep the URL
/// well-formed without pulling in a percent-encoding crate.
#[tauri::command]
pub async fn meetings_cancel_bot(bot_id: String) -> Result<(), String> {
    if bot_id.is_empty() {
        return Err("bot_id is required".to_string());
    }
    if !is_url_safe_id(&bot_id) {
        return Err(format!("bot_id has invalid characters: {bot_id:?}"));
    }
    let base = vault_base().await?;
    let auth = auth_header().await?;
    let url = format!("{base}/v1/bot/{bot_id}/cancel");
    let res = build_client()
        .post(url)
        .header("authorization", &auth)
        .send()
        .await
        .map_err(|e| format!("bot/cancel fetch: {e}"))?;
    let status = res.status();
    if !status.is_success() {
        let text = res.text().await.unwrap_or_default();
        return Err(format!("bot/cancel HTTP {status}: {text}"));
    }
    Ok(())
}

/// Allows only `[a-zA-Z0-9._-]+` — matches Recall.ai bot id shape (UUID with
/// optional underscores) and avoids the need for percent-encoding.
fn is_url_safe_id(s: &str) -> bool {
    !s.is_empty()
        && s.bytes().all(|b| {
            b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.'
        })
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allowlist_matches_indigo_ai() {
        assert!(is_allowed_email(Some("stefan@getindigo.ai")));
        assert!(is_allowed_email(Some("STEFAN@GetIndigo.AI")));
    }

    #[test]
    fn allowlist_rejects_other_domains() {
        assert!(!is_allowed_email(Some("someone@gmail.com")));
        assert!(!is_allowed_email(Some("admin@notindigo.ai")));
        // Look-alike domain — the leading `@` in ALLOWED_DOMAIN prevents
        // suffix matches like `forgetindigo.ai`.
        assert!(!is_allowed_email(Some("attacker@forgetindigo.ai")));
    }

    #[test]
    fn allowlist_rejects_missing_email() {
        assert!(!is_allowed_email(None));
        assert!(!is_allowed_email(Some("")));
    }

    /// Serde shape lock-in — what the frontend gets is what the modal needs.
    #[test]
    fn scheduled_bot_round_trips_camel_case() {
        let json = r#"{
            "botId": "bot-abc",
            "meetingUrl": "https://meet.google.com/abc",
            "platform": "google_meet",
            "status": "scheduled",
            "calendarEventId": "evt-1",
            "meetingTitle": "Standup",
            "scheduledStartTime": "2026-05-15T10:00:00Z",
            "autoScheduled": true,
            "errorMessage": null
        }"#;
        let bot: ScheduledBot = serde_json::from_str(json).expect("parse");
        assert_eq!(bot.bot_id, "bot-abc");
        assert_eq!(bot.status, "scheduled");
        assert_eq!(bot.calendar_event_id.as_deref(), Some("evt-1"));
        assert!(bot.auto_scheduled);
        assert!(bot.error_message.is_none());
    }

    #[test]
    fn url_safe_id_accepts_uuid_shapes() {
        assert!(is_url_safe_id("abc123"));
        assert!(is_url_safe_id("550e8400-e29b-41d4-a716-446655440000"));
        assert!(is_url_safe_id("bot_abc.123"));
    }

    #[test]
    fn url_safe_id_rejects_path_traversal_and_specials() {
        assert!(!is_url_safe_id(""));
        assert!(!is_url_safe_id("../etc/passwd"));
        assert!(!is_url_safe_id("bot/abc"));
        assert!(!is_url_safe_id("bot abc"));
        assert!(!is_url_safe_id("bot?x=1"));
        assert!(!is_url_safe_id("bot#frag"));
    }

    #[test]
    fn meeting_event_parses_with_only_required_fields() {
        let json = r#"{
            "id": "evt-1",
            "start": {"dateTime": "2026-05-15T14:00:00Z"},
            "end": {"dateTime": "2026-05-15T15:00:00Z"},
            "status": "confirmed"
        }"#;
        let evt: MeetingEvent = serde_json::from_str(json).expect("parse");
        assert_eq!(evt.id, "evt-1");
        assert_eq!(evt.status, "confirmed");
        assert!(evt.summary.is_none());
        assert!(evt.hangout_link.is_none());
        assert_eq!(evt.start.date_time.as_deref(), Some("2026-05-15T14:00:00Z"));
    }
}
