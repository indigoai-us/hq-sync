//! Feature gate for the alt desktop UX surface.
//!
//! Indigo-only gate for the alternate popover/desktop UX in development.
//! Delegates entirely to `feature_gate::is_indigo_user()` — there is no
//! parallel cache (PRD US-001 hard rule: reuse the existing OnceLock cache).
//!
//! On cold start (cache uninitialised) the underlying `is_indigo_user()`
//! call awaits `compute_gate()` and returns the canonical email-derived
//! answer instead of falling back to false. This matters because the
//! popover mounts and invokes the gate before any cloud round-trip has
//! had a chance to seed an unrelated cache — we owe the caller the real
//! answer, not a default.
//!
//! See `src-tauri/src/commands/meetings.rs::meetings_feature_enabled` for
//! the reference pattern this command mirrors.
//!
//! Result type is `Result<bool, String>` to match the established gate
//! command shape, but `is_indigo_user()` itself never errors — the Ok arm
//! is always taken.
use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

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
    Ok(crate::util::feature_gate::is_indigo_user().await)
}

#[tauri::command]
pub async fn get_company_summary(slug: String) -> Result<CompanySummary, String> {
    if slug.trim().is_empty() {
        return Err("company slug is required".to_string());
    }

    Ok(CompanySummary {
        board: 0,
        activity: CompanyActivitySummary { last7d: 0 },
        deployments: 0,
        secrets: 0,
    })
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
        .get(url)
        .header("authorization", format!("Bearer {token}"))
        .send()
        .await
        .map_err(|e| format!("board fetch: {e}"))?;
    let status = res.status();
    let text = res.text().await.map_err(|e| format!("board read: {e}"))?;

    parse_board_response(status, &text)
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
        .get(url)
        .header("authorization", format!("Bearer {token}"))
        .send()
        .await
        .map_err(|e| format!("activity fetch: {e}"))?;
    let status = res.status();
    let text = res
        .text()
        .await
        .map_err(|e| format!("activity read: {e}"))?;

    parse_activity_response(status, &text)
}

#[tauri::command]
pub async fn get_company_deployments(slug: String) -> Result<Vec<DeploymentEntry>, String> {
    let _slug = normalize_slug(&slug)?;
    let url = deployments_url(HQ_DEPLOY_API_BASE);
    let token = cognito::get_valid_access_token()
        .await
        .map_err(|e| format!("auth: {e}"))?;

    let res = build_client()
        .get(url)
        .header("authorization", format!("Bearer {token}"))
        .send()
        .await
        .map_err(|e| format!("deployments fetch: {e}"))?;
    let status = res.status();
    let text = res
        .text()
        .await
        .map_err(|e| format!("deployments read: {e}"))?;

    parse_deployments_response(status, &text)
}

