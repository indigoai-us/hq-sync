//! `list_syncable_workspaces` + `connect_workspace_to_cloud` Tauri commands —
//! the source of truth for the menubar's main view.
//!
//! ## Mapping model (v0.1.23+)
//!
//! Local company folders map to cloud buckets via TWO redundant records:
//!
//! 1. **`companies/manifest.yaml`** — canonical, declared by the user. When
//!    `discover_local_companies` reads this file, each entry's `cloud_uid` and
//!    `bucket_name` (if present) are treated as authoritative. The runtime
//!    trusts these even when the cloud is unreachable.
//!
//! 2. **`companies/{slug}/.hq/config.json`** — per-folder runtime cache.
//!    Written by both `provision_missing_companies` (auto-flow) and
//!    `connect_workspace_to_cloud` (Connect button). Keeps the cloud UID
//!    co-located with the data it describes, so a copied/moved folder takes
//!    its mapping with it.
//!
//! ## Connect flow (dual-write)
//!
//! When the Connect button fires:
//!   1. Provision the cloud bucket (idempotent — `find_by_slug` + reuse).
//!   2. Write per-folder `.hq/config.json` (authoritative for runtime).
//!   3. **Patch the manifest entry** with `cloud_uid` + `bucket_name`. Best-effort:
//!      if the manifest is missing or unparseable, log + continue (the per-folder
//!      config is still correct).
//!
//! ## Mismatch detection (`Broken` state)
//!
//! If the manifest declares `cloud_uid: X` for a slug but the cloud (when
//! reachable) returns no membership for that slug, OR returns a different UID,
//! the workspace surfaces as `Broken`. The user can hit Connect to reconcile —
//! `connect_workspace_to_cloud` will re-find by slug and overwrite the manifest
//! `cloud_uid` with the current truth.
//!
//! ## TODO: `repair_manifest` Tauri command (deferred)
//!
//! A future repair flow should:
//!   - Walk every `companies/{slug}/.hq/config.json`, ensure each has a
//!     matching manifest entry with the same `cloud_uid` / `bucket_name`.
//!   - Cross-reference the cloud's membership list against the manifest;
//!     surface entries that exist in the cloud but have no local config
//!     (orphan memberships) and ask the user whether to write a folder skeleton.
//!   - Detect duplicate slugs, broken paths, and stale UIDs.
//!   - Surface findings in a Settings panel; do not auto-mutate without the
//!     user's confirmation per finding.
//!
//! Intentionally NOT shipped in v0.1.23 to keep scope tight. Per-row Connect
//! covers the common case (re-provision a single broken slug) without needing
//! the full repair surface.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::commands::config::{HqConfig, MenubarPrefs};
use crate::commands::provision::CompanyConfig;
use crate::commands::sync::{resolve_jwt, resolve_vault_api_url};
use crate::commands::vault_client::{CreateEntityInput, EntityInfo, MembershipInfo, VaultClient};
use crate::util::journal::read_journal;
use crate::util::logfile::log;
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
    /// Cloud entity + local folder both present, manifest matches cloud truth.
    Synced,
    /// Cloud entity exists; no local folder yet.
    CloudOnly,
    /// Local folder exists; no manifest cloud_uid AND no matching cloud membership.
    LocalOnly,
    /// Manifest declares a cloud_uid that doesn't match cloud reality.
    /// Reconnect to reconcile — only surfaced when cloud_reachable=true.
    Broken,
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
    /// Human-readable diagnostic when state is Broken. UI surfaces in the tooltip.
    pub broken_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspacesResult {
    pub workspaces: Vec<Workspace>,
    pub cloud_reachable: bool,
    pub error: Option<String>,
    pub hq_folder_path: String,
    /// Top-level manifest parse/IO error. Non-null means the user has a
    /// `companies/manifest.yaml` we couldn't read — UI surfaces a soft
    /// notice and falls back to folder enumeration.
    pub manifest_error: Option<String>,
}

// ── Internal: local company discovery ─────────────────────────────────────────

/// One entry from `companies/manifest.yaml`, resolved to absolute paths.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LocalCompanyEntry {
    pub slug: String,
    pub display_name: Option<String>,
    pub path: PathBuf,
    pub dir_exists: bool,
    /// Manifest-recorded cloud entity UID. None when the entry is local-only
    /// or when discovered via folder-enumeration fallback.
    pub cloud_uid: Option<String>,
    /// Manifest-recorded S3 bucket name. Always paired with `cloud_uid`.
    pub bucket_name: Option<String>,
}

/// Top-level shape of `companies/manifest.yaml`. Only `companies` is consumed;
/// other top-level fields are tolerated and ignored (forward compat with HQ
/// scripts that may grow new top-level keys).
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
    /// Cloud entity UID (`cmp_*`), written by `connect_workspace_to_cloud`.
    /// When present, the manifest is the canonical record of "this folder
    /// is connected to that cloud entity."
    #[serde(default)]
    cloud_uid: Option<String>,
    /// S3 bucket name (`hq-vault-cmp-{uid}`), written alongside `cloud_uid`.
    #[serde(default)]
    bucket_name: Option<String>,
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

/// Outcome of attempting to read the manifest. Distinguishes "no manifest
/// (use folder fallback)" from "manifest exists but is broken (surface error)".
pub(crate) enum ManifestLoad {
    Present(Vec<LocalCompanyEntry>),
    Absent,
    Failed(String),
}

