//! Detect and provision unprovisioned `cloud: true` companies.
//!
//! `provision_missing_companies` walks `$HQ/companies/*/company.yaml`, keeps
//! entries where `cloud: true`, and handles three cases:
//!   A. `.hq/config.json` present → verify entity still exists via find_by_slug;
//!      if not found, remove stale config and re-provision.
//!   B. `.hq/config.json` absent but YAML has `cloudCompanyUid` → migration:
//!      look up entity, write config.json using the legacy UID, do NOT touch YAML.
//!   C. Otherwise → find_by_slug-first / create-second idempotency, then
//!      provision_bucket, then write config.json.
//!
//! `company.yaml` is NEVER written back — the file is read-only from this module.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::commands::vault_client::{CreateEntityInput, VaultClient};

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

        // ── Path C: unprovisioned — find_by_slug first, create only if None ───
        let uid = match vault.find_entity_by_slug("company", &folder_name).await {
            Ok(Some(info)) => info.uid,
            Ok(None) => {
                let entity_name = company_yaml
                    .name
                    .unwrap_or_else(|| folder_name.clone());
                let input = CreateEntityInput {
                    entity_type: "company".to_string(),
                    slug: folder_name.clone(),
                    name: entity_name,
                    email: None,
                    owner_uid: None,
                };
                vault
                    .create_entity(&input)
                    .await
                    .map_err(|e| format!("create entity '{}': {e}", folder_name))?
                    .uid
            }
            Err(e) => {
                return Err(format!("find_by_slug '{}': {e}", folder_name));
            }
        };

        let bucket_info = vault
            .provision_bucket(&uid)
            .await
            .map_err(|e| format!("provision_bucket '{}' uid={uid}: {e}", folder_name))?;

        let cfg = CompanyConfig {
            company_uid: uid.clone(),
            company_slug: folder_name.clone(),
            bucket_name: bucket_info.bucket_name.clone(),
            vault_api_url: vault_api_url.to_string(),
        };
        write_company_config(&hq_config_path, &cfg)?;

        result.push(ProvisionedCompany {
            slug: folder_name,
            uid,
            bucket_name: bucket_info.bucket_name,
        });
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
}
