//! Local-PRD reader commands for the Projects surface (US-003).
//!
//! The Projects surface needs to list projects + read stories straight from the
//! local HQ tree — fast, offline, and cross-company — instead of round-tripping
//! to the vault for every render. These two commands scan the resolved HQ folder
//! and parse the on-disk `board.json` + `prd.json` files directly.
//!
//! Data shapes (modeled from real files):
//!   * `companies/<slug>/board.json` — `{ company, objectives[], initiatives[],
//!     projects[] }`. Each project: `id, title, description, status, scope, app,
//!     initiative_id, objective_id, prd_path, created_at, updated_at`.
//!   * `companies/<slug>/projects/<name>/prd.json` — `{ name, description,
//!     branchName, userStories[], metadata{} }`. Each story: `id, title,
//!     description, acceptanceCriteria[], passes, priority, labels[], dependsOn[],
//!     notes`.
//!
//! Both commands are gated by `feature_gate::is_indigo_user()` like the other
//! desktop-alt commands, and both must be allow-listed in
//! `capabilities/desktop-alt.json` + registered in `main.rs`.
//!
//! ## Vault fallback (AC #3)
//!
//! These commands are the *local* fast path. When the HQ folder cannot be
//! resolved to a real directory on disk, or no `companies/*/projects/*/prd.json`
//! exist, `get_local_projects` returns an **empty list** rather than erroring —
//! the desktop-alt frontend already calls the vault-backed `get_company_board`
//! (see `commands/desktop_alt.rs`) and treats an empty local list as "fall back
//! to the vault board". We deliberately do not call the vault API from inside
//! this module: keeping the local reader pure (filesystem only, no network, no
//! auth) makes it trivially testable and keeps the fallback decision in the
//! caller where the company context lives. A malformed individual `prd.json` /
//! `board.json` is skipped (logged), never panicked on — one bad file must not
//! blank the whole list.

use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::commands::config::{read_hq_config_lenient, MenubarPrefs};
use crate::util::paths;

/// One project row for the Projects list. Merges `board.json` project metadata
/// with `prd.json` story counts where a `prd_path` links them. Projects that
/// exist only as a `prd.json` (not referenced by any board) are still included.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LocalProject {
    /// Board project id (e.g. `in-proj-001`) when known, otherwise the prd
    /// directory name — always non-empty so the UI has a stable key.
    pub id: String,
    /// Display title — board `title`, falling back to prd `name`, then the id.
    pub title: String,
    #[serde(default)]
    pub description: String,
    /// Company slug the project belongs to (the `companies/<slug>/` dir).
    pub company: String,
    #[serde(default)]
    pub status: String,
    /// HQ-folder-relative path to the linked `prd.json`, when one exists.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prd_path: Option<String>,
    /// Total user stories in the linked prd (0 if no prd or unparseable).
    pub story_count: u32,
    /// Stories whose `passes == true`.
    pub stories_complete: u32,
}

/// A single user story, mirroring the prd.json story shape the Kanban + detail
/// views render.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LocalStory {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub acceptance_criteria: Vec<String>,
    #[serde(default)]
    pub passes: bool,
    #[serde(default)]
    pub priority: Option<String>,
    #[serde(default)]
    pub labels: Vec<String>,
    #[serde(default)]
    pub depends_on: Vec<String>,
    #[serde(default)]
    pub notes: Option<String>,
}

/// A parsed prd.json returned by `get_local_project_prd`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LocalProjectPrd {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub branch_name: Option<String>,
    #[serde(default)]
    pub user_stories: Vec<LocalStory>,
    /// Pass-through metadata object (company, goal, createdAt, …).
    #[serde(default)]
    pub metadata: serde_json::Value,
}

// ---- company goals (objectives + initiatives) ------------------------------

/// A single key result under an objective. The current board.json data carries
/// `key_results: []`, so every field is permissive (Option / serde default) —
/// this models whatever a populated KR might contain without erroring on the
/// empty case.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct KeyResult {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metric: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
}

/// One objective from a company `board.json` `objectives[]` entry.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Objective {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub timeframe: String,
    #[serde(default)]
    pub owner: Option<String>,
    #[serde(default)]
    pub key_results: Vec<KeyResult>,
    #[serde(default)]
    pub initiative_ids: Vec<String>,
    /// The Linear initiative this objective links to, when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub linear_initiative_id: Option<String>,
}

/// One initiative from a company `board.json` `initiatives[]` entry.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Initiative {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub status: String,
}

/// A company's GOALS surface: the objectives + initiatives from its
/// `board.json`. Returned by `get_local_company_goals`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CompanyGoals {
    pub objectives: Vec<Objective>,
    pub initiatives: Vec<Initiative>,
}

// ---- on-disk parse models (snake_case, matching the real JSON) -------------

/// `board.json` — only the fields we consume.
#[derive(Debug, Deserialize, Default)]
struct BoardFile {
    #[serde(default)]
    projects: Vec<BoardProject>,
}

/// `board.json` goals view — only the `objectives` + `initiatives` arrays. The
/// `Objective`/`Initiative` return structs are themselves `Deserialize` with
/// `#[serde(rename_all = "camelCase")]`; the on-disk JSON is snake_case, so we
/// parse via dedicated snake_case raw models below and convert.
#[derive(Debug, Deserialize, Default)]
struct BoardGoalsFile {
    #[serde(default)]
    objectives: Vec<RawObjective>,
    #[serde(default)]
    initiatives: Vec<RawInitiative>,
}