/// Read the manifest into a list of LocalCompanyEntry.
///
/// Three outcomes are distinguished:
///   - `Present(entries)`  — manifest parsed cleanly
///   - `Absent`            — file doesn't exist; caller falls back to dir enumeration
///   - `Failed(reason)`    — file exists but unreadable/unparseable; caller
///                           surfaces the error AND still falls back to dir enumeration
pub(crate) fn read_manifest(hq_root: &Path) -> ManifestLoad {
    let manifest_path = hq_root.join("companies").join("manifest.yaml");
    let bytes = match std::fs::read(&manifest_path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return ManifestLoad::Absent,
        Err(e) => return ManifestLoad::Failed(format!("read {}: {e}", manifest_path.display())),
    };
    let parsed: CompaniesManifest = match serde_yaml::from_slice(&bytes) {
        Ok(p) => p,
        Err(e) => {
            return ManifestLoad::Failed(format!(
                "parse {}: {e}",
                manifest_path.display()
            ));
        }
    };
    let entries = parsed
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
                cloud_uid: entry.cloud_uid,
                bucket_name: entry.bucket_name,
                slug,
                path,
            }
        })
        .collect();
    ManifestLoad::Present(entries)
}

/// Discover local companies. Manifest is canonical when present + parseable;
/// otherwise (or in addition to a parse error) we fall back to enumerating
/// `companies/*` directories.
///
/// Scaffolding entries (slug starts with `_`, e.g. `_template`) are dropped
/// from the enumeration fallback — they're an HQ convention for boilerplate,
/// not real companies. Manifest mode trusts the manifest fully.
///
/// Returns `(entries, manifest_error)` — the error is non-None only when the
/// manifest exists but couldn't be parsed.
pub(crate) fn discover_local_companies(
    hq_root: &Path,
) -> (Vec<LocalCompanyEntry>, Option<String>) {
    let raw = match read_manifest(hq_root) {
        ManifestLoad::Present(entries) => {
            // Manifest is canonical for the entries it lists, but the user can
            // also have on-disk company folders that pre-date the manifest or
            // were added by tools that don't update it. Union those in as
            // unconnected entries so they're still visible (and connectable)
            // in the UI — otherwise a folder-only company shows as Cloud Only
            // (via memberships pass) when it actually exists locally.
            let mut union = entries;
            let known: std::collections::HashSet<String> =
                union.iter().map(|e| e.slug.clone()).collect();
            for extra in folder_enumeration_fallback(hq_root) {
                if !known.contains(&extra.slug) {
                    union.push(extra);
                }
            }
            (union, None)
        }
        ManifestLoad::Absent => (folder_enumeration_fallback(hq_root), None),
        ManifestLoad::Failed(err) => {
            log("workspaces", &format!("manifest unreadable, using folder fallback: {err}"));
            (folder_enumeration_fallback(hq_root), Some(err))
        }
    };

    // Drop slug="personal" from the company list. The personal vault row
    // (assembled separately with kind=Personal, state=Personal) is the
    // canonical surface for the user's personal HQ — a manifest-declared
    // `personal` company would render as a duplicate Local Only row, and
    // its Connect button can't succeed (the Rust guard rejects slug=="personal"
    // because the personal vault auto-provisions via the person entity, not
    // the company-creation flow). Filter here so the duplicate never appears.
    let (mut entries, manifest_err) = raw;
    entries.retain(|e| e.slug != "personal");
    (entries, manifest_err)
}

fn folder_enumeration_fallback(hq_root: &Path) -> Vec<LocalCompanyEntry> {
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
                cloud_uid: None,
                bucket_name: None,
            }
        })
        .collect()
}

/// Walk `$hq_root/companies/*` and return (slug, abs-path) for every directory.
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

// ── Manifest patching ─────────────────────────────────────────────────────────

/// Patch `companies/manifest.yaml` to record `cloud_uid` and `bucket_name`
/// for the given slug. Returns Err on read/parse/write failures; callers treat
/// this as non-fatal (per-folder `.hq/config.json` is the authoritative
/// runtime record).
///
/// **Comments and ordering**: serde_yaml round-trips Mapping order but does
/// NOT preserve YAML comments. The HQ-side `/newcompany` script writes a
/// header comment we'll lose on first patch — acceptable trade-off given the
/// alternative (manual text patching) is fragile across formatting variants.
pub(crate) fn patch_manifest_with_cloud_info(
    manifest_path: &Path,
    slug: &str,
    cloud_uid: &str,
    bucket_name: &str,
) -> Result<(), String> {
    let bytes = std::fs::read(manifest_path)
        .map_err(|e| format!("read manifest: {e}"))?;
    let mut value: serde_yaml::Value = serde_yaml::from_slice(&bytes)
        .map_err(|e| format!("parse manifest: {e}"))?;

    let companies_key = serde_yaml::Value::String("companies".to_string());
    let mapping = value
        .as_mapping_mut()
        .ok_or_else(|| "manifest root is not a mapping".to_string())?;
    let companies = mapping
        .get_mut(&companies_key)
        .and_then(|v| v.as_mapping_mut())
        .ok_or_else(|| "manifest has no `companies` mapping".to_string())?;

    let slug_key = serde_yaml::Value::String(slug.to_string());
    let entry = companies
        .get_mut(&slug_key)
        .and_then(|v| v.as_mapping_mut())
        .ok_or_else(|| format!("manifest has no entry for slug '{slug}'"))?;

    entry.insert(
        serde_yaml::Value::String("cloud_uid".to_string()),
        serde_yaml::Value::String(cloud_uid.to_string()),
    );
    entry.insert(
        serde_yaml::Value::String("bucket_name".to_string()),
        serde_yaml::Value::String(bucket_name.to_string()),
    );

    let serialized = serde_yaml::to_string(&value)
        .map_err(|e| format!("serialize manifest: {e}"))?;

    // Atomic write: tmp → rename. Any failure leaves the original intact.
    let tmp = manifest_path.with_extension("yaml.tmp");
    std::fs::write(&tmp, &serialized).map_err(|e| format!("write tmp manifest: {e}"))?;
    std::fs::rename(&tmp, manifest_path)
        .map_err(|e| format!("rename manifest: {e}"))?;

    Ok(())
}

