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
//! command is gated by `feature_gate::desktop_features_enabled()` (GA — any
//! signed-in user), resolves the HQ folder
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
    /// The HQ pack a skill ships in, when its dir is a symlink into
    /// `core/packages/hq-pack-<pack>/skills/` (e.g. `engineering`). `None` for
    /// hand-authored skills that live directly under the scanned dir.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pack: Option<String>,
}

/// Combined library payload for one scope (root or a company).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LibraryItems {
    pub workers: Vec<LibraryWorker>,
    pub skills: Vec<LibrarySkill>,
}

/// Pack/worker/skill authorship attribution (US-001). Optional everywhere —
/// legacy worker.yaml / SKILL.md without an `author` block deserialize fine
/// (every field is `#[serde(default)]` and the whole struct is wrapped in an
/// `Option`), so this is strictly backwards-compatible.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Author {
    #[serde(default)]
    pub uid: String,
    #[serde(default)]
    pub handle: String,
    #[serde(default)]
    pub display_name: String,
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
    /// Optional authorship attribution (US-001). Absent on legacy workers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<Author>,
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
    /// Optional authorship attribution (US-001). Absent on legacy skills.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<Author>,
    /// The markdown body after the frontmatter fence — rendered by the frontend.
    #[serde(default)]
    pub body: String,
}

// ---- on-disk parse models --------------------------------------------------

/// One registry entry, built by hand from `serde_yaml::Value` (see
/// `read_registry`) rather than `Deserialize`d directly. The registry is
/// auto-generated and includes *template* entries whose `id`/`company` are YAML
/// placeholders like `{product}-gtm` — which YAML parses as a **map**, not a
/// string. A struct-level `Deserialize` aborts the WHOLE file on the first such
/// entry (the bug behind "0 workers"); pulling fields leniently per entry keeps
/// every well-formed worker and coerces a non-string id to the path's dir name.
#[derive(Debug, Default, Clone)]
struct RawWorkerEntry {
    id: String,
    path: String,
    type_: String,
    visibility: String,
    status: String,
    team: Option<String>,
    description: String,
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
    status: String,
    #[serde(default)]
    team: Option<String>,
    /// Optional authorship (US-001). Absent on legacy worker.yaml.
    #[serde(default)]
    author: Option<Author>,
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
    /// Optional authorship (US-001). Absent on legacy SKILL.md frontmatter.
    #[serde(default)]
    author: Option<Author>,
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
    if !crate::util::feature_gate::desktop_features_enabled().await {
        return Err("library reader requires a signed-in user".to_string());
    }
    let hq = resolve_hq_folder();
    Ok(scan_root_library(&hq))
}

/// List a single company's library: its private workers (registry entries whose
/// `path` is under `companies/<slug>/workers/`) plus `companies/<slug>/skills/*`.
#[tauri::command]
pub async fn get_library_company(company_slug: String) -> Result<LibraryItems, String> {
    if !crate::util::feature_gate::desktop_features_enabled().await {
        return Err("library reader requires a signed-in user".to_string());
    }
    let hq = resolve_hq_folder();
    scan_company_library(&hq, &company_slug)
}

/// Read one worker's `worker.yaml` by its HQ-relative directory path.
#[tauri::command]
pub async fn get_library_worker_detail(worker_path: String) -> Result<WorkerDetail, String> {
    if !crate::util::feature_gate::desktop_features_enabled().await {
        return Err("library reader requires a signed-in user".to_string());
    }
    let hq = resolve_hq_folder();
    read_worker_detail(&hq, &worker_path)
}

/// Read one skill's `SKILL.md` (frontmatter + body) by its HQ-relative path.
#[tauri::command]
pub async fn get_library_skill_detail(skill_path: String) -> Result<SkillDetail, String> {
    if !crate::util::feature_gate::desktop_features_enabled().await {
        return Err("library reader requires a signed-in user".to_string());
    }
    let hq = resolve_hq_folder();
    read_skill_detail(&hq, &skill_path)
}

// ---- pure scanners (explicit HQ root → unit-testable) ----------------------

