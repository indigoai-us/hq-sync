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

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

use crate::commands::cognito;
use crate::commands::sync::resolve_vault_api_url;
use crate::util::client_info::build_client;

// ── Feature flag ─────────────────────────────────────────────────────────────

/// Returns true iff the signed-in user's email ends in `@getindigo.ai`.
///
/// Delegates to `feature_gate::is_indigo_user()`, which caches the result for
/// the process lifetime (the Cognito email claim is stable across token
/// rotations). Quiet on missing/malformed tokens (returns false) so the
/// popover never breaks because the user is signed out.
#[tauri::command]
pub async fn meetings_feature_enabled() -> Result<bool, String> {
    Ok(crate::util::feature_gate::is_indigo_user().await)
}

// ── Data types (mirror hq-pro response shapes) ────────────────────────────────

/// Google calendar event as returned by hq-pro `GET /v1/calendar/events`.
/// Only the fields we render in the modal — the full shape lives in hq-pro's
/// CalendarEvent type. `sourceCalendarId` + `sourceCompanyUid` are added by
/// hq-pro at flatten-time (BE-4) so the modal can render the right company
/// badge per row and pass the right companyId on invite.
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
    /// Calendar this event came from. Set by hq-pro BE-4.
    #[serde(default, rename = "sourceCalendarId")]
    pub source_calendar_id: Option<String>,
    /// Company UID this calendar is mapped to (if any). Set by hq-pro BE-4.
    /// When present, pass as `company_id` on `meetings_invite_bot` so the
    /// transcript lands in the company's vault. When absent, omit on invite
    /// → meeting lands in personal.
    #[serde(default, rename = "sourceCompanyUid")]
    pub source_company_uid: Option<String>,
    /// Per-account ULID identifying which connected Google account this
    /// event was fetched from. Set by hq-pro BE-4 fan-out so the UI can
    /// render a per-account badge + drive a per-calendar filter (a person
    /// with two connected accounts wants to see events from both, tagged
    /// by source).
    #[serde(default, rename = "sourceAccountId")]
    pub source_account_id: Option<String>,
    /// Server-extracted meeting URL (BE-5). Picks the right URL across
    /// hangoutLink, conferenceData entry points, and Zoom/Teams links in
    /// the description. Prefer this over `hangout_link` for the "should I
    /// show an Invite button" check.
    #[serde(default, rename = "meetingUrl")]
    pub meeting_url: Option<String>,
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

// ── Detail window (Upcoming Meetings) ────────────────────────────────────────

/// Open or focus the standalone Upcoming Meetings window. Mirrors the
/// `new-files-detail` pattern — the modal-on-popover UX squeezed the existing
/// sync UI and made the list cramped. The detached window can grow, decorations
/// give the user a proper close affordance, and the popover stays untouched.
///
/// Unlike `open_new_files_detail`, there's no payload handshake — the window
/// fetches its own data via `meetings_list_upcoming` + `meetings_list_scheduled_bots`
/// directly on mount.
#[tauri::command]
pub async fn open_meetings_window(app: AppHandle) -> Result<(), String> {
    const LABEL: &str = "meetings-window";

    if let Some(window) = app.get_webview_window(LABEL) {
        window.show().map_err(|e| e.to_string())?;
        window.set_focus().map_err(|e| e.to_string())?;
        return Ok(());
    }

    // Use the bundled HQ app icon for this window's macOS dock / Cmd-Tab /
    // window-switcher representation. Without an explicit `.icon(...)` the
    // detached webview window falls back to the generic file-folder icon,
    // which reads as "some random app's window" in the switcher. The PNG
    // is baked into the binary at compile time so we don't depend on any
    // runtime filesystem path being correct.
    //
    // Future polish: composite a small calendar badge in the lower-right
    // corner so this window's icon is distinguishable from the main HQ
    // Sync window at a glance. Skipped here because (a) it requires an
    // image-processing step in the build pipeline and (b) the window-
    // switcher icon is rendered very small, so the badge would likely be
    // illegible at that scale anyway.
    const HQ_ICON_PNG: &[u8] =
        include_bytes!("../../icons/128x128@2x.png");
    let icon = tauri::image::Image::from_bytes(HQ_ICON_PNG)
        .map_err(|e| format!("load window icon: {e}"))?;

    tauri::WebviewWindowBuilder::new(
        &app,
        LABEL,
        tauri::WebviewUrl::App("index.html".into()),
    )
    .title("Upcoming Meetings")
    .inner_size(460.0, 600.0)
    .min_inner_size(380.0, 400.0)
    .resizable(true)
    .decorations(true)
    .icon(icon)
    .map_err(|e| format!("attach window icon: {e}"))?
    .visible(true)
    .build()
    .map_err(|e| e.to_string())?;

    Ok(())
}