/// Append a brand-new entry to `companies` for `slug` and stamp it with cloud
/// info. Used when sync downloads a cloud-only company and creates the local
/// folder as a side effect — the manifest needs to learn about the new folder
/// so subsequent loads don't miss it.
///
/// Idempotent: if `slug` already exists in the manifest, this is a no-op
/// (caller should use `patch_manifest_with_cloud_info` to update an existing
/// entry instead).
pub(crate) fn add_manifest_entry_for_synced_company(
    manifest_path: &Path,
    slug: &str,
    display_name: &str,
    cloud_uid: &str,
    bucket_name: &str,
) -> Result<(), String> {
    let bytes = std::fs::read(manifest_path)
        .map_err(|e| format!("read manifest: {e}"))?;
    let mut value: serde_yaml::Value = serde_yaml::from_slice(&bytes)
        .map_err(|e| format!("parse manifest: {e}"))?;

    let companies_key = serde_yaml::Value::String("companies".to_string());
    let mapping = value
        .as_mapping_mut()
        .ok_or_else(|| "manifest root is not a mapping".to_string())?;
    if !mapping.contains_key(&companies_key) {
        mapping.insert(
            companies_key.clone(),
            serde_yaml::Value::Mapping(serde_yaml::Mapping::new()),
        );
    }
    let companies = mapping
        .get_mut(&companies_key)
        .and_then(|v| v.as_mapping_mut())
        .ok_or_else(|| "manifest `companies` key is not a mapping".to_string())?;

    let slug_key = serde_yaml::Value::String(slug.to_string());
    if companies.contains_key(&slug_key) {
        // Caller bug — they should patch instead of add. Soft-no-op so we
        // don't regress the existing entry's other fields.
        return Ok(());
    }

    let mut entry = serde_yaml::Mapping::new();
    let s = |v: &str| serde_yaml::Value::String(v.to_string());
    entry.insert(s("name"), s(display_name));
    entry.insert(s("path"), s(&format!("companies/{slug}")));
    entry.insert(s("knowledge"), s(&format!("companies/{slug}/knowledge/")));
    entry.insert(s("cloud_uid"), s(cloud_uid));
    entry.insert(s("bucket_name"), s(bucket_name));
    companies.insert(slug_key, serde_yaml::Value::Mapping(entry));

    let serialized = serde_yaml::to_string(&value)
        .map_err(|e| format!("serialize manifest: {e}"))?;

    let tmp = manifest_path.with_extension("yaml.tmp");
    std::fs::write(&tmp, &serialized).map_err(|e| format!("write tmp manifest: {e}"))?;
    std::fs::rename(&tmp, manifest_path)
        .map_err(|e| format!("rename manifest: {e}"))?;

    Ok(())
}

/// Reconcile the manifest with the local `companies/*/` folder reality after a
/// sync run. For each on-disk folder NOT in the manifest, look up the cloud
/// entity by slug and either:
///   - Add a manifest entry stamped with cloud_uid + bucket_name (if cloud has
///     a matching entity)
///   - Skip the folder (no cloud match — leave for the user to Connect manually
///     or for a future repair pass)
///
/// Best-effort: each per-folder failure is logged but doesn't abort the rest.
/// Returns the number of entries newly added to the manifest.
pub(crate) async fn reconcile_manifest_after_sync(
    hq_root: &Path,
    vault: &VaultClient,
) -> Result<usize, String> {
    let manifest_path = hq_root.join("companies").join("manifest.yaml");
    if !manifest_path.exists() {
        // No manifest at all — out of scope here. /newcompany or first-run
        // setup is responsible for creating it.
        return Ok(0);
    }

    let known_slugs: std::collections::HashSet<String> = match read_manifest(hq_root) {
        ManifestLoad::Present(entries) => entries.into_iter().map(|e| e.slug).collect(),
        // Manifest unparseable — bail; we'd risk overwriting whatever the user
        // has in there. The folder-union in discover_local_companies still
        // gives the UI a workable view in the meantime.
        ManifestLoad::Failed(err) => {
            return Err(format!("manifest unreadable, refusing to patch: {err}"));
        }
        ManifestLoad::Absent => return Ok(0),
    };

    let mut added = 0usize;
    for (slug, _path) in list_local_company_folders(hq_root) {
        if slug.starts_with('_') {
            continue; // scaffolding folders (e.g. _template)
        }
        if known_slugs.contains(&slug) {
            continue; // already in manifest
        }
        // Look up the cloud entity by slug. If not found, skip — we don't
        // want to add a `cloud_uid`-less entry from here (Connect handles
        // that flow with full UI feedback).
        let entity = match vault.find_entity_by_slug("company", &slug).await {
            Ok(Some(e)) => e,
            Ok(None) => {
                log("workspaces", &format!("reconcile: no cloud entity for slug '{slug}', skipping"));
                continue;
            }
            Err(e) => {
                log("workspaces", &format!("reconcile: find_by_slug '{slug}' failed: {e}"));
                continue;
            }
        };
        let bucket = match entity.bucket_name.as_deref() {
            Some(b) => b.to_string(),
            None => {
                log(
                    "workspaces",
                    &format!("reconcile: cloud entity '{slug}' has no bucket — skipping (Connect to provision)"),
                );
                continue;
            }
        };
        let display_name = entity
            .name
            .clone()
            .unwrap_or_else(|| humanize_slug(&slug));
        if let Err(e) = add_manifest_entry_for_synced_company(
            &manifest_path,
            &slug,
            &display_name,
            &entity.uid,
            &bucket,
        ) {
            log("workspaces", &format!("reconcile: add manifest entry for '{slug}' failed: {e}"));
            continue;
        }
        log("workspaces", &format!("reconcile: added manifest entry for '{slug}' (uid={})", entity.uid));
        added += 1;
    }
    Ok(added)
}

