//! `list_syncable_workspaces` Tauri command — the source of truth for the
//! menubar's main view.
//!
//! Returns the UNION of:
//!   1. Person entity (always shown — guaranteed by Cognito PreTokenGeneration)
//!   2. Companies the caller is a member of (from `GET /membership/person/{uid}`)
//!   3. Local company folders under `$HQ/companies/*` (whether cloud-bound or not)
//!
//! Each workspace row carries a `state`:
//!   - `personal`   — the user's personal vault (always cloud-backed, local optional)
//!   - `synced`     — cloud entity + local folder both present
//!   - `cloud-only` — entity exists, no local folder yet ("Sync now" creates one)
//!   - `local-only` — folder exists, no matching cloud entity ("Connect to cloud")
//!
//! Cloud failures degrade gracefully: local-only workspaces are still returned,
//! `cloudReachable: false` is set, and the UI surfaces a softly-worded notice.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::commands::sync::{resolve_jwt, resolve_vault_api_url};
use crate::commands::vault_client::{EntityInfo, MembershipInfo, VaultClient};
use crate::commands::config::{HqConfig, MenubarPrefs};
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
    /// The user's personal vault. Always shown; local folder optional.
    Personal,
    /// Cloud entity + local folder are both present.
    Synced,
    /// Cloud entity exists; no local folder yet.
    CloudOnly,
    /// Local folder exists; no matching cloud entity yet.
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

// ── Internal helpers ──────────────────────────────────────────────────────────

/// Resolve hq_root from menubar.json + config.json (mirrors sync::resolve_hq_folder_path
/// without the async surface so we can call it before any vault traffic).
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

/// Walk `$hq_root/companies/*` and return the (slug, abs-path) of every
/// directory that exists. Folders without a `company.yaml` are still returned —
/// the user may have created the folder manually (local-only state).
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
        // Skip dotfiles (e.g. `.DS_Store` shouldn't appear, but be defensive).
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

/// Try to read `$hq_root/companies/{slug}/company.yaml` and pull a friendly
/// `name` out. Returns `None` if the file is missing or unparseable — callers
/// fall back to the slug.
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

/// `last_sync` from the journal, normalized to `Option<String>` (empty journal
/// → `None`). Errors reading the journal also collapse to `None` — last-synced
/// is decorative metadata, never blocking.
fn last_synced_at(slug: &str) -> Option<String> {
    let j = read_journal(slug).ok()?;
    if j.last_sync.is_empty() {
        None
    } else {
        Some(j.last_sync)
    }
}

/// Title-case the slug for the display fallback. We do NOT attempt
/// dictionary-style capitalization — just the first character per
/// hyphen-delimited word, so `synesis-strategy` → `Synesis Strategy`.
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