/// Parse the registry into entries, tolerating per-entry malformations.
///
/// We deserialize to a generic `serde_yaml::Value` and pull each worker's fields
/// by hand instead of `#[derive(Deserialize)]`-ing a typed struct. The registry
/// ships template entries (`id: {product}-gtm`, `company: {product}`) whose
/// values YAML reads as **maps**; a typed deserialize aborts the entire file on
/// the first one — the root cause of "0 workers". Here a non-string `id` simply
/// falls back to the worker dir's name (from `path`), so real company workers
/// with templated ids still surface. Missing file / non-YAML → empty.
fn read_registry(hq_root: &Path) -> Vec<RawWorkerEntry> {
    let path = hq_root.join("core/workers/registry.yaml");
    let Ok(raw) = std::fs::read_to_string(&path) else {
        return Vec::new();
    };
    let sanitized = sanitize_registry(&raw);
    let doc: serde_yaml::Value = match serde_yaml::from_str(&sanitized) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("[library-local] registry.yaml is not valid YAML: {e}");
            return Vec::new();
        }
    };
    let Some(workers) = doc.get("workers").and_then(|w| w.as_sequence()) else {
        return Vec::new();
    };

    let str_field = |w: &serde_yaml::Value, key: &str| -> String {
        w.get(key)
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string()
    };

    let mut out = Vec::new();
    for w in workers {
        let path = str_field(w, "path");
        // A plain id; else (empty, or a `{product}`-style placeholder left after
        // sanitizing) derive the dir name from the path so the entry still gets a
        // readable label instead of an ugly template token.
        let raw_id = w.get("id").and_then(|v| v.as_str()).unwrap_or("").trim();
        let id = if raw_id.is_empty() || raw_id.contains('{') {
            last_path_segment(&path)
        } else {
            raw_id.to_string()
        };
        let team = w
            .get("team")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .filter(|s| !s.trim().is_empty());
        out.push(RawWorkerEntry {
            id,
            path,
            type_: str_field(w, "type"),
            visibility: str_field(w, "visibility"),
            status: str_field(w, "status"),
            team,
            description: str_field(w, "description"),
        });
    }
    out
}

/// Neutralize the registry's *template* placeholder values before YAML parsing.
///
/// The generator emits entries like `id: {product}-gtm` and `company: {product}`.
/// `{...}` opens a YAML flow mapping, and the trailing `-gtm` makes the line
/// syntactically invalid — which fails the ENTIRE document parse (the "0 workers"
/// bug). We can't fix this per-entry because the failure happens before any entry
/// is visible. So we pre-process line-by-line: any `id:`/`company:` whose value
/// starts with `{` is wrapped in double quotes, turning the placeholder into a
/// harmless string. Real entries (plain strings) are untouched. Downstream,
/// `read_registry` derives a readable name from the path for any id still
/// carrying a `{` placeholder.
fn sanitize_registry(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len() + 64);
    for line in raw.split_inclusive('\n') {
        out.push_str(&sanitize_registry_line(line));
    }
    out
}

/// Quote a single `id:`/`company:` line whose value is a `{...}` placeholder.
/// Returns the line unchanged when it doesn't match. Preserves indentation, an
/// optional `- ` list prefix, and the trailing newline.
fn sanitize_registry_line(line: &str) -> String {
    let (content, nl) = match line.strip_suffix('\n') {
        Some(c) => (c, "\n"),
        None => (line, ""),
    };
    let indent_len = content.len() - content.trim_start().len();
    let (indent, after_indent) = content.split_at(indent_len);
    let (prefix, rest) = match after_indent.strip_prefix("- ") {
        Some(r) => ("- ", r),
        None => ("", after_indent),
    };
    for key in ["id:", "company:"] {
        if let Some(after_key) = rest.strip_prefix(key) {
            let val = after_key.trim_start();
            // Already-quoted or non-placeholder values are left alone.
            if val.starts_with('{') {
                let lead_len = after_key.len() - val.len();
                let lead = &after_key[..lead_len];
                let escaped = val.replace('\\', "\\\\").replace('"', "\\\"");
                return format!("{indent}{prefix}{key}{lead}\"{escaped}\"{nl}");
            }
            return format!("{content}{nl}");
        }
    }
    format!("{content}{nl}")
}