// ── Workspace assembly (testable, synchronous core) ───────────────────────────

/// Pure function: given resolved cloud data + local company entries, produce
/// the workspaces vec. No I/O, no async.
///
/// **Manifest-first semantics:** when a `LocalCompanyEntry` carries
/// `cloud_uid` (i.e. the manifest declares this is a connected workspace), we
/// trust it as authoritative state — even when cloud is unreachable. Cloud
/// data is for cross-reference only:
///   - cloud confirms manifest UID → Synced
///   - cloud disagrees (different UID, or no membership for slug) → Broken
///   - cloud unreachable → Synced (optimistic; trust the local cache)
pub(crate) fn assemble_workspaces<F>(
    hq_root: &Path,
    person: Option<&EntityInfo>,
    memberships: &[MembershipInfo],
    company_entities: &BTreeMap<String, EntityInfo>,
    local_companies: &[LocalCompanyEntry],
    cloud_reachable: bool,
    last_synced_lookup: F,
) -> Vec<Workspace>
where
    F: Fn(&str) -> Option<String>,
{
    // Index entities by slug for manifest cross-reference (memberships use UIDs).
    let entities_by_slug: BTreeMap<&str, &EntityInfo> = company_entities
        .values()
        .map(|e| (e.slug.as_str(), e))
        .collect();
    // Index local entries by slug for the cloud-only pass below.
    let local_by_slug: BTreeMap<&str, &LocalCompanyEntry> = local_companies
        .iter()
        .map(|e| (e.slug.as_str(), e))
        .collect();

    let mut by_slug: BTreeMap<String, Workspace> = BTreeMap::new();

    // 1. Local companies (manifest-first).
    for entry in local_companies {
        if !entry.dir_exists {
            // Phantom manifest entry — drop it (no folder = nothing to act on).
            continue;
        }

        let display_name = entry
            .display_name
            .clone()
            .unwrap_or_else(|| humanize_slug(&entry.slug));
        let local_path_str = Some(entry.path.to_string_lossy().to_string());

        let cloud_entity_for_slug = entities_by_slug.get(entry.slug.as_str()).copied();
        let membership_status = cloud_entity_for_slug.and_then(|ent| {
            memberships
                .iter()
                .find(|m| m.company_uid == ent.uid)
                .map(|m| m.status.clone())
        });

        let (state, cloud_uid, bucket_name, broken_reason) = match (&entry.cloud_uid, cloud_entity_for_slug, cloud_reachable) {
            // Manifest says connected, cloud confirms (UIDs match) → Synced.
            (Some(manifest_uid), Some(ent), true) if &ent.uid == manifest_uid => (
                WorkspaceState::Synced,
                Some(ent.uid.clone()),
                ent.bucket_name.clone().or_else(|| entry.bucket_name.clone()),
                None,
            ),
            // Manifest says connected, cloud has slug but UID differs → Broken.
            (Some(manifest_uid), Some(ent), true) => (
                WorkspaceState::Broken,
                Some(manifest_uid.clone()),
                entry.bucket_name.clone(),
                Some(format!(
                    "manifest cloud_uid {manifest_uid} does not match cloud entity {} for this slug",
                    ent.uid
                )),
            ),
            // Manifest says connected, cloud has no entry for this slug → Broken.
            (Some(manifest_uid), None, true) => (
                WorkspaceState::Broken,
                Some(manifest_uid.clone()),
                entry.bucket_name.clone(),
                Some(format!(
                    "manifest cloud_uid {manifest_uid} not found in your cloud memberships"
                )),
            ),
            // Manifest says connected, cloud unreachable → trust manifest (Synced).
            (Some(manifest_uid), _, false) => (
                WorkspaceState::Synced,
                Some(manifest_uid.clone()),
                entry.bucket_name.clone(),
                None,
            ),
            // Manifest silent, cloud has matching slug → Synced (cloud-driven).
            (None, Some(ent), true) => (
                WorkspaceState::Synced,
                Some(ent.uid.clone()),
                ent.bucket_name.clone(),
                None,
            ),
            // Manifest silent, cloud has no matching slug (or unreachable) → LocalOnly.
            (None, _, _) => (
                WorkspaceState::LocalOnly,
                None,
                None,
                None,
            ),
        };

        by_slug.insert(
            entry.slug.clone(),
            Workspace {
                slug: entry.slug.clone(),
                display_name,
                kind: WorkspaceKind::Company,
                state,
                cloud_uid,
                bucket_name,
                has_local_folder: true,
                local_path: local_path_str,
                membership_status,
                last_synced_at: last_synced_lookup(&entry.slug),
                broken_reason,
            },
        );
    }

    // 2. Cloud-only companies — memberships whose slug isn't represented locally.
    for mem in memberships {
        let entity = match company_entities.get(&mem.company_uid) {
            Some(e) => e,
            None => continue,
        };
        if by_slug.contains_key(&entity.slug) {
            continue;
        }
        let display_name = entity
            .name
            .clone()
            .or_else(|| local_by_slug.get(entity.slug.as_str()).and_then(|e| e.display_name.clone()))
            .unwrap_or_else(|| humanize_slug(&entity.slug));
        by_slug.insert(
            entity.slug.clone(),
            Workspace {
                slug: entity.slug.clone(),
                display_name,
                kind: WorkspaceKind::Company,
                state: WorkspaceState::CloudOnly,
                cloud_uid: Some(entity.uid.clone()),
                bucket_name: entity.bucket_name.clone(),
                has_local_folder: false,
                local_path: None,
                membership_status: Some(mem.status.clone()),
                last_synced_at: last_synced_lookup(&entity.slug),
                broken_reason: None,
            },
        );
    }

    // 3. Personal — always first.
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
        broken_reason: None,
    });

    ordered.extend(by_slug.into_values());
    ordered
}

