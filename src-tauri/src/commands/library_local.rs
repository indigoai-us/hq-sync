//! Local reader for the Skills & Workers **Library** surface.
//!
//! The desktop-alt window can list every WORKER and SKILL available in the
//! user's HQ install — at the root/shared scope and scoped per company — without
//! round-tripping to any vault. These commands scan the resolved HQ folder and
//! parse the on-disk source of truth directly:
//!
//!   * WORKERS — `core/workers/registry.yaml` (auto-generated). Each entry:
//!     `{ id, path, type, visibility (public|private), status, team?, company?,
//!     description }`. Root/shared workers are `visibility: public`; a company's
//!     workers live under `companies/<slug>/workers/` (`visibility: private`).
//!     The `company:` field is unreliable — some templated entries carry a YAML
//!     *map* (`company: {product: ''}`) rather than a string — so the company
//!     slug is derived from the entry `path`, never the field. Per-worker detail
//!     comes from `<path>/worker.yaml` (`worker.{name,type,description}`, plus
//!     top-level `skills` + `instructions`, both of which vary in shape).
//!   * SKILLS — one `SKILL.md` per skill dir, with YAML frontmatter between `---`
//!     fences (`name`, `description`, optional `allowed-tools`) and a markdown
//!     body. Root skills live in `.claude/skills/*` (many are symlinks into
//!     `core/packages/hq-pack-*/skills/`), personal skills in `personal/skills/*`,
//!     and a company's skills in `companies/<slug>/skills/*`.
//!
//! Like the other desktop-alt readers (`commands/projects_local.rs`), every
//! command is gated by `feature_gate::is_indigo_user()`, resolves the HQ folder
//! with the standard 4-tier resolver, and guards detail reads with the lexical
//! `is_within` path-traversal check. Parsing is lenient: a missing registry or an
//! unreadable/garbage individual file is skipped (empty result), never a panic —
//! one bad worker.yaml must not blank the whole library.
//!
//! These are app-registered Tauri commands authorized by `core:default` in
//! `capabilities/desktop-alt.json` (custom commands are not gated by per-command
//! permission identifiers), so no allow-* tokens are added.

use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::commands::config::{read_hq_config_lenient, MenubarPrefs};
use crate::util::paths;

// ---- wire types (camelCase) ------------------------------------------------

/// One worker row for the Library list.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LibraryWorker {
    pub id: String,
    pub name: String,
    /// Worker type (e.g. `CodeWorker`, `OpsWorker`). Renamed off the YAML `type`.
    #[serde(rename = "type")]
    pub type_: String,
    #[serde(default)]
    pub description: String,
    /// `"root"` for shared/public workers, `"company"` for company-scoped.
    pub scope: String,
    /// Company slug for `company`-scoped workers; absent for root.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub company: Option<String>,
    #[serde(default)]
    pub status: String,
    /// HQ-folder-relative path to the worker dir (the `<path>/worker.yaml` lives
    /// here) — the key the detail command takes.
    pub path: String,
    #[serde(default)]
    pub team: Option<String>,
}

/// One skill row for the Library list.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LibrarySkill {
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// `"root"` | `"personal"` | `"company"`.
    pub scope: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub company: Option<String>,
    /// HQ-folder-relative path to the skill's `SKILL.md` — the detail key.
    pub path: String,
    #[serde(default)]
    pub allowed_tools: Vec<String>,
}

/// Combined library payload for one scope (root or a company).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LibraryItems {
    pub workers: Vec<LibraryWorker>,
    pub skills: Vec<LibrarySkill>,
}

/// A named skill reference inside a worker's detail.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WorkerSkillRef {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// Full worker detail from `<path>/worker.yaml`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WorkerDetail {
    pub id: String,
    pub name: String,
    #[serde(rename = "type")]
    pub type_: String,
    #[serde(default)]
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub team: Option<String>,
    #[serde(default)]
    pub skills: Vec<WorkerSkillRef>,
    /// Free-form instructions rendered as markdown by the frontend. Normalized to
    /// a single string (a YAML block scalar or a list of bullet strings).
    #[serde(default)]
    pub instructions: String,
}

