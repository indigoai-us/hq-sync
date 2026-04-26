//! `list_syncable_workspaces` + `connect_workspace_to_cloud` Tauri commands —
//! the source of truth for the menubar's main view.
//!
//! Returns the UNION of:
//!   1. Person entity (always shown — guaranteed by Cognito PreTokenGeneration)
//!   2. Companies the caller is a member of (`GET /membership/person/{uid}`)
//!   3. Local companies declared in `$HQ/companies/manifest.yaml` (canonical
//!      when present), falling back to enumerating `$HQ/companies/*`
//!      directories for HQs that predate the manifest.
//!
//! Each row carries a `state`:
//!   - `personal`   — the user's personal vault (always cloud-backed, local optional)
//!   - `synced`     — cloud entity + local folder both present
//!   - `cloud-only` — entity exists, no local folder yet
//!   - `local-only` — folder exists, no matching cloud entity (Connect button)
//!
//! Cloud failures degrade gracefully: local-only workspaces are still returned,
//! `cloudReachable: false` is set, and the UI surfaces a softly-worded notice.
//!
//! `connect_workspace_to_cloud(slug)` provisions a cloud bucket + writes the
//! per-company `.hq/config.json` so the next sync includes it. Idempotent:
//! reuses an existing entity if `find_by_slug` matches, only creating when
//! genuinely new.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::commands::config::{HqConfig, MenubarPrefs};
use crate::commands::provision::CompanyConfig;
use crate::commands::sync::{resolve_jwt, resolve_vault_api_url};
use crate::commands::vault_client::{CreateEntityInput, EntityInfo, MembershipInfo, VaultClient};
use crate::util::journal::read_journal;
use crate::util::paths;