/// Pure function: given resolved cloud data + local folder list, produce the
/// workspaces vec. No I/O, no async — easy to unit-test.
///
/// `last_synced_lookup` is injected so tests can assert without touching the
/// real journal directory.
pub(crate) fn assemble_workspaces<F>(
    hq_root: &Path,
    person: Option<&EntityInfo>,
    memberships: &[MembershipInfo],
    company_entities: &BTreeMap<String, EntityInfo>, // keyed by company UID
    local_folders: &[(String, PathBuf)],
    last_synced_lookup: F,
) -> Vec<Workspace>
where
    F: Fn(&str) -> Option<String>,
{
    let mut by_slug: BTreeMap<String, Workspace> = BTreeMap::new();

    // 1. Cloud companies (from memberships).
    //    `find_entity_by_uid` may have returned None for stale memberships —
    //    we silently skip those (a membership pointing at a deleted entity is
    //    not a workspace the user can interact with).
    for mem in memberships {
        let entity = match company_entities.get(&mem.company_uid) {
            Some(e) => e,
            None => continue,
        };
        let slug = entity.slug.clone();
        let display_name = entity
            .name
            .clone()
            .unwrap_or_else(|| humanize_slug(&slug));
        let local_path = hq_root.join("companies").join(&slug);
        let has_local = local_path.exists() && local_path.is_dir();
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
                local_path: has_local.then(|| local_path.to_string_lossy().to_string()),
                membership_status: Some(mem.status.clone()),
                last_synced_at: last_synced_lookup(&slug),
            },
        );
    }

    // 2. Local-only company folders (no matching cloud entry).
    for (slug, abs) in local_folders {
        if by_slug.contains_key(slug) {
            continue; // already represented as Synced from the cloud pass
        }
        let display_name = read_local_company_name(hq_root, slug)
            .unwrap_or_else(|| humanize_slug(slug));
        by_slug.insert(
            slug.clone(),
            Workspace {
                slug: slug.clone(),
                display_name,
                kind: WorkspaceKind::Company,
                state: WorkspaceState::LocalOnly,
                cloud_uid: None,
                bucket_name: None,
                has_local_folder: true,
                local_path: Some(abs.to_string_lossy().to_string()),
                membership_status: None,
                last_synced_at: last_synced_lookup(slug),
            },
        );
    }

    // 3. Personal — always first in the list (insertion order: prepend).
    //    BTreeMap orders alphabetically, so we'd need "_personal" or a
    //    separate prepend. Use a Vec with explicit ordering instead of relying
    //    on map iteration order.
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

    // Then companies, sorted alphabetically by slug for stable rendering.
    ordered.extend(by_slug.into_values());

    ordered
}

// ── Tauri command ─────────────────────────────────────────────────────────────