/// Last non-empty path segment (worker dir name) from a trailing-slash path like
/// `companies/liverecover/workers/gtm/` → `gtm`. Empty string when none.
fn last_path_segment(path: &str) -> String {
    path.trim_end_matches('/')
        .rsplit('/')
        .find(|s| !s.is_empty())
        .unwrap_or("")
        .to_string()
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

fn push_unique_worker(workers: &mut Vec<LibraryWorker>, worker: LibraryWorker) {
    let next_path = worker.path.trim_end_matches('/');
    if workers
        .iter()
        .any(|existing| existing.path.trim_end_matches('/') == next_path)
    {
        return;
    }
    workers.push(worker);
}

/// Scan `worker.yaml` files from the real filesystem as a fallback/augmentation
/// to the generated registry. The registry can be stale or locally malformed,
/// but each worker directory is still the source of detail truth.
fn scan_worker_yaml_dir(
    hq_root: &Path,
    workers_dir: &Path,
    scope: &str,
    company: Option<String>,
) -> Vec<LibraryWorker> {
    let mut yaml_files = Vec::new();
    collect_worker_yaml_files(workers_dir, &mut yaml_files);
    yaml_files.sort();

    yaml_files
        .iter()
        .filter_map(|path| worker_from_yaml(hq_root, path, scope, company.clone()))
        .collect()
}

fn collect_worker_yaml_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if name.starts_with('.') || name.starts_with('_') {
            continue;
        }

        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_file() && name == "worker.yaml" {
            out.push(path);
            continue;
        }
        if file_type.is_dir() {
            collect_worker_yaml_files(&path, out);
        }
    }
}

fn worker_from_yaml(
    hq_root: &Path,
    yaml_path: &Path,
    scope: &str,
    company: Option<String>,
) -> Option<LibraryWorker> {
    if yaml_path.file_name().and_then(|n| n.to_str()) != Some("worker.yaml") {
        return None;
    }
    if !is_within(hq_root, yaml_path) {
        return None;
    }

    let raw = std::fs::read_to_string(yaml_path).ok()?;
    let parsed: WorkerFile = serde_yaml::from_str(&raw).ok()?;
    let dir = yaml_path.parent()?;
    let rel = dir
        .strip_prefix(hq_root)
        .ok()?
        .to_string_lossy()
        .replace('\\', "/");
    let path = if rel.ends_with('/') {
        rel
    } else {
        format!("{rel}/")
    };
    let fallback_id = last_path_segment(&path);
    let id = if parsed.worker.id.trim().is_empty() {
        fallback_id
    } else {
        parsed.worker.id.clone()
    };
    let name = if parsed.worker.name.trim().is_empty() {
        id.clone()
    } else {
        parsed.worker.name.clone()
    };
    let status = if parsed.worker.status.trim().is_empty() {
        "active".to_string()
    } else {
        parsed.worker.status.clone()
    };

    Some(LibraryWorker {
        id,
        name,
        type_: parsed.worker.type_.clone(),
        description: parsed.worker.description.clone(),
        scope: scope.to_string(),
        company,
        status,
        path,
        team: parsed.worker.team.clone(),
    })
}