// ── Public types ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum WorkspaceKind {
    Personal,
    Company,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum WorkspaceState {
    Personal,
    Synced,
    CloudOnly,
    LocalOnly,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Workspace {
    pub slug: String,
    pub display_name: String,
    pub kind: WorkspaceKind,
    pub state: WorkspaceState,
    pub cloud_uid: Option<String>,
    pub bucket_name: Option<String>,
    pub has_local_folder: bool,
    pub local_path: Option<String>,
    pub membership_status: Option<String>,
    pub last_synced_at: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspacesResult {
    pub workspaces: Vec<Workspace>,
    pub cloud_reachable: bool,
    pub error: Option<String>,
    pub hq_folder_path: String,
}

// ── Internal: local company discovery ─────────────────────────────────────────

/// One entry from `companies/manifest.yaml`, resolved to absolute paths.
/// Constructed by `discover_local_companies`; consumed by `assemble_workspaces`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LocalCompanyEntry {
    pub slug: String,
    pub display_name: Option<String>,
    pub path: PathBuf,
    pub dir_exists: bool,
}

/// Top-level shape of `companies/manifest.yaml`. We only consume `companies`;
/// any other top-level fields (e.g. version, description) are tolerated and
/// ignored — the file is shared with HQ scripts that may grow new keys.
#[derive(Debug, Deserialize)]
struct CompaniesManifest {
    #[serde(default)]
    companies: BTreeMap<String, CompanyManifestEntry>,
}

#[derive(Debug, Deserialize)]
struct CompanyManifestEntry {
    #[serde(default)]
    name: Option<String>,
    /// Path relative to `hq_root`. Defaults to `companies/{slug}` when absent.
    #[serde(default)]
    path: Option<String>,
}

/// Resolve hq_root from menubar.json + config.json (mirrors sync.rs without
/// the async surface so we can call it before any vault traffic).
fn resolve_hq_folder_path() -> Result<PathBuf, String> {
    let config_path = paths::config_json_path()?;
    let menubar_path = paths::menubar_json_path()?;

    let menubar_prefs: Option<MenubarPrefs> = if menubar_path.exists() {
        std::fs::read_to_string(&menubar_path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
    } else {
        None
    };

    let config: Option<HqConfig> = if config_path.exists() {
        std::fs::read_to_string(&config_path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
    } else {
        None
    };

    Ok(paths::resolve_hq_folder(
        config.as_ref().and_then(|c| c.hq_folder_path.as_deref()),
        menubar_prefs.as_ref().and_then(|p| p.hq_path.as_deref()),
    ))
}

/// Discover local companies. `companies/manifest.yaml` is canonical when it
/// exists; otherwise we fall back to enumerating `companies/*` directories.
///
/// Scaffolding entries (`_template/` and similar `_`-prefixed names) are
/// dropped from the enumeration fallback — they're an HQ convention for
/// boilerplate, not real companies. Manifest mode trusts the manifest fully:
/// if the user listed a `_thing`, they meant it.
pub(crate) fn discover_local_companies(hq_root: &Path) -> Vec<LocalCompanyEntry> {
    let manifest_path = hq_root.join("companies").join("manifest.yaml");
    if let Ok(bytes) = std::fs::read(&manifest_path) {
        if let Ok(parsed) = serde_yaml::from_slice::<CompaniesManifest>(&bytes) {
            return parsed
                .companies
                .into_iter()
                .map(|(slug, entry)| {
                    let path = entry
                        .path
                        .as_deref()
                        .map(|p| hq_root.join(p))
                        .unwrap_or_else(|| hq_root.join("companies").join(&slug));
                    LocalCompanyEntry {
                        dir_exists: path.is_dir(),
                        display_name: entry.name,
                        slug,
                        path,
                    }
                })
                .collect();
        }
    }

    // Fallback for HQs without a manifest. Skip dotfiles and underscore-prefix
    // scaffolding (`_template`, `_archive`, etc).
    list_local_company_folders(hq_root)
        .into_iter()
        .filter(|(slug, _)| !slug.starts_with('_'))
        .map(|(slug, path)| {
            let display_name = read_local_company_name(hq_root, &slug);
            LocalCompanyEntry {
                slug,
                display_name,
                dir_exists: true,
                path,
            }
        })
        .collect()
}

/// Walk `$hq_root/companies/*` and return (slug, abs-path) for every directory.
/// Used as the manifest-less fallback inside `discover_local_companies`.
fn list_local_company_folders(hq_root: &Path) -> Vec<(String, PathBuf)> {
    let companies_dir = hq_root.join("companies");
    let entries = match std::fs::read_dir(&companies_dir) {
        Ok(e) => e,
        Err(_) => return vec![],
    };

    let mut out = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let name = match entry.file_name().into_string() {
            Ok(n) => n,
            Err(_) => continue,
        };
        if name.starts_with('.') {
            continue;
        }
        out.push((name, path));
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// Try `$hq_root/companies/{slug}/company.yaml` for a friendly `name`.
/// Returns `None` when missing/unparseable; callers fall back to the slug.
fn read_local_company_name(hq_root: &Path, slug: &str) -> Option<String> {
    #[derive(serde::Deserialize)]
    struct YamlSlice {
        name: Option<String>,
    }
    let yaml_path = hq_root.join("companies").join(slug).join("company.yaml");
    let bytes = std::fs::read(&yaml_path).ok()?;
    let parsed: YamlSlice = serde_yaml::from_slice(&bytes).ok()?;
    parsed.name
}

fn last_synced_at(slug: &str) -> Option<String> {
    let j = read_journal(slug).ok()?;
    if j.last_sync.is_empty() {
        None
    } else {
        Some(j.last_sync)
    }
}

fn humanize_slug(slug: &str) -> String {
    slug.split('-')
        .map(|w| {
            let mut chars = w.chars();
            match chars.next() {
                Some(c) => c.to_uppercase().chain(chars).collect::<String>(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

// ── Workspace assembly (testable, synchronous core) ───────────────────────────

/// Pure function: given resolved cloud data + local company entries, produce
/// the workspaces vec. No I/O, no async. The injected `last_synced_lookup`
/// keeps tests independent of the real journal directory.
pub(crate) fn assemble_workspaces<F>(
    hq_root: &Path,
    person: Option<&EntityInfo>,
    memberships: &[MembershipInfo],
    company_entities: &BTreeMap<String, EntityInfo>,
    local_companies: &[LocalCompanyEntry],
    last_synced_lookup: F,
) -> Vec<Workspace>
where
    F: Fn(&str) -> Option<String>,
{
    // Index local entries by slug so the cloud-membership pass can pick up
    // the manifest's display_name + path field even when the entity has
    // no `name` / no local folder of its own.
    let local_by_slug: BTreeMap<&str, &LocalCompanyEntry> = local_companies
        .iter()
        .map(|e| (e.slug.as_str(), e))
        .collect();

    let mut by_slug: BTreeMap<String, Workspace> = BTreeMap::new();

    // 1. Cloud companies (from memberships).
    for mem in memberships {
        let entity = match company_entities.get(&mem.company_uid) {
            Some(e) => e,
            None => continue,
        };
        let slug = entity.slug.clone();
        let local_entry = local_by_slug.get(slug.as_str()).copied();
        let has_local = local_entry.map_or(false, |e| e.dir_exists);
        let display_name = entity
            .name
            .clone()
            .or_else(|| local_entry.and_then(|e| e.display_name.clone()))
            .unwrap_or_else(|| humanize_slug(&slug));
        by_slug.insert(
            slug.clone(),
            Workspace {
                slug: slug.clone(),
                display_name,
                kind: WorkspaceKind::Company,
                state: if has_local {
                    WorkspaceState::Synced
                } else {
                    WorkspaceState::CloudOnly
                },
                cloud_uid: Some(entity.uid.clone()),
                bucket_name: entity.bucket_name.clone(),
                has_local_folder: has_local,
                local_path: if has_local {
                    local_entry.map(|e| e.path.to_string_lossy().to_string())
                } else {
                    None
                },
                membership_status: Some(mem.status.clone()),
                last_synced_at: last_synced_lookup(&slug),
            },
        );
    }

    // 2. Local-only companies (no matching cloud membership).
    //    Phantom manifest entries (declared but no folder on disk) are dropped
    //    — there's nothing the user can act on (Connect button needs a folder).
    for entry in local_companies {
        if by_slug.contains_key(&entry.slug) {
            continue;
        }
        if !entry.dir_exists {
            continue;
        }
        let display_name = entry
            .display_name
            .clone()
            .unwrap_or_else(|| humanize_slug(&entry.slug));
        by_slug.insert(
            entry.slug.clone(),
            Workspace {
                slug: entry.slug.clone(),
                display_name,
                kind: WorkspaceKind::Company,
                state: WorkspaceState::LocalOnly,
                cloud_uid: None,
                bucket_name: None,
                has_local_folder: true,
                local_path: Some(entry.path.to_string_lossy().to_string()),
                membership_status: None,
                last_synced_at: last_synced_lookup(&entry.slug),
            },
        );
    }

    // 3. Personal — always first (insertion order via Vec, not BTreeMap).
    let mut ordered: Vec<Workspace> = Vec::with_capacity(by_slug.len() + 1);
    let personal_local = hq_root.exists() && hq_root.is_dir();
    let (personal_uid, personal_bucket) = match person {
        Some(p) => (Some(p.uid.clone()), p.bucket_name.clone()),
        None => (None, None),
    };
    let personal_display = person
        .and_then(|p| p.name.clone())
        .unwrap_or_else(|| "Personal".to_string());
    ordered.push(Workspace {
        slug: "personal".to_string(),
        display_name: personal_display,
        kind: WorkspaceKind::Personal,
        state: WorkspaceState::Personal,
        cloud_uid: personal_uid,
        bucket_name: personal_bucket,
        has_local_folder: personal_local,
        local_path: personal_local.then(|| hq_root.to_string_lossy().to_string()),
        membership_status: None,
        last_synced_at: last_synced_lookup("personal"),
    });

    // Companies sorted alphabetically by slug for stable rendering.
    ordered.extend(by_slug.into_values());

    ordered
}

// ── Tauri command: list_syncable_workspaces ───────────────────────────────────

#[tauri::command]
pub async fn list_syncable_workspaces() -> Result<WorkspacesResult, String> {
    let hq_root = resolve_hq_folder_path()?;
    let hq_folder_path = hq_root.to_string_lossy().to_string();
    let local_companies = discover_local_companies(&hq_root);

    // Cloud branch — failures captured into `cloud_reachable: false` rather
    // than propagated, so local-only data still renders when offline.
    let cloud_outcome: Result<
        (Option<EntityInfo>, Vec<MembershipInfo>, BTreeMap<String, EntityInfo>),
        String,
    > = async {
        let vault_url = resolve_vault_api_url()?;
        let jwt = resolve_jwt().await?;
        let vault = VaultClient::new(&vault_url, &jwt);

        // Person — pick the canonical (oldest createdAt, then uid).
        let mut persons = vault
            .list_entities_by_type("person")
            .await
            .map_err(|e| format!("list person entities: {e}"))?;
        persons.sort_by(|a, b| match a.created_at.cmp(&b.created_at) {
            std::cmp::Ordering::Equal => a.uid.cmp(&b.uid),
            ord => ord,
        });
        let person = persons.into_iter().next();

        let memberships = match &person {
            Some(p) => vault
                .list_memberships(&p.uid)
                .await
                .map_err(|e| format!("list memberships: {e}"))?,
            None => Vec::new(),
        };

        // Resolve each membership's company entity sequentially. Sequential
        // (not parallel) keeps the request count predictable and avoids
        // blowing past the vault Lambda's concurrency budget. A 404 on a
        // stale membership is silently dropped.
        let mut entities: BTreeMap<String, EntityInfo> = BTreeMap::new();
        for mem in &memberships {
            if entities.contains_key(&mem.company_uid) {
                continue;
            }
            match vault.find_entity_by_uid(&mem.company_uid).await {
                Ok(Some(e)) => {
                    entities.insert(mem.company_uid.clone(), e);
                }
                Ok(None) => {}
                Err(e) => {
                    return Err(format!(
                        "fetch entity {} for membership {}: {e}",
                        mem.company_uid, mem.uid
                    ));
                }
            }
        }

        Ok((person, memberships, entities))
    }
    .await;

    let (cloud_reachable, error, person, memberships, entities) = match cloud_outcome {
        Ok((p, m, e)) => (true, None, p, m, e),
        Err(e) => (false, Some(e), None, Vec::new(), BTreeMap::new()),
    };

    let workspaces = assemble_workspaces(
        &hq_root,
        person.as_ref(),
        &memberships,
        &entities,
        &local_companies,
        last_synced_at,
    );

    Ok(WorkspacesResult {
        workspaces,
        cloud_reachable,
        error,
        hq_folder_path,
    })
}

// ── Tauri command: connect_workspace_to_cloud ─────────────────────────────────

/// Provision a cloud bucket for the given local company `slug` and write the
/// per-company `.hq/config.json` so subsequent syncs include it.
///
/// Idempotent: if an entity with this slug already exists, we reuse its UID
/// rather than creating a duplicate. `provision_bucket` is itself idempotent
/// at the vault layer.
///
/// Refused for `slug == "personal"` — the Personal vault is auto-provisioned
/// via the Cognito PreTokenGeneration trigger and uses `vend-self`, not the
/// company create + provision path.
#[tauri::command]
pub async fn connect_workspace_to_cloud(slug: String) -> Result<(), String> {
    if slug.is_empty() {
        return Err("slug is required".to_string());
    }
    if slug == "personal" {
        return Err(
            "the Personal vault is auto-provisioned — no manual connect needed"
                .to_string(),
        );
    }

    let hq_root = resolve_hq_folder_path()?;
    let folder = hq_root.join("companies").join(&slug);
    if !folder.is_dir() {
        return Err(format!(
            "no local folder at companies/{slug} — cannot connect a missing directory"
        ));
    }

    let vault_url = resolve_vault_api_url()?;
    let jwt = resolve_jwt().await?;
    let vault = VaultClient::new(&vault_url, &jwt);

    let display_name = read_local_company_name(&hq_root, &slug)
        .unwrap_or_else(|| humanize_slug(&slug));

    let uid = match vault
        .find_entity_by_slug("company", &slug)
        .await
        .map_err(|e| format!("find_by_slug '{slug}': {e}"))?
    {
        Some(info) => info.uid,
        None => vault
            .create_entity(&CreateEntityInput {
                entity_type: "company".to_string(),
                slug: slug.clone(),
                name: display_name,
                email: None,
                owner_uid: None,
            })
            .await
            .map_err(|e| format!("create entity '{slug}': {e}"))?
            .uid,
    };

    let bucket = vault
        .provision_bucket(&uid)
        .await
        .map_err(|e| format!("provision_bucket for {uid}: {e}"))?;

    let config = CompanyConfig {
        company_uid: uid,
        company_slug: slug.clone(),
        bucket_name: bucket.bucket_name,
        vault_api_url: vault_url,
    };
    let config_path = folder.join(".hq").join("config.json");
    if let Some(parent) = config_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("create_dir_all {}: {e}", parent.display()))?;
    }
    let body = serde_json::to_string_pretty(&config)
        .map_err(|e| format!("serialize config: {e}"))?;
    std::fs::write(&config_path, body)
        .map_err(|e| format!("write {}: {e}", config_path.display()))?;

    Ok(())
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn person(uid: &str, bucket: Option<&str>) -> EntityInfo {
        EntityInfo {
            uid: uid.into(),
            slug: format!("{uid}-slug"),
            entity_type: "person".into(),
            name: Some("Stefan".into()),
            bucket_name: bucket.map(str::to_string),
            status: "active".into(),
            created_at: "2026-01-01T00:00:00Z".into(),
        }
    }

    fn company_entity(uid: &str, slug: &str, name: Option<&str>) -> EntityInfo {
        EntityInfo {
            uid: uid.into(),
            slug: slug.into(),
            entity_type: "company".into(),
            name: name.map(str::to_string),
            bucket_name: Some(format!("hq-vault-{}", uid.replace('_', "-"))),
            status: "active".into(),
            created_at: "2026-02-01T00:00:00Z".into(),
        }
    }

    fn membership(uid: &str, person_uid: &str, company_uid: &str, status: &str) -> MembershipInfo {
        MembershipInfo {
            uid: uid.into(),
            person_uid: person_uid.into(),
            company_uid: company_uid.into(),
            status: status.into(),
            role: Some("member".into()),
            created_at: Some("2026-03-01T00:00:00Z".into()),
        }
    }

    fn local(slug: &str, hq_root: &Path, exists: bool, name: Option<&str>) -> LocalCompanyEntry {
        let path = hq_root.join("companies").join(slug);
        if exists {
            std::fs::create_dir_all(&path).unwrap();
        }
        LocalCompanyEntry {
            slug: slug.into(),
            display_name: name.map(str::to_string),
            path,
            dir_exists: exists,
        }
    }

    fn write_manifest(hq_root: &Path, contents: &str) {
        let dir = hq_root.join("companies");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("manifest.yaml"), contents).unwrap();
    }

    // ── humanize_slug ─────────────────────────────────────────────────────

    #[test]
    fn humanize_slug_basic() {
        assert_eq!(humanize_slug("indigo"), "Indigo");
        assert_eq!(humanize_slug("synesis-strategy"), "Synesis Strategy");
        assert_eq!(humanize_slug(""), "");
    }

    // ── assemble_workspaces ───────────────────────────────────────────────

    #[test]
    fn personal_always_first_zero_companies() {
        let tmp = TempDir::new().unwrap();
        let p = person("prs_x", Some("hq-vault-prs-x"));
        let result = assemble_workspaces(
            tmp.path(),
            Some(&p),
            &[],
            &BTreeMap::new(),
            &[],
            |_| None,
        );
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].slug, "personal");
        assert_eq!(result[0].kind, WorkspaceKind::Personal);
        assert_eq!(result[0].state, WorkspaceState::Personal);
        assert_eq!(result[0].cloud_uid.as_deref(), Some("prs_x"));
        assert_eq!(result[0].bucket_name.as_deref(), Some("hq-vault-prs-x"));
        assert!(result[0].has_local_folder);
    }

    #[test]
    fn personal_present_without_person_entity() {
        let tmp = TempDir::new().unwrap();
        let result = assemble_workspaces(
            tmp.path(),
            None,
            &[],
            &BTreeMap::new(),
            &[],
            |_| None,
        );
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].slug, "personal");
        assert!(result[0].cloud_uid.is_none());
        assert_eq!(result[0].display_name, "Personal");
    }

    #[test]
    fn membership_with_local_folder_is_synced() {
        let tmp = TempDir::new().unwrap();
        let p = person("prs_x", None);
        let mem = membership("mem_1", "prs_x", "cmp_a", "active");
        let mut entities = BTreeMap::new();
        entities.insert(
            "cmp_a".to_string(),
            company_entity("cmp_a", "acme", Some("Acme Corp")),
        );
        let entries = vec![local("acme", tmp.path(), true, None)];

        let result = assemble_workspaces(
            tmp.path(),
            Some(&p),
            &[mem],
            &entities,
            &entries,
            |_| None,
        );
        assert_eq!(result.len(), 2);
        assert_eq!(result[1].slug, "acme");
        assert_eq!(result[1].state, WorkspaceState::Synced);
        assert_eq!(result[1].display_name, "Acme Corp");
        assert_eq!(result[1].membership_status.as_deref(), Some("active"));
        assert!(result[1].has_local_folder);
        assert!(result[1].cloud_uid.is_some());
    }

    #[test]
    fn membership_without_local_folder_is_cloud_only() {
        let tmp = TempDir::new().unwrap();
        let p = person("prs_x", None);
        let mem = membership("mem_2", "prs_x", "cmp_b", "pending");
        let mut entities = BTreeMap::new();
        entities.insert(
            "cmp_b".to_string(),
            company_entity("cmp_b", "newco", None),
        );

        let result = assemble_workspaces(
            tmp.path(),
            Some(&p),
            &[mem],
            &entities,
            &[],
            |_| None,
        );
        assert_eq!(result[1].slug, "newco");
        assert_eq!(result[1].state, WorkspaceState::CloudOnly);
        assert!(!result[1].has_local_folder);
        assert_eq!(result[1].membership_status.as_deref(), Some("pending"));
        assert_eq!(result[1].display_name, "Newco");
    }

    #[test]
    fn local_folder_without_cloud_is_local_only() {
        let tmp = TempDir::new().unwrap();
        let p = person("prs_x", None);
        let entries = vec![local("test-company", tmp.path(), true, None)];
        let result = assemble_workspaces(
            tmp.path(),
            Some(&p),
            &[],
            &BTreeMap::new(),
            &entries,
            |_| None,
        );
        assert_eq!(result.len(), 2);
        assert_eq!(result[1].slug, "test-company");
        assert_eq!(result[1].state, WorkspaceState::LocalOnly);
        assert!(result[1].cloud_uid.is_none());
        assert!(result[1].has_local_folder);
        assert_eq!(result[1].display_name, "Test Company");
    }

    /// Manifest entries with no folder on disk are dropped — phantom rows
    /// confuse users (no Connect button can target nothing).
    #[test]
    fn manifest_entry_without_folder_is_dropped() {
        let tmp = TempDir::new().unwrap();
        let p = person("prs_x", None);
        let entries = vec![local("phantom", tmp.path(), false, Some("Phantom Co"))];
        let result = assemble_workspaces(
            tmp.path(),
            Some(&p),
            &[],
            &BTreeMap::new(),
            &entries,
            |_| None,
        );
        assert_eq!(result.len(), 1, "only Personal remains");
        assert_eq!(result[0].slug, "personal");
    }

    #[test]
    fn stale_membership_with_missing_entity_is_dropped() {
        let tmp = TempDir::new().unwrap();
        let p = person("prs_x", None);
        let mem = membership("mem_stale", "prs_x", "cmp_gone", "active");
        let result = assemble_workspaces(
            tmp.path(),
            Some(&p),
            &[mem],
            &BTreeMap::new(),
            &[],
            |_| None,
        );
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].slug, "personal");
    }

    #[test]
    fn last_synced_lookup_invoked_per_workspace() {
        let tmp = TempDir::new().unwrap();
        let p = person("prs_x", None);
        let entries = vec![local("foo", tmp.path(), true, None)];
        let result = assemble_workspaces(
            tmp.path(),
            Some(&p),
            &[],
            &BTreeMap::new(),
            &entries,
            |slug| match slug {
                "personal" => Some("2026-04-25T00:00:00Z".into()),
                "foo" => Some("2026-04-24T12:00:00Z".into()),
                _ => None,
            },
        );
        assert_eq!(result[0].last_synced_at.as_deref(), Some("2026-04-25T00:00:00Z"));
        assert_eq!(result[1].last_synced_at.as_deref(), Some("2026-04-24T12:00:00Z"));
    }

    #[test]
    fn companies_sorted_alphabetically() {
        let tmp = TempDir::new().unwrap();
        let p = person("prs_x", None);
        let mems = vec![
            membership("m1", "prs_x", "cmp_z", "active"),
            membership("m2", "prs_x", "cmp_a", "active"),
            membership("m3", "prs_x", "cmp_m", "active"),
        ];
        let mut entities = BTreeMap::new();
        entities.insert("cmp_z".into(), company_entity("cmp_z", "zoo", None));
        entities.insert("cmp_a".into(), company_entity("cmp_a", "alpha", None));
        entities.insert("cmp_m".into(), company_entity("cmp_m", "mango", None));
        let result = assemble_workspaces(
            tmp.path(),
            Some(&p),
            &mems,
            &entities,
            &[],
            |_| None,
        );
        let slugs: Vec<&str> = result.iter().map(|w| w.slug.as_str()).collect();
        assert_eq!(slugs, vec!["personal", "alpha", "mango", "zoo"]);
    }

    #[test]
    fn membership_and_local_folder_no_duplicate() {
        let tmp = TempDir::new().unwrap();
        let p = person("prs_x", None);
        let mem = membership("mem_1", "prs_x", "cmp_a", "active");
        let mut entities = BTreeMap::new();
        entities.insert("cmp_a".into(), company_entity("cmp_a", "acme", Some("Acme")));
        let entries = vec![local("acme", tmp.path(), true, None)];
        let result = assemble_workspaces(
            tmp.path(),
            Some(&p),
            &[mem],
            &entities,
            &entries,
            |_| None,
        );
        assert_eq!(result.len(), 2);
        assert_eq!(result[1].slug, "acme");
        assert_eq!(result[1].state, WorkspaceState::Synced);
    }

    /// Display-name fallback chain: entity.name → manifest.name → humanized slug.
    #[test]
    fn display_name_fallback_chain() {
        let tmp = TempDir::new().unwrap();
        let p = person("prs_x", None);
        let mem = membership("m1", "prs_x", "cmp_a", "active");
        let mut entities = BTreeMap::new();
        entities.insert("cmp_a".into(), company_entity("cmp_a", "acme", None));
        let entries = vec![local("acme", tmp.path(), true, Some("Acme From Manifest"))];
        let result = assemble_workspaces(
            tmp.path(),
            Some(&p),
            &[mem],
            &entities,
            &entries,
            |_| None,
        );
        assert_eq!(result[1].display_name, "Acme From Manifest");
    }

    // ── discover_local_companies ─────────────────────────────────────────

    #[test]
    fn discover_uses_manifest_when_present() {
        let tmp = TempDir::new().unwrap();
        write_manifest(
            tmp.path(),
            r#"
companies:
  alpha:
    name: "Alpha Co"
    path: "companies/alpha"
  beta:
    name: "Beta"
    path: "companies/beta"
"#,
        );
        std::fs::create_dir_all(tmp.path().join("companies/alpha")).unwrap();
        // beta folder intentionally missing → dir_exists must be false

        let result = discover_local_companies(tmp.path());
        assert_eq!(result.len(), 2);
        let alpha = result.iter().find(|e| e.slug == "alpha").unwrap();
        assert_eq!(alpha.display_name.as_deref(), Some("Alpha Co"));
        assert!(alpha.dir_exists);
        let beta = result.iter().find(|e| e.slug == "beta").unwrap();
        assert!(
            !beta.dir_exists,
            "manifest entry without folder must report dir_exists: false"
        );
    }

    /// Default path = `companies/{slug}` when manifest's `path` field is omitted.
    #[test]
    fn discover_defaults_path_when_manifest_omits_it() {
        let tmp = TempDir::new().unwrap();
        write_manifest(
            tmp.path(),
            r#"
companies:
  acme:
    name: "Acme"
"#,
        );
        std::fs::create_dir_all(tmp.path().join("companies/acme")).unwrap();
        let result = discover_local_companies(tmp.path());
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].path, tmp.path().join("companies/acme"));
        assert!(result[0].dir_exists);
    }

    /// Unknown top-level fields in the manifest are tolerated (forward compat).
    #[test]
    fn discover_tolerates_unknown_manifest_fields() {
        let tmp = TempDir::new().unwrap();
        write_manifest(
            tmp.path(),
            r#"
version: 2
description: "extras tolerated"
companies:
  foo:
    name: "Foo"
"#,
        );
        std::fs::create_dir_all(tmp.path().join("companies/foo")).unwrap();
        let result = discover_local_companies(tmp.path());
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].slug, "foo");
    }

    /// Fallback (no manifest): enumerate folders, skip `_template` scaffolding.
    #[test]
    fn discover_fallback_skips_underscore_scaffolding() {
        let tmp = TempDir::new().unwrap();
        std::fs::create_dir_all(tmp.path().join("companies/_template")).unwrap();
        std::fs::create_dir_all(tmp.path().join("companies/_archive")).unwrap();
        std::fs::create_dir_all(tmp.path().join("companies/real-co")).unwrap();
        // No manifest.yaml — fallback path.
        let result = discover_local_companies(tmp.path());
        let slugs: Vec<&str> = result.iter().map(|e| e.slug.as_str()).collect();
        assert_eq!(
            slugs,
            vec!["real-co"],
            "underscore-prefixed dirs must not be listed in fallback mode"
        );
    }

    /// Fallback: no manifest, no companies/ dir → empty.
    #[test]
    fn discover_no_companies_dir_is_empty() {
        let tmp = TempDir::new().unwrap();
        let result = discover_local_companies(tmp.path());
        assert!(result.is_empty());
    }

    /// Fallback reads `company.yaml` for friendly names.
    #[test]
    fn discover_fallback_reads_company_yaml_for_name() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().join("companies/acme");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("company.yaml"), "name: Acme Industries\n").unwrap();
        let result = discover_local_companies(tmp.path());
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].display_name.as_deref(), Some("Acme Industries"));
    }

    /// Manifest with no `companies:` key at all → empty list (don't crash).
    #[test]
    fn discover_manifest_without_companies_key_is_empty() {
        let tmp = TempDir::new().unwrap();
        write_manifest(tmp.path(), "version: 1\n");
        let result = discover_local_companies(tmp.path());
        assert!(result.is_empty());
    }

    /// Manifest mode does NOT filter underscore-prefix entries — if the user
    /// listed `_archive` in their manifest, they meant it.
    #[test]
    fn discover_manifest_mode_keeps_underscore_entries() {
        let tmp = TempDir::new().unwrap();
        write_manifest(
            tmp.path(),
            r#"
companies:
  _archive:
    name: "Archive"
    path: "companies/_archive"
"#,
        );
        std::fs::create_dir_all(tmp.path().join("companies/_archive")).unwrap();
        let result = discover_local_companies(tmp.path());
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].slug, "_archive");
    }

    // ── list_local_company_folders (helper, manifest-less path) ──────────

    #[test]
    fn list_local_company_folders_skips_dotfiles_and_files() {
        let tmp = TempDir::new().unwrap();
        std::fs::create_dir_all(tmp.path().join("companies/foo")).unwrap();
        std::fs::create_dir_all(tmp.path().join("companies/.hidden")).unwrap();
        std::fs::write(tmp.path().join("companies/loose-file.txt"), "x").unwrap();

        let folders = list_local_company_folders(tmp.path());
        let names: Vec<&str> = folders.iter().map(|(s, _)| s.as_str()).collect();
        assert_eq!(names, vec!["foo"]);
    }
}
