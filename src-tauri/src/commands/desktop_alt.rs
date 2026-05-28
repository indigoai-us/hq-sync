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

use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

use crate::commands::cognito;
use crate::commands::sync::resolve_vault_api_url;
use crate::commands::workspaces::{Workspace, WorkspaceState};
use crate::util::client_info::build_client;

const WINDOW_LABEL: &str = "desktop-alt";

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

fn json_code(value: &serde_json::Value) -> Option<&str> {
    value.get("code").and_then(|v| v.as_str()).or_else(|| {
        value
            .get("error")
            .and_then(|e| e.get("code"))
            .and_then(|v| v.as_str())
    })
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
    fn company_uid_resolution_rejects_broken_and_states_without_cloud_identity() {
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
    }
}