/// Full skill detail from a `SKILL.md` (frontmatter + body).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SkillDetail {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub allowed_tools: Vec<String>,
    /// The markdown body after the frontmatter fence — rendered by the frontend.
    #[serde(default)]
    pub body: String,
}

// ---- on-disk parse models --------------------------------------------------

/// `registry.yaml` top level — only the `workers` array.
#[derive(Debug, Deserialize, Default)]
struct RegistryFile {
    #[serde(default)]
    workers: Vec<RawWorkerEntry>,
}

/// One registry entry. Lenient: every field is optional/defaulted and `company`
/// absorbs both the string and `{product: ''}`-map cases via `serde_yaml::Value`
/// (we ignore it and derive the slug from `path`).
#[derive(Debug, Deserialize, Default)]
struct RawWorkerEntry {
    #[serde(default)]
    id: String,
    #[serde(default)]
    path: String,
    #[serde(default, rename = "type")]
    type_: String,
    #[serde(default)]
    visibility: String,
    #[serde(default)]
    status: String,
    #[serde(default)]
    team: Option<String>,
    #[serde(default)]
    description: String,
    #[serde(default)]
    #[allow(dead_code)]
    company: Option<serde_yaml::Value>,
}

/// `worker.yaml` — the `worker:` block plus top-level `skills` / `instructions`.
#[derive(Debug, Deserialize, Default)]
struct WorkerFile {
    #[serde(default)]
    worker: WorkerBlock,
    /// May be a list of strings or a list of `{name, description}` maps.
    #[serde(default)]
    skills: Option<serde_yaml::Value>,
    /// May be a block-scalar string or a list of strings.
    #[serde(default)]
    instructions: Option<serde_yaml::Value>,
}

#[derive(Debug, Deserialize, Default)]
struct WorkerBlock {
    #[serde(default)]
    id: String,
    #[serde(default)]
    name: String,
    #[serde(default, rename = "type")]
    type_: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    team: Option<String>,
}

/// `SKILL.md` frontmatter — `name`, `description`, optional `allowed-tools`.
#[derive(Debug, Deserialize, Default)]
struct SkillFrontmatter {
    #[serde(default)]
    name: String,
    #[serde(default)]
    description: String,
    /// May be a YAML list or a comma-separated string.
    #[serde(default, rename = "allowed-tools")]
    allowed_tools: Option<serde_yaml::Value>,
}

// ---- HQ folder resolution (mirrors projects_local.rs) ----------------------

/// Resolve the user's HQ folder using the standard 4-tier resolver.
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

// ---- commands --------------------------------------------------------------

/// List the ROOT/shared library: public workers from `registry.yaml` plus skills
/// from `.claude/skills/*` (scope `root`) and `personal/skills/*` (scope
/// `personal`). Empty (not an error) when nothing resolves.
#[tauri::command]
pub async fn get_library_root() -> Result<LibraryItems, String> {
    if !crate::util::feature_gate::is_indigo_user().await {
        return Err("library reader is Indigo-only".to_string());
    }
    let hq = resolve_hq_folder();
    Ok(scan_root_library(&hq))
}

/// List a single company's library: its private workers (registry entries whose
/// `path` is under `companies/<slug>/workers/`) plus `companies/<slug>/skills/*`.
#[tauri::command]
pub async fn get_library_company(company_slug: String) -> Result<LibraryItems, String> {
    if !crate::util::feature_gate::is_indigo_user().await {
        return Err("library reader is Indigo-only".to_string());
    }
    let hq = resolve_hq_folder();
    scan_company_library(&hq, &company_slug)
}

/// Read one worker's `worker.yaml` by its HQ-relative directory path.
#[tauri::command]
pub async fn get_library_worker_detail(worker_path: String) -> Result<WorkerDetail, String> {
    if !crate::util::feature_gate::is_indigo_user().await {
        return Err("library reader is Indigo-only".to_string());
    }
    let hq = resolve_hq_folder();
    read_worker_detail(&hq, &worker_path)
}

/// Read one skill's `SKILL.md` (frontmatter + body) by its HQ-relative path.
#[tauri::command]
pub async fn get_library_skill_detail(skill_path: String) -> Result<SkillDetail, String> {
    if !crate::util::feature_gate::is_indigo_user().await {
        return Err("library reader is Indigo-only".to_string());
    }
    let hq = resolve_hq_folder();
    read_skill_detail(&hq, &skill_path)
}