// ── Tauri command: list_syncable_workspaces ───────────────────────────────────

#[tauri::command]
pub async fn list_syncable_workspaces() -> Result<WorkspacesResult, String> {
    let hq_root = resolve_hq_folder_path()?;
    let hq_folder_path = hq_root.to_string_lossy().to_string();
    let (local_companies, manifest_error) = discover_local_companies(&hq_root);

    let cloud_outcome: Result<
        (Option<EntityInfo>, Vec<MembershipInfo>, BTreeMap<String, EntityInfo>),
        String,
    > = async {
        let vault_url = resolve_vault_api_url()?;
        let jwt = resolve_jwt().await?;
        let vault = VaultClient::new(&vault_url, &jwt);

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
                        mem.company_uid,
                        mem.display_id()
                    ));
                }
            }
        }

        Ok((person, memberships, entities))
    }
    .await;

    let (cloud_reachable, error, person, memberships, entities) = match cloud_outcome {
        Ok((p, m, e)) => (true, None, p, m, e),
        Err(e) => {
            // Surface cloud errors to the persistent log alongside the UI
            // tooltip — the menubar's "Cloud unreachable" notice gives the
            // user a hover-tooltip with the message, but the log is the
            // canonical place to grep when reproducing or debugging without
            // a popover open. Pre-v0.1.25 schema mismatches (missing
            // membership uid) propagated as silent failures here.
            log("workspaces", &format!("cloud branch failed: {e}"));
            (false, Some(e), None, Vec::new(), BTreeMap::new())
        }
    };

    let workspaces = assemble_workspaces(
        &hq_root,
        person.as_ref(),
        &memberships,
        &entities,
        &local_companies,
        cloud_reachable,
        last_synced_at,
    );

    Ok(WorkspacesResult {
        workspaces,
        cloud_reachable,
        error,
        hq_folder_path,
        manifest_error,
    })
}

// ── Tauri command: connect_workspace_to_cloud ─────────────────────────────────