#[derive(Debug, Deserialize, Default)]
struct RawObjective {
    #[serde(default)]
    id: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    status: String,
    #[serde(default)]
    timeframe: String,
    #[serde(default)]
    owner: Option<String>,
    #[serde(default)]
    key_results: Vec<RawKeyResult>,
    #[serde(default)]
    initiative_ids: Vec<String>,
    #[serde(default)]
    linear_initiative_id: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
struct RawKeyResult {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    metric: Option<String>,
    #[serde(default)]
    target: Option<serde_json::Value>,
    #[serde(default)]
    current: Option<serde_json::Value>,
    #[serde(default)]
    unit: Option<String>,
    #[serde(default)]
    status: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
struct RawInitiative {
    #[serde(default)]
    id: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    status: String,
}

impl From<RawKeyResult> for KeyResult {
    fn from(k: RawKeyResult) -> Self {
        KeyResult {
            id: k.id,
            title: k.title,
            metric: k.metric,
            target: k.target,
            current: k.current,
            unit: k.unit,
            status: k.status,
        }
    }
}

impl From<RawObjective> for Objective {
    fn from(o: RawObjective) -> Self {
        Objective {
            id: o.id,
            title: o.title,
            description: o.description,
            status: o.status,
            timeframe: o.timeframe,
            owner: o.owner,
            key_results: o.key_results.into_iter().map(KeyResult::from).collect(),
            initiative_ids: o.initiative_ids,
            linear_initiative_id: o.linear_initiative_id,
        }
    }
}

impl From<RawInitiative> for Initiative {
    fn from(i: RawInitiative) -> Self {
        Initiative {
            id: i.id,
            title: i.title,
            description: i.description,
            status: i.status,
        }
    }
}

impl From<BoardGoalsFile> for CompanyGoals {
    fn from(b: BoardGoalsFile) -> Self {
        CompanyGoals {
            objectives: b.objectives.into_iter().map(Objective::from).collect(),
            initiatives: b.initiatives.into_iter().map(Initiative::from).collect(),
        }
    }
}

#[derive(Debug, Deserialize, Default, Clone)]
struct BoardProject {
    #[serde(default)]
    id: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    status: String,
    #[serde(default)]
    prd_path: Option<String>,
}

/// `prd.json` — the raw on-disk shape. Stories use camelCase keys, so this
/// model renames into snake_case Rust fields.
#[derive(Debug, Deserialize, Default)]
struct PrdFile {
    #[serde(default)]
    name: String,
    #[serde(default)]
    description: String,
    #[serde(default, rename = "branchName")]
    branch_name: Option<String>,
    #[serde(default, rename = "userStories")]
    user_stories: Vec<PrdStory>,
    #[serde(default)]
    metadata: serde_json::Value,
}

#[derive(Debug, Deserialize, Default)]
struct PrdStory {
    #[serde(default)]
    id: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    description: String,
    #[serde(default, rename = "acceptanceCriteria")]
    acceptance_criteria: Vec<String>,
    #[serde(default)]
    passes: bool,
    #[serde(default)]
    priority: Option<String>,
    #[serde(default)]
    labels: Vec<String>,
    #[serde(default, rename = "dependsOn")]
    depends_on: Vec<String>,
    #[serde(default)]
    notes: Option<String>,
}

impl From<PrdStory> for LocalStory {
    fn from(s: PrdStory) -> Self {
        LocalStory {
            id: s.id,
            title: s.title,
            description: s.description,
            acceptance_criteria: s.acceptance_criteria,
            passes: s.passes,
            priority: s.priority,
            labels: s.labels,
            depends_on: s.depends_on,
            notes: s.notes,
        }
    }
}

impl From<PrdFile> for LocalProjectPrd {
    fn from(p: PrdFile) -> Self {
        LocalProjectPrd {
            name: p.name,
            description: p.description,
            branch_name: p.branch_name,
            user_stories: p.user_stories.into_iter().map(LocalStory::from).collect(),
            metadata: p.metadata,
        }
    }
}

/// `(total, complete)` story counts for a parsed prd.
fn story_counts(prd: &PrdFile) -> (u32, u32) {
    let total = u32::try_from(prd.user_stories.len()).unwrap_or(u32::MAX);
    let complete = u32::try_from(prd.user_stories.iter().filter(|s| s.passes).count())
        .unwrap_or(u32::MAX);
    (total, complete)
}

/// Resolve the user's HQ folder using the standard 4-tier resolver, the same
/// way every other CLI-spawning command in this app does (mirrors
/// `commands/packages.rs::resolve_hq_folder`).
fn resolve_hq_folder() -> PathBuf {
    let menubar_prefs: Option<MenubarPrefs> = paths::menubar_json_path()
        .ok()
        .filter(|p| p.exists())
        .and_then(|p| std::fs::read_to_string(&p).ok())
        .and_then(|s| serde_json::from_str(&s).ok());
    let config = read_hq_config_lenient().ok().flatten();
    paths::resolve_hq_folder(
        config.as_ref().and_then(|c| c.hq_folder_path.as_deref()),
        menubar_prefs.as_ref().and_then(|p| p.hq_path.as_deref()),
    )
}

/// List projects across every company by scanning the local HQ tree.
///
/// Reads `companies/<slug>/board.json` for project metadata and
/// `companies/<slug>/projects/<name>/prd.json` for story data, merging the two
/// where a board project's `prd_path` points at a real prd. Projects that exist
/// only as a `prd.json` (no board entry) are still listed.
///
/// Returns an **empty list** (not an error) when the HQ folder doesn't resolve
/// to a directory or has no companies — the frontend treats empty-local as
/// "fall back to the vault board" (see module docs, AC #3). Individual
/// malformed `board.json` / `prd.json` files are skipped, never fatal.
#[tauri::command]
pub async fn get_local_projects() -> Result<Vec<LocalProject>, String> {
    if !crate::util::feature_gate::is_indigo_user().await {
        return Err("projects reader is Indigo-only".to_string());
    }
    let hq = resolve_hq_folder();
    Ok(scan_local_projects(&hq))
}

/// Pure, testable scanner — takes an explicit HQ root so tests can point it at a
/// fixture tree. Never panics: unreadable dirs/files are skipped.
fn scan_local_projects(hq_root: &Path) -> Vec<LocalProject> {
    let companies_dir = hq_root.join("companies");
    let entries = match std::fs::read_dir(&companies_dir) {
        Ok(e) => e,
        // No companies dir (HQ folder unresolved or empty) → empty list so the
        // caller falls back to the vault.
        Err(_) => return Vec::new(),
    };

    let mut out: Vec<LocalProject> = Vec::new();

    for entry in entries.flatten() {
        let company_path = entry.path();
        if !company_path.is_dir() {
            continue;
        }
        let slug = match company_path.file_name().and_then(|n| n.to_str()) {
            Some(s) if !s.starts_with('.') => s.to_string(),
            _ => continue,
        };

        // Track which prd.json paths a board already accounts for, so we can
        // append unlinked prds afterward without duplicating.
        let mut linked_prds: std::collections::HashSet<String> = std::collections::HashSet::new();

        // 1. board.json projects (with prd-linked story counts where possible).
        let board_path = company_path.join("board.json");
        if let Some(board) = read_json_lenient::<BoardFile>(&board_path) {
            for project in board.projects {
                let prd_counts = project.prd_path.as_deref().and_then(|rel| {
                    let abs = hq_root.join(rel);
                    // Only count prds that live inside the HQ folder.
                    if is_within(hq_root, &abs) {
                        read_json_lenient::<PrdFile>(&abs).map(|prd| story_counts(&prd))
                    } else {
                        None
                    }
                });
                if let Some(rel) = project.prd_path.as_deref() {
                    linked_prds.insert(normalize_rel(rel));
                }
                let (story_count, stories_complete) = prd_counts.unwrap_or((0, 0));
                let id = if project.id.trim().is_empty() {
                    project.title.clone()
                } else {
                    project.id.clone()
                };
                out.push(LocalProject {
                    id,
                    title: if project.title.trim().is_empty() {
                        project.prd_path.clone().unwrap_or_default()
                    } else {
                        project.title.clone()
                    },
                    description: project.description,
                    company: slug.clone(),
                    status: project.status,
                    prd_path: project.prd_path,
                    story_count,
                    stories_complete,
                });
            }
        }

        // 2. prd.json files not referenced by the board — include them too so a
        //    freshly-created project shows up before the board is regenerated.
        let projects_dir = company_path.join("projects");
        for prd_path in find_prd_files(&projects_dir) {
            let rel = match prd_path.strip_prefix(hq_root) {
                Ok(r) => r.to_string_lossy().replace('\\', "/"),
                Err(_) => continue,
            };
            if linked_prds.contains(&normalize_rel(&rel)) {
                continue;
            }
            let Some(prd) = read_json_lenient::<PrdFile>(&prd_path) else {
                continue;
            };
            let (story_count, stories_complete) = story_counts(&prd);
            // Project name from prd, falling back to the parent dir name.
            let dir_name = prd_path
                .parent()
                .and_then(|p| p.file_name())
                .and_then(|n| n.to_str())
                .unwrap_or("project")
                .to_string();
            let title = if prd.name.trim().is_empty() {
                dir_name.clone()
            } else {
                prd.name.clone()
            };
            out.push(LocalProject {
                id: dir_name,
                title,
                description: prd.description,
                company: slug.clone(),
                status: String::new(),
                prd_path: Some(rel),
                story_count,
                stories_complete,
            });
        }
    }

    out
}

/// Read + parse a single project's prd.json by HQ-folder-relative path.
///
/// Validates that the resolved path stays inside the HQ folder (no `..`
/// traversal, no absolute escape) before reading — AC #2.
#[tauri::command]
pub async fn get_local_project_prd(prd_path: String) -> Result<LocalProjectPrd, String> {
    if !crate::util::feature_gate::is_indigo_user().await {
        return Err("projects reader is Indigo-only".to_string());
    }
    let hq = resolve_hq_folder();
    read_project_prd(&hq, &prd_path)
}

/// Pure body for `get_local_project_prd` — takes an explicit HQ root so it's
/// unit-testable and the traversal guard is verifiable.
fn read_project_prd(hq_root: &Path, prd_path: &str) -> Result<LocalProjectPrd, String> {
    let rel = prd_path.trim();
    if rel.is_empty() {
        return Err("prd_path is required".to_string());
    }
    let abs = hq_root.join(rel);
    if !is_within(hq_root, &abs) {
        return Err(format!("prd_path escapes the HQ folder: {prd_path:?}"));
    }
    if abs.file_name().and_then(|n| n.to_str()) != Some("prd.json") {
        return Err("prd_path must point at a prd.json file".to_string());
    }
    let prd = read_json_lenient::<PrdFile>(&abs)
        .ok_or_else(|| format!("could not read or parse prd.json at {prd_path:?}"))?;
    Ok(LocalProjectPrd::from(prd))
}

/// Read a project's sibling `README.md` by the project's HQ-folder-relative
/// `prd.json` path (US-009).
///
/// The README is expected to live alongside the prd (`<dir>/README.md`). We take
/// the *prd* path rather than a free-form file path so the same path-traversal
/// guard as `get_local_project_prd` applies and the frontend never has to
/// construct a README path itself. Returns `Ok(None)` when no README exists (a
/// project without one is normal, not an error); `Err` only on a path-escape or
/// an unreadable-but-present file.
#[tauri::command]
pub async fn get_local_project_readme(prd_path: String) -> Result<Option<String>, String> {
    if !crate::util::feature_gate::is_indigo_user().await {
        return Err("projects reader is Indigo-only".to_string());
    }
    let hq = resolve_hq_folder();
    read_project_readme(&hq, &prd_path)
}

/// Pure body for `get_local_project_readme` — explicit HQ root for testing.
///
/// Derives the project directory from the prd path (its parent), then reads
/// `<dir>/README.md`. Reuses the same lexical `is_within` guard so a malicious
/// `prd_path` can't escape the HQ folder.
fn read_project_readme(hq_root: &Path, prd_path: &str) -> Result<Option<String>, String> {
    let rel = prd_path.trim();
    if rel.is_empty() {
        return Err("prd_path is required".to_string());
    }
    let prd_abs = hq_root.join(rel);
    if !is_within(hq_root, &prd_abs) {
        return Err(format!("prd_path escapes the HQ folder: {prd_path:?}"));
    }
    if prd_abs.file_name().and_then(|n| n.to_str()) != Some("prd.json") {
        return Err("prd_path must point at a prd.json file".to_string());
    }
    let Some(dir) = prd_abs.parent() else {
        return Ok(None);
    };
    let readme = dir.join("README.md");
    // Defense-in-depth: the derived README must also stay inside the HQ folder.
    if !is_within(hq_root, &readme) {
        return Err("README path escapes the HQ folder".to_string());
    }
    if !readme.is_file() {
        return Ok(None);
    }
    match std::fs::read_to_string(&readme) {
        Ok(content) => Ok(Some(content)),
        Err(e) => Err(format!("could not read README.md: {e}")),
    }
}

/// Read a company's GOALS (objectives + initiatives) from its `board.json`
/// under the resolved HQ folder.
///
/// Powers a per-company board UI that renders OKRs. Reads
/// `companies/<company_slug>/board.json` and returns only its `objectives[]` +
/// `initiatives[]` (the projects live behind `get_local_projects`). Indigo-gated
/// like the other local readers.
///
/// A missing or unparseable `board.json` yields an **empty** `CompanyGoals`
/// rather than an error — a company without a board simply has no goals yet, and
/// the caller can fall back to the vault. The only hard errors are a
/// `company_slug` that escapes the HQ folder (path-traversal guard) or the gate
/// rejecting a non-Indigo caller.
#[tauri::command]
pub async fn get_local_company_goals(company_slug: String) -> Result<CompanyGoals, String> {
    if !crate::util::feature_gate::is_indigo_user().await {
        return Err("goals reader is Indigo-only".to_string());
    }
    let hq = resolve_hq_folder();
    read_company_goals(&hq, &company_slug)
}

/// Pure body for `get_local_company_goals` — takes an explicit HQ root so it's
/// unit-testable and the traversal guard is verifiable.
///
/// Validates the slug stays inside `companies/` under the HQ folder (no `..`
/// traversal, no nested path, no absolute escape), then leniently parses the
/// company's `board.json`. Missing/garbage board → empty `CompanyGoals`.
fn read_company_goals(hq_root: &Path, company_slug: &str) -> Result<CompanyGoals, String> {
    let slug = company_slug.trim();
    if slug.is_empty() {
        return Err("company_slug is required".to_string());
    }
    // A slug is a single directory name — reject anything with separators or
    // traversal components before it ever touches the filesystem.
    if slug.contains('/') || slug.contains('\\') || slug == "." || slug == ".." {
        return Err(format!("invalid company_slug: {company_slug:?}"));
    }
    let board_path = hq_root.join("companies").join(slug).join("board.json");
    // Defense-in-depth: the resolved path must stay inside the HQ folder.
    if !is_within(hq_root, &board_path) {
        return Err(format!(
            "company_slug escapes the HQ folder: {company_slug:?}"
        ));
    }
    // Missing/unparseable board.json → empty goals (not an error).
    Ok(read_json_lenient::<BoardGoalsFile>(&board_path)
        .map(CompanyGoals::from)
        .unwrap_or_default())
}

// ---- writes (US-010) -------------------------------------------------------

/// Persist a project's `status` (and refresh its `updated_at`) back to the
/// company `board.json` under the resolved HQ folder.
///
/// Local-write counterpart to the read commands above — makes the desktop-alt
/// board a control center (US-010). The frontend updates its store optimistically
/// and calls this to persist; a returned `Err` is the rollback signal.
///
/// Inputs (HQ-relative, validated):
///   * `board_path` — HQ-folder-relative path ending in `board.json`.
///   * `project_id` — the `id` of the project entry to mutate.
///   * `status`     — the new status string (an editable-status value).
///
/// Safety/correctness (AC #1, #2):
///   * Indigo-gated, same as the readers.
///   * `is_within` lexical guard rejects any `..`/absolute escape, and the target
///     must be a `board.json` — only `companies/*/board.json` is writable.
///   * The write is atomic + round-trip-validated: we parse the existing JSON,
///     mutate the matching project in the parsed tree, re-serialize, write to a
///     sibling temp file, then rename over the original. A parse/serialize
///     failure aborts before any rename, so a bad write can never corrupt the
///     file in place.
#[tauri::command]
pub async fn set_local_project_status(
    board_path: String,
    project_id: String,
    status: String,
) -> Result<(), String> {
    if !crate::util::feature_gate::is_indigo_user().await {
        return Err("projects writer is Indigo-only".to_string());
    }
    let hq = resolve_hq_folder();
    write_project_status(&hq, &board_path, &project_id, &status)
}

/// Pure body for `set_local_project_status` — explicit HQ root for testing.
fn write_project_status(
    hq_root: &Path,
    board_path: &str,
    project_id: &str,
    status: &str,
) -> Result<(), String> {
    let rel = board_path.trim();
    if rel.is_empty() {
        return Err("board_path is required".to_string());
    }
    if project_id.trim().is_empty() {
        return Err("project_id is required".to_string());
    }
    let abs = hq_root.join(rel);
    if !is_within(hq_root, &abs) {
        return Err(format!("board_path escapes the HQ folder: {board_path:?}"));
    }
    if abs.file_name().and_then(|n| n.to_str()) != Some("board.json") {
        return Err("board_path must point at a board.json file".to_string());
    }

    // Parse the existing JSON into a generic tree (preserving every field we
    // don't touch), mutate the matching project, re-serialize.
    let bytes = std::fs::read(&abs)
        .map_err(|e| format!("could not read board.json at {board_path:?}: {e}"))?;
    let mut tree: serde_json::Value = serde_json::from_slice(&bytes)
        .map_err(|e| format!("board.json at {board_path:?} is not valid JSON: {e}"))?;

    let projects = tree
        .get_mut("projects")
        .and_then(|p| p.as_array_mut())
        .ok_or_else(|| "board.json has no `projects` array".to_string())?;

    let target = projects
        .iter_mut()
        .find(|p| p.get("id").and_then(|v| v.as_str()) == Some(project_id))
        .ok_or_else(|| format!("no project with id {project_id:?} in board.json"))?;

    let obj = target
        .as_object_mut()
        .ok_or_else(|| "matched project is not a JSON object".to_string())?;
    obj.insert(
        "status".to_string(),
        serde_json::Value::String(status.to_string()),
    );
    obj.insert(
        "updated_at".to_string(),
        serde_json::Value::String(now_iso8601()),
    );

    atomic_write_json(&abs, &tree)
}

/// Persist a story's `passes` toggle back to the project's `prd.json` (optional
/// US-010 nicety). Same gate + guard + atomic-write discipline as the status
/// write; the `prd_path` must point at a `prd.json` inside the HQ folder.
#[tauri::command]
pub async fn set_local_story_passes(
    prd_path: String,
    story_id: String,
    passes: bool,
) -> Result<(), String> {
    if !crate::util::feature_gate::is_indigo_user().await {
        return Err("projects writer is Indigo-only".to_string());
    }
    let hq = resolve_hq_folder();
    write_story_passes(&hq, &prd_path, &story_id, passes)
}

/// Pure body for `set_local_story_passes` — explicit HQ root for testing.
fn write_story_passes(
    hq_root: &Path,
    prd_path: &str,
    story_id: &str,
    passes: bool,
) -> Result<(), String> {
    let rel = prd_path.trim();
    if rel.is_empty() {
        return Err("prd_path is required".to_string());
    }
    if story_id.trim().is_empty() {
        return Err("story_id is required".to_string());
    }
    let abs = hq_root.join(rel);
    if !is_within(hq_root, &abs) {
        return Err(format!("prd_path escapes the HQ folder: {prd_path:?}"));
    }
    if abs.file_name().and_then(|n| n.to_str()) != Some("prd.json") {
        return Err("prd_path must point at a prd.json file".to_string());
    }

    let bytes = std::fs::read(&abs)
        .map_err(|e| format!("could not read prd.json at {prd_path:?}: {e}"))?;
    let mut tree: serde_json::Value = serde_json::from_slice(&bytes)
        .map_err(|e| format!("prd.json at {prd_path:?} is not valid JSON: {e}"))?;

    let stories = tree
        .get_mut("userStories")
        .and_then(|s| s.as_array_mut())
        .ok_or_else(|| "prd.json has no `userStories` array".to_string())?;

    let target = stories
        .iter_mut()
        .find(|s| s.get("id").and_then(|v| v.as_str()) == Some(story_id))
        .ok_or_else(|| format!("no story with id {story_id:?} in prd.json"))?;

    let obj = target
        .as_object_mut()
        .ok_or_else(|| "matched story is not a JSON object".to_string())?;
    obj.insert("passes".to_string(), serde_json::Value::Bool(passes));

    atomic_write_json(&abs, &tree)
}

/// Atomically write a JSON value to `target` (2-space indent + trailing
/// newline): serialize first (so a serialize failure aborts before any I/O),
/// write to a sibling `.tmp` file, fsync it, then rename over the target. The
/// rename is atomic on the same filesystem, so a reader never sees a partial
/// file and a crash mid-write leaves the original intact.
fn atomic_write_json(target: &Path, value: &serde_json::Value) -> Result<(), String> {
    let mut serialized = serde_json::to_string_pretty(value)
        .map_err(|e| format!("could not serialize JSON: {e}"))?;
    serialized.push('\n');

    let dir = target
        .parent()
        .ok_or_else(|| "target has no parent directory".to_string())?;
    // Unique temp name (pid + nanos) so concurrent writes can't clobber a shared
    // temp file. Same dir as the target → rename stays on one filesystem.
    let tmp_name = format!(
        ".{}.{}.{}.tmp",
        target
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("board.json"),
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0),
    );
    let tmp_path = dir.join(tmp_name);