// ---- pure scanners (explicit HQ root → unit-testable) ----------------------

/// Parse the registry once, returning every entry. Missing/garbage → empty.
fn read_registry(hq_root: &Path) -> Vec<RawWorkerEntry> {
    let path = hq_root.join("core/workers/registry.yaml");
    let Ok(raw) = std::fs::read_to_string(&path) else {
        return Vec::new();
    };
    match serde_yaml::from_str::<RegistryFile>(&raw) {
        Ok(f) => f.workers,
        Err(e) => {
            eprintln!("[library-local] skipping unparseable registry.yaml: {e}");
            Vec::new()
        }
    }
}

/// Derive a company slug from a worker `path` of the form
/// `companies/<slug>/workers/...`. `None` for non-company (e.g. core/) paths.
fn company_slug_from_path(path: &str) -> Option<String> {
    let rest = path.strip_prefix("companies/")?;
    let slug = rest.split('/').next()?;
    if slug.is_empty() {
        None
    } else {
        Some(slug.to_string())
    }
}

fn worker_row(entry: &RawWorkerEntry, scope: &str, company: Option<String>) -> LibraryWorker {
    let name = if entry.id.trim().is_empty() {
        entry.path.clone()
    } else {
        entry.id.clone()
    };
    LibraryWorker {
        id: entry.id.clone(),
        name,
        type_: entry.type_.clone(),
        description: entry.description.clone(),
        scope: scope.to_string(),
        company,
        status: entry.status.clone(),
        path: entry.path.clone(),
        team: entry.team.clone(),
    }
}

fn scan_root_library(hq_root: &Path) -> LibraryItems {
    let workers = read_registry(hq_root)
        .iter()
        .filter(|e| e.visibility == "public")
        .map(|e| worker_row(e, "root", None))
        .collect();

    let mut skills = scan_skills_dir(hq_root, &hq_root.join(".claude/skills"), "root", None);
    skills.extend(scan_skills_dir(
        hq_root,
        &hq_root.join("personal/skills"),
        "personal",
        None,
    ));

    LibraryItems { workers, skills }
}

fn scan_company_library(hq_root: &Path, company_slug: &str) -> Result<LibraryItems, String> {
    let slug = validate_slug(company_slug)?;

    let prefix = format!("companies/{slug}/workers/");
    let workers = read_registry(hq_root)
        .iter()
        .filter(|e| e.path.starts_with(&prefix))
        .map(|e| worker_row(e, "company", Some(slug.clone())))
        .collect();

    let skills_dir = hq_root.join("companies").join(&slug).join("skills");
    let skills = scan_skills_dir(hq_root, &skills_dir, "company", Some(slug.clone()));

    Ok(LibraryItems { workers, skills })
}

/// Scan one skills directory (one level deep). Each child dir's `SKILL.md` is
/// read (following symlinks transparently). Dirs whose name starts with `_` or
/// `.` are skipped. An unreadable/garbage `SKILL.md` is skipped, never fatal.
fn scan_skills_dir(
    hq_root: &Path,
    skills_dir: &Path,
    scope: &str,
    company: Option<String>,
) -> Vec<LibrarySkill> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(skills_dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let dir_name = match entry.file_name().into_string() {
            Ok(n) => n,
            Err(_) => continue,
        };
        if dir_name.starts_with('_') || dir_name.starts_with('.') {
            continue;
        }
        // `.join("SKILL.md")` + read_to_string follows a symlinked skill dir.
        let skill_md = entry.path().join("SKILL.md");
        let Ok(raw) = std::fs::read_to_string(&skill_md) else {
            continue;
        };
        let (front_yaml, _body) = split_frontmatter(&raw);
        let front = front_yaml
            .and_then(|y| serde_yaml::from_str::<SkillFrontmatter>(y).ok())
            .unwrap_or_default();
        let name = if front.name.trim().is_empty() {
            dir_name.clone()
        } else {
            front.name.clone()
        };
        let rel = match skill_md.strip_prefix(hq_root) {
            Ok(r) => r.to_string_lossy().replace('\\', "/"),
            Err(_) => continue,
        };
        out.push(LibrarySkill {
            name,
            description: front.description.clone(),
            scope: scope.to_string(),
            company: company.clone(),
            path: rel,
            allowed_tools: normalize_tools(front.allowed_tools.as_ref()),
        });
    }
    out
}