// ── HTTP wrappers ─────────────────────────────────────────────────────────────

async fn auth_header() -> Result<String, String> {
    // Use the centralised valid-token helper so an expired access token
    // refreshes + persists transparently. The old version read the raw
    // stored access_token, which silently broke every meetings call
    // once the 1h Cognito TTL elapsed (popover sync runs `get_auth_state`
    // periodically and refreshed there, hiding the bug from the main UI).
    let token = cognito::get_valid_access_token()
        .await
        .map_err(|e| format!("auth: {e}"))?;
    Ok(format!("Bearer {token}"))
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

// ── Per-account calendar lookup (multi-account filter UX) ─────────────────────

/// One connected Google account on the signed-in person. Mirrors hq-pro's
/// `GET /v1/google/accounts` row shape (BE-3). The UI uses `email` as the
/// display label in the multi-account calendar filter dropdown — accountId
/// remains the stable identifier across email changes (BE-1).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GoogleAccount {
    #[serde(rename = "accountId")]
    pub account_id: String,
    pub email: Option<String>,
    #[serde(default)]
    pub scope: Option<String>,
    #[serde(default, rename = "connectedAt")]
    pub connected_at: Option<String>,
    #[serde(default)]
    pub capabilities: Vec<String>,
}

/// One calendar from `GET /v1/calendar/calendars?accountId=…` (BE-4).
/// Only the subset of fields the multi-account filter dropdown needs —
/// `summary` is what we render to the user, `id` is what we match events
/// against (via event.sourceCalendarId).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GoogleCalendar {
    pub id: String,
    pub summary: String,
    #[serde(default)]
    pub primary: bool,
    #[serde(default, rename = "accessRole")]
    pub access_role: Option<String>,
}

/// `GET /v1/google/accounts` — list every connected Google account on the
/// signed-in person. Returns empty vec when the user has no connections
/// (e.g. fresh install). The frontend uses this to label events with the
/// source account email and to drive the calendar-filter dropdown.
#[tauri::command]
pub async fn meetings_list_accounts() -> Result<Vec<GoogleAccount>, String> {
    let base = vault_base().await?;
    let auth = auth_header().await?;
    let res = build_client()
        .get(format!("{base}/v1/google/accounts"))
        .header("authorization", &auth)
        .send()
        .await
        .map_err(|e| format!("accounts fetch: {e}"))?;
    let status = res.status();
    let text = res
        .text()
        .await
        .map_err(|e| format!("accounts read: {e}"))?;
    if !status.is_success() {
        return Err(format!("accounts HTTP {status}: {text}"));
    }
    let parsed: AccountsResponse = serde_json::from_str(&text)
        .map_err(|e| format!("accounts parse: {e} — body: {text}"))?;
    Ok(parsed.accounts)
}

#[derive(Deserialize)]
struct AccountsResponse {
    #[serde(default)]
    accounts: Vec<GoogleAccount>,
}

/// Combined response from `meetings_list_calendars_for_account` — every
/// calendar visible to the account, PLUS the user's currently-enabled
/// selection for that account. The frontend uses `selected_calendar_ids`
/// to filter the upcoming-meetings calendar-filter dropdown down to
/// calendars the user has actually opted into in hq-console (otherwise
/// the dropdown lists calendars that hq-pro doesn't fan out against,
/// creating a "I checked the box but no events appeared" confusion).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountCalendars {
    pub calendars: Vec<GoogleCalendar>,
    /// Calendar IDs in this account's per-account
    /// `preferences.selectedCalendars[]` — i.e. the ones the user has
    /// toggled ON in hq-console's Manage panel. The filter dropdown
    /// should narrow to this subset only.
    pub selected_calendar_ids: Vec<String>,
}

/// `GET /v1/calendar/calendars?accountId=…` — list every calendar visible
/// to one connected account AND surface that account's currently-enabled
/// selection. The frontend calls this once per account (in parallel) so
/// the multi-account filter dropdown can render calendar names grouped
/// by account, scoped to what's actually being fanned-out against by
/// hq-pro.
#[tauri::command]
pub async fn meetings_list_calendars_for_account(
    account_id: String,
) -> Result<AccountCalendars, String> {
    let base = vault_base().await?;
    let auth = auth_header().await?;
    // accountId is an `acct_{ulid}` (Crockford base32 + ASCII underscore) —
    // URL-safe by construction, no encoding needed. We still guard the
    // shape so a malformed value from the renderer surfaces as a clean
    // 400 from hq-pro rather than a malformed URL.
    if !account_id.starts_with("acct_") {
        return Err(format!("invalid accountId: {account_id}"));
    }
    let url = format!("{base}/v1/calendar/calendars?accountId={account_id}");
    let res = build_client()
        .get(url)
        .header("authorization", &auth)
        .send()
        .await
        .map_err(|e| format!("calendars fetch: {e}"))?;
    let status = res.status();
    let text = res
        .text()
        .await
        .map_err(|e| format!("calendars read: {e}"))?;
    if !status.is_success() {
        return Err(format!("calendars HTTP {status}: {text}"));
    }
    let parsed: CalendarsResponse = serde_json::from_str(&text)
        .map_err(|e| format!("calendars parse: {e} — body: {text}"))?;
    Ok(AccountCalendars {
        calendars: parsed.calendars,
        selected_calendar_ids: parsed
            .selected_calendars
            .into_iter()
            .map(|s| s.id)
            .collect(),
    })
}

