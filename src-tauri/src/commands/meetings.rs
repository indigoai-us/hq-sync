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
    /// Server-extracted meeting signals. Preserve as raw JSON so the
    /// renderer cache can aggregate counts without coupling this command to
    /// the hq-pro signal schema.
    #[serde(default)]
    pub signals: Option<serde_json::Value>,
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
    /// `POST /v1/bot/invite` returns a slimmer body than `GET /v1/bot/list`
    /// and omits this field — a fresh manual invite is never auto-scheduled,
    /// so default to `false` rather than failing the whole parse (which would
    /// surface a spurious "Couldn't invite the bot." toast even though the bot
    /// was scheduled successfully server-side).
    #[serde(default)]
    pub auto_scheduled: bool,
    pub error_message: Option<String>,
}

/// Mirrors hq-pro's `OntologyParticipant` — resolved participant from the
/// ontology endpoint attached to the invite payload before the bot is created.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OntologyParticipant {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entity_id: Option<String>,
    pub canonical_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f64>,
    /// "invitee" | "organizer" | "ontology" | "historical" | "recall"
    pub source: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct InviteBotBody {
    meeting_url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    calendar_event_id: Option<String>,
    /// Ontology-resolved participants (US-005). Absent when fetch timed out.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    participants: Vec<OntologyParticipant>,
}