fn read_worker_detail(hq_root: &Path, worker_path: &str) -> Result<WorkerDetail, String> {
    let rel = worker_path.trim();
    if rel.is_empty() {
        return Err("worker_path is required".to_string());
    }
    let dir = hq_root.join(rel);
    if !is_within(hq_root, &dir) {
        return Err(format!("worker_path escapes the HQ folder: {worker_path:?}"));
    }
    let yaml_path = dir.join("worker.yaml");
    if !is_within(hq_root, &yaml_path) {
        return Err("worker.yaml path escapes the HQ folder".to_string());
    }
    let raw = std::fs::read_to_string(&yaml_path)
        .map_err(|e| format!("could not read worker.yaml at {worker_path:?}: {e}"))?;
    let parsed: WorkerFile = serde_yaml::from_str(&raw)
        .map_err(|e| format!("worker.yaml at {worker_path:?} is not valid YAML: {e}"))?;

    Ok(WorkerDetail {
        id: parsed.worker.id,
        name: parsed.worker.name,
        type_: parsed.worker.type_,
        description: parsed.worker.description,
        team: parsed.worker.team,
        skills: normalize_worker_skills(parsed.skills.as_ref()),
        instructions: normalize_instructions(parsed.instructions.as_ref()),
    })
}

fn read_skill_detail(hq_root: &Path, skill_path: &str) -> Result<SkillDetail, String> {
    let rel = skill_path.trim();
    if rel.is_empty() {
        return Err("skill_path is required".to_string());
    }
    let abs = hq_root.join(rel);
    if !is_within(hq_root, &abs) {
        return Err(format!("skill_path escapes the HQ folder: {skill_path:?}"));
    }
    if abs.file_name().and_then(|n| n.to_str()) != Some("SKILL.md") {
        return Err("skill_path must point at a SKILL.md file".to_string());
    }
    let raw = std::fs::read_to_string(&abs)
        .map_err(|e| format!("could not read SKILL.md at {skill_path:?}: {e}"))?;
    let (front_yaml, body) = split_frontmatter(&raw);
    let front = front_yaml
        .and_then(|y| serde_yaml::from_str::<SkillFrontmatter>(y).ok())
        .unwrap_or_default();
    Ok(SkillDetail {
        name: front.name,
        description: front.description,
        allowed_tools: normalize_tools(front.allowed_tools.as_ref()),
        body: body.trim_start_matches('\n').to_string(),
    })
}

// ---- parse helpers ---------------------------------------------------------

/// Split a `SKILL.md` into `(frontmatter_yaml, body)`. When the file starts with
/// a `---` fence, returns the YAML between the first two fences plus the
/// remainder as body. With no fence, frontmatter is `None` and the whole file is
/// the body.
fn split_frontmatter(raw: &str) -> (Option<&str>, &str) {
    // Normalize only for the leading-fence check; we slice the original.
    let trimmed_start = raw.trim_start_matches('\u{feff}');
    let offset = raw.len() - trimmed_start.len();
    let after_bom = &raw[offset..];

    if !(after_bom.starts_with("---\n") || after_bom.starts_with("---\r\n")) {
        return (None, raw);
    }
    // Position just after the opening fence line.
    let first_nl = match after_bom.find('\n') {
        Some(p) => p + 1,
        None => return (None, raw),
    };
    let rest = &after_bom[first_nl..];
    // Find a line that is exactly `---` (closing fence).
    let mut search_base = 0usize;
    for line in rest.split_inclusive('\n') {
        let line_trimmed = line.trim_end_matches(['\r', '\n']);
        if line_trimmed == "---" {
            let yaml = &rest[..search_base];
            let body_start = search_base + line.len();
            let body = &rest[body_start..];
            return (Some(yaml), body);
        }
        search_base += line.len();
    }
    // Unterminated frontmatter → treat whole file as body.
    (None, raw)
}