#[derive(Deserialize)]
struct CalendarsResponse {
    #[serde(default)]
    calendars: Vec<GoogleCalendar>,
    /// Per-account selectedCalendars entries from hq-pro's BE-4 response.
    /// We only need the `id` to filter the dropdown, but defer-parsing the
    /// full shape keeps us forward-compatible with any new fields hq-pro
    /// adds (e.g. companyUid which we don't use here).
    #[serde(default, rename = "selectedCalendars")]
    selected_calendars: Vec<SelectedCalendarRef>,
}

#[derive(Deserialize)]
struct SelectedCalendarRef {
    id: String,
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

/// `GET /membership/me` — caller's memberships, enriched with `companyName`.
/// Used by the modal to render human-readable company badges instead of
/// raw `cmp_…` UIDs.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompanyMembership {
    pub company_uid: String,
    #[serde(default)]
    pub company_name: Option<String>,
    #[serde(default)]
    pub role: Option<String>,
    pub status: String,
}

#[tauri::command]
pub async fn meetings_list_memberships() -> Result<Vec<CompanyMembership>, String> {
    let base = vault_base().await?;
    let auth = auth_header().await?;
    // Strip the "Bearer " prefix to match VaultClient's expected token format.
    let token = auth.strip_prefix("Bearer ").unwrap_or(&auth);
    let client = crate::commands::vault_client::VaultClient::new(&base, token);
    let memberships = client
        .list_my_memberships()
        .await
        .map_err(|e| format!("memberships fetch: {e}"))?;
    // Project down to the fields the modal needs — keeps the JSON wire
    // payload tight and decouples the UI from the internal vault type.
    Ok(memberships
        .into_iter()
        .map(|m| CompanyMembership {
            company_uid: m.company_uid,
            company_name: m.company_name,
            role: m.role,
            status: m.status,
        })
        .collect())
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
        use crate::util::feature_gate::is_allowed_email;
        assert!(is_allowed_email(Some("stefan@getindigo.ai")));
        assert!(is_allowed_email(Some("STEFAN@GetIndigo.AI")));
    }

    #[test]
    fn allowlist_rejects_other_domains() {
        use crate::util::feature_gate::is_allowed_email;
        assert!(!is_allowed_email(Some("someone@gmail.com")));
        assert!(!is_allowed_email(Some("admin@notindigo.ai")));
        // Look-alike domain — the leading `@` in ALLOWED_DOMAIN prevents
        // suffix matches like `forgetindigo.ai`.
        assert!(!is_allowed_email(Some("attacker@forgetindigo.ai")));
    }

    #[test]
    fn allowlist_rejects_missing_email() {
        use crate::util::feature_gate::is_allowed_email;
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
        assert!(evt.source_calendar_id.is_none());
        assert!(evt.source_company_uid.is_none());
        assert_eq!(evt.start.date_time.as_deref(), Some("2026-05-15T14:00:00Z"));
    }

    /// BE-4 enhancement — events tagged with source calendar + company.
    #[test]
    fn meeting_event_includes_source_calendar_tagging() {
        let json = r#"{
            "id": "evt-1",
            "summary": "Sync w/ Spencer",
            "start": {"dateTime": "2026-05-15T14:00:00Z"},
            "end": {"dateTime": "2026-05-15T15:00:00Z"},
            "status": "confirmed",
            "hangoutLink": "https://meet.google.com/abc-defg-hij",
            "sourceCalendarId": "stefan@getindigo.ai",
            "sourceCompanyUid": "cmp_indigo"
        }"#;
        let evt: MeetingEvent = serde_json::from_str(json).expect("parse");
        assert_eq!(evt.summary.as_deref(), Some("Sync w/ Spencer"));
        assert_eq!(
            evt.source_calendar_id.as_deref(),
            Some("stefan@getindigo.ai"),
        );
        assert_eq!(evt.source_company_uid.as_deref(), Some("cmp_indigo"));
        assert_eq!(
            evt.hangout_link.as_deref(),
            Some("https://meet.google.com/abc-defg-hij"),
        );
    }
}