/// Fetch ontology-resolved participants for a meeting, with a hard 2-second
/// timeout. Returns an empty vec on any error (timeout, network, non-200) so
/// the bot invite is never blocked. Called just before `POST /v1/bot/invite`.
async fn fetch_participants_best_effort(
    base: &str,
    auth: &str,
    meeting_url: &str,
    event_id: Option<&str>,
) -> Vec<OntologyParticipant> {
    let mut req = build_client()
        .get(format!("{base}/v1/ontology/participants"))
        .header("authorization", auth)
        .query(&[("meetingUrl", meeting_url)]);
    if let Some(id) = event_id.filter(|s| !s.is_empty()) {
        req = req.query(&[("eventId", id)]);
    }
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        req.send(),
    )
    .await;
    let res = match result {
        Ok(Ok(r)) => r,
        Ok(Err(e)) => {
            crate::util::logfile::log(
                "meetings",
                &format!("participants fetch error: {e}"),
            );
            return vec![];
        }
        Err(_elapsed) => {
            crate::util::logfile::log("meetings", "participants fetch timed out (2s)");
            return vec![];
        }
    };
    if !res.status().is_success() {
        return vec![];
    }
    let text = match res.text().await {
        Ok(t) => t,
        Err(_) => return vec![],
    };
    #[derive(Deserialize)]
    struct ParticipantsResponse {
        #[serde(default)]
        participants: Vec<OntologyParticipant>,
    }
    serde_json::from_str::<ParticipantsResponse>(&text)
        .map(|r| r.participants)
        .unwrap_or_default()
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
    // Fetch ontology-resolved participants best-effort (US-005).
    // 2-second deadline; empty vec on any failure so the invite always fires.
    let participants = fetch_participants_best_effort(
        &base,
        &auth,
        &meeting_url,
        calendar_event_id.as_deref(),
    )
    .await;

    let body = InviteBotBody {
        meeting_url,
        calendar_event_id,
        participants,
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

/// `POST /v1/bot/join-now` — force the Recall.ai bot to join NOW for
/// `meeting_url`, regardless of any pre-scheduled `join_at`. Backs the
/// MeetingsWindow row's bot-join-now icon button. Server-side
/// (`bot.service.ts::joinBotNow`) decides the right path:
///
///   - existing `scheduled` bot → PATCH Recall `join_at = now`
///   - existing `joining`/`recording` bot → no-op, return as-is
///   - otherwise → flip any `completed` siblings to `failed`, then create
///     a fresh bot with no `join_at` so Recall joins immediately
///
/// Same request shape as `meetings_invite_bot` so the UI handler can
/// hand off the same `meeting_url` + `calendar_event_id` + `company_id`
/// triple without re-derivation.
#[tauri::command]
pub async fn meetings_join_bot_now(
    meeting_url: String,
    calendar_event_id: Option<String>,
    company_id: Option<String>,
) -> Result<ScheduledBot, String> {
    let base = vault_base().await?;
    let auth = auth_header().await?;
    let mut url = format!("{base}/v1/bot/join-now");
    if let Some(cid) = company_id.as_ref() {
        if !cid.is_empty() {
            url.push_str(&format!("?companyId={cid}"));
        }
    }

    let body = InviteBotBody {
        meeting_url,
        calendar_event_id,
        // join-now reuses an existing bot record when one already exists
        // for the meeting; if not, the server creates a fresh one. We
        // don't have ontology participants in scope here (we'd need a
        // calendar event id resolvable to the meeting), so let the server
        // either reuse the existing bot's `participants` array or accept
        // an empty list for the fresh-create case. Best-effort enrichment
        // for the bot-invite path lives in `meetings_invite_bot`.
        participants: Vec::new(),
    };
    let res = build_client()
        .post(url)
        .header("authorization", &auth)
        .header("content-type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("bot/join-now fetch: {e}"))?;
    let status = res.status();
    let text = res
        .text()
        .await
        .map_err(|e| format!("bot/join-now read: {e}"))?;
    if !status.is_success() {
        return Err(format!("bot/join-now HTTP {status}: {text}"));
    }
    serde_json::from_str(&text)
        .map_err(|e| format!("bot/join-now parse: {e} — body: {text}"))
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

// ── Detection-triggered notification commands ─────────────────────────────────

/// Payload for [`meetings_notify_detected`].
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NotifyDetectedPayload {
    /// The detected meeting URL (primary stable key for dedup).
    pub meeting_url: Option<String>,
    /// SDK window id for the detection — used by the notification's
    /// action-button thread to route a Record click back to
    /// `start_recording`. Caller passes `event.payload.windowId` straight
    /// through; absent only for very old bridge versions.
    pub window_id: Option<String>,
    /// Platform string from the SDK (e.g. "zoom", "meet").
    pub platform: Option<String>,
    /// Meeting title or calendar event summary, if known.
    pub summary: Option<String>,
    /// SDK source event id — fallback stable key when `meeting_url` is absent.
    pub source_event_id: Option<String>,
}

/// Check whether an active bot already exists for the given meeting URL.
///
/// Returns `Some(bot)` when a bot with an active status (`scheduled`,
/// `joining_call`, `in_call_recording`, `in_call_not_recording`) is found.
/// The Svelte handler calls this before `meetings_notify_detected`; a bot
/// already in the room means we should skip the notification.
///
/// Query: `GET /v1/bot/list?meetingUrl=<url>[&eventId=<id>]`
#[tauri::command]
pub async fn meetings_check_bot_for_url(
    meeting_url: String,
    event_id: Option<String>,
) -> Result<Option<ScheduledBot>, String> {
    let base = vault_base().await?;
    let auth = auth_header().await?;
    let mut req = build_client()
        .get(format!("{base}/v1/bot/list"))
        .header("authorization", &auth)
        .query(&[("meetingUrl", meeting_url.as_str())]);
    if let Some(id) = event_id.as_deref().filter(|s| !s.is_empty()) {
        req = req.query(&[("eventId", id)]);
    }
    let res = req.send().await.map_err(|e| format!("bot/list check: {e}"))?;
    let status = res.status();
    let text = res.text().await.map_err(|e| format!("bot/list check read: {e}"))?;
    if !status.is_success() {
        return Err(format!("bot/list check HTTP {status}: {text}"));
    }
    let parsed: BotsResponse = serde_json::from_str(&text)
        .map_err(|e| format!("bot/list check parse: {e} — body: {text}"))?;
    let active = parsed.bots.into_iter().find(|b| {
        matches!(
            b.status.as_str(),
            "scheduled" | "joining_call" | "in_call_recording" | "in_call_not_recording"
        )
    });
    Ok(active)
}

/// Fire a macOS notification for a detected meeting, gated on:
///
/// 1. `notifications` pref in `~/.hq/menubar.json` (read fresh on each call
///    so pref changes take effect without restart — US-007 requirement)
/// 2. `meetingDetectNotify.enabled` and the per-platform allow-list
/// 3. Meeting-notify ledger dedup (suppress if already notified within 6 h)
///
/// Returns `true` when the notification was fired, `false` if suppressed.
/// Also increments the tray `Prompt` badge on a successful fire.
#[tauri::command]
pub async fn meetings_notify_detected(
    app: AppHandle,
    payload: NotifyDetectedPayload,
) -> Result<bool, String> {
    use crate::commands::settings::get_settings;
    use crate::tray::{get_prompt_pending, set_prompt_badge};
    use crate::util::meeting_ledger::{claim_notify, stable_key};
    use chrono::Utc;
    use tauri::Emitter;

    // 1. Top-level notifications pref.
    let settings = get_settings().await?;
    if !settings.notifications.unwrap_or(true) {
        return Ok(false);
    }

    // 2. Meeting-detect-notify sub-pref + per-platform filter.
    let mdn = settings.meeting_detect_notify.as_ref();
    if !mdn.and_then(|m| m.enabled).unwrap_or(true) {
        return Ok(false);
    }
    let platform_lc = payload
        .platform
        .as_deref()
        .unwrap_or("")
        .to_ascii_lowercase();
    if !platform_lc.is_empty() {
        if let Some(allowed) = mdn.and_then(|m| m.platforms.as_ref()) {
            if !allowed.is_empty()
                && !allowed.iter().any(|p| p.to_ascii_lowercase() == platform_lc)
            {
                return Ok(false);
            }
        }
    }

    // 3. Dedup via ledger — atomic single-flight claim.
    //
    // The SDK emits `meeting:detected` once but Tauri fans it out to EVERY
    // webview, so both the popover (App.svelte) and the desktop-alt window
    // (activeMeetings.ts) can invoke this command for the same meeting
    // concurrently. `claim_notify` takes a process-wide lock and performs
    // read→should_suppress→mark(Notified)→write as one indivisible step, so the
    // second caller observes the first's just-written entry and loses the race.
    // This is the authoritative guard against double-notifying; the source-level
    // dedup in activeMeetings.ts is defence-in-depth on top of it.
    //
    // The claim runs synchronously and BEFORE any `.await` below — the ledger
    // lock guard must never be held across an await point (it is dropped here,
    // before the async banner dispatch).
    let key = match stable_key(
        payload.meeting_url.as_deref(),
        payload.source_event_id.as_deref(),
    ) {
        Some(k) => k,
        None => return Ok(false),
    };
    let now = Utc::now();
    if !claim_notify(&key, now) {
        // Either already notified/recorded within the suppression window, or a
        // concurrent caller won the claim. Do not fire a second banner.
        return Ok(false);
    }

    // 4. Build notification body: "<Platform>: <summary or url>".
    let body = build_notification_body(
        &platform_lc,
        payload.summary.as_deref(),
        payload.meeting_url.as_deref(),
    );

    // 5. Custom-banner path (default). When `customBanner` is on, deliver the
    // detection through the in-app liquid-glass banner — the same HQ-branded
    // surface as DM / share / update — instead of the native UN/osascript/legacy
    // stack below. The banner runs in-process, so a body-click opens the Meetings
    // window straight from `App.svelte`'s banner-action router (no UN delegate
    // needed) and the chip starts recording. The native delivery below is kept
    // only as the `customBanner: false` fallback (older macOS / opt-out).
    if crate::commands::banner::custom_banner_enabled() {
        let title = build_notification_title(&platform_lc);
        let window_id = payload.window_id.clone().unwrap_or_default();
        if let Err(e) = crate::commands::banner::show_meeting_banner(
            app.clone(),
            title,
            body.clone(),
            window_id,
            platform_lc.clone(),
        )
        .await
        {
            crate::util::logfile::log(
                "meetings",
                &format!("custom meeting banner failed: {e}"),
            );
        }
        // Ledger already marked Notified by `claim_notify` above (the claim is
        // what authorised this fire), so we only bump the tray badge here.
        set_prompt_badge(&app, get_prompt_pending() + 1);
        return Ok(true);
    }

    // 6. Native fallback (customBanner off). Fire via `mac-notification-sys` directly so we
    // can attach a "Record" action button AND learn when the user clicks
    // the notification body. `tauri-plugin-notification` 2.3.3's desktop
    // path ignores `action_type_id` (it's mobile-only at that layer), so
    // we bypass it for this specific surface and keep the rest of the
    // app's plugin-based notifications unchanged.
    //
    // Wire shape:
    //   Notification body click  → emit `notification:meeting-action`
    //                              with action="open"
    //   "Record" action button   → emit `notification:meeting-action`
    //                              with action="record"
    //   Anything else (dismiss,  → no event (silently dropped)
    //   ignore, timeout)
    //
    // The Svelte side listens for the event and either focuses the
    // popover (action=open) or invokes `start_recording` directly
    // (action=record).
    //
    // mac-notification-sys's `send()` is blocking when `wait_for_click`
    // is true — it returns the response only after the user interacts.
    // We spawn a dedicated thread per notification so the async Tauri
    // command itself never blocks; the thread captures the windowId via
    // closure and lives until the user dismisses the notification (or
    // macOS auto-dismisses it).
    // Register the bundle identifier with mac-notification-sys once per
    // process lifetime. Without this, the library calls
    // `get_bundle_identifier_or_default("use_default")` internally, which
    // triggers a macOS "Choose Application" picker because Launch
    // Services can't resolve the literal string "use_default" to an
    // installed app (observed in field test on 2026-05-25). Calling
    // `set_application` with our real bundle id makes notifications
    // attribute correctly to HQ Sync and skips the picker.
    //
    // `set_application` itself is guarded by an internal `Once`, so
    // calling it on every notification send would be safe — but wrapping
    // in our own OnceLock keeps the log line at one-per-process.
    static NOTIFICATION_APP_INIT: OnceLock<()> = OnceLock::new();
    NOTIFICATION_APP_INIT.get_or_init(|| {
        const BUNDLE_ID: &str = "ai.indigo.hq-sync-menubar";
        match mac_notification_sys::set_application(BUNDLE_ID) {
            Ok(()) => {
                crate::util::logfile::log(
                    "meetings",
                    &format!("mac-notification-sys: registered bundle {BUNDLE_ID}"),
                );
            }
            Err(e) => {
                crate::util::logfile::log(
                    "meetings",
                    &format!(
                        "mac-notification-sys: set_application({BUNDLE_ID}) failed: {e}"
                    ),
                );
            }
        }
    });

    let window_id_for_thread = payload
        .window_id
        .clone()
        .unwrap_or_default();
    let platform_for_thread = platform_lc.clone();
    let title_for_thread = build_notification_title(&platform_lc);
    let body_for_thread = body.clone();
    let app_for_thread = app.clone();
    std::thread::spawn(move || {
        // ── Clickable delivery: UNUserNotificationCenter ────────────────
        // When notification permission is *granted*, deliver through
        // UNUserNotificationCenter instead of osascript. A UN banner carries
        // a click callback (via the delegate installed at launch in
        // `un_notify.rs`), so clicking the banner opens the desktop-alt
        // "HQ Meetings" window — even on a *cold* click where no frontend
        // `notification:meeting-action` listener exists yet (the delegate
        // opens the window straight from Rust). UN silently DROPS the request
        // when status != "granted", so we gate strictly on "granted" and fall
        // through to the always-visible osascript path otherwise — zero
        // regression for non-granted users. One delivery path per branch, so
        // there's never a double banner.
        #[cfg(target_os = "macos")]
        if crate::commands::notifications::current_authorization_status() == "granted" {
            crate::commands::un_notify::deliver_clickable(
                &title_for_thread,
                &body_for_thread,
                &window_id_for_thread,
                &platform_for_thread,
            );
            return;
        }

        // ── Primary delivery: AppleScript ───────────────────────────────
        // macOS 26 (Sequoia) blocks NSUserNotification from being mixed
        // with UNUserNotificationCenter in the same process. usernoted
        // logs:
        //   Error: Legacy client ai.indigo.hq-sync-menubar connecting to
        //   modern client. Denying message N from <LegacyConnection>.
        // Once anything in the process touches UN (the
        // `notification_permission_state` IPC probe is enough), the
        // legacy `mac-notification-sys` deliver path is silently denied
        // forever. `osascript display notification` doesn't go through
        // the per-process legacy/modern gate — it's NotificationCenter's
        // own scripting bridge — so it fires reliably regardless.
        //
        // Trade-off vs. the legacy library: no action button. We lose
        // the inline "Record" button on the banner. Mitigation: the
        // popover's calendar icon now tints yellow on detection (see
        // MeetingIcon.svelte), so the user has an obvious in-app affordance
        // to act on the detection. Click on the notification body still
        // focuses HQ Sync (macOS's default behavior for any app's
        // notification), and our `notification:meeting-action` event
        // listener treats that as action="open" via the popover.
        //
        // Long-term: migrate to UNUserNotificationCenter via objc2 +
        // UNNotificationCategory + UNNotificationAction("RECORD") so the
        // Record button comes back. Tracked as a follow-up.
        let osa_body = body_for_thread.replace('\\', "\\\\").replace('"', "\\\"");
        let osa_title = title_for_thread.replace('\\', "\\\\").replace('"', "\\\"");
        let script = format!(
            "display notification \"{osa_body}\" with title \"{osa_title}\""
        );
        match std::process::Command::new("/usr/bin/osascript")
            .args(["-e", &script])
            .output()
        {
            Ok(out) if out.status.success() => {
                crate::util::logfile::log(
                    "meetings",
                    "osascript notification fired",
                );
            }
            Ok(out) => {
                let stderr = String::from_utf8_lossy(&out.stderr);
                crate::util::logfile::log(
                    "meetings",
                    &format!(
                        "osascript notification non-zero exit ({}): {}",
                        out.status, stderr
                    ),
                );
            }
            Err(e) => {
                crate::util::logfile::log(
                    "meetings",
                    &format!("osascript spawn failed: {e}"),
                );
            }
        }

        // ── Secondary delivery: legacy mac-notification-sys ────────────
        // Still attempted because: (1) some users may be on older macOS
        // where the legacy path works (Catalina/Big Sur/Monterey/Ventura),
        // and (2) it carries the Record action button via the response
        // thread. On Sequoia+ this thread's `send()` is denied by
        // usernoted and either errors immediately or returns
        // NotificationResponse::None — both paths are logged below so the
        // failure mode is visible.
        let mut notification = mac_notification_sys::Notification::default();
        notification
            .title(&title_for_thread)
            .message(&body_for_thread)
            // `main_button` attaches a single action button labelled
            // "Record". On older macOS this surfaces inline (alert
            // style) or on hover (banner style). Sequoia denies the
            // whole deliver, so this never reaches the user there.
            .main_button(mac_notification_sys::MainButton::SingleAction("Record"))
            // Block the thread until the user interacts (or macOS
            // auto-dismisses, which surfaces as None).
            .wait_for_click(true);
        match notification.send() {
            Ok(resp) => {
                // Log the raw response variant so a "user said they got no
                // notification" report can be cross-referenced against what
                // mac-notification-sys actually returned. None/timeout reads
                // identically to "macOS silently dropped the banner due to
                // missing UNUserNotificationCenter authorization" in the
                // logs — surfacing the variant makes that distinguishable.
                let variant = match &resp {
                    mac_notification_sys::NotificationResponse::ActionButton(n) => {
                        format!("ActionButton({n})")
                    }
                    mac_notification_sys::NotificationResponse::Click => "Click".to_string(),
                    mac_notification_sys::NotificationResponse::CloseButton(_) => {
                        "CloseButton".to_string()
                    }
                    mac_notification_sys::NotificationResponse::Reply(_) => "Reply".to_string(),
                    mac_notification_sys::NotificationResponse::None => "None".to_string(),
                };
                crate::util::logfile::log(
                    "meetings",
                    &format!("mac-notification-sys response: {variant}"),
                );
                let action = match resp {
                    mac_notification_sys::NotificationResponse::ActionButton(name)
                        if name.eq_ignore_ascii_case("record") =>
                    {
                        Some("record")
                    }
                    mac_notification_sys::NotificationResponse::Click => Some("open"),
                    // CloseButton, Reply, None — no actionable signal for us.
                    _ => None,
                };
                if let Some(action) = action {
                    let payload = crate::events::NotificationMeetingActionEvent {
                        action: action.to_string(),
                        window_id: window_id_for_thread,
                        platform: platform_for_thread,
                    };
                    if let Err(e) = app_for_thread
                        .emit(crate::events::EVENT_NOTIFICATION_MEETING_ACTION, &payload)
                    {
                        crate::util::logfile::log(
                            "meetings",
                            &format!(
                                "emit notification:meeting-action failed: {e}"
                            ),
                        );
                    }
                }
            }
            Err(e) => {
                crate::util::logfile::log(
                    "meetings",
                    &format!("mac-notification-sys send failed: {e}"),
                );
            }
        }
    });

    // 6. Bump tray badge. The ledger was already marked Notified atomically by
    // `claim_notify` above (the claim is what authorised this fire), so there is
    // no second read-modify-write here — that non-atomic re-write was the source
    // of the double-notification race.
    set_prompt_badge(&app, get_prompt_pending() + 1);

    Ok(true)
}

/// Decrement the meeting-prompt tray badge by one (saturating).
///
/// Call when the user opens MeetingsWindow or acts on a notification.
/// Saturating so a double-call doesn't underflow to `usize::MAX`.
#[tauri::command]
pub async fn meetings_clear_prompt_badge(app: AppHandle) {
    use crate::tray::{get_prompt_pending, set_prompt_badge};
    set_prompt_badge(&app, get_prompt_pending().saturating_sub(1));
}

/// Build the notification body for a detected meeting.
///
/// Format:
/// - With a summary: `"<Platform>: <summary>"` (calendar-derived path)
/// - With a real URL: `"<Platform>: <url>"` (active-app path)
/// - With a synthetic `recall-window:<id>` URL or no URL at all:
///   `"<Platform> meeting"` (URL-less SDK detection — typical for
///   unscheduled Zoom meetings joined from the desktop app; we don't want
///   to leak the synthetic key into the user-visible notification)
///
/// `platform_lc` is the lowercased platform discriminator (e.g. `"zoom"`).
/// Empty falls back to `"Meeting"`.
fn build_notification_body(
    platform_lc: &str,
    summary: Option<&str>,
    meeting_url: Option<&str>,
) -> String {
    let display = {
        let mut p = if platform_lc.is_empty() {
            "Meeting".to_string()
        } else {
            platform_lc.to_string()
        };
        if let Some(c) = p.get_mut(0..1) {
            c.make_ascii_uppercase();
        }
        p
    };
    let is_synthetic_url = |u: &str| u.starts_with("recall-window:");
    match summary.filter(|s| !s.is_empty()) {
        Some(s) => format!("{display}: {s}"),
        None => match meeting_url.filter(|s| !s.is_empty()) {
            Some(u) if !is_synthetic_url(u) => format!("{display}: {u}"),
            _ => format!("{display} meeting"),
        },
    }
}

/// Build the notification *title* for a detected meeting.
///
/// Kept separate from [`build_notification_body`] — that function's output is
/// pinned by a battery of regression tests, so the title gets its own builder
/// rather than threading platform formatting through the body path.
///
/// Format:
/// - Known platform: `"<Platform> meeting detected"` (e.g. `"Zoom meeting
///   detected"`) — less generic than the old flat `"Meeting detected"`, which
///   gave the user no hint which app triggered it.
/// - Empty platform: `"Meeting detected"` (graceful fallback; avoids the
///   awkward `" meeting detected"` / doubled-word shapes).
///
/// `platform_lc` is the lowercased platform discriminator (e.g. `"zoom"`).
fn build_notification_title(platform_lc: &str) -> String {
    if platform_lc.is_empty() {
        return "Meeting detected".to_string();
    }
    let mut display = platform_lc.to_string();
    if let Some(c) = display.get_mut(0..1) {
        c.make_ascii_uppercase();
    }
    format!("{display} meeting detected")
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

    /// Regression — `POST /v1/bot/invite` returns a slimmer body than
    /// `GET /v1/bot/list`, omitting `autoScheduled` (and the Optional
    /// fields). The invite command deserializes that response into
    /// `ScheduledBot`; if the struct treats `autoScheduled` as required the
    /// parse fails and the UI shows a spurious "Couldn't invite the bot."
    /// toast even though the bot was scheduled successfully. The slim body
    /// must parse, defaulting `auto_scheduled` to false.
    #[test]
    fn scheduled_bot_parses_slim_invite_response_without_auto_scheduled() {
        let json = r#"{
            "botId": "bot-abc",
            "status": "scheduled",
            "meetingUrl": "https://us06web.zoom.us/j/85906",
            "platform": "zoom",
            "createdAt": "2026-05-29T12:00:00Z"
        }"#;
        let bot: ScheduledBot = serde_json::from_str(json).expect("slim invite body must parse");
        assert_eq!(bot.bot_id, "bot-abc");
        assert_eq!(bot.status, "scheduled");
        assert_eq!(bot.platform, "zoom");
        assert!(!bot.auto_scheduled, "manual invite defaults to not auto-scheduled");
        assert!(bot.calendar_event_id.is_none());
        assert!(bot.meeting_title.is_none());
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
        assert!(evt.signals.is_none());
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
            "sourceCompanyUid": "cmp_indigo",
            "signals": {
                "actions": ["Send notes"],
                "decisions": [{"title": "Ship"}],
                "risks": []
            }
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
        assert_eq!(
            evt.signals
                .as_ref()
                .and_then(|signals| signals["actions"].as_array())
                .map(Vec::len),
            Some(1),
        );
    }

    // ── build_notification_body ──────────────────────────────────────────────
    // Regression: URL-less SDK detections (typically unscheduled Zoom from
    // the desktop app) reach us with a synthetic `recall-window:<id>` URL
    // from the bridge. The notification body must NOT leak that key — it
    // should render as "<Platform> meeting" instead.

    #[test]
    fn build_notification_body_uses_summary_when_present() {
        let body = build_notification_body(
            "zoom",
            Some("Weekly standup"),
            Some("https://zoom.us/j/123"),
        );
        assert_eq!(body, "Zoom: Weekly standup");
    }

    #[test]
    fn build_notification_body_uses_url_when_no_summary() {
        let body = build_notification_body(
            "meet",
            None,
            Some("https://meet.google.com/abc-defg-hij"),
        );
        assert_eq!(body, "Meet: https://meet.google.com/abc-defg-hij");
    }

    #[test]
    fn build_notification_body_hides_synthetic_recall_window_url() {
        // The bridge emits this shape when the SDK detects a meeting window
        // but can't scrape a real join URL.
        let body = build_notification_body(
            "zoom",
            None,
            Some("recall-window:43F5EBF4-8949-4DD4-B075-2E8EF68AAA30"),
        );
        assert_eq!(body, "Zoom meeting");
        // Hard check: no part of the synthetic key leaks.
        assert!(!body.contains("recall-window"), "synthetic key leaked: {body}");
        assert!(!body.contains("43F5EBF4"), "windowId leaked: {body}");
    }

    #[test]
    fn build_notification_body_summary_wins_over_synthetic_url() {
        // If for some reason the SDK gave us both, summary should still win.
        let body = build_notification_body(
            "zoom",
            Some("Quick chat"),
            Some("recall-window:abc"),
        );
        assert_eq!(body, "Zoom: Quick chat");
    }

    #[test]
    fn build_notification_body_falls_back_when_url_and_summary_missing() {
        let body = build_notification_body("teams", None, None);
        assert_eq!(body, "Teams meeting");
    }

    #[test]
    fn build_notification_body_handles_unknown_platform() {
        // Bridge maps unrecognised platforms to "other" — verify graceful
        // rendering without panicking on the first-char uppercase.
        let body = build_notification_body("", None, None);
        assert_eq!(body, "Meeting meeting");
        // ^^ ugly-but-stable; this path requires both platform AND url AND
        // summary to be missing, which currently can't happen — the bridge
        // always sends at least the synthetic URL. Keeps the function total.
    }

    // ── build_notification_title ─────────────────────────────────────────────
    // The title names the platform so the banner isn't the generic
    // "Meeting detected" — e.g. "Zoom meeting detected". Empty platform must
    // degrade gracefully to the bare "Meeting detected" (no doubled words, no
    // leading space).

    #[test]
    fn build_notification_title_names_known_platform() {
        assert_eq!(build_notification_title("zoom"), "Zoom meeting detected");
        assert_eq!(build_notification_title("meet"), "Meet meeting detected");
        assert_eq!(build_notification_title("teams"), "Teams meeting detected");
    }

    #[test]
    fn build_notification_title_falls_back_on_empty_platform() {
        // Must not produce " meeting detected" or "Meeting meeting detected".
        let title = build_notification_title("");
        assert_eq!(title, "Meeting detected");
        assert!(!title.starts_with(' '), "leading space leaked: {title:?}");
    }

    #[test]
    fn build_notification_body_treats_empty_strings_as_missing() {
        // The Svelte handler can forward `meetingUrl: ""` / `summary: ""`
        // depending on how the payload was constructed.
        let body = build_notification_body("zoom", Some(""), Some(""));
        assert_eq!(body, "Zoom meeting");
    }
}
