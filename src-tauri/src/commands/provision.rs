//! Detect and provision unprovisioned `cloud: true` companies.
//!
//! `provision_missing_companies` walks `$HQ/companies/*/company.yaml`, keeps
//! entries where `cloud: true`, and handles three cases:
//!   A. `.hq/config.json` present → verify entity still exists via find_by_slug;
//!      if not found, remove stale config and re-provision via CLI.
//!   B. `.hq/config.json` absent but YAML has `cloudCompanyUid` → migration:
//!      look up entity, write config.json using the legacy UID, do NOT touch YAML.
//!   C. Otherwise → delegate to `hq cloud provision company <slug>` (the
//!      canonical CLI subcommand from `@indigoai-us/hq-cli`), which performs
//!      GET-then-POST idempotency, atomic manifest patch, atomic
//!      `.hq/config.json` write, AND triggers an initial sync via `share()`.
//!
//! `company.yaml` is read-only from this module with one deliberate exception:
//! `demote_company_to_local` flips `cloud: true → false` when hq-pro has
//! soft-tombstoned the cloud entity. Without that flip, the next sync would
//! re-enter Path C and create a fresh cloud company — exactly what the user
//! tried to avoid by hitting Delete in the console.
//!
//! ## Why Paths A + B stay inline (not CLI)
//!
//! Path A is a pure local-cache fast path: if `.hq/config.json` already exists
//! and the cloud entity is still alive, there is nothing to do. Spawning the
//! CLI would re-run idempotency checks the local cache already short-circuits.
//!
//! Path B is a one-shot migration from the legacy `cloudCompanyUid` field
//! that older `hq-installer` versions wrote into `company.yaml`. The CLI has
//! no equivalent of "promote a known UID into a config.json without touching
//! the entity"; it would either reuse-by-slug (different UID) or re-create
//! (also different UID). Keeping the migration inline preserves the legacy
//! UID exactly as recorded.
//!
//! Only Path C goes through the CLI — that is where the GET-then-POST,
//! manifest patch, config write, and initial sync all happen behind one
//! canonical implementation.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::commands::run_cli_provision::{run_cli_provision, CliProvisionError};
use crate::commands::vault_client::VaultClient;
use crate::util::logfile::log;

// ── Public types ──────────────────────────────────────────────────────────────

/// Per-company `.hq/config.json` schema (pinned — plan.md §Step 5).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompanyConfig {
    pub company_uid: String,
    pub company_slug: String,
    pub bucket_name: String,
    pub vault_api_url: String,
}

/// Returned by `provision_missing_companies` for each newly-provisioned
/// (or legacy-migrated) company.
#[derive(Debug, Clone)]
pub struct ProvisionedCompany {
    pub slug: String,
    pub uid: String,
    pub bucket_name: String,
}

// ── Internal YAML shape ───────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct CompanyYaml {
    cloud: Option<bool>,
    name: Option<String>,
    /// Legacy field written by earlier versions of hq-installer.
    /// Present means the company was provisioned before `.hq/config.json` was
    /// introduced.  Must not be written back.
    #[serde(rename = "cloudCompanyUid")]
    cloud_company_uid: Option<String>,
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Atomic write: serialize `config` → temp file → rename.
fn write_company_config(config_path: &Path, config: &CompanyConfig) -> Result<(), String> {
    if let Some(parent) = config_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("create_dir_all {}: {e}", parent.display()))?;
    }
    let body = serde_json::to_string_pretty(config)
        .map_err(|e| format!("serialize config: {e}"))?;
    let tmp = config_path.with_file_name(format!(
        ".config.json.tmp.{}",
        std::process::id()
    ));
    std::fs::write(&tmp, &body).map_err(|e| format!("write tmp config: {e}"))?;
    std::fs::rename(&tmp, config_path)
        .map_err(|e| format!("rename config: {e}"))?;
    Ok(())
}