/// The ROOT library is an ALL-SCOPES view: core (public workers + `.claude/skills`),
/// the personal overlay (`personal/skills`), AND every company's private workers +
/// company-scoped skills. The desktop UI narrows it with a client-side facet
/// filter (Core / Personal / per-company), so the backend just returns the union.
fn scan_root_library(hq_root: &Path) -> LibraryItems {
    let registry = read_registry(hq_root);

    // Core: public/shared workers + the root + personal skill dirs.
    let mut workers: Vec<LibraryWorker> = registry
        .iter()
        .filter(|e| e.visibility == "public")
        .map(|e| worker_row(e, "root", None))
        .collect();
    for worker in scan_worker_yaml_dir(hq_root, &hq_root.join("core/workers/public"), "root", None)
    {
        push_unique_worker(&mut workers, worker);
    }
    let mut skills = scan_skills_dir(hq_root, &hq_root.join(".claude/skills"), "root", None);
    skills.extend(scan_skills_dir(
        hq_root,
        &hq_root.join("personal/skills"),
        "personal",
        None,
    ));

    // Every company's private workers (by registry path prefix) + its skills.
    for slug in company_slugs(hq_root, &registry) {
        let prefix = format!("companies/{slug}/workers/");
        for e in registry.iter().filter(|e| e.path.starts_with(&prefix)) {
            push_unique_worker(&mut workers, worker_row(e, "company", Some(slug.clone())));
        }
        let workers_dir = hq_root.join("companies").join(&slug).join("workers");
        for worker in scan_worker_yaml_dir(hq_root, &workers_dir, "company", Some(slug.clone())) {
            push_unique_worker(&mut workers, worker);
        }
        let skills_dir = hq_root.join("companies").join(&slug).join("skills");
        skills.extend(scan_skills_dir(
            hq_root,
            &skills_dir,
            "company",
            Some(slug.clone()),
        ));
    }

    LibraryItems { workers, skills }
}

/// The set of company slugs to fan the root library across: every real dir under
/// `companies/` (skipping hidden/`_` entries and non-dirs like `manifest.yaml`),
/// unioned with any slug that appears in a registry worker path (so a company
/// with workers but no on-disk skills dir is still represented). Sorted.
fn company_slugs(hq_root: &Path, registry: &[RawWorkerEntry]) -> Vec<String> {
    let mut set: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    if let Ok(entries) = std::fs::read_dir(hq_root.join("companies")) {
        for entry in entries.flatten() {
            if !entry.path().is_dir() {
                continue;
            }
            if let Some(name) = entry.file_name().to_str() {
                if !name.starts_with('.') && !name.starts_with('_') {
                    set.insert(name.to_string());
                }
            }
        }
    }
    for e in registry {
        if let Some(slug) = slug_from_worker_path(&e.path) {
            set.insert(slug);
        }
    }
    set.into_iter().collect()
}

/// Company slug from a `companies/<slug>/workers/...` registry path. `None` for
/// core/ paths.
fn slug_from_worker_path(path: &str) -> Option<String> {
    let rest = path.strip_prefix("companies/")?;
    let slug = rest.split('/').next()?;
    if slug.is_empty() {
        None
    } else {
        Some(slug.to_string())
    }
}

fn scan_company_library(hq_root: &Path, company_slug: &str) -> Result<LibraryItems, String> {
    let slug = validate_slug(company_slug)?;

    let prefix = format!("companies/{slug}/workers/");
    let mut workers: Vec<LibraryWorker> = read_registry(hq_root)
        .iter()
        .filter(|e| e.path.starts_with(&prefix))
        .map(|e| worker_row(e, "company", Some(slug.clone())))
        .collect();
    let workers_dir = hq_root.join("companies").join(&slug).join("workers");
    for worker in scan_worker_yaml_dir(hq_root, &workers_dir, "company", Some(slug.clone())) {
        push_unique_worker(&mut workers, worker);
    }

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
        // A symlinked skill dir usually points into core/packages/hq-pack-<pack>/;
        // surface the pack so the UI can badge it.
        let pack = detect_pack(&entry.path());
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
            pack,
        });
    }
    out
}

/// If `skill_dir` is a symlink into `core/packages/hq-pack-<pack>/...`, return
/// `<pack>` (e.g. `engineering`). Reads the link target lexically — no
/// canonicalization, so it works on relative `../../core/packages/...` links and
/// doesn't touch unrelated files. `None` when the dir isn't a pack symlink.
fn detect_pack(skill_dir: &Path) -> Option<String> {
    let target = std::fs::read_link(skill_dir).ok()?;
    let s = target.to_string_lossy();
    let after = s.split("hq-pack-").nth(1)?;
    let pack = after.split('/').next().unwrap_or("");
    if pack.is_empty() {
        None
    } else {
        Some(pack.to_string())
    }
}

