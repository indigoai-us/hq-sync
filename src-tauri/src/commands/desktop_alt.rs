//! Feature gate for the expanded desktop window surface.
//!
//! GA gate for the expanded popover/desktop window. This surface graduated
//! from the Indigo-only dogfood: it now delegates to
//! `feature_gate::desktop_features_enabled()`, which admits **any** signed-in
//! user (non-empty email claim). There is no parallel cache here — the GA
//! gate owns its own process-lifetime OnceLock cache.
//!
//! On cold start (cache uninitialised) the underlying
//! `desktop_features_enabled()` call awaits `compute_ga_gate()` and returns
//! the canonical email-derived answer instead of falling back to false. This
//! matters because the popover mounts and invokes the gate before any cloud
//! round-trip has had a chance to seed an unrelated cache — we owe the caller
//! the real answer, not a default.
//!
//! See `src-tauri/src/commands/meetings.rs::meetings_feature_enabled` for
//! the reference pattern this command mirrors.
//!
//! Result type is `Result<bool, String>` to match the established gate
//! command shape, but `desktop_features_enabled()` itself never errors — the
//! Ok arm is always taken.
use std::collections::BTreeMap;
use std::sync::{Mutex, OnceLock};

use chrono::{DateTime, Utc};
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};

use crate::commands::cognito;
use crate::commands::sync::resolve_vault_api_url;
use crate::commands::workspaces::{Workspace, WorkspaceState};
use crate::util::client_info::build_client;