/// Provision a cloud bucket for the given local company `slug` and write BOTH:
///   1. Per-folder `companies/{slug}/.hq/config.json` (authoritative runtime).
///   2. Manifest patch with `cloud_uid` + `bucket_name` (best-effort —
///      logs and continues if manifest is missing/broken).
///
/// Idempotent: if an entity with this slug exists, we reuse its UID.
/// Reconnect-safe: re-running on a Broken workspace re-finds the cloud entity
/// and overwrites both records with the current truth.
#[tauri::command]
pub async fn connect_workspace_to_cloud(slug: String) -> Result<(), String> {
    log("workspaces", &format!("connect: slug='{slug}' start"));
    if slug.is_empty() {
        return Err("slug is required".to_string());
    }
    if slug == "personal" {
        return Err(
            "the Personal vault is auto-provisioned — no manual connect needed"
                .to_string(),
        );
    }

    let hq_root = resolve_hq_folder_path().map_err(|e| {
        log("workspaces", &format!("connect '{slug}': hq_root resolve failed: {e}"));
        e
    })?;
    log("workspaces", &format!("connect '{slug}': hq_root={}", hq_root.display()));

    // Resolve the folder path. Prefer the manifest's `path` field when set
    // (custom layouts); fall back to `companies/{slug}` for default HQs.
    let folder = match read_manifest(&hq_root) {
        ManifestLoad::Present(entries) => entries
            .into_iter()
            .find(|e| e.slug == slug)
            .map(|e| e.path)
            .unwrap_or_else(|| hq_root.join("companies").join(&slug)),
        _ => hq_root.join("companies").join(&slug),
    };
    log("workspaces", &format!("connect '{slug}': folder={}", folder.display()));

    if !folder.is_dir() {
        let err = format!(
            "no local folder at {} — cannot connect a missing directory",
            folder.display()
        );
        log("workspaces", &format!("connect '{slug}': {err}"));
        return Err(err);
    }

    let vault_url = resolve_vault_api_url().map_err(|e| {
        log("workspaces", &format!("connect '{slug}': vault_url resolve failed: {e}"));
        e
    })?;
    let jwt = resolve_jwt().await.map_err(|e| {
        log("workspaces", &format!("connect '{slug}': jwt resolve failed: {e}"));
        e
    })?;
    let vault = VaultClient::new(&vault_url, &jwt);

    let display_name = read_local_company_name(&hq_root, &slug)
        .unwrap_or_else(|| humanize_slug(&slug));

    log("workspaces", &format!("connect '{slug}': find_entity_by_slug start"));
    let uid = match vault
        .find_entity_by_slug("company", &slug)
        .await
        .map_err(|e| {
            let err = format!("find_by_slug '{slug}': {e}");
            log("workspaces", &format!("connect '{slug}': {err}"));
            err
        })? {
        Some(info) => {
            log("workspaces", &format!("connect '{slug}': found existing uid={}", info.uid));
            info.uid
        }
        None => {
            log("workspaces", &format!("connect '{slug}': creating new entity"));
            vault
                .create_entity(&CreateEntityInput {
                    entity_type: "company".to_string(),
                    slug: slug.clone(),
                    name: display_name,
                    email: None,
                    owner_uid: None,
                })
                .await
                .map_err(|e| {
                    let err = format!("create entity '{slug}': {e}");
                    log("workspaces", &format!("connect '{slug}': {err}"));
                    err
                })?
                .uid
        }
    };

    log("workspaces", &format!("connect '{slug}': provision_bucket uid={uid}"));
    let bucket = vault
        .provision_bucket(&uid)
        .await
        .map_err(|e| {
            let err = format!("provision_bucket for {uid}: {e}");
            log("workspaces", &format!("connect '{slug}': {err}"));
            err
        })?;
    log("workspaces", &format!("connect '{slug}': bucket={}", bucket.bucket_name));

    // Write 1: per-folder .hq/config.json (authoritative for runtime).
    let config = CompanyConfig {
        company_uid: uid.clone(),
        company_slug: slug.clone(),
        bucket_name: bucket.bucket_name.clone(),
        vault_api_url: vault_url,
    };
    let config_path = folder.join(".hq").join("config.json");
    if let Some(parent) = config_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            let err = format!("create_dir_all {}: {e}", parent.display());
            log("workspaces", &format!("connect '{slug}': {err}"));
            err
        })?;
    }
    let body = serde_json::to_string_pretty(&config)
        .map_err(|e| format!("serialize config: {e}"))?;
    std::fs::write(&config_path, body).map_err(|e| {
        let err = format!("write {}: {e}", config_path.display());
        log("workspaces", &format!("connect '{slug}': {err}"));
        err
    })?;
    log("workspaces", &format!("connect '{slug}': wrote config to {}", config_path.display()));

    // Write 2: patch manifest (best-effort). A failure here doesn't roll back
    // the per-folder config — runtime is correct, only audit/UI is degraded
    // until the next connect or repair pass.
    let manifest_path = hq_root.join("companies").join("manifest.yaml");
    if manifest_path.exists() {
        if let Err(e) = patch_manifest_with_cloud_info(
            &manifest_path,
            &slug,
            &uid,
            &bucket.bucket_name,
        ) {
            log("workspaces", &format!("manifest patch for '{slug}' failed (non-fatal): {e}"));
        } else {
            log("workspaces", &format!("connect '{slug}': manifest patched"));
        }
    }

    log("workspaces", &format!("connect '{slug}': complete"));
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
            // Tests historically mocked a top-level uid; the live API
            // returns membership_key instead. Synthesize one here so the
            // struct literal is complete.
            membership_key: Some(format!("{person_uid}#{company_uid}")),
        }
    }

    fn local(slug: &str, hq_root: &Path, exists: bool, name: Option<&str>) -> LocalCompanyEntry {
        local_full(slug, hq_root, exists, name, None, None)
    }

    fn local_full(
        slug: &str,
        hq_root: &Path,
        exists: bool,
        name: Option<&str>,
        cloud_uid: Option<&str>,
        bucket_name: Option<&str>,
    ) -> LocalCompanyEntry {
        let path = hq_root.join("companies").join(slug);
        if exists {
            std::fs::create_dir_all(&path).unwrap();
        }
        LocalCompanyEntry {
            slug: slug.into(),
            display_name: name.map(str::to_string),
            path,
            dir_exists: exists,
            cloud_uid: cloud_uid.map(str::to_string),
            bucket_name: bucket_name.map(str::to_string),
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

    // ── assemble_workspaces (manifest-first) ──────────────────────────────

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
            true,
            |_| None,
        );
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].slug, "personal");
        assert_eq!(result[0].kind, WorkspaceKind::Personal);
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
            true,
            |_| None,
        );
        assert_eq!(result.len(), 1);
        assert!(result[0].cloud_uid.is_none());
    }

    #[test]
    fn manifest_uid_matches_cloud_membership_is_synced() {
        let tmp = TempDir::new().unwrap();
        let p = person("prs_x", None);
        let mem = membership("mem_1", "prs_x", "cmp_a", "active");
        let mut entities = BTreeMap::new();
        entities.insert("cmp_a".to_string(), company_entity("cmp_a", "acme", Some("Acme")));
        let entries = vec![local_full("acme", tmp.path(), true, Some("Acme"), Some("cmp_a"), Some("hq-vault-cmp-a"))];

        let result = assemble_workspaces(
            tmp.path(),
            Some(&p),
            &[mem],
            &entities,
            &entries,
            true,
            |_| None,
        );
        assert_eq!(result[1].state, WorkspaceState::Synced);
        assert_eq!(result[1].cloud_uid.as_deref(), Some("cmp_a"));
        assert_eq!(result[1].membership_status.as_deref(), Some("active"));
        assert!(result[1].broken_reason.is_none());
    }

    #[test]
    fn manifest_uid_disagrees_with_cloud_is_broken() {
        let tmp = TempDir::new().unwrap();
        let p = person("prs_x", None);
        let mem = membership("mem_1", "prs_x", "cmp_NEW", "active");
        let mut entities = BTreeMap::new();
        entities.insert("cmp_NEW".to_string(), company_entity("cmp_NEW", "acme", Some("Acme")));
        let entries = vec![local_full("acme", tmp.path(), true, Some("Acme"), Some("cmp_OLD"), Some("hq-vault-cmp-old"))];

        let result = assemble_workspaces(
            tmp.path(),
            Some(&p),
            &[mem],
            &entities,
            &entries,
            true,
            |_| None,
        );
        assert_eq!(result[1].state, WorkspaceState::Broken);
        assert_eq!(result[1].cloud_uid.as_deref(), Some("cmp_OLD"));
        let reason = result[1].broken_reason.as_ref().unwrap();
        assert!(reason.contains("cmp_OLD"));
        assert!(reason.contains("cmp_NEW"));
    }

    #[test]
    fn manifest_uid_with_no_cloud_membership_is_broken() {
        let tmp = TempDir::new().unwrap();
        let p = person("prs_x", None);
        let entries = vec![local_full("acme", tmp.path(), true, None, Some("cmp_GONE"), Some("hq-vault-cmp-gone"))];

        let result = assemble_workspaces(
            tmp.path(),
            Some(&p),
            &[],
            &BTreeMap::new(),
            &entries,
            true,
            |_| None,
        );
        assert_eq!(result[1].state, WorkspaceState::Broken);
        assert_eq!(result[1].cloud_uid.as_deref(), Some("cmp_GONE"));
    }

    /// Cloud unreachable → trust manifest optimistically (Synced, not Broken).
    #[test]
    fn manifest_uid_with_cloud_unreachable_is_synced_optimistic() {
        let tmp = TempDir::new().unwrap();
        let p = person("prs_x", None);
        let entries = vec![local_full("acme", tmp.path(), true, None, Some("cmp_a"), Some("hq-vault-cmp-a"))];

        let result = assemble_workspaces(
            tmp.path(),
            Some(&p),
            &[],
            &BTreeMap::new(),
            &entries,
            false,
            |_| None,
        );
        assert_eq!(result[1].state, WorkspaceState::Synced);
        assert!(result[1].broken_reason.is_none());
    }

    #[test]
    fn manifest_silent_with_cloud_membership_is_synced() {
        let tmp = TempDir::new().unwrap();
        let p = person("prs_x", None);
        let mem = membership("mem_1", "prs_x", "cmp_a", "active");
        let mut entities = BTreeMap::new();
        entities.insert("cmp_a".to_string(), company_entity("cmp_a", "acme", Some("Acme")));
        let entries = vec![local("acme", tmp.path(), true, Some("Acme"))];

        let result = assemble_workspaces(
            tmp.path(),
            Some(&p),
            &[mem],
            &entities,
            &entries,
            true,
            |_| None,
        );
        assert_eq!(result[1].state, WorkspaceState::Synced);
        assert_eq!(result[1].cloud_uid.as_deref(), Some("cmp_a"));
    }

    #[test]
    fn manifest_silent_with_no_cloud_membership_is_local_only() {
        let tmp = TempDir::new().unwrap();
        let p = person("prs_x", None);
        let entries = vec![local("test-co", tmp.path(), true, None)];
        let result = assemble_workspaces(
            tmp.path(),
            Some(&p),
            &[],
            &BTreeMap::new(),
            &entries,
            true,
            |_| None,
        );
        assert_eq!(result[1].state, WorkspaceState::LocalOnly);
        assert!(result[1].cloud_uid.is_none());
    }

    #[test]
    fn membership_without_local_folder_is_cloud_only() {
        let tmp = TempDir::new().unwrap();
        let p = person("prs_x", None);
        let mem = membership("mem_1", "prs_x", "cmp_b", "pending");
        let mut entities = BTreeMap::new();
        entities.insert("cmp_b".to_string(), company_entity("cmp_b", "newco", None));
        let result = assemble_workspaces(
            tmp.path(),
            Some(&p),
            &[mem],
            &entities,
            &[],
            true,
            |_| None,
        );
        assert_eq!(result[1].state, WorkspaceState::CloudOnly);
        assert_eq!(result[1].membership_status.as_deref(), Some("pending"));
    }

    #[test]
    fn manifest_entry_without_folder_is_dropped() {
        let tmp = TempDir::new().unwrap();
        let p = person("prs_x", None);
        let entries = vec![local_full("phantom", tmp.path(), false, Some("Phantom"), Some("cmp_p"), None)];
        let result = assemble_workspaces(
            tmp.path(),
            Some(&p),
            &[],
            &BTreeMap::new(),
            &entries,
            true,
            |_| None,
        );
        assert_eq!(result.len(), 1);
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
            true,
            |_| None,
        );
        assert_eq!(result.len(), 1);
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
            true,
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
        let entries = vec![
            local("zoo", tmp.path(), true, None),
            local("alpha", tmp.path(), true, None),
            local("mango", tmp.path(), true, None),
        ];
        let result = assemble_workspaces(
            tmp.path(),
            Some(&p),
            &[],
            &BTreeMap::new(),
            &entries,
            true,
            |_| None,
        );
        let slugs: Vec<&str> = result.iter().map(|w| w.slug.as_str()).collect();
        assert_eq!(slugs, vec!["personal", "alpha", "mango", "zoo"]);
    }

    #[test]
    fn display_name_fallback_chain() {
        let tmp = TempDir::new().unwrap();
        let p = person("prs_x", None);
        let entries = vec![local("acme", tmp.path(), true, Some("Acme From Manifest"))];
        let result = assemble_workspaces(
            tmp.path(),
            Some(&p),
            &[],
            &BTreeMap::new(),
            &entries,
            true,
            |_| None,
        );
        assert_eq!(result[1].display_name, "Acme From Manifest");
    }

    // ── discover_local_companies / read_manifest ──────────────────────────

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

        let (entries, err) = discover_local_companies(tmp.path());
        assert!(err.is_none());
        assert_eq!(entries.len(), 2);
        let alpha = entries.iter().find(|e| e.slug == "alpha").unwrap();
        assert!(alpha.dir_exists);
        let beta = entries.iter().find(|e| e.slug == "beta").unwrap();
        assert!(!beta.dir_exists);
    }

    #[test]
    fn discover_reads_manifest_cloud_fields() {
        let tmp = TempDir::new().unwrap();
        write_manifest(
            tmp.path(),
            r#"
companies:
  alpha:
    name: "Alpha"
    path: "companies/alpha"
    cloud_uid: "cmp_01ABC"
    bucket_name: "hq-vault-cmp-01ABC"
"#,
        );
        std::fs::create_dir_all(tmp.path().join("companies/alpha")).unwrap();

        let (entries, _) = discover_local_companies(tmp.path());
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].cloud_uid.as_deref(), Some("cmp_01ABC"));
        assert_eq!(entries[0].bucket_name.as_deref(), Some("hq-vault-cmp-01ABC"));
    }

    /// Broken manifest YAML → fall back to dir enumeration AND surface error.
    /// Uses an unclosed single-quoted scalar — YAML's parser must reject this
    /// (it's not just a missing `companies:` key, which serde_yaml would
    /// happily deserialize as an empty manifest via #[serde(default)]).
    #[test]
    fn discover_broken_manifest_falls_back_with_error() {
        let tmp = TempDir::new().unwrap();
        write_manifest(tmp.path(), "companies:\n  acme:\n    name: 'unclosed scalar\n");
        std::fs::create_dir_all(tmp.path().join("companies/foo")).unwrap();

        let (entries, err) = discover_local_companies(tmp.path());
        assert!(err.is_some(), "unclosed quote must fail YAML parse, got entries={entries:?}");
        assert!(err.as_ref().unwrap().contains("parse"));
        let slugs: Vec<&str> = entries.iter().map(|e| e.slug.as_str()).collect();
        assert_eq!(slugs, vec!["foo"]);
    }

    #[test]
    fn discover_no_manifest_no_error() {
        let tmp = TempDir::new().unwrap();
        let (_, err) = discover_local_companies(tmp.path());
        assert!(err.is_none());
    }

    #[test]
    fn discover_fallback_skips_underscore_scaffolding() {
        let tmp = TempDir::new().unwrap();
        std::fs::create_dir_all(tmp.path().join("companies/_template")).unwrap();
        std::fs::create_dir_all(tmp.path().join("companies/real-co")).unwrap();
        let (entries, _) = discover_local_companies(tmp.path());
        let slugs: Vec<&str> = entries.iter().map(|e| e.slug.as_str()).collect();
        assert_eq!(slugs, vec!["real-co"]);
    }

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
        let (entries, _) = discover_local_companies(tmp.path());
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].slug, "_archive");
    }

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

    // ── patch_manifest_with_cloud_info ────────────────────────────────────

    #[test]
    fn patch_manifest_writes_cloud_uid_and_bucket() {
        let tmp = TempDir::new().unwrap();
        write_manifest(
            tmp.path(),
            r#"
companies:
  alpha:
    name: "Alpha"
    path: "companies/alpha"
"#,
        );
        let manifest_path = tmp.path().join("companies").join("manifest.yaml");

        patch_manifest_with_cloud_info(
            &manifest_path,
            "alpha",
            "cmp_NEW",
            "hq-vault-cmp-NEW",
        )
        .unwrap();

        let (entries, _) = discover_local_companies(tmp.path());
        let alpha = entries.iter().find(|e| e.slug == "alpha").unwrap();
        assert_eq!(alpha.cloud_uid.as_deref(), Some("cmp_NEW"));
        assert_eq!(alpha.bucket_name.as_deref(), Some("hq-vault-cmp-NEW"));
        assert_eq!(alpha.display_name.as_deref(), Some("Alpha"));
    }

    /// Reconnect after Broken: existing cloud_uid is overwritten cleanly.
    #[test]
    fn patch_manifest_overwrites_existing_cloud_uid() {
        let tmp = TempDir::new().unwrap();
        write_manifest(
            tmp.path(),
            r#"
companies:
  alpha:
    name: "Alpha"
    path: "companies/alpha"
    cloud_uid: "cmp_OLD"
    bucket_name: "hq-vault-cmp-OLD"
"#,
        );
        let manifest_path = tmp.path().join("companies").join("manifest.yaml");

        patch_manifest_with_cloud_info(
            &manifest_path,
            "alpha",
            "cmp_NEW",
            "hq-vault-cmp-NEW",
        )
        .unwrap();

        let (entries, _) = discover_local_companies(tmp.path());
        let alpha = entries.iter().find(|e| e.slug == "alpha").unwrap();
        assert_eq!(alpha.cloud_uid.as_deref(), Some("cmp_NEW"));
        assert_eq!(alpha.bucket_name.as_deref(), Some("hq-vault-cmp-NEW"));
    }

    #[test]
    fn patch_manifest_unknown_slug_errors() {
        let tmp = TempDir::new().unwrap();
        write_manifest(
            tmp.path(),
            r#"
companies:
  alpha:
    name: "Alpha"
"#,
        );
        let manifest_path = tmp.path().join("companies").join("manifest.yaml");
        let err = patch_manifest_with_cloud_info(&manifest_path, "ghost", "cmp_X", "bucket-X")
            .expect_err("missing slug must error");
        assert!(err.contains("ghost"));
    }

    #[test]
    fn patch_manifest_without_companies_key_errors() {
        let tmp = TempDir::new().unwrap();
        write_manifest(tmp.path(), "version: 1\n");
        let manifest_path = tmp.path().join("companies").join("manifest.yaml");
        let err = patch_manifest_with_cloud_info(&manifest_path, "any", "cmp_X", "bucket-X")
            .expect_err("missing companies key must error");
        assert!(err.to_lowercase().contains("companies"));
    }

    #[test]
    fn patch_manifest_cleans_up_tmp() {
        let tmp = TempDir::new().unwrap();
        write_manifest(
            tmp.path(),
            r#"
companies:
  alpha:
    name: "Alpha"
"#,
        );
        let manifest_path = tmp.path().join("companies").join("manifest.yaml");
        patch_manifest_with_cloud_info(&manifest_path, "alpha", "cmp_X", "bucket-X").unwrap();
        let tmp_path = manifest_path.with_extension("yaml.tmp");
        assert!(!tmp_path.exists());
    }
}