fn read_worker_detail(hq_root: &Path, worker_path: &str) -> Result<WorkerDetail, String> {
    let rel = worker_path.trim();
    if rel.is_empty() {
        return Err("worker_path is required".to_string());
    }
    let dir = hq_root.join(rel);
    if !is_within(hq_root, &dir) {
        return Err(format!(
            "worker_path escapes the HQ folder: {worker_path:?}"
        ));
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
        author: parsed.worker.author,
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
        author: front.author,
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
  - id: {product}-gtm
    path: companies/liverecover/workers/gtm/
    type: OpsWorker
    visibility: private
    company: {product}
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

        // A packaged skill: a real dir under core/packages/hq-pack-engineering/
        // symlinked into .claude/skills (mirrors how packs surface). detect_pack
        // should read the link and report pack="engineering".
        let pack_skill = root.join("core/packages/hq-pack-engineering/skills/land");
        fs::create_dir_all(&pack_skill).unwrap();
        fs::write(
            pack_skill.join("SKILL.md"),
            "---\nname: land\ndescription: Land a PR\n---\n\n# Land\n",
        )
        .unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&pack_skill, claude_skills.join("land")).unwrap();

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

        // A skill WITH an author block (US-001) — must surface uid/handle/displayName.
        fs::create_dir_all(claude_skills.join("authored")).unwrap();
        fs::write(
            claude_skills.join("authored/SKILL.md"),
            "---\nname: authored\ndescription: Has an author\nauthor:\n  uid: prs_abc123\n  handle: corey\n  displayName: Corey Epstein\n---\n\n# Authored\n",
        )
        .unwrap();

        // A worker.yaml WITH an author block (US-001).
        let authored_worker = root.join("core/workers/public/dev-team/authored");
        fs::create_dir_all(&authored_worker).unwrap();
        fs::write(
            authored_worker.join("worker.yaml"),
            "worker:\n  id: authored\n  name: \"Authored\"\n  type: CodeWorker\n  team: dev-team\n  description: \"Has an author\"\n  author:\n    uid: prs_abc123\n    handle: corey\n    displayName: Corey Epstein\n",
        )
        .unwrap();

        root
    }

    #[test]
    fn root_library_aggregates_all_scopes() {
        let root = make_fixture_tree();
        let items = scan_root_library(&root);

        // Core worker is present and scoped "root".
        let architect = items.workers.iter().find(|w| w.id == "architect").unwrap();
        assert_eq!(architect.scope, "root");
        assert_eq!(architect.type_, "CodeWorker");
        assert_eq!(architect.team.as_deref(), Some("dev-team"));

        // Company workers now surface on root too, scoped to their company.
        let cmo = items.workers.iter().find(|w| w.id == "cmo-indigo").unwrap();
        assert_eq!(cmo.scope, "company");
        assert_eq!(cmo.company.as_deref(), Some("indigo"));
        let gtm = items.workers.iter().find(|w| w.id == "gtm").unwrap();
        assert_eq!(gtm.company.as_deref(), Some("liverecover"));

        // Core + personal + company skills, `_shared` skipped.
        let names: Vec<_> = items.skills.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"run"));
        assert!(names.contains(&"plan"));
        assert!(names.contains(&"impeccable"));
        assert!(names.contains(&"signals")); // company (indigo) skill on root
        assert!(!names.contains(&"_shared"));
        // The indigo skill carries its company scope.
        let signals = items.skills.iter().find(|s| s.name == "signals").unwrap();
        assert_eq!(signals.scope, "company");
        assert_eq!(signals.company.as_deref(), Some("indigo"));

        let run = items.skills.iter().find(|s| s.name == "run").unwrap();
        assert_eq!(run.scope, "root");
        assert_eq!(run.allowed_tools, vec!["Read", "Grep", "Bash"]);

        let imp = items
            .skills
            .iter()
            .find(|s| s.name == "impeccable")
            .unwrap();
        assert_eq!(imp.scope, "personal");
        assert_eq!(imp.allowed_tools, vec!["Read", "Edit"]);

        // A symlinked packaged skill is read and its pack detected.
        #[cfg(unix)]
        {
            let land = items.skills.iter().find(|s| s.name == "land").unwrap();
            assert_eq!(land.pack.as_deref(), Some("engineering"));
            // A hand-authored skill carries no pack.
            assert!(run.pack.is_none());
        }

        let _ = fs::remove_dir_all(&root);
    }

    /// Regression for the "0 workers" bug: an UNQUOTED template entry
    /// (`id: {product}-gtm`) is YAML-parsed as a map, which previously aborted
    /// the whole registry deserialize and blanked every worker. The reader must
    /// now tolerate it (deriving the name from the path) and keep all the
    /// well-formed entries.
    #[test]
    fn templated_registry_entry_does_not_blank_the_list() {
        let root = make_fixture_tree();

        // Root still sees the public worker despite the malformed template entry,
        // and the templated company worker surfaces (path-derived name "gtm").
        let root_items = scan_root_library(&root);
        assert!(root_items.workers.iter().any(|w| w.id == "architect"));
        assert!(root_items
            .workers
            .iter()
            .any(|w| w.id == "gtm" && w.company.as_deref() == Some("liverecover")));

        // The templated liverecover worker survives under its company, with a
        // path-derived name instead of the raw `{product}` placeholder.
        let lr = scan_company_library(&root, "liverecover").expect("liverecover");
        assert_eq!(lr.workers.len(), 1);
        assert_eq!(lr.workers[0].id, "gtm");
        assert_eq!(lr.workers[0].name, "gtm");
        assert_eq!(lr.workers[0].company.as_deref(), Some("liverecover"));

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn worker_yaml_scan_keeps_library_visible_when_registry_is_broken() {
        let root = make_fixture_tree();
        fs::write(
            root.join("core/workers/registry.yaml"),
            "workers:\n  - id: broken\n    path:\n    type:\n    visibility:\n    status:\n",
        )
        .unwrap();

        let root_items = scan_root_library(&root);
        let architect = root_items
            .workers
            .iter()
            .find(|w| w.path == "core/workers/public/dev-team/architect/")
            .expect("root worker from worker.yaml");
        assert_eq!(architect.id, "architect");
        assert_eq!(architect.scope, "root");
        assert_eq!(architect.status, "active");

        let cmo = root_items
            .workers
            .iter()
            .find(|w| w.path == "companies/indigo/workers/cmo/")
            .expect("company worker from worker.yaml");
        assert_eq!(cmo.id, "cmo-indigo");
        assert_eq!(cmo.company.as_deref(), Some("indigo"));

        let indigo = scan_company_library(&root, "indigo").expect("indigo library");
        assert_eq!(indigo.workers.len(), 1);
        assert_eq!(indigo.workers[0].path, "companies/indigo/workers/cmo/");

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
        assert_eq!(
            detail.skills[0].description.as_deref(),
            Some("Plan weekly content")
        );
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
        let root =
            std::env::temp_dir().join(format!("hq-library-local-empty-{}", std::process::id()));
        let _ = fs::create_dir_all(&root);
        let items = scan_root_library(&root);
        assert!(items.workers.is_empty());
        assert!(items.skills.is_empty());
        let _ = fs::remove_dir_all(&root);
    }

    /// US-001: a SKILL.md WITHOUT an author block reads cleanly (author=None),
    /// and one WITH an author surfaces uid/handle/displayName. Backwards-compat:
    /// the legacy `run` skill (no author) must still parse.
    #[test]
    fn skill_detail_reads_optional_author() {
        let root = make_fixture_tree();

        // Legacy skill: no author block → author is None, still valid.
        let plain = read_skill_detail(&root, ".claude/skills/run/SKILL.md").expect("run detail");
        assert!(plain.author.is_none());

        // Authored skill: uid/handle/displayName surfaced.
        let authored =
            read_skill_detail(&root, ".claude/skills/authored/SKILL.md").expect("authored detail");
        let a = authored.author.expect("author present");
        assert_eq!(a.uid, "prs_abc123");
        assert_eq!(a.handle, "corey");
        assert_eq!(a.display_name, "Corey Epstein");

        let _ = fs::remove_dir_all(&root);
    }

    /// US-001: worker.yaml author block is optional. Legacy worker (architect,
    /// no author) reads as None; an authored worker surfaces the attribution.
    #[test]
    fn worker_detail_reads_optional_author() {
        let root = make_fixture_tree();

        // Legacy worker: no author → None, parse still succeeds.
        let plain = read_worker_detail(&root, "core/workers/public/dev-team/architect/")
            .expect("architect detail");
        assert!(plain.author.is_none());

        // Authored worker: attribution surfaced.
        let authored = read_worker_detail(&root, "core/workers/public/dev-team/authored/")
            .expect("authored worker detail");
        let a = authored.author.expect("author present");
        assert_eq!(a.uid, "prs_abc123");
        assert_eq!(a.handle, "corey");
        assert_eq!(a.display_name, "Corey Epstein");

        let _ = fs::remove_dir_all(&root);
    }

    /// Regression for the "I created a `glm-5-2` worker but it isn't in the
    /// library" report: a company-scoped worker whose id carries digits + hyphens
    /// must surface in BOTH the root (all-scopes) and the per-company library —
    /// via the generated registry AND, when the registry hasn't regenerated yet,
    /// via the on-disk worker.yaml fallback.
    #[test]
    fn hyphenated_numeric_worker_id_surfaces_in_library() {
        let root = make_fixture_tree();
        let dir = root.join("companies/indigo/workers/glm-5-2");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("worker.yaml"),
            "worker:\n  id: glm-5-2\n  name: \"GLM 5 2\"\n  type: CodeWorker\n  version: \"1.0\"\n  description: \"A glm-5-2 worker\"\n",
        )
        .unwrap();

        // Case 1: the registry still has its original (pre-creation) entries — the
        // reindex hook hasn't run yet. The on-disk worker.yaml fallback must still
        // surface glm-5-2.
        let stale = scan_root_library(&root);
        let from_fs = stale
            .workers
            .iter()
            .find(|w| w.path == "companies/indigo/workers/glm-5-2/")
            .expect("glm-5-2 surfaces from worker.yaml even with a stale registry");
        assert_eq!(from_fs.id, "glm-5-2");
        assert_eq!(from_fs.scope, "company");
        assert_eq!(from_fs.company.as_deref(), Some("indigo"));

        let stale_company = scan_company_library(&root, "indigo").expect("indigo library");
        assert!(
            stale_company
                .workers
                .iter()
                .any(|w| w.id == "glm-5-2"),
            "glm-5-2 must appear in the per-company library from worker.yaml"
        );

        // Case 2: the registry regenerates and now lists glm-5-2 as a private
        // company worker. It must still surface (and stay unique, not doubled).
        fs::write(
            root.join("core/workers/registry.yaml"),
            "version: \"5.0\"\nworkers:\n  - id: glm-5-2\n    path: companies/indigo/workers/glm-5-2/\n    type: CodeWorker\n    visibility: private\n    company: indigo\n    status: active\n    description: \"A glm-5-2 worker\"\n",
        )
        .unwrap();

        let fresh = scan_root_library(&root);
        let matches: Vec<_> = fresh
            .workers
            .iter()
            .filter(|w| w.path == "companies/indigo/workers/glm-5-2/")
            .collect();
        assert_eq!(matches.len(), 1, "glm-5-2 listed exactly once, not duplicated");
        assert_eq!(matches[0].id, "glm-5-2");
        assert_eq!(matches[0].scope, "company");

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn split_frontmatter_handles_no_fence() {
        let (front, body) = split_frontmatter("# Just markdown\n\nNo frontmatter.");
        assert!(front.is_none());
        assert!(body.contains("# Just markdown"));
    }
}