/// Produce the workspaces union for the menubar.
///
/// Resolution order:
///   1. Resolve hq_root locally (always succeeds if the env is sane).
///   2. Walk local company folders.
///   3. Try to resolve JWT + vault URL → fetch person + memberships + entities.
///      Any failure here returns `cloudReachable: false` with the locally
///      known data still populated.
///
/// The Personal row is ALWAYS included in the result, even if cloud fetches
/// fail — the row carries `cloudUid: None` in that degraded case, and the UI
/// shows it with a "cloud unreachable" hint instead of an empty list.
#[tauri::command]
pub async fn list_syncable_workspaces() -> Result<WorkspacesResult, String> {
    let hq_root = resolve_hq_folder_path()?;
    let hq_folder_path = hq_root.to_string_lossy().to_string();
    let local_folders = list_local_company_folders(&hq_root);

    // Try the cloud branch. We don't propagate errors out — instead we capture
    // them so the UI can show local-only data when the cloud is unreachable.
    let cloud_outcome: Result<(Option<EntityInfo>, Vec<MembershipInfo>, BTreeMap<String, EntityInfo>), String> = async {
        let vault_url = resolve_vault_api_url()?;
        let jwt = resolve_jwt().await?;
        let vault = VaultClient::new(&vault_url, &jwt);

        // Person entity — pick the canonical (oldest createdAt, then uid).
        let mut persons = vault
            .list_entities_by_type("person")
            .await
            .map_err(|e| format!("list person entities: {e}"))?;
        persons.sort_by(|a, b| match a.created_at.cmp(&b.created_at) {
            std::cmp::Ordering::Equal => a.uid.cmp(&b.uid),
            ord => ord,
        });
        let person = persons.into_iter().next();

        // Memberships — only meaningful if we have a person.
        let memberships = match &person {
            Some(p) => vault
                .list_memberships(&p.uid)
                .await
                .map_err(|e| format!("list memberships: {e}"))?,
            None => Vec::new(),
        };

        // Resolve each membership's company entity sequentially. Sequential
        // (not parallel) keeps the request count predictable for users with
        // ~5–20 memberships and avoids blowing past the vault Lambda's
        // concurrency budget. A 404 on a stale membership is silently
        // dropped (entity gone, but the membership row persists server-side).
        let mut entities: BTreeMap<String, EntityInfo> = BTreeMap::new();
        for mem in &memberships {
            if entities.contains_key(&mem.company_uid) {
                continue;
            }
            match vault.find_entity_by_uid(&mem.company_uid).await {
                Ok(Some(e)) => {
                    entities.insert(mem.company_uid.clone(), e);
                }
                Ok(None) => {
                    // Stale membership; drop it from the union view.
                }
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
        &local_folders,
        last_synced_at,
    );

    Ok(WorkspacesResult {
        workspaces,
        cloud_reachable,
        error,
        hq_folder_path,
    })
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

    fn make_dir(root: &Path, rel: &str) -> PathBuf {
        let p = root.join(rel);
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn humanize_slug_basic() {
        assert_eq!(humanize_slug("indigo"), "Indigo");
        assert_eq!(humanize_slug("synesis-strategy"), "Synesis Strategy");
        assert_eq!(humanize_slug(""), "");
    }

    /// Personal is always first in the result, even when there are no
    /// memberships and no local folders.
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
        assert!(result[0].has_local_folder, "tmp dir exists");
    }

    /// Personal row appears even when the person entity could not be fetched
    /// (cloud unreachable). cloud_uid is None in that case.
    #[test]
    fn personal_present_without_person_entity() {
        let tmp = TempDir::new().unwrap();
        let result = assemble_workspaces(
            tmp.path(),
            None, // cloud unreachable
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

    /// A membership pointing at a company we successfully resolved becomes a
    /// Synced row (because the local folder exists).
    #[test]
    fn membership_with_local_folder_is_synced() {
        let tmp = TempDir::new().unwrap();
        make_dir(tmp.path(), "companies/acme");
        let p = person("prs_x", None);
        let mem = membership("mem_1", "prs_x", "cmp_a", "active");
        let mut entities = BTreeMap::new();
        entities.insert("cmp_a".to_string(), company_entity("cmp_a", "acme", Some("Acme Corp")));

        let result = assemble_workspaces(
            tmp.path(),
            Some(&p),
            &[mem],
            &entities,
            &[("acme".into(), tmp.path().join("companies/acme"))],
            |_| None,
        );

        // [Personal, acme]
        assert_eq!(result.len(), 2);
        assert_eq!(result[1].slug, "acme");
        assert_eq!(result[1].state, WorkspaceState::Synced);
        assert_eq!(result[1].display_name, "Acme Corp");
        assert_eq!(result[1].membership_status.as_deref(), Some("active"));
        assert!(result[1].has_local_folder);
        assert!(result[1].cloud_uid.is_some());
    }

    /// A membership with no local folder yet is `cloud-only`.
    #[test]
    fn membership_without_local_folder_is_cloud_only() {
        let tmp = TempDir::new().unwrap();
        let p = person("prs_x", None);
        let mem = membership("mem_2", "prs_x", "cmp_b", "pending");
        let mut entities = BTreeMap::new();
        entities.insert("cmp_b".to_string(), company_entity("cmp_b", "newco", None));

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
        // Display name falls back to humanized slug when entity has no name.
        assert_eq!(result[1].display_name, "Newco");
    }

    /// A local folder with no matching cloud membership is `local-only`.
    #[test]
    fn local_folder_without_cloud_is_local_only() {
        let tmp = TempDir::new().unwrap();
        make_dir(tmp.path(), "companies/test-company");
        let p = person("prs_x", None);
        let result = assemble_workspaces(
            tmp.path(),
            Some(&p),
            &[],
            &BTreeMap::new(),
            &[("test-company".into(), tmp.path().join("companies/test-company"))],
            |_| None,
        );
        assert_eq!(result.len(), 2);
        assert_eq!(result[1].slug, "test-company");
        assert_eq!(result[1].state, WorkspaceState::LocalOnly);
        assert!(result[1].cloud_uid.is_none());
        assert!(result[1].has_local_folder);
        assert_eq!(result[1].display_name, "Test Company");
    }

    /// Stale memberships (entity not in the resolved-entities map) are silently
    /// dropped — they would otherwise produce a row the user can't act on.
    #[test]
    fn stale_membership_with_missing_entity_is_dropped() {
        let tmp = TempDir::new().unwrap();
        let p = person("prs_x", None);
        let mem = membership("mem_stale", "prs_x", "cmp_gone", "active");
        // entities map intentionally empty (find_entity_by_uid returned None)
        let result = assemble_workspaces(
            tmp.path(),
            Some(&p),
            &[mem],
            &BTreeMap::new(),
            &[],
            |_| None,
        );
        assert_eq!(result.len(), 1, "only Personal remains; stale membership dropped");
        assert_eq!(result[0].slug, "personal");
    }

    /// The `last_synced_at` lookup is invoked for every workspace.
    #[test]
    fn last_synced_lookup_invoked_per_workspace() {
        let tmp = TempDir::new().unwrap();
        make_dir(tmp.path(), "companies/foo");
        let p = person("prs_x", None);
        let result = assemble_workspaces(
            tmp.path(),
            Some(&p),
            &[],
            &BTreeMap::new(),
            &[("foo".into(), tmp.path().join("companies/foo"))],
            |slug| match slug {
                "personal" => Some("2026-04-25T00:00:00Z".into()),
                "foo" => Some("2026-04-24T12:00:00Z".into()),
                _ => None,
            },
        );
        assert_eq!(result[0].last_synced_at.as_deref(), Some("2026-04-25T00:00:00Z"));
        assert_eq!(result[1].last_synced_at.as_deref(), Some("2026-04-24T12:00:00Z"));
    }

    /// Companies are sorted alphabetically by slug for stable rendering.
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

    /// Local folder + matching membership = single Synced row, never duplicated.
    #[test]
    fn membership_and_local_folder_no_duplicate() {
        let tmp = TempDir::new().unwrap();
        make_dir(tmp.path(), "companies/acme");
        let p = person("prs_x", None);
        let mem = membership("mem_1", "prs_x", "cmp_a", "active");
        let mut entities = BTreeMap::new();
        entities.insert("cmp_a".into(), company_entity("cmp_a", "acme", Some("Acme")));

        let result = assemble_workspaces(
            tmp.path(),
            Some(&p),
            &[mem],
            &entities,
            &[("acme".into(), tmp.path().join("companies/acme"))],
            |_| None,
        );
        // [Personal, acme] — only ONE acme row even though it appears in both
        // memberships and local_folders.
        assert_eq!(result.len(), 2);
        assert_eq!(result[1].slug, "acme");
        assert_eq!(result[1].state, WorkspaceState::Synced);
    }

    /// `read_local_company_name` falls back to humanized slug when YAML missing.
    #[test]
    fn read_local_company_name_missing_yaml_returns_none() {
        let tmp = TempDir::new().unwrap();
        make_dir(tmp.path(), "companies/foo");
        // No company.yaml written.
        let name = read_local_company_name(tmp.path(), "foo");
        assert!(name.is_none());
    }

    /// `read_local_company_name` returns the YAML's `name` field when present.
    #[test]
    fn read_local_company_name_with_yaml() {
        let tmp = TempDir::new().unwrap();
        let dir = make_dir(tmp.path(), "companies/foo");
        std::fs::write(dir.join("company.yaml"), "name: Foo Industries\n").unwrap();
        let name = read_local_company_name(tmp.path(), "foo");
        assert_eq!(name.as_deref(), Some("Foo Industries"));
    }

    /// `list_local_company_folders` skips dotfiles + non-directories.
    #[test]
    fn list_local_company_folders_skips_dotfiles_and_files() {
        let tmp = TempDir::new().unwrap();
        make_dir(tmp.path(), "companies/foo");
        make_dir(tmp.path(), "companies/.hidden");
        std::fs::write(tmp.path().join("companies/loose-file.txt"), "x").unwrap();

        let folders = list_local_company_folders(tmp.path());
        let names: Vec<&str> = folders.iter().map(|(s, _)| s.as_str()).collect();
        assert_eq!(names, vec!["foo"]);
    }

    /// `list_local_company_folders` returns empty (not error) when companies dir absent.
    #[test]
    fn list_local_company_folders_no_companies_dir_is_empty() {
        let tmp = TempDir::new().unwrap();
        // No companies/ subdirectory created.
        let folders = list_local_company_folders(tmp.path());
        assert!(folders.is_empty());
    }
}