/// Open or focus the Indigo-only alternate desktop UX window.
///
/// The window is declared in `tauri.conf.json` as hidden, so normal app
/// startup does not surface it. This command is still defensive and can
/// rebuild the window if it was closed earlier in the session.
#[tauri::command]
pub async fn open_desktop_alt_window(app: AppHandle) -> Result<(), String> {
    if !desktop_alt_enabled().await? {
        return Err("desktop-alt is Indigo-only".to_string());
    }

    if let Some(window) = app.get_webview_window(WINDOW_LABEL) {
        window.show().map_err(|e| e.to_string())?;
        window.set_focus().map_err(|e| e.to_string())?;
        return Ok(());
    }

    tauri::WebviewWindowBuilder::new(
        &app,
        WINDOW_LABEL,
        tauri::WebviewUrl::App("desktop-alt.html".into()),
    )
    .title("HQ")
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

fn deployments_url(base: &str) -> String {
    format!("{}/api/apps/me", base.trim_end_matches('/'))
}

fn parse_board_response(status: StatusCode, text: &str) -> Result<CompanyBoard, String> {
    if status == StatusCode::NO_CONTENT {
        return Ok(CompanyBoard::default());
    }
    if status == StatusCode::NOT_FOUND {
        return if is_board_not_provisioned(text) {
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
        return Ok(CompanyBoard::default());
    }

    parse_company_board(text)
}

fn parse_activity_response(status: StatusCode, text: &str) -> Result<CompanyActivity, String> {
    if status == StatusCode::NO_CONTENT {
        return Ok(CompanyActivity::default());
    }
    if status == StatusCode::NOT_FOUND {
        return if is_activity_not_provisioned(text) {
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

fn parse_deployments_response(
    status: StatusCode,
    text: &str,
) -> Result<Vec<DeploymentEntry>, String> {
    if status == StatusCode::NO_CONTENT {
        return Ok(Vec::new());
    }
    if status == StatusCode::NOT_FOUND {
        return if is_deployments_not_provisioned(text) {
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
        return Ok(Vec::new());
    }

    parse_deployment_entries(text)
}

fn parse_deployment_entries(text: &str) -> Result<Vec<DeploymentEntry>, String> {
    let value: serde_json::Value =
        serde_json::from_str(text).map_err(|e| format!("deployments parse: {e}"))?;
    let rows = deployment_rows(&value)
        .ok_or_else(|| "deployments parse: missing apps array".to_string())?;
    rows.iter().map(deployment_entry_from_value).collect()
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
        .ok_or_else(|| "deployments parse: app missing subdomain".to_string())?;
    let url = string_field(value, &["url"])
        .and_then(|url| normalize_deployment_host(&url))
        .unwrap_or_else(|| format!("{sub}.{HQ_DEPLOY_APP_DOMAIN}"));

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
    let host = host.split('/').next().unwrap_or(host).trim();
    (!host.is_empty()).then(|| host.to_string())
}

fn subdomain_from_url(url: &str) -> Option<String> {
    let host = normalize_deployment_host(url)?;
    host.strip_suffix(&format!(".{HQ_DEPLOY_APP_DOMAIN}"))
        .map(str::to_string)
        .or_else(|| host.split('.').next().map(str::to_string))
        .filter(|sub| !sub.is_empty())
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
    use crate::util::feature_gate::is_allowed_email;

    // Note: `desktop_alt_enabled` itself depends on the on-disk Cognito
    // token cache so it isn't a pure unit-test target — the canonical
    // gate logic it delegates to is covered by the unit tests in
    // `util/feature_gate.rs` (test_positive_cases / test_negative_cases),
    // plus the command-specific assertions below that re-exercise the
    // allowlist contract this command is bound to.

    /// US-001 AC #4: command-path positive case for `@getindigo.ai`.
    #[test]
    fn desktop_alt_gate_admits_indigo_email() {
        assert!(is_allowed_email(Some("stefan@getindigo.ai")));
        assert!(is_allowed_email(Some("STEFAN@GetIndigo.AI")));
    }

    /// US-001 AC #4: command-path negative case for non-allowed emails.
    #[test]
    fn desktop_alt_gate_rejects_non_indigo_email() {
        assert!(!is_allowed_email(Some("someone@gmail.com")));
        assert!(!is_allowed_email(Some("admin@notindigo.ai")));
        // Look-alike — leading `@` in ALLOWED_DOMAIN blocks suffix match
        // on `forgetindigo.ai`.
        assert!(!is_allowed_email(Some("attacker@forgetindigo.ai")));
    }

    /// US-001 AC #4: missing/empty emails return false (never default-true).
    #[test]
    fn desktop_alt_gate_rejects_missing_email() {
        assert!(!is_allowed_email(None));
        assert!(!is_allowed_email(Some("")));
    }

    #[tokio::test]
    async fn company_summary_returns_placeholder_counts() {
        let summary = super::get_company_summary("acme".to_string())
            .await
            .expect("valid slug should return a summary");

        assert_eq!(summary.board, 0);
        assert_eq!(summary.activity.last7d, 0);
        assert_eq!(summary.deployments, 0);
        assert_eq!(summary.secrets, 0);
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
            super::parse_deployments_response(reqwest::StatusCode::NO_CONTENT, "").unwrap(),
            Vec::<super::DeploymentEntry>::new()
        );
        assert_eq!(
            super::parse_deployments_response(reqwest::StatusCode::OK, " \n ").unwrap(),
            Vec::<super::DeploymentEntry>::new()
        );
        assert_eq!(
            super::parse_deployments_response(
                reqwest::StatusCode::NOT_FOUND,
                r#"{"code":"deployments-not-provisioned"}"#
            )
            .unwrap(),
            Vec::<super::DeploymentEntry>::new()
        );
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
    }
}