    {
        use std::io::Write;
        let mut f = std::fs::File::create(&tmp_path)
            .map_err(|e| format!("could not create temp file: {e}"))?;
        f.write_all(serialized.as_bytes())
            .map_err(|e| format!("could not write temp file: {e}"))?;
        f.sync_all()
            .map_err(|e| format!("could not flush temp file: {e}"))?;
    }

    std::fs::rename(&tmp_path, target).map_err(|e| {
        // Best-effort cleanup so a failed rename doesn't leave a stray temp file.
        let _ = std::fs::remove_file(&tmp_path);
        format!("could not commit write: {e}")
    })
}

/// Current UTC time as an ISO-8601 / RFC-3339 `Z` string (no chrono dep).
fn now_iso8601() -> String {
    // Days-since-epoch → civil date via Howard Hinnant's algorithm, then HMS.
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let (hh, mm, ss) = (rem / 3600, (rem % 3600) / 60, rem % 60);

    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };

    format!("{y:04}-{m:02}-{d:02}T{hh:02}:{mm:02}:{ss:02}Z")
}

/// Parse a JSON file leniently: `None` on missing/unreadable/garbage (never a
/// panic). Used so one bad file can be skipped instead of failing the scan.
fn read_json_lenient<T: serde::de::DeserializeOwned>(path: &Path) -> Option<T> {
    let bytes = std::fs::read(path).ok()?;
    match serde_json::from_slice::<T>(&bytes) {
        Ok(v) => Some(v),
        Err(e) => {
            eprintln!(
                "[projects-local] skipping unparseable {}: {e}",
                path.display()
            );
            None
        }
    }
}