/// Normalize an `allowed-tools` value (YAML list OR comma-separated string) to a
/// clean `Vec<String>`.
fn normalize_tools(value: Option<&serde_yaml::Value>) -> Vec<String> {
    match value {
        Some(serde_yaml::Value::String(s)) => s
            .split(',')
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty())
            .collect(),
        Some(serde_yaml::Value::Sequence(seq)) => seq
            .iter()
            .filter_map(|v| v.as_str().map(|s| s.trim().to_string()))
            .filter(|t| !t.is_empty())
            .collect(),
        _ => Vec::new(),
    }
}

/// Normalize a worker's `skills` (list of strings OR list of `{name,description}`
/// maps) into `Vec<WorkerSkillRef>`.
fn normalize_worker_skills(value: Option<&serde_yaml::Value>) -> Vec<WorkerSkillRef> {
    let Some(serde_yaml::Value::Sequence(seq)) = value else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for item in seq {
        match item {
            serde_yaml::Value::String(s) => {
                if !s.trim().is_empty() {
                    out.push(WorkerSkillRef {
                        name: s.trim().to_string(),
                        description: None,
                    });
                }
            }
            serde_yaml::Value::Mapping(_) => {
                let name = item
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim()
                    .to_string();
                if name.is_empty() {
                    continue;
                }
                let description = item
                    .get("description")
                    .and_then(|v| v.as_str())
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty());
                out.push(WorkerSkillRef { name, description });
            }
            _ => {}
        }
    }
    out
}