/// Flip `cloud: true` → `cloud: false` in `companies/{slug}/company.yaml` and
/// preserve every other key. No-op when the field is already `false` or
/// missing. Atomic (tmp + rename).
fn flip_company_cloud_off(yaml_path: &Path) -> Result<(), String> {
    let bytes = std::fs::read(yaml_path)
        .map_err(|e| format!("read {}: {e}", yaml_path.display()))?;
    let mut value: serde_yaml::Value = serde_yaml::from_slice(&bytes)
        .map_err(|e| format!("parse {}: {e}", yaml_path.display()))?;
    let mapping = value
        .as_mapping_mut()
        .ok_or_else(|| format!("{} root is not a mapping", yaml_path.display()))?;
    mapping.insert(
        serde_yaml::Value::String("cloud".to_string()),
        serde_yaml::Value::Bool(false),
    );
    let serialized = serde_yaml::to_string(&value)
        .map_err(|e| format!("serialize {}: {e}", yaml_path.display()))?;
    let tmp = yaml_path.with_extension("yaml.tmp");
    std::fs::write(&tmp, &serialized)
        .map_err(|e| format!("write tmp {}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, yaml_path)
        .map_err(|e| format!("rename {} → {}: {e}", tmp.display(), yaml_path.display()))?;
    Ok(())
}

/// Remove `cloud_uid` and `bucket_name` from the `companies.{slug}` entry of
/// `companies/manifest.yaml`. Other fields on the entry (e.g. `name`, `path`)
/// are preserved. The slug entry itself is preserved so the company stays
/// listed locally. No-op when the manifest is missing or doesn't have a
/// matching slug entry.
fn strip_manifest_cloud_for_slug(manifest_path: &Path, slug: &str) -> Result<(), String> {
    if !manifest_path.exists() {
        return Ok(());
    }
    let bytes = std::fs::read(manifest_path)
        .map_err(|e| format!("read manifest: {e}"))?;
    let mut value: serde_yaml::Value = serde_yaml::from_slice(&bytes)
        .map_err(|e| format!("parse manifest: {e}"))?;
    let companies_key = serde_yaml::Value::String("companies".to_string());
    let Some(mapping) = value.as_mapping_mut() else {
        return Ok(());
    };
    let Some(companies) = mapping.get_mut(&companies_key).and_then(|v| v.as_mapping_mut()) else {
        return Ok(());
    };
    let slug_key = serde_yaml::Value::String(slug.to_string());
    let Some(entry) = companies.get_mut(&slug_key).and_then(|v| v.as_mapping_mut()) else {
        return Ok(());
    };
    entry.remove(&serde_yaml::Value::String("cloud_uid".to_string()));
    entry.remove(&serde_yaml::Value::String("bucket_name".to_string()));
    let serialized = serde_yaml::to_string(&value)
        .map_err(|e| format!("serialize manifest: {e}"))?;
    let tmp = manifest_path.with_extension("yaml.tmp");
    std::fs::write(&tmp, &serialized).map_err(|e| format!("write tmp manifest: {e}"))?;
    std::fs::rename(&tmp, manifest_path)
        .map_err(|e| format!("rename manifest: {e}"))?;
    Ok(())
}

/// Convert a previously-cloud company to local-only after hq-pro has
/// soft-tombstoned the cloud entity:
///   1. Remove `companies/{slug}/.hq/config.json` (cloud-bound runtime cache).
///   2. Flip `cloud: true → false` in `companies/{slug}/company.yaml` so the
///      next sync doesn't re-enter the provision path.
///   3. Strip `cloud_uid` + `bucket_name` from `companies/manifest.yaml`'s
///      slug entry. The local folder + entry stays.
///
/// Idempotent: every step is no-op-safe when its target is already in the
/// post-demote state. Tolerant of missing manifest / missing config — only
/// the YAML flip needs to succeed for the demote to be stable.
pub(crate) fn demote_company_to_local(hq_root: &Path, slug: &str) -> Result<(), String> {
    let folder = hq_root.join("companies").join(slug);
    let yaml_path = folder.join("company.yaml");
    let config_path = folder.join(".hq").join("config.json");
    let manifest_path = hq_root.join("companies").join("manifest.yaml");

    if config_path.exists() {
        std::fs::remove_file(&config_path)
            .map_err(|e| format!("remove {}: {e}", config_path.display()))?;
    }
    if yaml_path.exists() {
        flip_company_cloud_off(&yaml_path)?;
    }
    strip_manifest_cloud_for_slug(&manifest_path, slug)?;
    Ok(())
}

// ── Core logic ────────────────────────────────────────────────────────────────

/// Walk `$hq_root/companies/*/company.yaml`, detect unprovisioned `cloud: true`
/// companies, provision them, and return the list of newly-provisioned entries.
///
/// `vault_api_url` is written verbatim into each company's `.hq/config.json`.
pub async fn provision_missing_companies(
    hq_root: &Path,
    vault: &VaultClient,
    vault_api_url: &str,
) -> Result<Vec<ProvisionedCompany>, String> {
    let companies_dir = hq_root.join("companies");
    if !companies_dir.exists() {
        return Ok(vec![]);
    }

    let entries = std::fs::read_dir(&companies_dir)
        .map_err(|e| format!("read companies dir {}: {e}", companies_dir.display()))?;

    let mut result: Vec<ProvisionedCompany> = Vec::new();

    for entry in entries {
        let entry = entry.map_err(|e| format!("dir entry error: {e}"))?;
        let folder_path = entry.path();
        if !folder_path.is_dir() {
            continue;
        }
        let folder_name = match entry.file_name().into_string() {
            Ok(n) => n,
            Err(_) => continue, // non-UTF-8 folder names are silently skipped
        };

        let yaml_path = folder_path.join("company.yaml");
        if !yaml_path.exists() {
            continue;
        }

        // Read YAML read-only — bytes preserved so SHA256 can be validated by callers
        let yaml_bytes = std::fs::read(&yaml_path)
            .map_err(|e| format!("read {}: {e}", yaml_path.display()))?;
        let company_yaml: CompanyYaml = serde_yaml::from_slice(&yaml_bytes)
            .map_err(|e| format!("parse {}: {e}", yaml_path.display()))?;

        if !company_yaml.cloud.unwrap_or(false) {
            continue;
        }

        let hq_config_path: PathBuf = folder_path.join(".hq").join("config.json");

        // ── Path A: config.json already present ────────────────────────────────
        if hq_config_path.exists() {
            match vault.find_entity_by_slug("company", &folder_name).await {
                Ok(Some(info)) if info.deleted == Some(true) => {
                    // Cloud entity tombstoned via hq-console (Settings → Delete).
                    // Demote to local-only: drop .hq/config.json + manifest cloud
                    // refs and flip company.yaml `cloud: true → false`. Skip the
                    // re-provision path so we don't silently mint a fresh
                    // cloud company the user just deleted.
                    log(
                        "provision",
                        &format!(
                            "demote '{}': cloud entity is soft-tombstoned (deleted=true), converting to local-only",
                            folder_name
                        ),
                    );
                    if let Err(e) = demote_company_to_local(hq_root, &folder_name) {
                        // Non-fatal: log + skip. The next sync re-detects
                        // deleted=true and tries again.
                        log(
                            "provision",
                            &format!("demote '{folder_name}' failed (non-fatal): {e}"),
                        );
                    }
                    continue;
                }
                Ok(Some(_)) => continue, // provisioned and verified
                Ok(None) => {
                    // Stale config — entity gone; remove and fall through to re-provision
                    let _ = std::fs::remove_file(&hq_config_path);
                }
                Err(e) => {
                    return Err(format!(
                        "vault lookup for '{}': {e}",
                        folder_name
                    ));
                }
            }
        }

        // ── Path B: legacy cloudCompanyUid migration ───────────────────────────
        if let Some(ref legacy_uid) = company_yaml.cloud_company_uid {
            match vault.find_entity_by_slug("company", &folder_name).await {
                Ok(Some(info)) => {
                    // If the entity has no bucket yet, provision it now — same contract as Path C.
                    let bucket_name = match info.bucket_name {
                        Some(b) => b,
                        None => vault
                            .provision_bucket(legacy_uid)
                            .await
                            .map_err(|e| format!("provision_bucket legacy '{}' uid={legacy_uid}: {e}", folder_name))?
                            .bucket_name,
                    };
                    let cfg = CompanyConfig {
                        company_uid: legacy_uid.clone(),
                        company_slug: folder_name.clone(),
                        bucket_name: bucket_name.clone(),
                        vault_api_url: vault_api_url.to_string(),
                    };
                    write_company_config(&hq_config_path, &cfg)?;
                    result.push(ProvisionedCompany {
                        slug: folder_name,
                        uid: legacy_uid.clone(),
                        bucket_name,
                    });
                    continue;
                }
                Ok(None) => {
                    // Legacy UID in YAML but entity not found — fall through to full provision
                }
                Err(e) => {
                    return Err(format!(
                        "vault legacy lookup for '{}': {e}",
                        folder_name
                    ));
                }
            }
        }

        // ── Path C: unprovisioned — delegate to `hq cloud provision company` ─
        //
        // The CLI subprocess is the canonical source of truth for:
        //   * GET-then-POST entity idempotency
        //   * Atomic `companies/manifest.yaml` patch (cloud_uid + bucket_name)
        //   * Atomic `companies/<slug>/.hq/config.json` write
        //   * Initial sync via `share()` from `@indigoai-us/hq-cloud`
        //
        // We pass through a friendly display `--name` from the YAML when present
        // (the CLI defaults to slug otherwise). On exit code 3 the CLI still
        // writes the config + manifest before failing, and the partial result
        // carries the `cloud_uid` — we record the company so the caller's
        // "newly provisioned" emit fires for UI feedback, then surface the
        // sync error so the operator can investigate.
        //
        // NB: we deliberately do NOT fall back to the legacy direct-vault
        // path on CLI failure. Doing so would re-introduce the divergence
        // this refactor exists to eliminate (see
        // workspace/reports/cloud-promote-architecture-2026-04-27.md).
        let display_name = company_yaml.name.as_deref();
        match run_cli_provision(&folder_name, display_name, hq_root).await {
            Ok(cli_result) => {
                result.push(ProvisionedCompany {
                    slug: folder_name,
                    uid: cli_result.cloud_uid,
                    bucket_name: cli_result.bucket_name,
                });
            }
            Err(CliProvisionError::Sync { partial, message }) => {
                // Entity + manifest + config all succeeded — only the initial
                // sync failed. Record the provisioned company (so the UI shows
                // "ready, sync pending") and propagate the error so callers
                // surface a notice. Subsequent sync runs will retry uploads
                // through the normal `first_push` path.
                if let Some(p) = partial {
                    result.push(ProvisionedCompany {
                        slug: folder_name.clone(),
                        uid: p.cloud_uid,
                        bucket_name: p.bucket_name,
                    });
                }
                return Err(format!(
                    "provision '{folder_name}' via hq CLI: {message}"
                ));
            }
            Err(e) => {
                return Err(format!(
                    "provision '{folder_name}' via hq CLI: {e}"
                ));
            }
        }
    }

    Ok(result)
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};
    use tempfile::TempDir;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn vault(server: &MockServer) -> VaultClient {
        VaultClient::new(server.uri(), "test-jwt")
    }

    const VAULT_URL: &str = "https://vault.test.getindigo.ai";

    /// Create a company directory with an optional company.yaml and return the
    /// yaml path (if created).
    fn setup_company(root: &Path, slug: &str, yaml: Option<&str>) -> PathBuf {
        let dir = root.join("companies").join(slug);
        std::fs::create_dir_all(&dir).unwrap();
        let yaml_path = dir.join("company.yaml");
        if let Some(content) = yaml {
            std::fs::write(&yaml_path, content).unwrap();
        }
        yaml_path
    }

    fn sha256_file(path: &Path) -> String {
        let bytes = std::fs::read(path).unwrap();
        format!("{:x}", Sha256::digest(&bytes))
    }

    fn entity_json(uid: &str, slug: &str, bucket: Option<&str>) -> serde_json::Value {
        let mut v = serde_json::json!({
            "entity": {
                "uid": uid,
                "slug": slug,
                "type": "company",
                "status": "active",
                "createdAt": "2026-01-01T00:00:00Z"
            }
        });
        if let Some(b) = bucket {
            v["entity"]["bucketName"] = serde_json::Value::String(b.to_string());
        }
        v
    }

    fn bucket_json(bucket: &str) -> serde_json::Value {
        serde_json::json!({ "bucketName": bucket, "kmsKeyId": "key-1" })
    }

    // (a) cloud: false → skipped
    #[tokio::test]
    async fn test_cloud_false_skipped() {
        let tmp = TempDir::new().unwrap();
        setup_company(tmp.path(), "acme", Some("cloud: false\nname: Acme\n"));
        let server = MockServer::start().await;
        let result = provision_missing_companies(tmp.path(), &vault(&server), VAULT_URL)
            .await
            .unwrap();
        assert!(result.is_empty());
        assert!(server.received_requests().await.unwrap().is_empty());
    }

    // (b) no company.yaml → skipped
    #[tokio::test]
    async fn test_no_yaml_skipped() {
        let tmp = TempDir::new().unwrap();
        setup_company(tmp.path(), "acme", None); // directory but no yaml
        let server = MockServer::start().await;
        let result = provision_missing_companies(tmp.path(), &vault(&server), VAULT_URL)
            .await
            .unwrap();
        assert!(result.is_empty());
        assert!(server.received_requests().await.unwrap().is_empty());
    }

    // (c) .hq/config.json present + find_by_slug returns 200 → skipped (no provisioning)
    #[tokio::test]
    async fn test_config_json_exists_and_entity_200_skipped() {
        let tmp = TempDir::new().unwrap();
        let slug = "acme";
        setup_company(tmp.path(), slug, Some("cloud: true\nname: Acme\n"));
        // Write an existing config.json
        let hq_dir = tmp.path().join("companies").join(slug).join(".hq");
        std::fs::create_dir_all(&hq_dir).unwrap();
        let cfg = CompanyConfig {
            company_uid: "cmp_existing".to_string(),
            company_slug: slug.to_string(),
            bucket_name: "hq-vault-cmp-existing".to_string(),
            vault_api_url: VAULT_URL.to_string(),
        };
        std::fs::write(
            hq_dir.join("config.json"),
            serde_json::to_string_pretty(&cfg).unwrap(),
        )
        .unwrap();

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(format!("/entity/by-slug/company/{slug}")))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(&entity_json("cmp_existing", slug, Some("hq-vault-cmp-existing"))),
            )
            .mount(&server)
            .await;

        let result = provision_missing_companies(tmp.path(), &vault(&server), VAULT_URL)
            .await
            .unwrap();
        assert!(result.is_empty(), "already-provisioned company must be skipped");
        // Only find_by_slug was called — no create_entity, no provision_bucket
        let reqs = server.received_requests().await.unwrap();
        assert!(
            reqs.iter().all(|r| r.url.path().contains("by-slug")),
            "only by-slug calls expected; got: {:?}",
            reqs.iter().map(|r| r.url.path()).collect::<Vec<_>>()
        );
    }

    // (d) legacy cloudCompanyUid, no .hq/config.json → migration; YAML unchanged
    #[tokio::test]
    async fn test_legacy_uid_migration_yaml_unchanged() {
        let tmp = TempDir::new().unwrap();
        let slug = "legacy-co";
        let yaml_content = "cloud: true\nname: Legacy Co\ncloudCompanyUid: cmp_legacy\n";
        let yaml_path = setup_company(tmp.path(), slug, Some(yaml_content));
        let sha_before = sha256_file(&yaml_path);

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(format!("/entity/by-slug/company/{slug}")))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(&entity_json(
                    "cmp_legacy",
                    slug,
                    Some("hq-vault-cmp-legacy"),
                )),
            )
            .mount(&server)
            .await;

        let result = provision_missing_companies(tmp.path(), &vault(&server), VAULT_URL)
            .await
            .unwrap();

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].uid, "cmp_legacy");
        assert_eq!(result[0].bucket_name, "hq-vault-cmp-legacy");

        // config.json must have been written
        let config_path = tmp
            .path()
            .join("companies")
            .join(slug)
            .join(".hq")
            .join("config.json");
        assert!(config_path.exists());
        let written: CompanyConfig =
            serde_json::from_str(&std::fs::read_to_string(&config_path).unwrap()).unwrap();
        assert_eq!(written.company_uid, "cmp_legacy");
        assert_eq!(written.bucket_name, "hq-vault-cmp-legacy");

        // YAML must be byte-for-byte unchanged
        let sha_after = sha256_file(&yaml_path);
        assert_eq!(sha_before, sha_after, "company.yaml was modified");
    }

    // (d2) legacy cloudCompanyUid, entity found but bucket_name: None → provision_bucket called
    #[tokio::test]
    async fn test_legacy_uid_entity_without_bucket_provisions() {
        let tmp = TempDir::new().unwrap();
        let slug = "legacy-no-bucket";
        let yaml_content = "cloud: true\nname: Legacy No Bucket\ncloudCompanyUid: cmp_legacy\n";
        setup_company(tmp.path(), slug, Some(yaml_content));

        let server = MockServer::start().await;
        // find_by_slug returns entity with NO bucket
        Mock::given(method("GET"))
            .and(path(format!("/entity/by-slug/company/{slug}")))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(&entity_json("cmp_legacy", slug, None)),
            )
            .mount(&server)
            .await;
        // provision_bucket called because bucket was absent
        Mock::given(method("POST"))
            .and(path("/provision/bucket"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(&bucket_json("hq-vault-cmp-legacy")),
            )
            .mount(&server)
            .await;

        let result = provision_missing_companies(tmp.path(), &vault(&server), VAULT_URL)
            .await
            .unwrap();

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].uid, "cmp_legacy");
        assert_eq!(result[0].bucket_name, "hq-vault-cmp-legacy");

        // provision_bucket must have been called exactly once with companyUid == "cmp_legacy"
        let reqs = server.received_requests().await.unwrap();
        let bucket_calls: Vec<_> = reqs
            .iter()
            .filter(|r| r.url.path() == "/provision/bucket")
            .collect();
        assert_eq!(bucket_calls.len(), 1, "provision_bucket must be called exactly once");
        let body: serde_json::Value = serde_json::from_slice(&bucket_calls[0].body).unwrap();
        assert_eq!(body["companyUid"], "cmp_legacy");

        // config.json must have non-empty bucket name
        let config_path = tmp
            .path()
            .join("companies")
            .join(slug)
            .join(".hq")
            .join("config.json");
        let written: CompanyConfig =
            serde_json::from_str(&std::fs::read_to_string(&config_path).unwrap()).unwrap();
        assert_eq!(written.bucket_name, "hq-vault-cmp-legacy");
        assert!(!written.bucket_name.is_empty(), "bucket_name must not be empty");
    }

    // (e) new folder → create + provision + write config.json; YAML unchanged
    #[tokio::test]
    async fn test_new_folder_provisioned_yaml_unchanged() {
        let tmp = TempDir::new().unwrap();
        let slug = "new-co";
        let yaml_content = "cloud: true\nname: New Co\n";
        let yaml_path = setup_company(tmp.path(), slug, Some(yaml_content));
        let sha_before = sha256_file(&yaml_path);

        let server = MockServer::start().await;
        // find_by_slug → 404 (not found)
        Mock::given(method("GET"))
            .and(path(format!("/entity/by-slug/company/{slug}")))
            .respond_with(ResponseTemplate::new(404).set_body_json(&serde_json::json!({
                "message": "not found"
            })))
            .mount(&server)
            .await;
        // create_entity → new uid
        Mock::given(method("POST"))
            .and(path("/entity"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(&entity_json("cmp_new", slug, None)),
            )
            .mount(&server)
            .await;
        // provision_bucket → bucket
        Mock::given(method("POST"))
            .and(path("/provision/bucket"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(&bucket_json("hq-vault-cmp-new")),
            )
            .mount(&server)
            .await;

        let result = provision_missing_companies(tmp.path(), &vault(&server), VAULT_URL)
            .await
            .unwrap();

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].uid, "cmp_new");
        assert_eq!(result[0].bucket_name, "hq-vault-cmp-new");

        let config_path = tmp
            .path()
            .join("companies")
            .join(slug)
            .join(".hq")
            .join("config.json");
        let written: CompanyConfig =
            serde_json::from_str(&std::fs::read_to_string(&config_path).unwrap()).unwrap();
        assert_eq!(written.company_uid, "cmp_new");

        // YAML byte-for-byte unchanged
        let sha_after = sha256_file(&yaml_path);
        assert_eq!(sha_before, sha_after, "company.yaml was modified");
    }

    // (f) find_by_slug returns existing UID → create_entity NEVER called;
    //     provision_bucket("cmp_preexisting") called; config.json has "cmp_preexisting"
    #[tokio::test]
    async fn test_find_by_slug_reuses_uid_no_create() {
        let tmp = TempDir::new().unwrap();
        let slug = "pre-existing";
        setup_company(tmp.path(), slug, Some("cloud: true\nname: Pre Co\n"));

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(format!("/entity/by-slug/company/{slug}")))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(&entity_json(
                    "cmp_preexisting",
                    slug,
                    None,
                )),
            )
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/provision/bucket"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(&bucket_json("hq-vault-cmp-preexisting")),
            )
            .mount(&server)
            .await;

        let result = provision_missing_companies(tmp.path(), &vault(&server), VAULT_URL)
            .await
            .unwrap();

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].uid, "cmp_preexisting");

        // Verify create_entity was NEVER called
        let reqs = server.received_requests().await.unwrap();
        let create_calls: Vec<_> = reqs
            .iter()
            .filter(|r| r.method == wiremock::http::Method::POST && r.url.path() == "/entity")
            .collect();
        assert!(
            create_calls.is_empty(),
            "create_entity must not be called when find_by_slug returns an entity: {:?}",
            create_calls
        );

        // Verify provision_bucket was called (with cmp_preexisting in body)
        let bucket_calls: Vec<_> = reqs
            .iter()
            .filter(|r| r.url.path() == "/provision/bucket")
            .collect();
        assert_eq!(bucket_calls.len(), 1, "provision_bucket must be called once");
        let body: serde_json::Value =
            serde_json::from_slice(&bucket_calls[0].body).unwrap();
        assert_eq!(body["companyUid"], "cmp_preexisting");

        // config.json must use cmp_preexisting
        let config_path = tmp
            .path()
            .join("companies")
            .join(slug)
            .join(".hq")
            .join("config.json");
        let written: CompanyConfig =
            serde_json::from_str(&std::fs::read_to_string(&config_path).unwrap()).unwrap();
        assert_eq!(written.company_uid, "cmp_preexisting");
    }

    // (g) find_by_slug returns null → create_entity called exactly once;
    //     config.json uses the new UID
    #[tokio::test]
    async fn test_find_by_slug_null_creates_entity_once() {
        let tmp = TempDir::new().unwrap();
        let slug = "brand-new";
        setup_company(tmp.path(), slug, Some("cloud: true\nname: Brand New\n"));

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(format!("/entity/by-slug/company/{slug}")))
            .respond_with(
                ResponseTemplate::new(404)
                    .set_body_json(&serde_json::json!({ "message": "not found" })),
            )
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/entity"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(&entity_json("cmp_created", slug, None)),
            )
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/provision/bucket"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(&bucket_json("hq-vault-cmp-created")),
            )
            .mount(&server)
            .await;

        let result = provision_missing_companies(tmp.path(), &vault(&server), VAULT_URL)
            .await
            .unwrap();

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].uid, "cmp_created");

        // create_entity called exactly once
        let reqs = server.received_requests().await.unwrap();
        let create_calls: Vec<_> = reqs
            .iter()
            .filter(|r| r.method == wiremock::http::Method::POST && r.url.path() == "/entity")
            .collect();
        assert_eq!(create_calls.len(), 1, "create_entity must be called exactly once");

        // config.json uses the created UID
        let config_path = tmp
            .path()
            .join("companies")
            .join(slug)
            .join(".hq")
            .join("config.json");
        let written: CompanyConfig =
            serde_json::from_str(&std::fs::read_to_string(&config_path).unwrap()).unwrap();
        assert_eq!(written.company_uid, "cmp_created");
    }

    // ── demote_company_to_local ─────────────────────────────────────────────────

    fn read_yaml_value(path: &Path) -> serde_yaml::Value {
        serde_yaml::from_slice(&std::fs::read(path).unwrap()).unwrap()
    }

    /// Helper: minimal happy-path manifest with one slug entry carrying cloud refs.
    fn write_manifest(root: &Path, slug: &str) {
        let dir = root.join("companies");
        std::fs::create_dir_all(&dir).unwrap();
        let body = format!(
            "companies:\n  {slug}:\n    name: Acme\n    cloud_uid: cmp_old\n    bucket_name: hq-vault-cmp-old\n"
        );
        std::fs::write(dir.join("manifest.yaml"), body).unwrap();
    }

    #[test]
    fn demote_clears_config_flips_yaml_strips_manifest_cloud() {
        let tmp = TempDir::new().unwrap();
        let slug = "acme";
        let yaml_path = setup_company(
            tmp.path(),
            slug,
            Some("cloud: true\nname: Acme\n"),
        );
        let hq_dir = tmp.path().join("companies").join(slug).join(".hq");
        std::fs::create_dir_all(&hq_dir).unwrap();
        std::fs::write(hq_dir.join("config.json"), "{}").unwrap();
        write_manifest(tmp.path(), slug);

        demote_company_to_local(tmp.path(), slug).unwrap();

        // .hq/config.json removed.
        assert!(!hq_dir.join("config.json").exists());
        // company.yaml has cloud: false.
        let yaml = read_yaml_value(&yaml_path);
        assert_eq!(
            yaml.get("cloud").and_then(serde_yaml::Value::as_bool),
            Some(false)
        );
        assert_eq!(
            yaml.get("name").and_then(serde_yaml::Value::as_str),
            Some("Acme")
        );
        // manifest entry kept, cloud_uid + bucket_name stripped.
        let manifest = read_yaml_value(&tmp.path().join("companies").join("manifest.yaml"));
        let entry = manifest
            .get("companies")
            .and_then(|v| v.get(slug))
            .and_then(serde_yaml::Value::as_mapping)
            .unwrap();
        assert!(!entry.contains_key("cloud_uid"));
        assert!(!entry.contains_key("bucket_name"));
        // Other fields (e.g. `name`) preserved.
        assert!(entry.contains_key("name"));
    }

    #[test]
    fn demote_is_idempotent_when_already_local() {
        let tmp = TempDir::new().unwrap();
        let slug = "acme";
        let yaml_path = setup_company(
            tmp.path(),
            slug,
            Some("cloud: false\nname: Acme\n"),
        );
        // No .hq/config.json. Manifest with no cloud refs.
        std::fs::create_dir_all(tmp.path().join("companies")).unwrap();
        std::fs::write(
            tmp.path().join("companies").join("manifest.yaml"),
            format!("companies:\n  {slug}:\n    name: Acme\n"),
        )
        .unwrap();

        demote_company_to_local(tmp.path(), slug).unwrap();

        let yaml = read_yaml_value(&yaml_path);
        assert_eq!(
            yaml.get("cloud").and_then(serde_yaml::Value::as_bool),
            Some(false)
        );
    }

    #[test]
    fn demote_tolerates_missing_manifest() {
        // A user could have a company folder + yaml but no top-level manifest.
        // Demote must not blow up — the .hq/config.json delete + cloud:false
        // flip are still meaningful.
        let tmp = TempDir::new().unwrap();
        let slug = "acme";
        let yaml_path = setup_company(
            tmp.path(),
            slug,
            Some("cloud: true\nname: Acme\n"),
        );

        demote_company_to_local(tmp.path(), slug).unwrap();

        let yaml = read_yaml_value(&yaml_path);
        assert_eq!(
            yaml.get("cloud").and_then(serde_yaml::Value::as_bool),
            Some(false)
        );
    }

    // ── Path A: deleted=true triggers demote, NOT re-provision ──────────────────

    #[tokio::test]
    async fn config_json_present_and_entity_deleted_demotes_silently() {
        let tmp = TempDir::new().unwrap();
        let slug = "tombstoned";
        let yaml_path = setup_company(
            tmp.path(),
            slug,
            Some("cloud: true\nname: Tomb\n"),
        );
        let hq_dir = tmp.path().join("companies").join(slug).join(".hq");
        std::fs::create_dir_all(&hq_dir).unwrap();
        let cfg = CompanyConfig {
            company_uid: "cmp_tomb".to_string(),
            company_slug: slug.to_string(),
            bucket_name: "hq-vault-cmp-tomb".to_string(),
            vault_api_url: VAULT_URL.to_string(),
        };
        std::fs::write(
            hq_dir.join("config.json"),
            serde_json::to_string_pretty(&cfg).unwrap(),
        )
        .unwrap();
        write_manifest(tmp.path(), slug);

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(format!("/entity/by-slug/company/{slug}")))
            .respond_with(ResponseTemplate::new(200).set_body_json(&serde_json::json!({
                "entity": {
                    "uid": "cmp_tomb", "slug": slug, "type": "company",
                    "name": "Tomb", "status": "active",
                    "createdAt": "2026-01-01T00:00:00Z",
                    "deleted": true
                }
            })))
            .mount(&server)
            .await;

        let result = provision_missing_companies(tmp.path(), &vault(&server), VAULT_URL)
            .await
            .unwrap();

        // Demoted, not re-provisioned: no entries in result, no POST traffic.
        assert!(result.is_empty(), "deleted company must not be re-provisioned");
        let reqs = server.received_requests().await.unwrap();
        assert!(
            reqs.iter().all(|r| r.method == wiremock::http::Method::GET),
            "demote path must not POST to /entity or /provision/bucket"
        );

        // Demote side-effects landed.
        assert!(!hq_dir.join("config.json").exists());
        let yaml = read_yaml_value(&yaml_path);
        assert_eq!(
            yaml.get("cloud").and_then(serde_yaml::Value::as_bool),
            Some(false)
        );
    }
}
