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

// ---- on-disk parse models (snake_case, matching the real JSON) -------------

/// `board.json` — only the fields we consume.
#[derive(Debug, Deserialize, Default)]
struct BoardFile {
    #[serde(default)]
    projects: Vec<BoardProject>,
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
}