/// Find every `projects/*/prd.json` (one level deep) under a company's
/// `projects/` dir. Skips unreadable dirs. Does not recurse into `_archive`'s
/// nested layout beyond one level — board.json links cover archived prds.
fn find_prd_files(projects_dir: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let Ok(entries) = std::fs::read_dir(projects_dir) else {
        return found;
    };
    for entry in entries.flatten() {
        let dir = entry.path();
        if !dir.is_dir() {
            continue;
        }
        let candidate = dir.join("prd.json");
        if candidate.is_file() {
            found.push(candidate);
        }
    }
    found
}

/// Normalize a relative path for set membership (collapse `./`, unify slashes).
fn normalize_rel(rel: &str) -> String {
    rel.trim_start_matches("./").replace('\\', "/")
}

/// True iff `candidate`, after lexical normalization, is contained within
/// `root`. Rejects `..` traversal and absolute escapes WITHOUT touching the
/// filesystem (so it works on non-existent paths too). We normalize lexically
/// rather than canonicalize because the target file may not exist yet and
/// canonicalize would also resolve symlinks we don't want to chase.
fn is_within(root: &Path, candidate: &Path) -> bool {
    let normalized = lexically_normalize(candidate);
    let root_norm = lexically_normalize(root);
    normalized.starts_with(&root_norm)
}