/// Normalize a worker's `instructions` (block-scalar string OR list of bullet
/// strings) into a single markdown string.
fn normalize_instructions(value: Option<&serde_yaml::Value>) -> String {
    match value {
        Some(serde_yaml::Value::String(s)) => s.clone(),
        Some(serde_yaml::Value::Sequence(seq)) => seq
            .iter()
            .filter_map(|v| v.as_str())
            .map(|s| format!("- {}", s.trim()))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

/// Validate a company slug is a single safe directory name.
fn validate_slug(company_slug: &str) -> Result<String, String> {
    let slug = company_slug.trim();
    if slug.is_empty() {
        return Err("company_slug is required".to_string());
    }
    if slug.contains('/') || slug.contains('\\') || slug == "." || slug == ".." {
        return Err(format!("invalid company_slug: {company_slug:?}"));
    }
    Ok(slug.to_string())
}

// ---- path-traversal guard (mirrors projects_local.rs) ----------------------

/// True iff `candidate`, after lexical normalization, is contained within
/// `root`. Rejects `..` traversal and absolute escapes without touching the
/// filesystem (works on non-existent paths and doesn't chase symlinks).
fn is_within(root: &Path, candidate: &Path) -> bool {
    let normalized = lexically_normalize(candidate);
    let root_norm = lexically_normalize(root);
    normalized.starts_with(&root_norm)
}

/// Collapse `.` and `..` components lexically. A leading `..` that would escape
/// the prefix is preserved so `is_within` rejects it.
fn lexically_normalize(path: &Path) -> PathBuf {
    let mut stack: Vec<Component> = Vec::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => match stack.last() {
                Some(Component::Normal(_)) => {
                    stack.pop();
                }
                _ => stack.push(component),
            },
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
    fn make_fixture_tree() -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "hq-library-local-test-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            SEQ.fetch_add(1, Ordering::Relaxed),
        ));

        // registry.yaml: a public worker, a private indigo worker, and a
        // templated entry whose `company:` is a MAP (must not break parsing).
        let workers_dir = root.join("core/workers");
        fs::create_dir_all(&workers_dir).unwrap();
        let registry = r#"version: "5.0"
workers:
  - id: architect
    path: core/workers/public/dev-team/architect/
    type: CodeWorker
    visibility: public
    team: dev-team
    status: active
    description: "System design"
  - id: cmo-indigo
    path: companies/indigo/workers/cmo/
    type: OpsWorker
    visibility: private
    company: indigo
    status: active
    description: "Indigo CMO"
  - id: "{product}-gtm"
    path: companies/liverecover/workers/gtm/
    type: OpsWorker
    visibility: private
    company: {product: ''}
    status: active
    description: "templated"
"#;
        fs::write(workers_dir.join("registry.yaml"), registry).unwrap();

        // A worker.yaml for the indigo cmo (skills as maps, instructions as list).
        let cmo = root.join("companies/indigo/workers/cmo");
        fs::create_dir_all(&cmo).unwrap();
        let cmo_yaml = r#"worker:
  id: cmo-indigo
  name: "CMO Worker - Indigo"
  type: OpsWorker
  company: indigo
  description: "Indigo CMO"
instructions:
  - "Do the first thing."
  - "Then the second thing."
skills:
  - name: content-calendar
    description: "Plan weekly content"
  - name: draft-post
"#;
        fs::write(cmo.join("worker.yaml"), cmo_yaml).unwrap();

        // A worker.yaml with a block-scalar instructions + string skills.
        let arch = root.join("core/workers/public/dev-team/architect");
        fs::create_dir_all(&arch).unwrap();
        let arch_yaml = "worker:\n  id: architect\n  name: \"Architect\"\n  type: CodeWorker\n  team: dev-team\n  description: \"System design\"\ninstructions: |\n  # Architect\n\n  Design things.\n";
        fs::write(arch.join("worker.yaml"), arch_yaml).unwrap();

        // Root skills: one normal, one with allowed-tools as a string, plus a
        // `_shared` dir that must be skipped.
        let claude_skills = root.join(".claude/skills");
        fs::create_dir_all(claude_skills.join("run")).unwrap();
        fs::write(
            claude_skills.join("run/SKILL.md"),
            "---\nname: run\ndescription: Run a worker\nallowed-tools: Read, Grep, Bash\n---\n\n# Run\n\nBody here.\n",
        )
        .unwrap();
        fs::create_dir_all(claude_skills.join("plan")).unwrap();
        fs::write(
            claude_skills.join("plan/SKILL.md"),
            "---\nname: plan\ndescription: Plan a project\n---\n\n# Plan\n",
        )
        .unwrap();
        fs::create_dir_all(claude_skills.join("_shared")).unwrap();
        fs::write(claude_skills.join("_shared/SKILL.md"), "should be skipped").unwrap();

        // Personal skills.
        let personal_skills = root.join("personal/skills");
        fs::create_dir_all(personal_skills.join("impeccable")).unwrap();
        fs::write(
            personal_skills.join("impeccable/SKILL.md"),
            "---\nname: impeccable\ndescription: Improve a UI\nallowed-tools:\n  - Read\n  - Edit\n---\n\nBody.\n",
        )
        .unwrap();

        // Company (indigo) skills.
        let indigo_skills = root.join("companies/indigo/skills");
        fs::create_dir_all(indigo_skills.join("signals")).unwrap();
        fs::write(
            indigo_skills.join("signals/SKILL.md"),
            "---\nname: signals\ndescription: Surface action items\n---\n\n# Signals\n",
        )
        .unwrap();

        root
    }

    #[test]
    fn root_library_filters_public_workers_and_reads_skills() {
        let root = make_fixture_tree();
        let items = scan_root_library(&root);

        // Only the public worker (architect) is in the root scope.
        assert_eq!(items.workers.len(), 1);
        assert_eq!(items.workers[0].id, "architect");
        assert_eq!(items.workers[0].scope, "root");
        assert_eq!(items.workers[0].type_, "CodeWorker");
        assert_eq!(items.workers[0].team.as_deref(), Some("dev-team"));

        // Root + personal skills, `_shared` skipped.
        let names: Vec<_> = items.skills.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"run"));
        assert!(names.contains(&"plan"));
        assert!(names.contains(&"impeccable"));
        assert!(!names.contains(&"_shared"));

        let run = items.skills.iter().find(|s| s.name == "run").unwrap();
        assert_eq!(run.scope, "root");
        assert_eq!(run.allowed_tools, vec!["Read", "Grep", "Bash"]);

        let imp = items.skills.iter().find(|s| s.name == "impeccable").unwrap();
        assert_eq!(imp.scope, "personal");
        assert_eq!(imp.allowed_tools, vec!["Read", "Edit"]);

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn company_library_filters_by_path_prefix() {
        let root = make_fixture_tree();
        let items = scan_company_library(&root, "indigo").expect("company library");

        assert_eq!(items.workers.len(), 1);
        assert_eq!(items.workers[0].id, "cmo-indigo");
        assert_eq!(items.workers[0].scope, "company");
        assert_eq!(items.workers[0].company.as_deref(), Some("indigo"));

        assert_eq!(items.skills.len(), 1);
        assert_eq!(items.skills[0].name, "signals");
        assert_eq!(items.skills[0].scope, "company");
        assert_eq!(items.skills[0].company.as_deref(), Some("indigo"));

        // A company with no workers/skills dirs → empty, not an error.
        let empty = scan_company_library(&root, "acme").expect("empty company");
        assert!(empty.workers.is_empty());
        assert!(empty.skills.is_empty());

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn company_library_rejects_bad_slug() {
        let root = make_fixture_tree();
        for evil in ["../../etc", "..", ".", "foo/bar", "in\\digo"] {
            assert!(scan_company_library(&root, evil).is_err(), "slug {evil:?}");
        }
        assert!(scan_company_library(&root, "  ").is_err());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn worker_detail_normalizes_list_instructions_and_map_skills() {
        let root = make_fixture_tree();
        let detail =
            read_worker_detail(&root, "companies/indigo/workers/cmo/").expect("cmo detail");
        assert_eq!(detail.name, "CMO Worker - Indigo");
        assert_eq!(detail.type_, "OpsWorker");
        assert_eq!(detail.skills.len(), 2);
        assert_eq!(detail.skills[0].name, "content-calendar");
        assert_eq!(detail.skills[0].description.as_deref(), Some("Plan weekly content"));
        assert_eq!(detail.skills[1].name, "draft-post");
        assert!(detail.skills[1].description.is_none());
        // List instructions become markdown bullets.
        assert!(detail.instructions.contains("- Do the first thing."));
        assert!(detail.instructions.contains("- Then the second thing."));

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn worker_detail_block_scalar_instructions() {
        let root = make_fixture_tree();
        let detail = read_worker_detail(&root, "core/workers/public/dev-team/architect/")
            .expect("architect detail");
        assert_eq!(detail.name, "Architect");
        assert!(detail.instructions.contains("# Architect"));
        assert!(detail.instructions.contains("Design things."));
        // No top-level `skills` key → empty.
        assert!(detail.skills.is_empty());

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn skill_detail_splits_frontmatter_and_body() {
        let root = make_fixture_tree();
        let detail = read_skill_detail(&root, ".claude/skills/run/SKILL.md").expect("run detail");
        assert_eq!(detail.name, "run");
        assert_eq!(detail.description, "Run a worker");
        assert_eq!(detail.allowed_tools, vec!["Read", "Grep", "Bash"]);
        assert!(detail.body.contains("# Run"));
        assert!(detail.body.contains("Body here."));
        // Frontmatter must not leak into the body.
        assert!(!detail.body.contains("allowed-tools"));

        // A skill with no allowed-tools still parses.
        let plan = read_skill_detail(&root, ".claude/skills/plan/SKILL.md").expect("plan detail");
        assert_eq!(plan.name, "plan");
        assert!(plan.allowed_tools.is_empty());

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn detail_rejects_traversal_and_wrong_target() {
        let root = make_fixture_tree();
        for evil in ["../../../etc/passwd", "companies/../../secrets/"] {
            assert!(read_worker_detail(&root, evil).is_err(), "worker {evil:?}");
        }
        for evil in [
            "../../../etc/passwd",
            ".claude/skills/run/notes.txt",
            "companies/indigo/board.json",
        ] {
            assert!(read_skill_detail(&root, evil).is_err(), "skill {evil:?}");
        }
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn missing_registry_is_empty_not_panic() {
        let root = std::env::temp_dir().join(format!(
            "hq-library-local-empty-{}",
            std::process::id()
        ));
        let _ = fs::create_dir_all(&root);
        let items = scan_root_library(&root);
        assert!(items.workers.is_empty());
        assert!(items.skills.is_empty());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn split_frontmatter_handles_no_fence() {
        let (front, body) = split_frontmatter("# Just markdown\n\nNo frontmatter.");
        assert!(front.is_none());
        assert!(body.contains("# Just markdown"));
    }
}