const WINDOW_LABEL: &str = "desktop-alt";
const HQ_DEPLOY_API_BASE: &str = "https://api.indigo-hq.com";
const HQ_DEPLOY_APP_DOMAIN: &str = "indigo-hq.com";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BoardCard {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub subtitle: Option<String>,
    #[serde(default)]
    pub href: Option<String>,
    #[serde(default)]
    pub labels: Vec<String>,
    #[serde(default)]
    pub assignee_initials: Option<String>,
    #[serde(default)]
    pub tag: Option<String>,
    #[serde(default)]
    pub age: Option<String>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CompanyBoard {
    #[serde(default)]
    pub inbox: Vec<BoardCard>,
    #[serde(default)]
    pub doing: Vec<BoardCard>,
    #[serde(default)]
    pub review: Vec<BoardCard>,
    #[serde(default)]
    pub done: Vec<BoardCard>,
}

impl CompanyBoard {
    /// Total cards across every column — the board count shown in the
    /// Company header and tab badge.
    pub fn card_count(&self) -> u32 {
        let total = self.inbox.len() + self.doing.len() + self.review.len() + self.done.len();
        u32::try_from(total).unwrap_or(u32::MAX)
    }
}

impl CompanyActivity {
    /// Activity in the last 7 days. The vault activity payload already
    /// rolls this up as `stats.files7` (files touched in the trailing 7
    /// days); we surface that directly as the header's `last7d` count.
    pub fn last7d(&self) -> u32 {
        self.stats.files7
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CompanyActivity {
    #[serde(default)]
    pub stats: ActivityStats,
    #[serde(default)]
    pub sparkline: Vec<u32>,
    #[serde(default)]
    pub recent: Vec<ActivityEntry>,
    #[serde(default)]
    pub top: Vec<ActivityContributor>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ActivityStats {
    #[serde(default)]
    pub files7: u32,
    #[serde(default)]
    pub edits7: u32,
    #[serde(default)]
    pub members: u32,
    #[serde(default)]
    pub vault_size: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ActivityEntry {
    #[serde(default)]
    pub who: String,
    #[serde(default)]
    pub what: String,
    #[serde(default)]
    pub file: String,
    #[serde(default)]
    pub when: String,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ActivityContributor {
    #[serde(default)]
    pub who: String,
    #[serde(default)]
    pub edits: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DeploymentEntry {
    pub sub: String,
    pub url: String,
    pub state: String,
    pub last_deploy: String,
    pub size: String,
    pub ver: String,
    pub pwd: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SecretItem {
    pub key: String,
    pub upd: String,
    pub rot: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SecretEnv {
    pub env: String,
    pub count: usize,
    pub items: Vec<SecretItem>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LiveBoardModel {
    #[serde(default)]
    projects: Vec<LiveBoardProject>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LiveBoardProject {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    uid: Option<String>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    assignee_initials: Option<String>,
    #[serde(default)]
    assignee: Option<LiveBoardAssignee>,
    #[serde(default)]
    tag: Option<String>,
    #[serde(default)]
    kind: Option<String>,
    #[serde(default)]
    #[serde(rename = "type")]
    source_type: Option<String>,
    #[serde(default)]
    project_type: Option<String>,
    #[serde(default)]
    labels: Vec<String>,
    #[serde(default)]
    updated_at: Option<String>,
    #[serde(default)]
    created_at: Option<String>,
    #[serde(default)]
    age: Option<String>,
    #[serde(flatten)]
    extra: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LiveBoardAssignee {
    #[serde(default)]
    initials: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    email: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CompanyActivitySummary {
    pub last7d: u32,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CompanySummary {
    pub board: u32,
    pub activity: CompanyActivitySummary,
    pub deployments: u32,
    pub secrets: u32,
}

#[tauri::command]
pub async fn desktop_alt_enabled() -> Result<bool, String> {
    Ok(crate::util::feature_gate::desktop_features_enabled().await)
}

/// Admin gate for the desktop-alt Moderation surface (UX only — the server is
/// the sole authorization boundary). True iff the signed-in email ends in
/// `@getindigo.ai`.
///
/// Distinct from [`desktop_alt_enabled`], which is the GA gate (true for any
/// signed-in user) controlling access to the window itself. The Moderation nav
/// row + panel must use THIS gate so normal HQ users never see the reviewer
/// surface — a non-admin who reaches the underlying commands still gets a 403.
#[tauri::command]
pub async fn desktop_alt_is_admin() -> Result<bool, String> {
    Ok(crate::util::feature_gate::is_indigo_user().await)
}

#[tauri::command]
pub async fn get_company_summary(slug: String) -> Result<CompanySummary, String> {
    if slug.trim().is_empty() {
        return Err("company slug is required".to_string());
    }

    // Aggregate the four real per-panel commands. Each surface is
    // best-effort: a non-auth failure (404 not-provisioned, network, parse)
    // contributes 0 so one dead endpoint doesn't zero the others. Auth
    // failures are different — they must propagate so the UI can route to
    // sign-in rather than silently rendering empty counts.
    // DIAGNOSTIC: capture each surface's raw Result (count or error string)
    // before collapsing, so a "panel shows 0" can be traced to the exact
    // surface + reason. Counts and error messages only — never secret values.
    let board_res = get_company_board(slug.clone())
        .await
        .map(|b| b.card_count());
    let activity_res = get_company_activity(slug.clone()).await.map(|a| a.last7d());
    let deployments_res = get_company_deployments(slug.clone())
        .await
        .map(|d| u32::try_from(d.len()).unwrap_or(u32::MAX));
    let secrets_res = get_company_secrets(slug)
        .await
        .map(|s| u32::try_from(s.len()).unwrap_or(u32::MAX));
    eprintln!(
        "[desktop-alt] summary surfaces: board={board_res:?} activity={activity_res:?} deployments={deployments_res:?} secrets={secrets_res:?}"
    );

    let board = summary_count_or_auth(board_res)?;
    let last7d = summary_count_or_auth(activity_res)?;
    let deployments = summary_count_or_auth(deployments_res)?;
    let secrets = summary_count_or_auth(secrets_res)?;
    eprintln!(
        "[desktop-alt] summary final: board={board} activity={last7d} deployments={deployments} secrets={secrets}"
    );

    Ok(CompanySummary {
        board,
        activity: CompanyActivitySummary { last7d },
        deployments,
        secrets,
    })
}

/// Collapse a per-surface command result into the count for the summary.
/// Auth-required errors propagate (so the UI routes to sign-in); every
/// other error degrades to `0` for that surface so the rest still render.
fn summary_count_or_auth(result: Result<u32, String>) -> Result<u32, String> {
    match result {
        Ok(count) => Ok(count),
        Err(err) if is_auth_required_error(&err) => Err(err),
        Err(_) => Ok(0),
    }
}

fn is_auth_required_error(err: &str) -> bool {
    err.starts_with("AUTH_REQUIRED:")
}

#[tauri::command]
pub async fn get_company_board(slug: String) -> Result<CompanyBoard, String> {
    let slug = normalize_slug(&slug)?;
    let company_uid = resolve_company_uid(&slug).await?;
    let url = board_url(&vault_base()?, &company_uid)?;
    let token = cognito::get_valid_access_token()
        .await
        .map_err(|e| format!("auth: {e}"))?;

    let res = build_client()
        .get(&url)
        .header("authorization", format!("Bearer {token}"))
        .send()
        .await
        .map_err(|e| format!("board fetch: {e}"))?;
    let status = res.status();
    let text = res.text().await.map_err(|e| format!("board read: {e}"))?;
    eprintln!(
        "[desktop-alt] board GET {url} -> HTTP {} ({} bytes): {}",
        status,
        text.len(),
        text.chars().take(200).collect::<String>()
    );

    parse_board_response(status, &text)
}

/// Per-project creator for the Projects list Lead column. The cloud board model
/// already derives each project's creator from its prd.json's S3 `created-by`
/// author metadata (resolved honestly server-side — never fabricated), so we
/// just expose it here. Rows are matchable to `get_local_projects` by board
/// `id` (same board.json project id) or by `prdPath`. Only projects that
/// actually carry a creator are returned; everything else stays "Unassigned".
#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ProjectCreator {
    pub id: String,
    pub prd_path: Option<String>,
    pub creator: String,
}

#[derive(Debug, Deserialize)]
struct BoardCreatorEnvelope {
    #[serde(default)]
    projects: Vec<BoardCreatorProject>,
}

#[derive(Debug, Deserialize)]
struct BoardCreatorProject {
    #[serde(default)]
    id: String,
    #[serde(default, rename = "prdPath")]
    prd_path: Option<String>,
    #[serde(default, rename = "createdByName")]
    created_by_name: Option<String>,
}

#[tauri::command]
pub async fn get_company_project_creators(slug: String) -> Result<Vec<ProjectCreator>, String> {
    let slug = normalize_slug(&slug)?;
    let company_uid = resolve_company_uid(&slug).await?;
    let url = board_url(&vault_base()?, &company_uid)?;
    let token = cognito::get_valid_access_token()
        .await
        .map_err(|e| format!("auth: {e}"))?;
    let res = build_client()
        .get(&url)
        .header("authorization", format!("Bearer {token}"))
        .send()
        .await
        .map_err(|e| format!("creators fetch: {e}"))?;
    let status = res.status();
    let text = res.text().await.map_err(|e| format!("creators read: {e}"))?;
    // A missing / unprovisioned board (or no ACL) is not an error here — the
    // Lead column simply falls back to "Unassigned". Only the body parse below
    // can fail, and only on a 2xx with malformed JSON.
    if !status.is_success() {
        return Ok(Vec::new());
    }
    parse_project_creators(&text).map_err(|e| format!("creators parse: {e}"))
}

/// Pure parse of the board JSON into creator rows: keep only projects that
/// carry a non-empty `createdByName`, so the frontend map only contains real
/// creators (everything else stays "Unassigned"). Testable in isolation.
fn parse_project_creators(text: &str) -> Result<Vec<ProjectCreator>, String> {
    let env: BoardCreatorEnvelope = serde_json::from_str(text).map_err(|e| e.to_string())?;
    Ok(env
        .projects
        .into_iter()
        .filter_map(|p| {
            let creator = p.created_by_name?;
            let creator = creator.trim().to_string();
            if creator.is_empty() {
                return None;
            }
            Some(ProjectCreator {
                id: p.id,
                prd_path: p.prd_path,
                creator,
            })
        })
        .collect())
}

#[tauri::command]
pub async fn get_company_activity(slug: String) -> Result<CompanyActivity, String> {
    let slug = normalize_slug(&slug)?;
    let company_uid = resolve_company_uid(&slug).await?;
    let url = activity_url(&vault_base()?, &company_uid)?;
    let token = cognito::get_valid_access_token()
        .await
        .map_err(|e| format!("auth: {e}"))?;

    let res = build_client()
        .get(&url)
        .header("authorization", format!("Bearer {token}"))
        .send()
        .await
        .map_err(|e| format!("activity fetch: {e}"))?;
    let status = res.status();
    let text = res
        .text()
        .await
        .map_err(|e| format!("activity read: {e}"))?;
    eprintln!(
        "[desktop-alt] activity GET {url} -> HTTP {} ({} bytes): {}",
        status,
        text.len(),
        text.chars().take(200).collect::<String>()
    );

    parse_activity_response(status, &text)
}

#[tauri::command]
pub async fn get_company_deployments(slug: String) -> Result<Vec<DeploymentEntry>, String> {
    let slug = normalize_slug(&slug)?;
    let url = deployments_url(HQ_DEPLOY_API_BASE);
    let token = cognito::get_valid_access_token()
        .await
        .map_err(|e| format!("auth: {e}"))?;

    let res = build_client()
        .get(&url)
        .header("authorization", format!("Bearer {token}"))
        .header("x-org-slug", &slug)
        .send()
        .await
        .map_err(|e| format!("deployments fetch: {e}"))?;
    let status = res.status();
    let text = res
        .text()
        .await
        .map_err(|e| format!("deployments read: {e}"))?;
    eprintln!(
        "[desktop-alt] deployments GET {url} (x-org-slug={slug}) -> HTTP {} ({} bytes): {}",
        status,
        text.len(),
        text.chars().take(200).collect::<String>()
    );

    parse_deployments_response(status, &text, &slug)
}

#[tauri::command]
pub async fn get_company_secrets(slug: String) -> Result<Vec<SecretEnv>, String> {
    let slug = normalize_slug(&slug)?;
    let company_uid = resolve_company_uid(&slug).await?;
    let url = secrets_url(&vault_base()?, &company_uid)?;
    let token = cognito::get_valid_access_token()
        .await
        .map_err(|e| format!("auth: {e}"))?;

    let res = build_client()
        .get(&url)
        .header("authorization", format!("Bearer {token}"))
        .send()
        .await
        .map_err(|e| format!("secrets fetch: {e}"))?;
    let status = res.status();
    let text = res.text().await.map_err(|e| format!("secrets read: {e}"))?;
    // Secrets response bodies can carry plaintext secret values, so log
    // only status + byte length here (never a body snippet).
    eprintln!(
        "[desktop-alt] secrets GET {url} -> HTTP {} ({} bytes)",
        status,
        text.len()
    );

    parse_secrets_response(status, &text)
}

/// Route the desktop-alt window should land on the next time it mounts. Set by
/// callers that open the window with a specific intent — e.g. a "meeting
/// detected" notification click wants the Meetings screen, not the default Sync
/// screen. The frontend consumes this once on mount via
/// `desktop_alt_consume_pending_route`. For an already-open window we instead
/// emit `desktop:navigate` (see `open_desktop_alt_window_inner`), so the
/// pending slot is only relevant to a fresh build.
static PENDING_ROUTE: OnceLock<Mutex<Option<String>>> = OnceLock::new();

fn pending_route_cell() -> &'static Mutex<Option<String>> {
    PENDING_ROUTE.get_or_init(|| Mutex::new(None))
}

fn set_pending_route(route: Option<&str>) {
    if let Ok(mut cell) = pending_route_cell().lock() {
        *cell = route.map(|r| r.to_string());
    }
}

/// Take (and clear) the route the desktop-alt window should open on. Returns
/// `None` when nothing was queued — the frontend then keeps its default initial
/// route. Called once by `DesktopApp` on mount.
#[tauri::command]
pub fn desktop_alt_consume_pending_route() -> Option<String> {
    pending_route_cell()
        .lock()
        .ok()
        .and_then(|mut cell| cell.take())
}

/// Dev-only render audit for local desktop verification. No-ops unless
/// `HQ_DEV_AUDIT_DESKTOP_RENDER=1` is set before launch.
#[tauri::command]
pub fn desktop_alt_dev_audit_render(
    company_row_count: usize,
    footer: Option<String>,
    names: Vec<String>,
    has_more_companies_text: bool,
) {
    if std::env::var("HQ_DEV_AUDIT_DESKTOP_RENDER").ok().as_deref() != Some("1") {
        return;
    }

    let sample = names.into_iter().take(12).collect::<Vec<_>>().join(" | ");
    let footer = footer.unwrap_or_default();
    let line = format!(
        "render company_rows={company_row_count} has_more_companies_text={has_more_companies_text} footer={footer:?} sample={sample:?}"
    );
    crate::util::logfile::log("desktop-alt-dev", &line);
    eprintln!("[desktop-alt-dev] {line}");
}

/// Open or focus the expanded desktop window (GA — any signed-in user).
///
/// The window is declared in `tauri.conf.json` as hidden, so normal app
/// startup does not surface it. This command is still defensive and can
/// rebuild the window if it was closed earlier in the session.
///
/// `route` (optional) lands the window on a specific screen — e.g. `"meetings"`
/// from the meeting-detected notification. Omitted (the manual "open new UX"
/// button) keeps the default Sync screen.
#[tauri::command]
pub async fn open_desktop_alt_window(app: AppHandle, route: Option<String>) -> Result<(), String> {
    open_desktop_alt_window_inner(app, route.as_deref()).await
}

/// Window open/focus body, callable from non-command contexts (e.g. the
/// `UNUserNotificationCenter` delegate handling a cold notification click,
/// where no `#[tauri::command]` invocation is in flight). Keeps the GA
/// gate (signed-in check) so the delegate path is defense-in-depth too.
///
/// `route` routes the window to a screen: an already-open window gets a live
/// `desktop:navigate` event; a fresh build queues the route for the frontend
/// to consume on mount.
pub async fn open_desktop_alt_window_inner(
    app: AppHandle,
    route: Option<&str>,
) -> Result<(), String> {
    if !desktop_alt_enabled().await? {
        return Err("desktop-alt requires a signed-in user".to_string());
    }

    // One HQ window at a time: opening the desktop view hides the classic
    // popover (whether summoned via shortcut, menu, or the popover's own
    // "Open desktop view" button).
    if let Some(popover) = app.get_webview_window("main") {
        let _ = popover.hide();
    }

    if let Some(window) = app.get_webview_window(WINDOW_LABEL) {
        window.show().map_err(|e| e.to_string())?;
        window.set_focus().map_err(|e| e.to_string())?;
        // Already mounted: it won't re-consume a pending route, so push the
        // navigation live. Fire-and-forget — a missing listener is harmless.
        if let Some(route) = route {
            let _ = app.emit("desktop:navigate", route);
        }
        return Ok(());
    }

    // Fresh build: queue the route so the window picks it up on mount via
    // `desktop_alt_consume_pending_route`.
    set_pending_route(route);

    tauri::WebviewWindowBuilder::new(
        &app,
        WINDOW_LABEL,
        tauri::WebviewUrl::App("desktop-alt.html".into()),
    )
    // Empty native title: the Overlay title bar would otherwise paint "HQ"
    // over the custom titlebar's sync-status text (the verdict). The window's
    // own UI provides the heading, so the macOS title is intentionally blank.
    .title("")
    .inner_size(1180.0, 760.0)
    .min_inner_size(960.0, 600.0)
    .resizable(true)
    .decorations(true)
    .title_bar_style(tauri::TitleBarStyle::Overlay)
    .transparent(false)
    .visible(true)
    .build()
    .map_err(|e| e.to_string())?;

    Ok(())
}

fn normalize_slug(slug: &str) -> Result<String, String> {
    let slug = slug.trim();
    if slug.is_empty() {
        return Err("company slug is required".to_string());
    }
    Ok(slug.to_string())
}

async fn resolve_company_uid(slug: &str) -> Result<String, String> {
    let result = crate::commands::workspaces::list_syncable_workspaces().await?;
    resolve_company_uid_from_workspaces(result.workspaces, slug)
}

fn resolve_company_uid_from_workspaces(
    workspaces: Vec<Workspace>,
    slug: &str,
) -> Result<String, String> {
    let workspace = workspaces
        .into_iter()
        .find(|workspace| workspace.slug == slug)
        .ok_or_else(|| format!("company '{slug}' was not found"))?;
    if workspace.state == WorkspaceState::Broken {
        let reason = workspace
            .broken_reason
            .as_deref()
            .unwrap_or("workspace cloud mapping is broken");
        if let Some(live_cloud_uid) = live_cloud_uid_from_broken_reason(reason) {
            return Ok(live_cloud_uid);
        }
        return Err(format!("company '{slug}' is not synced: {reason}"));
    }
    if !matches!(
        workspace.state,
        WorkspaceState::Synced | WorkspaceState::CloudOnly
    ) {
        return Err(format!(
            "company '{slug}' is not synced (state: {:?})",
            workspace.state
        ));
    }
    workspace
        .cloud_uid
        .ok_or_else(|| format!("company '{slug}' is not connected to cloud"))
}

fn live_cloud_uid_from_broken_reason(reason: &str) -> Option<String> {
    let reason = reason.strip_prefix("manifest cloud_uid ")?;
    let (manifest_uid, reason) = reason.split_once(" does not match cloud entity ")?;
    let live_uid = reason.strip_suffix(" for this slug")?;
    if manifest_uid.is_empty()
        || live_uid.is_empty()
        || manifest_uid == live_uid
        || !is_url_safe_id(live_uid)
    {
        return None;
    }
    Some(live_uid.to_string())
}

fn vault_base() -> Result<String, String> {
    resolve_vault_api_url().map(|u| u.trim_end_matches('/').to_string())
}

fn board_url(base: &str, company_uid: &str) -> Result<String, String> {
    if !is_url_safe_id(company_uid) {
        return Err(format!(
            "company uid has invalid characters: {company_uid:?}"
        ));
    }
    Ok(format!(
        "{}/companies/{}/board",
        base.trim_end_matches('/'),
        company_uid
    ))
}

fn activity_url(base: &str, company_uid: &str) -> Result<String, String> {
    if !is_url_safe_id(company_uid) {
        return Err(format!(
            "company uid has invalid characters: {company_uid:?}"
        ));
    }
    Ok(format!(
        "{}/companies/{}/activity",
        base.trim_end_matches('/'),
        company_uid
    ))
}

/// Build the hq-deploy URL for the company Deployments panel.
///
/// Uses the ORG-scoped `GET /api/apps` (not the personal `GET /api/apps/me`).
/// The panel is a *company* dashboard: it must show every app in the org —
/// the same set the `hq deploy` CLI and the console table show — not just the
/// apps owned by the signed-in caller. `/api/apps/me` post-filters server-side
/// to `ownerId === userId`, so for a member viewing co-collaborators' apps it
/// returns `{"apps":[]}` (HTTP 200, empty) and the panel rendered 0 even when
/// the org has live deployments. Org scoping comes from the `x-org-slug`
/// header the caller already sends; the response shape is byte-identical
/// (`{"apps":[…]}`, no `orgSlug` on rows), so the parser is unchanged.
fn deployments_url(base: &str) -> String {
    format!("{}/api/apps", base.trim_end_matches('/'))
}

fn secrets_url(base: &str, company_uid: &str) -> Result<String, String> {
    if !is_url_safe_id(company_uid) {
        return Err(format!(
            "company uid has invalid characters: {company_uid:?}"
        ));
    }
    Ok(format!(
        "{}/secrets/{}",
        base.trim_end_matches('/'),
        company_uid
    ))
}

fn parse_board_response(status: StatusCode, text: &str) -> Result<CompanyBoard, String> {
    if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
        return Err(format!("AUTH_REQUIRED: board (HTTP {status})"));
    }
    if status == StatusCode::NO_CONTENT {
        return Ok(CompanyBoard::default());
    }
    if status == StatusCode::NOT_FOUND {
        return if is_board_not_provisioned(text) {
            eprintln!("[desktop-alt] board 404 not-provisioned -> empty board: {text}");
            Ok(CompanyBoard::default())
        } else {
            Err(format!("board HTTP {status}: {text}"))
        };
    }
    if !status.is_success() {
        return Err(format!("board HTTP {status}: {text}"));
    }

    let text = text.trim();
    if text.is_empty() {
        eprintln!("[desktop-alt] board {status} empty body -> empty board");
        return Ok(CompanyBoard::default());
    }

    parse_company_board(text)
}

fn parse_activity_response(status: StatusCode, text: &str) -> Result<CompanyActivity, String> {
    if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
        return Err(format!("AUTH_REQUIRED: activity (HTTP {status})"));
    }
    if status == StatusCode::NO_CONTENT {
        return Ok(CompanyActivity::default());
    }
    if status == StatusCode::NOT_FOUND {
        return if is_activity_not_provisioned(text) {
            eprintln!("[desktop-alt] activity 404 not-provisioned -> empty activity: {text}");
            Ok(CompanyActivity::default())
        } else {
            Err(format!("activity HTTP {status}: {text}"))
        };
    }
    if !status.is_success() {
        return Err(format!("activity HTTP {status}: {text}"));
    }

    let text = text.trim();
    if text.is_empty() {
        eprintln!("[desktop-alt] activity {status} empty body -> empty activity");
        return Ok(CompanyActivity::default());
    }

    parse_company_activity(text)
}

fn parse_company_board(text: &str) -> Result<CompanyBoard, String> {
    let value: serde_json::Value =
        serde_json::from_str(text).map_err(|e| format!("board parse: {e}"))?;
    if value.get("projects").and_then(|v| v.as_array()).is_some() {
        let live: LiveBoardModel =
            serde_json::from_value(value).map_err(|e| format!("board parse: {e}"))?;
        return Ok(live.into_company_board());
    }
    serde_json::from_value(value).map_err(|e| format!("board parse: {e}"))
}

fn parse_company_activity(text: &str) -> Result<CompanyActivity, String> {
    serde_json::from_str(text).map_err(|e| format!("activity parse: {e}"))
}

fn parse_secrets_response(status: StatusCode, text: &str) -> Result<Vec<SecretEnv>, String> {
    if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
        return Err(format!("AUTH_REQUIRED: secrets (HTTP {status})"));
    }
    if status == StatusCode::NO_CONTENT {
        return Ok(Vec::new());
    }
    if status == StatusCode::NOT_FOUND {
        return if is_secrets_not_provisioned(text) {
            eprintln!("[desktop-alt] secrets 404 not-provisioned -> empty secrets");
            Ok(Vec::new())
        } else {
            Err(format!("secrets HTTP {status}"))
        };
    }
    if !status.is_success() {
        return Err(format!("secrets HTTP {status}"));
    }

    let text = text.trim();
    if text.is_empty() {
        eprintln!("[desktop-alt] secrets {status} empty body -> empty secrets");
        return Ok(Vec::new());
    }

    parse_secret_envs(text)
}

fn parse_secret_envs(text: &str) -> Result<Vec<SecretEnv>, String> {
    let value: serde_json::Value =
        serde_json::from_str(text).map_err(|e| format!("secrets parse: {e}"))?;

    // STRUCTURE-ONLY diagnostic: logs the JSON shape (top-level kind, top-level
    // object key names, candidate array lengths, and the FIRST row's key names)
    // so a real-response shape mismatch is observable. NEVER logs any value —
    // only the *names* of keys and the *lengths* of arrays. Secret values must
    // never reach a log (see e2e/desktop-alt/secrets-never-leak.spec.ts).
    eprintln!(
        "[desktop-alt] secrets structure: {}",
        secret_structure_summary(&value)
    );

    let rows =
        secret_rows(&value).ok_or_else(|| "secrets parse: missing secrets array".to_string())?;
    let mut grouped: BTreeMap<String, Vec<SecretItem>> = BTreeMap::new();

    for row in rows {
        let Some(raw_key) = secret_key(row) else {
            continue;
        };
        let (env, key) = secret_env_and_key(row, &raw_key);
        grouped.entry(env).or_default().push(SecretItem {
            key,
            upd: secret_updated_at(row),
            rot: secret_rotation(row),
        });
    }

    Ok(grouped
        .into_iter()
        .map(|(env, mut items)| {
            items.sort_by(|a, b| a.key.cmp(&b.key));
            SecretEnv {
                env,
                count: items.len(),
                items,
            }
        })
        .collect())
}

/// Build a values-free description of a secrets JSON payload for diagnostics.
///
/// Reveals the top-level kind, top-level object key names, the lengths of the
/// candidate arrays `secret_rows` probes, and the key names present on the
/// first row of whichever array is found. It deliberately emits only key
/// *names* and array *lengths* — never a value, string contents, or anything
/// derived from a secret — so it is safe to write to stderr.
fn secret_structure_summary(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Array(rows) => {
            format!(
                "top-level array (len={}); first-row keys=[{}]",
                rows.len(),
                first_row_key_names(rows.first())
            )
        }
        serde_json::Value::Object(map) => {
            let top_keys: Vec<&str> = map.keys().map(String::as_str).collect();
            // Lengths of the arrays `secret_rows` knows how to read, so a
            // "found the envelope but it's empty/elsewhere" case is visible.
            let mut array_lens: Vec<String> = Vec::new();
            for path in ["secrets", "items", "data", "parameters", "body"] {
                if let Some(len) = map.get(path).and_then(|v| v.as_array()).map(Vec::len) {
                    array_lens.push(format!("{path}[]={len}"));
                }
            }
            let first_row_keys = secret_rows(value)
                .map(|rows| first_row_key_names(rows.first()))
                .unwrap_or_else(|| "<no array matched secret_rows>".to_string());
            format!(
                "top-level object; keys=[{}]; arrays=[{}]; first-row keys=[{}]",
                top_keys.join(","),
                array_lens.join(","),
                first_row_keys
            )
        }
        other => format!("top-level {} (not array/object)", json_kind(other)),
    }
}

/// Comma-joined key names of a JSON object row (names only, never values).
fn first_row_key_names(row: Option<&serde_json::Value>) -> String {
    match row {
        Some(serde_json::Value::Object(map)) => {
            map.keys().map(String::as_str).collect::<Vec<_>>().join(",")
        }
        Some(other) => format!("<row is {}>", json_kind(other)),
        None => "<empty>".to_string(),
    }
}

fn json_kind(value: &serde_json::Value) -> &'static str {
    match value {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "bool",
        serde_json::Value::Number(_) => "number",
        serde_json::Value::String(_) => "string",
        serde_json::Value::Array(_) => "array",
        serde_json::Value::Object(_) => "object",
    }
}

fn secret_rows(value: &serde_json::Value) -> Option<&Vec<serde_json::Value>> {
    if let Some(rows) = value.as_array() {
        return Some(rows);
    }
    value
        .get("secrets")
        .and_then(|v| v.as_array())
        .or_else(|| {
            value
                .get("body")
                .and_then(|body| body.get("secrets"))
                .and_then(|v| v.as_array())
        })
        .or_else(|| {
            value
                .get("data")
                .and_then(|data| data.get("secrets"))
                .and_then(|v| v.as_array())
        })
        .or_else(|| value.get("items").and_then(|v| v.as_array()))
}

fn secret_key(value: &serde_json::Value) -> Option<String> {
    string_field(
        value,
        &[
            "key",
            "name",
            "path",
            "secretPath",
            "secretName",
            "parameterName",
        ],
    )
}

fn secret_env_and_key(value: &serde_json::Value, raw_key: &str) -> (String, String) {
    if let Some(env) = string_field(value, &["env", "environment", "stage"]) {
        return (env, raw_key.to_string());
    }

    let key = raw_key.trim_matches('/');
    if let Some((env, rest)) = key.split_once('/') {
        if !env.is_empty() && !rest.is_empty() {
            return (env.to_string(), rest.trim_start_matches('/').to_string());
        }
    }
    if let Some((env, rest)) = key.split_once(':') {
        if !env.is_empty() && !rest.is_empty() {
            return (env.to_string(), rest.to_string());
        }
    }

    ("default".to_string(), raw_key.to_string())
}

fn secret_updated_at(value: &serde_json::Value) -> String {
    string_field(
        value,
        &[
            "lastModifiedDate",
            "lastModified",
            "updatedAt",
            "modifiedAt",
            "createdAt",
        ],
    )
    .unwrap_or_else(|| "-".to_string())
}

fn secret_rotation(value: &serde_json::Value) -> String {
    string_field(
        value,
        &[
            "rotation",
            "rotationStatus",
            "rotationSchedule",
            "nextRotationDate",
            "rot",
        ],
    )
    .or_else(|| {
        bool_field(value, &["rotationEnabled"]).map(|enabled| {
            if enabled {
                "scheduled".to_string()
            } else {
                "manual".to_string()
            }
        })
    })
    .unwrap_or_else(|| "manual".to_string())
}

fn parse_deployments_response(
    status: StatusCode,
    text: &str,
    selected_slug: &str,
) -> Result<Vec<DeploymentEntry>, String> {
    if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
        return Err(format!("AUTH_REQUIRED: deployments (HTTP {status})"));
    }
    if status == StatusCode::NO_CONTENT {
        return Ok(Vec::new());
    }
    if status == StatusCode::NOT_FOUND {
        return if is_deployments_not_provisioned(text) {
            eprintln!("[desktop-alt] deployments 404 not-provisioned -> empty deployments: {text}");
            Ok(Vec::new())
        } else {
            Err(format!("deployments HTTP {status}: {text}"))
        };
    }
    if !status.is_success() {
        return Err(format!("deployments HTTP {status}: {text}"));
    }

    let text = text.trim();
    if text.is_empty() {
        eprintln!("[desktop-alt] deployments {status} empty body -> empty deployments");
        return Ok(Vec::new());
    }

    parse_deployment_entries(text, selected_slug)
}

fn parse_deployment_entries(
    text: &str,
    selected_slug: &str,
) -> Result<Vec<DeploymentEntry>, String> {
    let value: serde_json::Value =
        serde_json::from_str(text).map_err(|e| format!("deployments parse: {e}"))?;
    let rows = deployment_rows(&value)
        .ok_or_else(|| "deployments parse: missing apps array".to_string())?;

    // Per-row resilience: a single malformed app (unsafe subdomain/url, missing
    // subdomain, etc.) must NOT blank the entire panel. The org-scoped
    // `/api/apps` returns every app in the org, so one odd row would otherwise
    // collapse the whole list to an error and zero the Deployments count.
    // Skip+log the bad row and keep the rest. The log carries only the row's
    // subdomain/url shape (no secrets) — deployments are public hostnames.
    let mut entries = Vec::new();
    let mut skipped = 0usize;
    for row in rows
        .iter()
        .filter(|row| deployment_matches_selected_slug(row, selected_slug))
    {
        match deployment_entry_from_value(row) {
            Ok(entry) => entries.push(entry),
            Err(e) => {
                skipped += 1;
                eprintln!("[desktop-alt] deployments: skipping unparseable app row ({e})");
            }
        }
    }
    if skipped > 0 {
        eprintln!(
            "[desktop-alt] deployments: kept {} app(s), skipped {} unparseable",
            entries.len(),
            skipped
        );
    }
    Ok(entries)
}

fn deployment_rows(value: &serde_json::Value) -> Option<&Vec<serde_json::Value>> {
    if let Some(rows) = value.as_array() {
        return Some(rows);
    }
    value
        .get("apps")
        .and_then(|v| v.as_array())
        .or_else(|| value.get("deployments").and_then(|v| v.as_array()))
        .or_else(|| value.get("data").and_then(|v| v.as_array()))
}

fn deployment_entry_from_value(value: &serde_json::Value) -> Result<DeploymentEntry, String> {
    let sub = string_field(value, &["sub", "subdomain", "slug"])
        .or_else(|| string_field(value, &["url"]).and_then(|url| subdomain_from_url(&url)))
        .map(|sub| sub.to_ascii_lowercase())
        .ok_or_else(|| "deployments parse: app missing subdomain".to_string())?;
    if !is_safe_deployment_label(&sub) {
        return Err(format!(
            "deployments parse: app has unsafe subdomain: {sub:?}"
        ));
    }
    let url = match string_field(value, &["url"]) {
        Some(url) => normalize_deployment_host(&url)
            .ok_or_else(|| format!("deployments parse: app has unsafe url: {url:?}"))?,
        None => format!("{sub}.{HQ_DEPLOY_APP_DOMAIN}"),
    };

    Ok(DeploymentEntry {
        sub,
        url,
        state: normalize_deployment_state(value),
        last_deploy: deployment_last_deploy(value),
        size: deployment_size(value),
        ver: deployment_version(value),
        pwd: bool_field(
            value,
            &["pwd", "passwordProtected", "passwordLocked", "locked"],
        )
        .unwrap_or(false),
    })
}

fn deployment_matches_selected_slug(value: &serde_json::Value, selected_slug: &str) -> bool {
    deployment_org_slug(value)
        .map(|org_slug| org_slug == selected_slug)
        .unwrap_or(true)
}

fn deployment_org_slug(value: &serde_json::Value) -> Option<String> {
    string_field(value, &["orgSlug", "org_slug"]).or_else(|| {
        value.get("org").and_then(|org| {
            org.as_str()
                .map(|slug| slug.trim().to_string())
                .filter(|slug| !slug.is_empty())
                .or_else(|| string_field(org, &["slug", "orgSlug", "org_slug"]))
        })
    })
}

fn normalize_deployment_state(value: &serde_json::Value) -> String {
    if bool_field(value, &["active"]).is_some_and(|active| !active) {
        return "paused".to_string();
    }

    let status = string_field(value, &["deployStatus", "status", "state", "dnsStatus"])
        .or_else(|| nested_string_field(value, "latestDeploy", &["status", "state"]))
        .or_else(|| nested_string_field(value, "deploy", &["status", "state"]));
    match normalize_status(status.as_deref()).as_deref() {
        Some(
            "uploading" | "extracting" | "syncing" | "invalidating" | "building" | "pushing"
            | "deploying" | "stabilizing" | "pending" | "inprogress" | "in_progress" | "running",
        ) => "deploying".to_string(),
        Some("paused" | "disabled" | "suspended" | "inactive" | "deactivated" | "stopped") => {
            "paused".to_string()
        }
        Some("active" | "live" | "ready" | "healthy" | "deployed" | "complete" | "completed") => {
            "active".to_string()
        }
        _ => "paused".to_string(),
    }
}

fn deployment_last_deploy(value: &serde_json::Value) -> String {
    string_field(
        value,
        &[
            "lastDeploy",
            "lastDeployedAt",
            "deployedAt",
            "updatedAt",
            "createdAt",
        ],
    )
    .or_else(|| nested_string_field(value, "latestDeploy", &["updatedAt", "createdAt"]))
    .and_then(|timestamp| format_deployment_age(&timestamp, Utc::now()))
    .unwrap_or_else(|| "Never".to_string())
}

fn deployment_size(value: &serde_json::Value) -> String {
    string_field(value, &["size", "storage", "artifactSize"])
        .or_else(|| {
            number_field(value, &["sizeBytes", "bytes", "artifactSizeBytes"])
                .or_else(|| nested_number_field(value, "manifest", &["size", "sizeBytes"]))
                .or_else(|| nested_number_field(value, "latestDeploy", &["size", "sizeBytes"]))
                .map(format_bytes)
        })
        .unwrap_or_else(|| "-".to_string())
}

fn deployment_version(value: &serde_json::Value) -> String {
    string_field(value, &["ver", "version", "latestVersion"])
        .or_else(|| nested_string_field(value, "latestDeploy", &["ver", "version"]))
        .or_else(|| {
            number_field(value, &["version", "latestVersion"])
                .or_else(|| nested_number_field(value, "latestDeploy", &["version"]))
                .map(|version| format!("v{version}"))
        })
        .map(|version| {
            let version = version.trim();
            if version.is_empty() {
                "-".to_string()
            } else if version.bytes().all(|b| b.is_ascii_digit()) {
                format!("v{version}")
            } else {
                version.to_string()
            }
        })
        .unwrap_or_else(|| "-".to_string())
}

fn is_board_not_provisioned(text: &str) -> bool {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(text.trim()) else {
        return false;
    };
    json_code(&value)
        .map(|code| {
            matches!(
                code,
                "board-not-provisioned" | "board_not_provisioned" | "board-missing"
            )
        })
        .unwrap_or(false)
}

fn is_activity_not_provisioned(text: &str) -> bool {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(text.trim()) else {
        return false;
    };
    json_code(&value)
        .map(|code| {
            matches!(
                code,
                "activity-not-provisioned"
                    | "activity_not_provisioned"
                    | "activity-missing"
                    | "activity_missing"
                    | "company-activity-missing"
                    | "company_activity_missing"
            )
        })
        .unwrap_or(false)
}

fn is_deployments_not_provisioned(text: &str) -> bool {
    let text = text.trim();
    if text.is_empty() {
        return true;
    }
    let Ok(value) = serde_json::from_str::<serde_json::Value>(text) else {
        return false;
    };
    json_code(&value)
        .map(|code| {
            matches!(
                code,
                "deployments-not-provisioned"
                    | "deployments_not_provisioned"
                    | "deployments-missing"
                    | "deployments_missing"
                    | "apps-not-provisioned"
                    | "apps_not_provisioned"
                    | "not-provisioned"
                    | "not_provisioned"
            )
        })
        .unwrap_or(false)
}

fn is_secrets_not_provisioned(text: &str) -> bool {
    let text = text.trim();
    if text.is_empty() {
        return true;
    }
    let Ok(value) = serde_json::from_str::<serde_json::Value>(text) else {
        return false;
    };
    json_code(&value)
        .map(|code| {
            matches!(
                code,
                "secrets-not-provisioned"
                    | "secrets_not_provisioned"
                    | "secrets-missing"
                    | "secrets_missing"
                    | "not-provisioned"
                    | "not_provisioned"
            )
        })
        .unwrap_or(false)
}

fn json_code(value: &serde_json::Value) -> Option<&str> {
    value.get("code").and_then(|v| v.as_str()).or_else(|| {
        value
            .get("error")
            .and_then(|e| e.get("code"))
            .and_then(|v| v.as_str())
    })
}

fn string_field(value: &serde_json::Value, names: &[&str]) -> Option<String> {
    names.iter().find_map(|name| {
        value.get(*name).and_then(|v| {
            v.as_str()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
        })
    })
}

fn nested_string_field(value: &serde_json::Value, key: &str, names: &[&str]) -> Option<String> {
    value
        .get(key)
        .and_then(|nested| string_field(nested, names))
}

fn bool_field(value: &serde_json::Value, names: &[&str]) -> Option<bool> {
    names.iter().find_map(|name| {
        value.get(*name).and_then(|v| {
            v.as_bool().or_else(|| {
                v.as_str()
                    .map(|s| matches!(s.trim().to_ascii_lowercase().as_str(), "true" | "1" | "yes"))
            })
        })
    })
}

fn number_field(value: &serde_json::Value, names: &[&str]) -> Option<u64> {
    names.iter().find_map(|name| {
        value.get(*name).and_then(|v| {
            v.as_u64()
                .or_else(|| v.as_i64().and_then(|n| u64::try_from(n).ok()))
                .or_else(|| v.as_str().and_then(|s| s.trim().parse::<u64>().ok()))
        })
    })
}

fn nested_number_field(value: &serde_json::Value, key: &str, names: &[&str]) -> Option<u64> {
    value
        .get(key)
        .and_then(|nested| number_field(nested, names))
}

fn normalize_deployment_host(url: &str) -> Option<String> {
    let mut host = url.trim();
    if host.is_empty() {
        return None;
    }
    host = host
        .strip_prefix("https://")
        .or_else(|| host.strip_prefix("http://"))
        .unwrap_or(host);
    let host = host
        .split('/')
        .next()
        .unwrap_or(host)
        .trim()
        .trim_end_matches('.')
        .to_ascii_lowercase();
    is_safe_deployment_host(&host).then_some(host)
}

fn subdomain_from_url(url: &str) -> Option<String> {
    let host = normalize_deployment_host(url)?;
    host.strip_suffix(&format!(".{HQ_DEPLOY_APP_DOMAIN}"))
        .map(str::to_string)
        .filter(|sub| !sub.is_empty())
}

fn is_safe_deployment_host(host: &str) -> bool {
    host.strip_suffix(&format!(".{HQ_DEPLOY_APP_DOMAIN}"))
        .is_some_and(|sub| is_safe_deployment_label(sub))
}

fn is_safe_deployment_label(label: &str) -> bool {
    !label.is_empty()
        && label.len() <= 63
        && label
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && !label.starts_with('-')
        && !label.ends_with('-')
}

fn format_deployment_age(value: &str, now: DateTime<Utc>) -> Option<String> {
    let parsed = DateTime::parse_from_rfc3339(value.trim())
        .ok()?
        .with_timezone(&Utc);
    let seconds = now.signed_duration_since(parsed).num_seconds().max(0);
    Some(if seconds < 60 {
        "just now".to_string()
    } else if seconds < 60 * 60 {
        format!("{}m ago", seconds / 60)
    } else if seconds < 60 * 60 * 24 {
        format!("{}h ago", seconds / (60 * 60))
    } else if seconds < 60 * 60 * 24 * 30 {
        format!("{}d ago", seconds / (60 * 60 * 24))
    } else {
        parsed.format("%b %-d, %Y").to_string()
    })
}

fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else if value >= 10.0 {
        format!("{value:.0} {}", UNITS[unit])
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

impl LiveBoardModel {
    fn into_company_board(self) -> CompanyBoard {
        let mut board = CompanyBoard::default();
        for project in self.projects {
            match project.status_column() {
                BoardColumn::Inbox => board.inbox.push(project.into_board_card()),
                BoardColumn::Doing => board.doing.push(project.into_board_card()),
                BoardColumn::Review => board.review.push(project.into_board_card()),
                BoardColumn::Done => board.done.push(project.into_board_card()),
            }
        }
        board
    }
}

enum BoardColumn {
    Inbox,
    Doing,
    Review,
    Done,
}

impl LiveBoardProject {
    fn status_column(&self) -> BoardColumn {
        match normalize_status(self.status.as_deref()).as_deref() {
            Some("active" | "doing" | "inprogress" | "in_progress") => BoardColumn::Doing,
            Some("review" | "inreview" | "in_review") => BoardColumn::Review,
            Some("done" | "complete" | "completed" | "shipped") => BoardColumn::Done,
            Some("inbox" | "backlog" | "todo" | "to_do") | _ => BoardColumn::Inbox,
        }
    }

    fn into_board_card(self) -> BoardCard {
        let title = self
            .title
            .clone()
            .or_else(|| self.name.clone())
            .unwrap_or_else(|| "Untitled project".to_string());
        let assignee_initials = self
            .assignee_initials
            .clone()
            .or_else(|| self.assignee.as_ref().and_then(|a| a.initials.clone()))
            .or_else(|| {
                self.assignee
                    .as_ref()
                    .and_then(|a| derive_initials(a.name.as_deref().or(a.email.as_deref())))
            });
        let tag = self
            .tag
            .clone()
            .or_else(|| self.project_type.clone())
            .or_else(|| self.source_type.clone())
            .or_else(|| self.kind.clone())
            .or_else(|| self.labels.first().cloned());
        let age = self
            .age
            .clone()
            .or_else(|| self.updated_at.as_deref().and_then(format_board_date))
            .or_else(|| self.created_at.as_deref().and_then(format_board_date))
            .or_else(|| self.updated_at.clone())
            .or_else(|| self.created_at.clone());

        BoardCard {
            id: self.uid.clone().or(self.id.clone()).unwrap_or_default(),
            title,
            subtitle: None,
            href: None,
            labels: self.labels,
            assignee_initials,
            tag,
            age,
            extra: self.extra,
        }
    }
}

fn normalize_status(status: Option<&str>) -> Option<String> {
    status.map(|s| {
        s.trim()
            .to_ascii_lowercase()
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
            .collect()
    })
}

fn derive_initials(value: Option<&str>) -> Option<String> {
    let value = value?.trim();
    if value.is_empty() {
        return None;
    }
    let initials: String = value
        .split(|c: char| c.is_whitespace() || c == '.' || c == '@' || c == '-' || c == '_')
        .filter(|part| !part.is_empty())
        .take(2)
        .filter_map(|part| part.chars().next())
        .map(|c| c.to_ascii_uppercase())
        .collect();
    (!initials.is_empty()).then_some(initials)
}

fn format_board_date(value: &str) -> Option<String> {
    let parsed = chrono::DateTime::parse_from_rfc3339(value.trim()).ok()?;
    Some(parsed.format("%b %-d, %Y").to_string())
}

/// Allows only `[a-zA-Z0-9._-]+` for a path segment without percent-encoding.
fn is_url_safe_id(s: &str) -> bool {
    !s.is_empty()
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.')
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use crate::commands::workspaces::{Workspace, WorkspaceKind, WorkspaceState};
    use crate::util::feature_gate::email_present;

    // Note: `desktop_alt_enabled` itself depends on the on-disk Cognito
    // token cache so it isn't a pure unit-test target — the canonical
    // gate logic it delegates to is covered by the unit tests in
    // `util/feature_gate.rs` (ga_gate_admits_any_present_email /
    // ga_gate_rejects_signed_out), plus the command-specific assertions
    // below that re-exercise the GA presence contract this command is bound
    // to. The window graduated from the Indigo dogfood to GA.

    /// GA: the expanded desktop window is enabled for ANY signed-in user.
    #[test]
    fn desktop_alt_gate_admits_any_signed_in_user() {
        assert!(email_present(Some("stefan@getindigo.ai")));
        assert!(email_present(Some("qa@example.com")));
        assert!(email_present(Some("anyone@gmail.com")));
        // Former dogfood look-alike — now admitted, GA only checks presence.
        assert!(email_present(Some("attacker@forgetindigo.ai")));
    }

    /// GA: only signed-out (no email / empty) is rejected.
    #[test]
    fn desktop_alt_gate_rejects_signed_out() {
        assert!(!email_present(None));
        assert!(!email_present(Some("")));
        assert!(!email_present(Some("   ")));
    }

    #[test]
    fn company_board_card_count_sums_all_columns() {
        let card = |id: &str| super::BoardCard {
            id: id.to_string(),
            title: id.to_string(),
            subtitle: None,
            href: None,
            labels: Vec::new(),
            assignee_initials: None,
            tag: None,
            age: None,
            extra: std::collections::BTreeMap::new(),
        };
        let board = super::CompanyBoard {
            inbox: vec![card("a"), card("b")],
            doing: vec![card("c")],
            review: Vec::new(),
            done: vec![card("d"), card("e"), card("f")],
        };

        assert_eq!(board.card_count(), 6);
        assert_eq!(super::CompanyBoard::default().card_count(), 0);
    }

    #[test]
    fn company_activity_last7d_reads_files7_stat() {
        let activity = super::parse_activity_response(
            reqwest::StatusCode::OK,
            r#"{"stats":{"files7":9,"edits7":40}}"#,
        )
        .expect("activity should parse");

        assert_eq!(activity.last7d(), 9);
        assert_eq!(super::CompanyActivity::default().last7d(), 0);
    }

    #[test]
    fn summary_count_propagates_auth_but_degrades_other_errors_to_zero() {
        assert_eq!(super::summary_count_or_auth(Ok(7)).unwrap(), 7);
        // Non-auth failures (404 not-provisioned, network, parse) -> 0 so a
        // single dead surface doesn't zero the rest.
        assert_eq!(
            super::summary_count_or_auth(Err("board HTTP 404: nope".to_string())).unwrap(),
            0
        );
        // Auth failures propagate so the UI can route to sign-in.
        assert_eq!(
            super::summary_count_or_auth(Err(
                "AUTH_REQUIRED: board (HTTP 401 Unauthorized)".to_string()
            ))
            .unwrap_err(),
            "AUTH_REQUIRED: board (HTTP 401 Unauthorized)"
        );
    }

    #[test]
    fn parse_project_creators_keeps_only_real_creators() {
        // The board model carries createdByName per project (from S3 created-by).
        // We surface only projects with a non-empty creator; the rest stay
        // "Unassigned" on the desktop. Keyed by both id and prdPath.
        let body = r#"{
            "companyUid": "cmp_1",
            "goals": [],
            "projects": [
                {"id":"p1","prdPath":"companies/co/projects/a/prd.json","createdByName":"maya@x.com"},
                {"id":"p2","prdPath":"companies/co/projects/b/prd.json","createdBy":"sub_2"},
                {"id":"p3","prdPath":"companies/co/projects/c/prd.json","createdByName":"  "},
                {"id":"p4","createdByName":"corey@x.com"}
            ]
        }"#;
        let rows = super::parse_project_creators(body).expect("parses");
        // p2 (no name), p3 (blank) dropped; p1 + p4 kept.
        assert_eq!(rows.len(), 2);
        let p1 = rows.iter().find(|r| r.id == "p1").unwrap();
        assert_eq!(p1.creator, "maya@x.com");
        assert_eq!(p1.prd_path.as_deref(), Some("companies/co/projects/a/prd.json"));
        let p4 = rows.iter().find(|r| r.id == "p4").unwrap();
        assert_eq!(p4.creator, "corey@x.com");
        assert!(p4.prd_path.is_none());
    }

    #[test]
    fn parse_project_creators_tolerates_empty_or_missing_projects() {
        assert!(super::parse_project_creators(r#"{"companyUid":"c","goals":[]}"#).unwrap().is_empty());
        assert!(super::parse_project_creators(r#"{"projects":[]}"#).unwrap().is_empty());
        assert!(super::parse_project_creators("not json").is_err());
    }

    #[test]
    fn parse_responses_flag_auth_failures_as_auth_required() {
        assert!(
            super::parse_board_response(reqwest::StatusCode::UNAUTHORIZED, "")
                .unwrap_err()
                .starts_with("AUTH_REQUIRED: board")
        );
        assert!(
            super::parse_board_response(reqwest::StatusCode::FORBIDDEN, "")
                .unwrap_err()
                .starts_with("AUTH_REQUIRED: board")
        );
        assert!(
            super::parse_activity_response(reqwest::StatusCode::UNAUTHORIZED, "")
                .unwrap_err()
                .starts_with("AUTH_REQUIRED: activity")
        );
        assert!(
            super::parse_deployments_response(reqwest::StatusCode::FORBIDDEN, "", "test-org")
                .unwrap_err()
                .starts_with("AUTH_REQUIRED: deployments")
        );
        assert!(
            super::parse_secrets_response(reqwest::StatusCode::UNAUTHORIZED, "")
                .unwrap_err()
                .starts_with("AUTH_REQUIRED: secrets")
        );
    }

    #[tokio::test]
    async fn company_summary_rejects_empty_slug() {
        assert_eq!(
            super::get_company_summary("".to_string())
                .await
                .unwrap_err(),
            "company slug is required"
        );
        assert_eq!(
            super::get_company_summary("   ".to_string())
                .await
                .unwrap_err(),
            "company slug is required"
        );
    }

    #[tokio::test]
    async fn company_board_rejects_empty_slug_before_network() {
        assert_eq!(
            super::get_company_board("   ".to_string())
                .await
                .unwrap_err(),
            "company slug is required"
        );
    }

    #[tokio::test]
    async fn company_activity_rejects_empty_slug_before_network() {
        assert_eq!(
            super::get_company_activity("   ".to_string())
                .await
                .unwrap_err(),
            "company slug is required"
        );
    }

    #[tokio::test]
    async fn company_deployments_rejects_empty_slug_before_network() {
        assert_eq!(
            super::get_company_deployments("   ".to_string())
                .await
                .unwrap_err(),
            "company slug is required"
        );
    }

    #[tokio::test]
    async fn company_secrets_rejects_empty_slug_before_network() {
        assert_eq!(
            super::get_company_secrets("   ".to_string())
                .await
                .unwrap_err(),
            "company slug is required"
        );
    }

    #[test]
    fn company_board_deserializes_missing_columns_as_empty_arrays() {
        let board: super::CompanyBoard = serde_json::from_str(
            r#"{
                "inbox": [{"id": "card-1", "title": "One", "customField": 42}]
            }"#,
        )
        .expect("missing columns should default");

        assert_eq!(board.inbox.len(), 1);
        assert_eq!(board.inbox[0].id, "card-1");
        assert_eq!(board.inbox[0].title, "One");
        assert_eq!(board.inbox[0].extra["customField"], 42);
        assert!(board.doing.is_empty());
        assert!(board.review.is_empty());
        assert!(board.done.is_empty());
    }

    #[test]
    fn company_board_deserializes_empty_object_as_empty_board() {
        let board: super::CompanyBoard = serde_json::from_str("{}").unwrap();

        assert!(board.inbox.is_empty());
        assert!(board.doing.is_empty());
        assert!(board.review.is_empty());
        assert!(board.done.is_empty());
    }

    #[test]
    fn company_board_treats_missing_or_empty_response_as_empty_board() {
        let not_provisioned = super::parse_board_response(
            reqwest::StatusCode::NOT_FOUND,
            r#"{"code":"board-not-provisioned"}"#,
        )
        .expect("missing board.json should be an empty board");
        assert_eq!(not_provisioned, super::CompanyBoard::default());

        let no_content = super::parse_board_response(reqwest::StatusCode::NO_CONTENT, "")
            .expect("204 should be an empty board");
        assert_eq!(no_content, super::CompanyBoard::default());

        let empty_body = super::parse_board_response(reqwest::StatusCode::OK, " \n ")
            .expect("empty board.json should be an empty board");
        assert_eq!(empty_body, super::CompanyBoard::default());
    }

    #[test]
    fn company_board_rejects_generic_route_not_found() {
        let err = super::parse_board_response(
            reqwest::StatusCode::NOT_FOUND,
            r#"{"code":"not-found","message":"route not found"}"#,
        )
        .unwrap_err();

        assert!(err.contains("board HTTP 404"));
    }

    #[test]
    fn company_activity_deserializes_empty_object_as_empty_activity() {
        let activity: super::CompanyActivity = serde_json::from_str("{}").unwrap();

        assert_eq!(activity, super::CompanyActivity::default());
    }

    #[test]
    fn company_activity_deserializes_missing_arrays_and_stats_as_defaults() {
        let activity: super::CompanyActivity = serde_json::from_str(
            r#"{
                "stats": {"files7": 3},
                "recent": [{"who": "Ada", "extraField": "kept"}]
            }"#,
        )
        .expect("missing activity fields should default");

        assert_eq!(activity.stats.files7, 3);
        assert_eq!(activity.stats.edits7, 0);
        assert_eq!(activity.stats.members, 0);
        assert_eq!(activity.stats.vault_size, "");
        assert!(activity.sparkline.is_empty());
        assert_eq!(activity.recent.len(), 1);
        assert_eq!(activity.recent[0].who, "Ada");
        assert_eq!(activity.recent[0].what, "");
        assert_eq!(activity.recent[0].extra["extraField"], "kept");
        assert!(activity.top.is_empty());
    }

    #[test]
    fn company_activity_treats_missing_or_empty_response_as_empty_activity() {
        let not_provisioned = super::parse_activity_response(
            reqwest::StatusCode::NOT_FOUND,
            r#"{"code":"activity-not-provisioned"}"#,
        )
        .expect("missing activity should be empty activity");
        assert_eq!(not_provisioned, super::CompanyActivity::default());

        let nested_code = super::parse_activity_response(
            reqwest::StatusCode::NOT_FOUND,
            r#"{"error":{"code":"activity_missing"}}"#,
        )
        .expect("nested missing code should be empty activity");
        assert_eq!(nested_code, super::CompanyActivity::default());

        let no_content = super::parse_activity_response(reqwest::StatusCode::NO_CONTENT, "")
            .expect("204 should be empty activity");
        assert_eq!(no_content, super::CompanyActivity::default());

        let empty_body = super::parse_activity_response(reqwest::StatusCode::OK, " \n ")
            .expect("empty activity response should be empty activity");
        assert_eq!(empty_body, super::CompanyActivity::default());
    }

    #[test]
    fn company_activity_rejects_generic_route_not_found() {
        let err = super::parse_activity_response(
            reqwest::StatusCode::NOT_FOUND,
            r#"{"code":"not-found","message":"route not found"}"#,
        )
        .unwrap_err();

        assert!(err.contains("activity HTTP 404"));
    }

    #[test]
    fn company_activity_parses_live_camel_case_response() {
        let activity = super::parse_activity_response(
            reqwest::StatusCode::OK,
            r#"{
                "stats": {
                    "files7": 12,
                    "edits7": 34,
                    "members": 5,
                    "vaultSize": "1.2 MB"
                },
                "sparkline": [0, 2, 4, 3],
                "recent": [
                    {
                        "who": "Ada Lovelace",
                        "what": "edited",
                        "file": "plans/spec.md",
                        "when": "2026-05-27T12:00:00Z",
                        "source": "hq-sync"
                    }
                ],
                "top": [
                    {"who": "Ada Lovelace", "edits": 20},
                    {"who": "Grace Hopper", "edits": 14}
                ]
            }"#,
        )
        .expect("live activity should parse");

        assert_eq!(activity.stats.files7, 12);
        assert_eq!(activity.stats.edits7, 34);
        assert_eq!(activity.stats.members, 5);
        assert_eq!(activity.stats.vault_size, "1.2 MB");
        assert_eq!(activity.sparkline, vec![0, 2, 4, 3]);
        assert_eq!(activity.recent[0].who, "Ada Lovelace");
        assert_eq!(activity.recent[0].what, "edited");
        assert_eq!(activity.recent[0].file, "plans/spec.md");
        assert_eq!(activity.recent[0].when, "2026-05-27T12:00:00Z");
        assert_eq!(activity.recent[0].extra["source"], "hq-sync");
        assert_eq!(activity.top[0].edits, 20);
        assert_eq!(activity.top[1].who, "Grace Hopper");
    }

    #[test]
    fn company_deployments_parse_hq_deploy_apps_me_shape() {
        let deployments = super::parse_deployments_response(
            reqwest::StatusCode::OK,
            r#"{
                "apps": [
                    {
                        "id": "app-1",
                        "subdomain": "console",
                        "status": "active",
                        "dnsStatus": "active",
                        "active": true,
                        "passwordProtected": true,
                        "createdAt": "2026-05-27T12:00:00Z",
                        "url": "https://console.indigo-hq.com"
                    },
                    {
                        "id": "app-2",
                        "subdomain": "preview",
                        "status": "deploying",
                        "active": true,
                        "createdAt": "2026-05-27T11:00:00Z",
                        "url": "https://preview.indigo-hq.com/path"
                    },
                    {
                        "id": "app-3",
                        "subdomain": "paused-app",
                        "status": "active",
                        "active": false,
                        "passwordProtected": false,
                        "createdAt": "2026-05-27T10:00:00Z"
                    }
                ]
            }"#,
            "test-org",
        )
        .expect("apps/me response should parse");

        assert_eq!(deployments.len(), 3);
        assert_eq!(deployments[0].sub, "console");
        assert_eq!(deployments[0].url, "console.indigo-hq.com");
        assert_eq!(deployments[0].state, "active");
        assert_eq!(deployments[0].size, "-");
        assert_eq!(deployments[0].ver, "-");
        assert!(deployments[0].pwd);
        assert_eq!(deployments[1].url, "preview.indigo-hq.com");
        assert_eq!(deployments[1].state, "deploying");
        assert_eq!(deployments[2].url, "paused-app.indigo-hq.com");
        assert_eq!(deployments[2].state, "paused");
    }

    #[test]
    fn company_deployments_parse_optional_detail_fields() {
        let deployments = super::parse_deployments_response(
            reqwest::StatusCode::OK,
            r#"[
                {
                    "url": "https://portal.indigo-hq.com/",
                    "latestDeploy": {
                        "status": "live",
                        "version": 7,
                        "sizeBytes": 1536,
                        "updatedAt": "2020-01-02T12:00:00Z"
                    },
                    "pwd": false
                }
            ]"#,
            "test-org",
        )
        .expect("array response should parse");

        assert_eq!(
            deployments,
            vec![super::DeploymentEntry {
                sub: "portal".to_string(),
                url: "portal.indigo-hq.com".to_string(),
                state: "active".to_string(),
                last_deploy: "Jan 2, 2020".to_string(),
                size: "1.5 KB".to_string(),
                ver: "v7".to_string(),
                pwd: false,
            }]
        );
    }

    #[test]
    fn company_deployments_treats_empty_and_not_provisioned_as_empty() {
        assert_eq!(
            super::parse_deployments_response(reqwest::StatusCode::NO_CONTENT, "", "test-org")
                .unwrap(),
            Vec::<super::DeploymentEntry>::new()
        );
        assert_eq!(
            super::parse_deployments_response(reqwest::StatusCode::OK, " \n ", "test-org").unwrap(),
            Vec::<super::DeploymentEntry>::new()
        );
        assert_eq!(
            super::parse_deployments_response(
                reqwest::StatusCode::NOT_FOUND,
                r#"{"code":"deployments-not-provisioned"}"#,
                "test-org",
            )
            .unwrap(),
            Vec::<super::DeploymentEntry>::new()
        );
    }

    #[test]
    fn company_secrets_project_metadata_only_from_body_secrets() {
        let envs = super::parse_secrets_response(
            reqwest::StatusCode::OK,
            r#"{
                "body": {
                    "secrets": [
                        {
                            "key": "prod/DATABASE_URL",
                            "lastModifiedDate": "2026-05-27T12:00:00Z",
                            "rotationSchedule": "30d",
                            "value": "plaintext-ignored"
                        },
                        {
                            "secretPath": "prod/STRIPE_KEY",
                            "updatedAt": "2026-05-26T12:00:00Z",
                            "rotationEnabled": false,
                            "payload": {"value": "ignored"}
                        },
                        {
                            "name": "API_TOKEN",
                            "environment": "dev",
                            "rot": "manual",
                            "secret": "ignored"
                        }
                    ]
                }
            }"#,
        )
        .expect("metadata response should parse");

        assert_eq!(envs.len(), 2);
        assert_eq!(envs[0].env, "dev");
        assert_eq!(envs[0].count, 1);
        assert_eq!(envs[0].items[0].key, "API_TOKEN");
        assert_eq!(envs[0].items[0].upd, "-");
        assert_eq!(envs[0].items[0].rot, "manual");
        assert_eq!(envs[1].env, "prod");
        assert_eq!(envs[1].count, 2);
        assert_eq!(envs[1].items[0].key, "DATABASE_URL");
        assert_eq!(envs[1].items[0].upd, "2026-05-27T12:00:00Z");
        assert_eq!(envs[1].items[0].rot, "30d");
        assert_eq!(envs[1].items[1].key, "STRIPE_KEY");
        assert_eq!(envs[1].items[1].rot, "manual");

        let serialized = serde_json::to_value(&envs).unwrap();
        let serialized_text = serialized.to_string();
        assert!(!serialized_text.contains("plaintext-ignored"));
        assert!(!serialized_text.contains("\"value\""));
        assert!(!serialized_text.contains("\"secret\""));
        assert!(serialized.get(0).unwrap().get("items").is_some());
        assert!(serialized.get(0).unwrap().get("value").is_none());
    }

    /// Contract test against the VERBATIM hq-pro vault response shape
    /// (`src/vault-service/handlers/secrets.ts` → `handleList`):
    /// `{"secrets":[{name,type,lastModifiedDate,version,permission}],"companyUid"}`.
    /// Proves the parser yields a NON-EMPTY result for this exact shape — so a
    /// company that has secrets provisioned can never render 0 because of a
    /// parse mismatch. (If the panel ever shows 0, the cause is upstream: an
    /// empty SSM path, an auth/error body, or a different response — which the
    /// committed `[desktop-alt] secrets structure:` diagnostic will reveal.)
    /// SSM names with no `/` group under "default"; `ENV/KEY` names split.
    #[test]
    fn company_secrets_parses_verbatim_vault_handlelist_shape() {
        let envs = super::parse_secrets_response(
            reqwest::StatusCode::OK,
            r#"{
                "secrets": [
                    {"name": "DATABASE_URL", "type": "SecureString", "lastModifiedDate": "2026-05-27T12:00:00Z", "version": 4, "permission": "admin"},
                    {"name": "STRIPE_KEY", "type": "SecureString", "lastModifiedDate": "2026-05-26T09:00:00Z", "version": 1, "permission": "admin"},
                    {"name": "DEV/API_TOKEN", "type": "String", "lastModifiedDate": "2026-05-25T09:00:00Z", "version": 2, "permission": "admin"}
                ],
                "companyUid": "cmp_01HX"
            }"#,
        )
        .expect("verbatim vault handleList shape should parse");

        // Two env groups: "DEV" (from DEV/API_TOKEN) and "default" (the two
        // prefix-less names). A provisioned company is never 0.
        assert!(!envs.is_empty(), "provisioned secrets must not parse to 0");
        assert_eq!(envs.len(), 2);

        let dev = envs.iter().find(|e| e.env == "DEV").expect("DEV env group");
        assert_eq!(dev.count, 1);
        assert_eq!(dev.items[0].key, "API_TOKEN");
        assert_eq!(dev.items[0].upd, "2026-05-25T09:00:00Z");

        let default = envs
            .iter()
            .find(|e| e.env == "default")
            .expect("default env group");
        assert_eq!(default.count, 2);
        let keys: Vec<&str> = default.items.iter().map(|i| i.key.as_str()).collect();
        assert!(keys.contains(&"DATABASE_URL"));
        assert!(keys.contains(&"STRIPE_KEY"));

        // The `permission`/`type` metadata never carries a value, but assert
        // the serialized form stays values-free regardless.
        let serialized = serde_json::to_value(&envs).unwrap().to_string();
        assert!(!serialized.contains("\"value\""));
    }

    /// The structure diagnostic must describe shape (top-level kind, key
    /// names, array lengths, first-row key names) and NEVER leak a value.
    #[test]
    fn secret_structure_summary_is_values_free() {
        let object_shape = serde_json::json!({
            "companyUid": "cmp_01HX",
            "secrets": [
                {
                    "name": "prod/DATABASE_URL",
                    "type": "SecureString",
                    "lastModifiedDate": "2026-05-27T12:00:00Z",
                    "version": 4,
                    "permission": "admin",
                    "value": "super-secret-plaintext"
                }
            ]
        });
        let summary = super::secret_structure_summary(&object_shape);
        // Shape facts ARE present.
        assert!(summary.contains("top-level object"));
        assert!(summary.contains("companyUid"));
        assert!(summary.contains("secrets[]=1"));
        // Row key NAMES are present...
        assert!(summary.contains("name"));
        assert!(summary.contains("version"));
        // ...but no value strings ever are.
        assert!(!summary.contains("super-secret-plaintext"));
        assert!(!summary.contains("SecureString"));
        assert!(!summary.contains("prod/DATABASE_URL"));

        let array_shape = serde_json::json!([
            { "name": "the-secret-name", "value": "leak-me" }
        ]);
        let summary = super::secret_structure_summary(&array_shape);
        assert!(summary.contains("top-level array (len=1)"));
        // Field NAMES appear...
        assert!(summary.contains("name"));
        assert!(summary.contains("value"));
        // ...but neither the secret value nor the secret's name VALUE leaks.
        assert!(!summary.contains("leak-me"));
        assert!(!summary.contains("the-secret-name"));
    }

    #[test]
    fn company_secrets_treats_empty_and_not_provisioned_as_empty() {
        assert_eq!(
            super::parse_secrets_response(reqwest::StatusCode::NO_CONTENT, "").unwrap(),
            Vec::<super::SecretEnv>::new()
        );
        assert_eq!(
            super::parse_secrets_response(reqwest::StatusCode::OK, " \n ").unwrap(),
            Vec::<super::SecretEnv>::new()
        );
        assert_eq!(
            super::parse_secrets_response(
                reqwest::StatusCode::NOT_FOUND,
                r#"{"code":"secrets-not-provisioned"}"#,
            )
            .unwrap(),
            Vec::<super::SecretEnv>::new()
        );
    }

    #[test]
    fn company_secrets_rejects_generic_route_not_found() {
        let err = super::parse_secrets_response(
            reqwest::StatusCode::NOT_FOUND,
            r#"{"code":"not-found","message":"route not found"}"#,
        )
        .unwrap_err();

        assert!(err.contains("secrets HTTP 404"));
    }

    /// Regression for the "0 deployments despite HTTP 200 with data" bug.
    /// The real `GET /api/apps/me` rows do NOT carry an org-slug field (the
    /// server already scopes by the `x-org-slug` header), so the client-side
    /// slug filter must NOT drop them. Uses the exact production row shape.
    #[test]
    fn company_deployments_keeps_apps_me_rows_without_org_slug() {
        let deployments = super::parse_deployments_response(
            reqwest::StatusCode::OK,
            r#"{
                "apps": [
                    {
                        "id": "app_01HX",
                        "name": "nat-audit-indigo-api-3",
                        "subdomain": "nat-audit-indigo-api-3",
                        "type": "static",
                        "status": "active",
                        "dnsStatus": "failed",
                        "active": true,
                        "passwordProtected": false,
                        "ownerId": "user_01HX",
                        "createdAt": "2026-05-27T12:00:00Z",
                        "url": "https://nat-audit-indigo-api-3.indigo-hq.com"
                    }
                ]
            }"#,
            "indigo",
        )
        .expect("apps/me without orgSlug should parse");

        // The single row has no orgSlug field, so the server-trusted filter
        // must keep it — a count of 1, not 0.
        assert_eq!(deployments.len(), 1);
        assert_eq!(deployments[0].sub, "nat-audit-indigo-api-3");
        assert_eq!(deployments[0].state, "active");
    }

    /// The Deployments panel is a *company* dashboard, so it must hit the
    /// ORG-scoped `GET /api/apps` — never the personal `GET /api/apps/me`,
    /// which filters server-side to `ownerId === userId` and returned an
    /// empty `{"apps":[]}` (→ panel rendered 0) for any member viewing apps
    /// a co-collaborator created. Pin the path so it can't regress to `/me`.
    #[test]
    fn deployments_url_is_org_scoped_not_personal() {
        let url = super::deployments_url("https://api.indigo-hq.com");
        assert_eq!(url, "https://api.indigo-hq.com/api/apps");
        assert!(
            !url.ends_with("/me"),
            "deployments must use org-scoped /api/apps, not personal /api/apps/me: {url}"
        );
    }

    #[test]
    fn company_deployments_filters_rows_with_org_slug_when_present() {
        let deployments = super::parse_deployments_response(
            reqwest::StatusCode::OK,
            r#"{
                "apps": [
                    {"subdomain": "mine", "orgSlug": "selected-company"},
                    {"subdomain": "snake", "org_slug": "selected-company"},
                    {"subdomain": "nested", "org": {"slug": "selected-company"}},
                    {"subdomain": "legacy-without-org"},
                    {"subdomain": "theirs", "orgSlug": "other-company"},
                    {"subdomain": "other-nested", "org": {"slug": "other-company"}}
                ]
            }"#,
            "selected-company",
        )
        .expect("org-filtered response should parse");

        let subs: Vec<&str> = deployments
            .iter()
            .map(|deployment| deployment.sub.as_str())
            .collect();
        assert_eq!(subs, vec!["mine", "snake", "nested", "legacy-without-org"]);
    }

    #[test]
    fn deployment_helpers_normalize_state_url_age_and_size() {
        let now = chrono::Utc.with_ymd_and_hms(2026, 5, 27, 12, 0, 0).unwrap();

        assert_eq!(
            super::normalize_deployment_host("https://console.indigo-hq.com/path"),
            Some("console.indigo-hq.com".to_string())
        );
        assert_eq!(
            super::subdomain_from_url("https://console.indigo-hq.com/path"),
            Some("console".to_string())
        );
        assert_eq!(
            super::format_deployment_age("2026-05-27T11:57:00Z", now).as_deref(),
            Some("3m ago")
        );
        assert_eq!(
            super::format_deployment_age("2026-05-25T12:00:00Z", now).as_deref(),
            Some("2d ago")
        );
        assert_eq!(super::format_bytes(25 * 1024 * 1024), "25 MB");
    }

    #[test]
    fn deployment_helpers_reject_unsafe_hosts_before_shell_open() {
        // The host guard itself still rejects unsafe hosts outright.
        assert_eq!(
            super::normalize_deployment_host("https://evil.example.com"),
            None
        );
        assert_eq!(
            super::normalize_deployment_host("https://console.indigo-hq.com.evil.test"),
            None
        );
        assert_eq!(
            super::normalize_deployment_host("https://bad_sub.indigo-hq.com"),
            None
        );

        // Contract: an unsafe row is EXCLUDED from the parsed list — its URL can
        // never reach the UI to be shell-opened — but it does NOT fail the whole
        // batch. One malformed/hostile app must not blank every valid deployment.
        // (Regression: org-scoped `/api/apps` returns the whole org, so a single
        // odd row previously errored the collect and zeroed the entire panel.)
        let only_unsafe_url = super::parse_deployments_response(
            reqwest::StatusCode::OK,
            r#"[{"subdomain":"console","url":"https://evil.example.com"}]"#,
            "test-org",
        )
        .expect("an unsafe row is skipped, not turned into a batch error");
        assert!(
            only_unsafe_url.is_empty(),
            "the unsafe row must be excluded from results"
        );

        let only_unsafe_sub = super::parse_deployments_response(
            reqwest::StatusCode::OK,
            r#"[{"subdomain":"../console"}]"#,
            "test-org",
        )
        .expect("an unsafe subdomain row is skipped, not a batch error");
        assert!(only_unsafe_sub.is_empty());

        // The key fix: a mix of one hostile row and valid rows keeps the valid
        // ones and drops only the hostile one — and the unsafe host never appears
        // in any parsed entry (so it can never be shell-opened).
        let mixed = super::parse_deployments_response(
            reqwest::StatusCode::OK,
            r#"[
                {"subdomain":"good-app","url":"https://good-app.indigo-hq.com"},
                {"subdomain":"console","url":"https://evil.example.com"},
                {"subdomain":"another","url":"https://another.indigo-hq.com"}
            ]"#,
            "test-org",
        )
        .expect("valid rows survive alongside a skipped hostile row");
        assert_eq!(
            mixed.len(),
            2,
            "both safe rows are kept; only the hostile one drops"
        );
        let subs: Vec<&str> = mixed.iter().map(|d| d.sub.as_str()).collect();
        assert!(subs.contains(&"good-app"));
        assert!(subs.contains(&"another"));
        let serialized = serde_json::to_string(&mixed).unwrap();
        assert!(
            !serialized.contains("evil.example.com"),
            "the hostile host must never make it into a parsed entry"
        );
    }

    fn company_workspace(
        slug: &str,
        state: WorkspaceState,
        cloud_uid: Option<&str>,
        broken_reason: Option<&str>,
    ) -> Workspace {
        Workspace {
            slug: slug.to_string(),
            display_name: slug.to_string(),
            kind: WorkspaceKind::Company,
            state,
            cloud_uid: cloud_uid.map(str::to_string),
            bucket_name: None,
            has_local_folder: true,
            local_path: None,
            membership_status: None,
            role: None,
            last_synced_at: None,
            broken_reason: broken_reason.map(str::to_string),
            invited_by: None,
            invited_at: None,
        }
    }

    #[test]
    fn company_uid_resolution_allows_synced_and_cloud_only_with_cloud_identity() {
        assert_eq!(
            super::resolve_company_uid_from_workspaces(
                vec![company_workspace(
                    "acme",
                    WorkspaceState::Synced,
                    Some("cmp_synced"),
                    None
                )],
                "acme",
            )
            .unwrap(),
            "cmp_synced"
        );
        assert_eq!(
            super::resolve_company_uid_from_workspaces(
                vec![company_workspace(
                    "orbit",
                    WorkspaceState::CloudOnly,
                    Some("cmp_cloud"),
                    None
                )],
                "orbit",
            )
            .unwrap(),
            "cmp_cloud"
        );
    }

    #[test]
    fn company_uid_resolution_allows_broken_uid_mismatch_via_live_cloud_uid() {
        assert_eq!(
            super::resolve_company_uid_from_workspaces(
                vec![company_workspace(
                    "acme",
                    WorkspaceState::Broken,
                    Some("cmp_OLD"),
                    Some(
                        "manifest cloud_uid cmp_OLD does not match cloud entity cmp_NEW for this slug",
                    ),
                )],
                "acme",
            )
            .unwrap(),
            "cmp_NEW"
        );
    }

    #[test]
    fn company_uid_resolution_rejects_broken_without_live_cloud_membership() {
        let broken_err = super::resolve_company_uid_from_workspaces(
            vec![company_workspace(
                "acme",
                WorkspaceState::Broken,
                Some("cmp_old"),
                Some("manifest cloud_uid cmp_old not found in your cloud memberships"),
            )],
            "acme",
        )
        .unwrap_err();
        assert!(broken_err.contains("company 'acme' is not synced"));
        assert!(broken_err.contains("manifest cloud_uid cmp_old not found"));

        assert_eq!(
            super::resolve_company_uid_from_workspaces(
                vec![company_workspace(
                    "local",
                    WorkspaceState::LocalOnly,
                    None,
                    None
                )],
                "local",
            )
            .unwrap_err(),
            "company 'local' is not synced (state: LocalOnly)"
        );
        assert_eq!(
            super::resolve_company_uid_from_workspaces(
                vec![company_workspace(
                    "personal",
                    WorkspaceState::Personal,
                    Some("person_123"),
                    None
                )],
                "personal",
            )
            .unwrap_err(),
            "company 'personal' is not synced (state: Personal)"
        );
        assert_eq!(
            super::resolve_company_uid_from_workspaces(
                vec![company_workspace(
                    "cloud",
                    WorkspaceState::CloudOnly,
                    None,
                    None
                )],
                "cloud",
            )
            .unwrap_err(),
            "company 'cloud' is not connected to cloud"
        );
    }

    #[test]
    fn company_board_maps_live_projects_into_columns() {
        let board = super::parse_board_response(
            reqwest::StatusCode::OK,
            r#"{
                "companyUid": "cmp_01ABC",
                "goals": [],
                "projects": [
                    {
                        "uid": "p1",
                        "name": "Triage intake",
                        "status": "backlog",
                        "assignee": {"name": "Ada Lovelace"},
                        "labels": ["Ops"],
                        "createdAt": "2026-05-20T12:00:00Z"
                    },
                    {
                        "id": "p2",
                        "title": "Ship sync UX",
                        "status": "in_progress",
                        "assigneeInitials": "SJ",
                        "type": "Feature",
                        "updatedAt": "2026-05-21T12:00:00Z"
                    },
                    {"id": "p3", "title": "Review polish", "status": "review"},
                    {"id": "p4", "title": "Launch", "status": "shipped"}
                ]
            }"#,
        )
        .expect("live board should map to UI columns");

        assert_eq!(board.inbox.len(), 1);
        assert_eq!(board.inbox[0].id, "p1");
        assert_eq!(board.inbox[0].title, "Triage intake");
        assert_eq!(board.inbox[0].assignee_initials.as_deref(), Some("AL"));
        assert_eq!(board.inbox[0].tag.as_deref(), Some("Ops"));
        assert_eq!(board.inbox[0].age.as_deref(), Some("May 20, 2026"));

        assert_eq!(board.doing.len(), 1);
        assert_eq!(board.doing[0].id, "p2");
        assert_eq!(board.doing[0].title, "Ship sync UX");
        assert_eq!(board.doing[0].assignee_initials.as_deref(), Some("SJ"));
        assert_eq!(board.doing[0].tag.as_deref(), Some("Feature"));
        assert_eq!(board.doing[0].age.as_deref(), Some("May 21, 2026"));

        assert_eq!(board.review[0].id, "p3");
        assert_eq!(board.done[0].id, "p4");
    }

    #[test]
    fn board_helpers_validate_slug_and_build_url() {
        assert_eq!(super::normalize_slug(" acme ").unwrap(), "acme");
        assert_eq!(
            super::normalize_slug("   ").unwrap_err(),
            "company slug is required"
        );
        assert_eq!(
            super::board_url("https://hqapi.getindigo.ai/", "cmp_01ABC-def.2").unwrap(),
            "https://hqapi.getindigo.ai/companies/cmp_01ABC-def.2/board"
        );
        assert_eq!(
            super::board_url("https://hqapi.getindigo.ai", "cmp/bad").unwrap_err(),
            "company uid has invalid characters: \"cmp/bad\""
        );
        assert_eq!(
            super::activity_url("https://hqapi.getindigo.ai/", "cmp_01ABC-def.2").unwrap(),
            "https://hqapi.getindigo.ai/companies/cmp_01ABC-def.2/activity"
        );
        assert_eq!(
            super::activity_url("https://hqapi.getindigo.ai", "cmp/bad").unwrap_err(),
            "company uid has invalid characters: \"cmp/bad\""
        );
        assert_eq!(
            super::secrets_url("https://hqapi.getindigo.ai/", "cmp_01ABC-def.2").unwrap(),
            "https://hqapi.getindigo.ai/secrets/cmp_01ABC-def.2"
        );
        assert_eq!(
            super::secrets_url("https://hqapi.getindigo.ai", "cmp/bad").unwrap_err(),
            "company uid has invalid characters: \"cmp/bad\""
        );
    }
}