/// Collapse `.` and `..` components lexically. A leading `..` that would escape
/// the prefix is preserved as a `ParentDir` component so `is_within` rejects it.
fn lexically_normalize(path: &Path) -> PathBuf {
    let mut stack: Vec<Component> = Vec::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                match stack.last() {
                    Some(Component::Normal(_)) => {
                        stack.pop();
                    }
                    // Can't pop a root/prefix; keep the `..` so it can't match a
                    // root prefix in `starts_with`.
                    _ => stack.push(component),
                }
            }
            other => stack.push(other),
        }
    }
    let mut out = PathBuf::new();
    for c in stack {
        out.push(c.as_os_str());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// Build a throwaway HQ tree under a unique temp dir and return its root.
    ///
    /// The dir name mixes pid + a monotonic time component **and** a process-wide
    /// atomic counter so two fixtures built concurrently (tests run in parallel)
    /// can never collide on the same path — a same-nanosecond collision would
    /// otherwise let one test's tree leak into another's scan.
    fn make_fixture_tree() -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "hq-projects-local-test-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            SEQ.fetch_add(1, Ordering::Relaxed),
        ));
        let indigo = root.join("companies").join("indigo");
        let proj = indigo.join("projects").join("flagship");
        fs::create_dir_all(&proj).unwrap();

        // A valid prd.json with 3 stories (2 passing).
        let prd = r#"{
            "name": "Flagship",
            "description": "the flagship project",
            "branchName": "feature/flagship",
            "userStories": [
                {"id":"US-001","title":"one","acceptanceCriteria":["a","b"],"passes":true,"priority":"P0","labels":["x"],"dependsOn":[],"notes":"n"},
                {"id":"US-002","title":"two","passes":true},
                {"id":"US-003","title":"three","passes":false}
            ],
            "metadata": {"company":"indigo","goal":"ship"}
        }"#;
        fs::write(proj.join("prd.json"), prd).unwrap();

        // board.json: one project links the prd above, one is a garbage-prd link.
        let board = r#"{
            "company": "indigo",
            "projects": [
                {"id":"in-proj-001","title":"Flagship","description":"d","status":"active","prd_path":"companies/indigo/projects/flagship/prd.json"},
                {"id":"in-proj-002","title":"Broken","status":"archived","prd_path":"companies/indigo/projects/missing/prd.json"}
            ]
        }"#;
        fs::write(indigo.join("board.json"), board).unwrap();

        // A second company with an unlinked prd (no board.json at all).
        let solo = root
            .join("companies")
            .join("acme")
            .join("projects")
            .join("widget");
        fs::create_dir_all(&solo).unwrap();
        fs::write(
            solo.join("prd.json"),
            r#"{"name":"Widget","userStories":[{"id":"W-1","passes":false}]}"#,
        )
        .unwrap();

        // A garbage prd.json that must be skipped (not panic).
        let junk = root
            .join("companies")
            .join("acme")
            .join("projects")
            .join("junk");
        fs::create_dir_all(&junk).unwrap();
        fs::write(junk.join("prd.json"), "{ this is not json ]").unwrap();

        root
    }

    #[test]
    fn scan_merges_board_and_prd_counts() {
        let root = make_fixture_tree();
        let mut projects = scan_local_projects(&root);
        // Deterministic order for assertions.
        projects.sort_by(|a, b| (a.company.clone(), a.id.clone()).cmp(&(b.company.clone(), b.id.clone())));

        // acme: one valid unlinked prd ("widget"), junk skipped.
        let acme: Vec<_> = projects.iter().filter(|p| p.company == "acme").collect();
        assert_eq!(acme.len(), 1, "junk prd must be skipped, widget kept");
        assert_eq!(acme[0].title, "Widget");
        assert_eq!(acme[0].story_count, 1);
        assert_eq!(acme[0].stories_complete, 0);

        // indigo: two board projects. Flagship links a real prd → 3 stories, 2 done.
        let flagship = projects
            .iter()
            .find(|p| p.id == "in-proj-001")
            .expect("flagship board project present");
        assert_eq!(flagship.title, "Flagship");
        assert_eq!(flagship.story_count, 3);
        assert_eq!(flagship.stories_complete, 2);
        assert_eq!(
            flagship.prd_path.as_deref(),
            Some("companies/indigo/projects/flagship/prd.json")
        );

        // The board project whose prd_path is missing → 0/0, still listed.
        let broken = projects
            .iter()
            .find(|p| p.id == "in-proj-002")
            .expect("broken board project still listed");
        assert_eq!(broken.story_count, 0);
        assert_eq!(broken.stories_complete, 0);

        // The flagship prd is board-linked, so it must NOT also appear as an
        // unlinked prd row (no duplicate).
        let flagship_rows = projects
            .iter()
            .filter(|p| {
                p.prd_path.as_deref() == Some("companies/indigo/projects/flagship/prd.json")
            })
            .count();
        assert_eq!(flagship_rows, 1, "linked prd must not be duplicated");

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn missing_companies_dir_returns_empty() {
        let root = std::env::temp_dir().join(format!(
            "hq-projects-local-empty-{}",
            std::process::id()
        ));
        // Root exists but has no companies/ subdir.
        let _ = fs::create_dir_all(&root);
        assert!(scan_local_projects(&root).is_empty());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn read_prd_parses_stories() {
        let root = make_fixture_tree();
        let prd = read_project_prd(&root, "companies/indigo/projects/flagship/prd.json")
            .expect("prd parses");
        assert_eq!(prd.name, "Flagship");
        assert_eq!(prd.branch_name.as_deref(), Some("feature/flagship"));
        assert_eq!(prd.user_stories.len(), 3);
        let us1 = &prd.user_stories[0];
        assert_eq!(us1.id, "US-001");
        assert_eq!(us1.acceptance_criteria, vec!["a", "b"]);
        assert!(us1.passes);
        assert_eq!(us1.priority.as_deref(), Some("P0"));
        assert_eq!(us1.labels, vec!["x"]);
        assert_eq!(us1.notes.as_deref(), Some("n"));
        // metadata passes through.
        assert_eq!(prd.metadata["company"], "indigo");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn read_prd_garbage_file_errors_not_panics() {
        let root = make_fixture_tree();
        let err = read_project_prd(&root, "companies/acme/projects/junk/prd.json")
            .expect_err("garbage prd must Err, not panic");
        assert!(err.contains("could not read or parse"), "got: {err}");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn read_prd_missing_file_errors() {
        let root = make_fixture_tree();
        let err = read_project_prd(&root, "companies/indigo/projects/nope/prd.json")
            .expect_err("missing prd must Err");
        assert!(err.contains("could not read or parse"), "got: {err}");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn path_traversal_is_rejected() {
        let root = make_fixture_tree();
        for evil in [
            "../../../etc/passwd",
            "companies/../../secrets/prd.json",
            "/etc/passwd",
            "companies/indigo/../../../prd.json",
        ] {
            let res = read_project_prd(&root, evil);
            assert!(res.is_err(), "traversal {evil:?} must be rejected");
        }
        // Non-prd.json filename inside the tree is also rejected.
        let res = read_project_prd(&root, "companies/indigo/board.json");
        assert!(res.is_err(), "non-prd.json target must be rejected");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn read_readme_returns_sibling_content() {
        let root = make_fixture_tree();
        // No README yet → Ok(None).
        let none = read_project_readme(&root, "companies/indigo/projects/flagship/prd.json")
            .expect("missing README is Ok(None)");
        assert!(none.is_none(), "no README → None");

        // Write a sibling README and read it back.
        let readme_path = root
            .join("companies")
            .join("indigo")
            .join("projects")
            .join("flagship")
            .join("README.md");
        fs::write(&readme_path, "# Flagship\n\nHello **world**.").unwrap();
        let some = read_project_readme(&root, "companies/indigo/projects/flagship/prd.json")
            .expect("README reads")
            .expect("README present");
        assert!(some.contains("# Flagship"));
        assert!(some.contains("Hello **world**."));

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn read_readme_rejects_traversal_and_non_prd() {
        let root = make_fixture_tree();
        for evil in ["../../../etc/passwd", "companies/../../secrets/prd.json"] {
            assert!(
                read_project_readme(&root, evil).is_err(),
                "traversal {evil:?} must be rejected"
            );
        }
        // A non-prd.json target is rejected before any README is derived.
        assert!(
            read_project_readme(&root, "companies/indigo/board.json").is_err(),
            "non-prd.json target must be rejected"
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn is_within_lexical_guard() {
        let root = Path::new("/Users/x/HQ");
        assert!(is_within(root, &root.join("companies/indigo/prd.json")));
        assert!(!is_within(root, Path::new("/Users/x/HQ/../evil")));
        assert!(!is_within(root, Path::new("/etc/passwd")));
        assert!(is_within(root, &root.join("a/./b/../c")));
    }

    // ---- company goals -----------------------------------------------------

    /// Build a fixture HQ tree whose indigo board.json carries 2 objectives
    /// (one with a populated key_result, one with `[]`) + 1 initiative.
    fn make_goals_fixture_tree() -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "hq-projects-local-goals-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            SEQ.fetch_add(1, Ordering::Relaxed),
        ));
        let indigo = root.join("companies").join("indigo");
        fs::create_dir_all(&indigo).unwrap();

        let board = r#"{
            "company": "indigo",
            "objectives": [
                {
                    "id": "in-obj-001",
                    "title": "Desktop Experience",
                    "description": "Native desktop apps",
                    "timeframe": "2026",
                    "owner": "corey",
                    "status": "on_track",
                    "linear_initiative_id": null,
                    "initiative_ids": ["in-init-001"],
                    "key_results": [
                        {"id":"kr-1","title":"Ship 1.0","metric":"releases","target":1,"current":0,"unit":"count","status":"in_progress"}
                    ]
                },
                {
                    "id": "in-obj-002",
                    "title": "Platform Stability",
                    "description": "Reliability",
                    "timeframe": "2026",
                    "owner": null,
                    "status": "on_track",
                    "initiative_ids": ["in-init-002"],
                    "key_results": []
                }
            ],
            "initiatives": [
                {
                    "id": "in-init-001",
                    "title": "Desktop Experience",
                    "description": "Native desktop apps",
                    "status": "active"
                }
            ],
            "projects": []
        }"#;
        fs::write(indigo.join("board.json"), board).unwrap();
        root
    }

    #[test]
    fn read_company_goals_parses_objectives_and_initiatives() {
        let root = make_goals_fixture_tree();
        let goals = read_company_goals(&root, "indigo").expect("goals parse");

        assert_eq!(goals.objectives.len(), 2);
        assert_eq!(goals.initiatives.len(), 1);

        // Objective 1: populated key_result + owner + linked initiative.
        let obj1 = &goals.objectives[0];
        assert_eq!(obj1.id, "in-obj-001");
        assert_eq!(obj1.title, "Desktop Experience");
        assert_eq!(obj1.status, "on_track");
        assert_eq!(obj1.timeframe, "2026");
        assert_eq!(obj1.owner.as_deref(), Some("corey"));
        assert_eq!(obj1.initiative_ids, vec!["in-init-001"]);
        assert_eq!(obj1.key_results.len(), 1);
        let kr = &obj1.key_results[0];
        assert_eq!(kr.id.as_deref(), Some("kr-1"));
        assert_eq!(kr.title.as_deref(), Some("Ship 1.0"));
        assert_eq!(kr.metric.as_deref(), Some("releases"));
        assert_eq!(kr.unit.as_deref(), Some("count"));
        assert_eq!(kr.status.as_deref(), Some("in_progress"));

        // Objective 2: empty key_results, null owner.
        let obj2 = &goals.objectives[1];
        assert_eq!(obj2.id, "in-obj-002");
        assert!(obj2.owner.is_none());
        assert!(obj2.key_results.is_empty());

        // Initiative round-trips.
        let init = &goals.initiatives[0];
        assert_eq!(init.id, "in-init-001");
        assert_eq!(init.title, "Desktop Experience");
        assert_eq!(init.status, "active");

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn read_company_goals_missing_board_is_empty_not_panic() {
        let root = make_goals_fixture_tree();
        // A company with no board.json at all → empty goals, no error/panic.
        let goals = read_company_goals(&root, "acme").expect("missing board → empty goals");
        assert!(goals.objectives.is_empty());
        assert!(goals.initiatives.is_empty());
        assert_eq!(goals, CompanyGoals::default());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn read_company_goals_rejects_traversal_and_empty_slug() {
        let root = make_goals_fixture_tree();
        for evil in ["../../../etc", "..", ".", "foo/bar", "indigo/../secrets"] {
            assert!(
                read_company_goals(&root, evil).is_err(),
                "slug {evil:?} must be rejected"
            );
        }
        assert!(read_company_goals(&root, "   ").is_err(), "empty slug rejected");
        let _ = fs::remove_dir_all(&root);
    }

    // ---- writes (US-010) ---------------------------------------------------

    #[test]
    fn write_project_status_persists_and_round_trips() {
        let root = make_fixture_tree();
        let board_rel = "companies/indigo/board.json";

        // Sanity: the fixture board has in-proj-001 with status "active".
        let before: BoardFile =
            read_json_lenient(&root.join(board_rel)).expect("board parses before");
        let p0 = before
            .projects
            .iter()
            .find(|p| p.id == "in-proj-001")
            .expect("in-proj-001 present");
        assert_eq!(p0.status, "active");

        // Mutate → reread → assert the new status persisted.
        write_project_status(&root, board_rel, "in-proj-001", "completed")
            .expect("status write succeeds");

        let after_bytes = fs::read(root.join(board_rel)).unwrap();
        let after: serde_json::Value = serde_json::from_slice(&after_bytes).expect("still valid JSON");
        let proj = after["projects"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["id"] == "in-proj-001")
            .expect("in-proj-001 still present");
        assert_eq!(proj["status"], "completed");
        // updated_at was refreshed to an ISO-8601 Z timestamp.
        let updated = proj["updated_at"].as_str().expect("updated_at written");
        assert!(updated.ends_with('Z') && updated.contains('T'), "got: {updated}");

        // Untouched sibling project keeps its original status (no clobber).
        let other = after["projects"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["id"] == "in-proj-002")
            .expect("in-proj-002 preserved");
        assert_eq!(other["status"], "archived");

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn write_project_status_rejects_malformed_and_missing_targets() {
        let root = make_fixture_tree();

        // Path traversal / absolute escape is rejected.
        for evil in ["../../../etc/board.json", "/etc/board.json"] {
            assert!(
                write_project_status(&root, evil, "in-proj-001", "completed").is_err(),
                "traversal {evil:?} must be rejected"
            );
        }
        // A non-board.json target inside the tree is rejected.
        assert!(
            write_project_status(
                &root,
                "companies/indigo/projects/flagship/prd.json",
                "in-proj-001",
                "completed",
            )
            .is_err(),
            "non-board.json target must be rejected"
        );
        // An unknown project id is rejected (and the file is left untouched).
        let before = fs::read(root.join("companies/indigo/board.json")).unwrap();
        assert!(
            write_project_status(&root, "companies/indigo/board.json", "nope-id", "completed")
                .is_err(),
            "unknown project id must Err"
        );
        let after = fs::read(root.join("companies/indigo/board.json")).unwrap();
        assert_eq!(before, after, "rejected write must not mutate the file");

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn write_story_passes_toggles_and_preserves_siblings() {
        let root = make_fixture_tree();
        let prd_rel = "companies/indigo/projects/flagship/prd.json";

        // US-003 starts passes=false; flip to true.
        write_story_passes(&root, prd_rel, "US-003", true).expect("passes write succeeds");

        let prd = read_project_prd(&root, prd_rel).expect("prd still parses");
        let us3 = prd
            .user_stories
            .iter()
            .find(|s| s.id == "US-003")
            .expect("US-003 present");
        assert!(us3.passes, "US-003 must now pass");
        // A sibling story is untouched.
        let us1 = prd.user_stories.iter().find(|s| s.id == "US-001").unwrap();
        assert!(us1.passes, "US-001 still passes");

        // A bad target path is rejected.
        assert!(
            write_story_passes(&root, "../../evil/prd.json", "US-003", true).is_err(),
            "traversal must be rejected"
        );

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn now_iso8601_is_well_formed() {
        let s = now_iso8601();
        // YYYY-MM-DDTHH:MM:SSZ → 20 chars.
        assert_eq!(s.len(), 20, "got: {s}");
        assert!(s.ends_with('Z'));
        assert_eq!(&s[4..5], "-");
        assert_eq!(&s[10..11], "T");
    }
}
