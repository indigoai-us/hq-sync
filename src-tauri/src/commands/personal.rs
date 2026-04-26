//! Personal first-push: provision the caller's person entity bucket (once) and
//! upload personal HQ files (excluding the `companies/` tree) via /sts/vend-self.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::future::Future;
use std::sync::Arc;

use base64::Engine as _;
use bytes::Bytes;
use chrono::Utc;
use sha2::{Digest, Sha256};
use serde::{Deserialize, Serialize};
use tauri::Emitter;
use walkdir::WalkDir;

use aws_credential_types::Credentials;
use aws_sdk_s3::config::{Builder as S3ConfigBuilder, Region};
use aws_sdk_s3::primitives::ByteStream;

use crate::commands::vault_client::{VaultClient, VaultClientError, VendSelfInput};
use crate::events::{
    SyncPersonalFirstPushCompleteEvent, SyncPersonalFirstPushProgressEvent,
    SyncPersonalFirstPushSkippedEvent, SyncPersonalProvisionedEvent,
    SyncPersonalSkippedOwnershipMismatchEvent,
    EVENT_SYNC_PERSONAL_FIRST_PUSH_COMPLETE, EVENT_SYNC_PERSONAL_FIRST_PUSH_PROGRESS,
    EVENT_SYNC_PERSONAL_FIRST_PUSH_SKIPPED, EVENT_SYNC_PERSONAL_PROVISIONED,
    EVENT_SYNC_PERSONAL_SKIPPED_OWNERSHIP_MISMATCH,
};
use crate::util::ignore::IgnoreFilter;
use crate::util::journal::{read_journal, write_journal, Direction, JournalEntry};

// ── Types ─────────────────────────────────────────────────────────────────────

pub(crate) type BoxFuture<T> = Pin<Box<dyn Future<Output = T> + Send>>;

/// Dynamic-dispatch uploader used by both production (real S3) and tests (fake counter).
pub(crate) type UploaderFn = Arc<dyn Fn(String, Bytes, String) -> BoxFuture<UploadOutcome> + Send + Sync>;

#[derive(Debug)]
pub(crate) enum UploadOutcome {
    Ok,
    Transient(String),
    Permanent(String),
}

// ── Personal-vault path allowlist ─────────────────────────────────────────────

/// The only top-level directories under `hq_root/` that the personal vault
/// syncs. Everything else (root files, `modules/`, `packages/`, `repos/`,
/// `scripts/`, `settings/`, `social-content/`, etc.) is skipped to keep the
/// vault scoped to the user's actual knowledge work + tooling.
///
/// The conventional `companies/` tree is implicitly excluded because it isn't
/// in this list — companies sync via the runner's per-membership pass instead.
///
/// **TODO** (v0.1.26+): the spawned `hq-sync-runner` (Node, in
/// `@indigoai-us/hq-cloud`) does its own walk and currently uploads
/// everything under `hq_root` that isn't gitignored. Until the runner accepts
/// the same allowlist (via a new `--include-paths` CLI arg or a manifest
/// hint), the runner will still upload root files this list excludes. Mirror
/// this constant on the Node side when the runner gains the arg.
pub(crate) const PERSONAL_VAULT_PATHS: &[&str] = &[
    ".claude",   // skills + commands + settings
    "knowledge",
    "policies",
    "projects",
];

/// True when a relative path (relative to hq_root, forward-slash separators)
/// falls under one of the allowlisted personal-vault top-level dirs.
/// Empty / single-component paths (root files like `README.md`) return false.
/// Pre-walk every syncable target (personal allowlist + each company folder)
/// to count how many files we expect the runner to process. Fed into the UI
/// so the progress bar uses a real denominator instead of fake workspace
/// thirds. Best-effort: I/O errors on individual files are skipped silently
/// so a single broken inode doesn't tank the count.
pub(crate) fn count_files_to_sync(hq_root: &Path, company_slugs: &[String]) -> u64 {
    let filter = match crate::util::ignore::IgnoreFilter::for_hq_root(hq_root) {
        Ok(f) => f,
        Err(_) => return 0,
    };

    let mut total: u64 = 0;

    // Personal allowlist (.claude, knowledge, policies, projects at hq_root).
    for entry in WalkDir::new(hq_root).into_iter().filter_map(|e| e.ok()) {
        if !entry.file_type().is_file() {
            continue;
        }
        if !filter.should_sync(entry.path()) {
            continue;
        }
        let rel = match entry.path().strip_prefix(hq_root) {
            Ok(r) => r.to_string_lossy().replace('\\', "/"),
            Err(_) => continue,
        };
        if !is_personal_vault_path(&rel) {
            continue;
        }
        total += 1;
    }

    // Each company folder. The same .hqignore filter applies — companies
    // outside the slugs vec are skipped entirely (no walk).
    for slug in company_slugs {
        let dir = hq_root.join("companies").join(slug);
        if !dir.is_dir() {
            continue;
        }
        for entry in WalkDir::new(&dir).into_iter().filter_map(|e| e.ok()) {
            if !entry.file_type().is_file() {
                continue;
            }
            if !filter.should_sync(entry.path()) {
                continue;
            }
            total += 1;
        }
    }

    total
}

pub(crate) fn is_personal_vault_path(rel: &str) -> bool {
    let top = rel.split('/').next().unwrap_or("");
    if top.is_empty() {
        return false;
    }
    PERSONAL_VAULT_PATHS.contains(&top)
}

// ── Cache ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PersonEntityCache {
    pub person_uid: String,
    pub bucket_name: String,
    pub created_at: String,
}

fn cache_path() -> Result<PathBuf, String> {
    let home = dirs::home_dir().ok_or("cannot resolve home directory")?;
    Ok(home.join(".hq").join("person-entity.json"))
}

fn read_cache() -> Option<PersonEntityCache> {
    let p = cache_path().ok()?;
    let s = std::fs::read_to_string(&p).ok()?;
    serde_json::from_str(&s).ok()
}

fn write_cache(cache: &PersonEntityCache) -> Result<(), String> {
    let p = cache_path()?;
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let tmp = p.with_extension("json.tmp");
    let body = serde_json::to_string_pretty(cache).map_err(|e| e.to_string())?;
    let mut f = std::fs::File::create(&tmp).map_err(|e| e.to_string())?;
    f.write_all(body.as_bytes()).map_err(|e| e.to_string())?;
    f.sync_all().ok();
    std::fs::rename(&tmp, &p).map_err(|e| e.to_string())
}

pub(crate) fn delete_cache() {
    if let Ok(p) = cache_path() {
        let _ = std::fs::remove_file(p);
    }
}

// ── S3 helpers ────────────────────────────────────────────────────────────────

fn hex_to_bytes(hex: &str) -> Vec<u8> {
    (0..hex.len())
        .step_by(2)
        .filter(|&i| i + 2 <= hex.len())
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16)
            .expect("Sha256::digest() always emits valid hex"))
        .collect()
}

fn build_s3_client(
    access_key_id: &str,
    secret_access_key: &str,
    session_token: &str,
) -> aws_sdk_s3::Client {
    let creds = Credentials::new(
        access_key_id,
        secret_access_key,
        Some(session_token.to_string()),
        None,
        "hq-sync-personal-first-push",
    );
    // Hard-coded to us-east-1: vault Lambda always provisions buckets in us-east-1.
    let config = S3ConfigBuilder::new()
        .credentials_provider(creds)
        .region(Region::new("us-east-1"))
        .build();
    aws_sdk_s3::Client::from_conf(config)
}

// ── Upload retry ──────────────────────────────────────────────────────────────

async fn upload_with_retry(
    key: &str,
    data: Bytes,
    sha256_hex: &str,
    uploader: &UploaderFn,
) -> Result<(), String> {
    const MAX_ATTEMPTS: usize = 3;
    const DELAY_MS: [u64; 2] = [1000, 3000];

    let mut last_err = String::new();
    for attempt in 0..MAX_ATTEMPTS {
        if attempt > 0 {
            #[cfg(not(test))]
            tokio::time::sleep(std::time::Duration::from_millis(DELAY_MS[attempt - 1])).await;
        }
        match uploader(key.to_string(), data.clone(), sha256_hex.to_string()).await {
            UploadOutcome::Ok => return Ok(()),
            UploadOutcome::Transient(e) => last_err = e,
            UploadOutcome::Permanent(e) => return Err(format!("permanent upload error: {e}")),
        }
    }
    Err(format!("upload '{key}' failed after {MAX_ATTEMPTS} attempts: {last_err}"))
}

// ── Core upload algorithm ─────────────────────────────────────────────────────

/// Walk `hq_root/`, applying the ignore filter and excluding `companies/` prefix.
/// Slug is always `"personal"` → journal at state_dir/sync-journal.personal.json.
pub(crate) async fn run_personal_first_push<P, S>(
    hq_root: &Path,
    uploader: UploaderFn,
    on_progress: P,
    on_skip: S,
) -> Result<(usize, usize), String>
where
    P: Fn(usize, usize, Option<String>),
    S: Fn(String, String),
{
    let filter = IgnoreFilter::for_hq_root(hq_root)?;

    let mut file_paths: Vec<PathBuf> = Vec::new();
    for entry in WalkDir::new(hq_root).into_iter().filter_map(|e| e.ok()) {
        if !entry.file_type().is_file() {
            continue;
        }
        let abs = entry.path().to_path_buf();
        if !filter.should_sync(&abs) {
            continue;
        }
        let rel = match abs.strip_prefix(hq_root) {
            Ok(r) => r.to_string_lossy().replace('\\', "/"),
            Err(_) => continue,
        };
        if !is_personal_vault_path(&rel) {
            continue;
        }
        file_paths.push(abs);
    }

    let total = file_paths.len();
    let mut uploaded = 0usize;
    let mut skipped = 0usize;
    let mut journal = read_journal("personal")?;
    let now = Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);

    let mut upload_err: Option<String> = None;
    'upload: for (i, abs) in file_paths.into_iter().enumerate() {
        let rel_key = match abs.strip_prefix(hq_root) {
            Ok(p) => p.to_string_lossy().replace('\\', "/"),
            Err(e) => {
                upload_err = Some(format!("path strip error: {e}"));
                break 'upload;
            }
        };

        on_progress(i, total, Some(rel_key.clone()));

        if !IgnoreFilter::within_size_limit(&abs) {
            on_skip(rel_key.clone(), "exceeds 50MB limit".into());
            skipped += 1;
            continue;
        }

        let contents = match std::fs::read(&abs) {
            Ok(c) => Bytes::from(c),
            Err(e) => {
                upload_err = Some(format!("{}: {e}", abs.display()));
                break 'upload;
            }
        };
        let size = contents.len() as u64;
        let digest = Sha256::digest(&contents);
        let sha256_hex = format!("{:x}", digest);

        if let Some(entry) = journal.files.get(&rel_key) {
            if entry.hash == sha256_hex {
                skipped += 1;
                continue;
            }
        }

        match upload_with_retry(&rel_key, contents, &sha256_hex, &uploader).await {
            Ok(()) => {}
            Err(e) => {
                upload_err = Some(e);
                break 'upload;
            }
        }

        journal.files.insert(
            rel_key.clone(),
            JournalEntry {
                hash: sha256_hex,
                size,
                synced_at: now.clone(),
                direction: Direction::Up,
            },
        );
        write_journal("personal", &journal)?;
        uploaded += 1;
    }

    on_progress(total, total, None);

    journal.last_sync = now;
    let _ = write_journal("personal", &journal);

    if let Some(e) = upload_err {
        return Err(e);
    }

    Ok((uploaded, skipped))
}

// ── Cache validation ──────────────────────────────────────────────────────────

/// Returns Ok(true) if cache UID is still present, Ok(false) if confirmed gone,
/// Err if a transient error prevented the check (caller should keep the cache).
async fn validate_cache_via_list(vault: &VaultClient, cache: &PersonEntityCache) -> Result<bool, VaultClientError> {
    let entities = vault.list_entities_by_type("person").await?;
    Ok(entities.iter().any(|e| e.uid == cache.person_uid))
}

// ── Person resolution: cache → list+provision (no recursion) ─────────────────

/// Resolves (person_uid, bucket_name) using the local cache when valid, falling
/// back to a vault list + canonical sort + provision call if needed.
///
/// Cache validation uses `validate_cache_via_list` exclusively — the by-slug
/// route expects a Cognito sub / human identifier, not a UID like `prs_01HX...`.
/// On transient vault errors the cached data is used optimistically.
async fn resolve_or_provision<R: tauri::Runtime + 'static>(
    app: &tauri::AppHandle<R>,
    vault: &VaultClient,
) -> Result<(String, String), String> {
    if let Some(cache) = read_cache() {
        match validate_cache_via_list(vault, &cache).await {
            Ok(true) => return Ok((cache.person_uid, cache.bucket_name)),
            Ok(false) => {
                // Entity confirmed absent from vault — invalidate cache
                delete_cache();
            }
            Err(_) => {
                // Transient error (5xx, network) — proceed optimistically with cached data
                return Ok((cache.person_uid, cache.bucket_name));
            }
        }
    }

    // Cache miss or just invalidated: list all person entities and apply canonical sort
    let entities = vault
        .list_entities_by_type("person")
        .await
        .map_err(|e| format!("list person entities: {e}"))?;

    let mut sorted = entities;
    sorted.sort_by(|a, b| {
        let ac = a.created_at.as_str();
        let bc = b.created_at.as_str();
        match ac.cmp(bc) {
            std::cmp::Ordering::Equal => a.uid.cmp(&b.uid),
            ord => ord,
        }
    });
    let mut pick = sorted.into_iter().next().ok_or("no person entity for caller")?;

    if pick.bucket_name.is_none() {
        let bucket_info = vault
            .provision_bucket(&pick.uid)
            .await
            .map_err(|e| format!("provision_bucket for {}: {e}", pick.uid))?;
        pick.bucket_name = Some(bucket_info.bucket_name.clone());
        let _ = app.emit(
            EVENT_SYNC_PERSONAL_PROVISIONED,
            SyncPersonalProvisionedEvent {
                person_uid: pick.uid.clone(),
                bucket_name: bucket_info.bucket_name,
            },
        );
    }

    let resolved_bucket = pick.bucket_name.unwrap_or_default();
    let cache = PersonEntityCache {
        person_uid: pick.uid.clone(),
        bucket_name: resolved_bucket.clone(),
        created_at: pick.created_at.clone(),
    };
    let _ = write_cache(&cache);

    Ok((pick.uid, resolved_bucket))
}

// ── Public entry point ────────────────────────────────────────────────────────

pub async fn ensure_personal_bucket_and_first_push<R: tauri::Runtime + 'static>(
    app: &tauri::AppHandle<R>,
    vault: &VaultClient,
    hq_root: &Path,
) -> Result<(), String> {
    ensure_impl(app, vault, hq_root, None).await
}

/// Internal version that accepts an optional uploader override for tests.
/// When `uploader_override` is `None`, the real S3 client is used.
pub(crate) async fn ensure_impl<R: tauri::Runtime + 'static>(
    app: &tauri::AppHandle<R>,
    vault: &VaultClient,
    hq_root: &Path,
    uploader_override: Option<UploaderFn>,
) -> Result<(), String> {
    let (person_uid, bucket_name) = resolve_or_provision(app, vault).await?;

    // Obtain STS credentials via /sts/vend-self (never vend-child)
    let vend_result = match vault
        .vend_self(&VendSelfInput {
            person_uid: person_uid.clone(),
            duration_seconds: None,
        })
        .await
    {
        Ok(r) => r,
        Err(VaultClientError::SelfOwnershipMismatch) => {
            let _ = app.emit(
                EVENT_SYNC_PERSONAL_SKIPPED_OWNERSHIP_MISMATCH,
                SyncPersonalSkippedOwnershipMismatchEvent {
                    person_uid: person_uid.clone(),
                },
            );
            return Err("personal first-push aborted: SELF_OWNERSHIP_MISMATCH".to_string());
        }
        Err(e) => return Err(format!("vend_self for {person_uid}: {e}")),
    };

    let uploader: UploaderFn = match uploader_override {
        Some(f) => f,
        None => {
            let s3 = Arc::new(build_s3_client(
                &vend_result.credentials.access_key_id,
                &vend_result.credentials.secret_access_key,
                &vend_result.credentials.session_token,
            ));
            let bucket = bucket_name.clone();
            Arc::new(move |key: String, data: Bytes, sha256_hex: String| -> BoxFuture<UploadOutcome> {
                let s3 = s3.clone();
                let bucket = bucket.clone();
                Box::pin(async move {
                    let sha256_b64 = base64::engine::general_purpose::STANDARD
                        .encode(hex_to_bytes(&sha256_hex));
                    match s3
                        .put_object()
                        .bucket(&bucket)
                        .key(&key)
                        .body(ByteStream::from(data))
                        .checksum_sha256(sha256_b64)
                        .send()
                        .await
                    {
                        Ok(_) => UploadOutcome::Ok,
                        Err(e) => {
                            let status = e
                                .raw_response()
                                .map(|r| r.status().as_u16())
                                .unwrap_or(0);
                            if status == 0 || status >= 500 {
                                UploadOutcome::Transient(e.to_string())
                            } else {
                                UploadOutcome::Permanent(e.to_string())
                            }
                        }
                    }
                })
            })
        }
    };

    let app_progress = app.clone();
    let person_uid_progress = person_uid.clone();
    let app_skip = app.clone();
    let person_uid_skip = person_uid.clone();
    let puid_complete = person_uid.clone();

    let (files_uploaded, files_skipped) = run_personal_first_push(
        hq_root,
        uploader,
        move |done, total, file| {
            let _ = app_progress.emit(
                EVENT_SYNC_PERSONAL_FIRST_PUSH_PROGRESS,
                SyncPersonalFirstPushProgressEvent {
                    person_uid: person_uid_progress.clone(),
                    files_done: done,
                    files_total: total,
                    current_file: file,
                },
            );
        },
        move |key, reason| {
            let _ = app_skip.emit(
                EVENT_SYNC_PERSONAL_FIRST_PUSH_SKIPPED,
                SyncPersonalFirstPushSkippedEvent {
                    person_uid: person_uid_skip.clone(),
                    path: key,
                    reason,
                },
            );
        },
    )
    .await?;

    let _ = app.emit(
        EVENT_SYNC_PERSONAL_FIRST_PUSH_COMPLETE,
        SyncPersonalFirstPushCompleteEvent {
            person_uid: puid_complete,
            files_uploaded,
            files_skipped,
        },
    );

    Ok(())
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::EVENT_SYNC_PERSONAL_SKIPPED_OWNERSHIP_MISMATCH;
    use crate::util::test_support::ENV_MUTEX;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Mutex,
    };
    use tauri::Listener;
    use tempfile::TempDir;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn make_uploader(calls: Arc<Mutex<Vec<String>>>) -> UploaderFn {
        Arc::new(move |key: String, _data: Bytes, _sha256: String| -> BoxFuture<UploadOutcome> {
            calls.lock().unwrap().push(key);
            Box::pin(async { UploadOutcome::Ok })
        })
    }

    fn make_counter_uploader(counter: Arc<AtomicUsize>) -> UploaderFn {
        Arc::new(move |_key: String, _data: Bytes, _sha256: String| -> BoxFuture<UploadOutcome> {
            counter.fetch_add(1, Ordering::SeqCst);
            Box::pin(async { UploadOutcome::Ok })
        })
    }

    fn write_file(path: &Path, content: &[u8]) {
        if let Some(p) = path.parent() {
            std::fs::create_dir_all(p).unwrap();
        }
        std::fs::write(path, content).unwrap();
    }

    /// Realistic fixture: slug is a Cognito sub / email, NOT the same as uid.
    fn person_entity_json(uid: &str, slug: &str, bucket: Option<&str>, created_at: &str) -> serde_json::Value {
        let mut v = serde_json::json!({
            "uid": uid,
            "slug": slug,
            "type": "person",
            "status": "active",
            "createdAt": created_at,
        });
        if let Some(b) = bucket {
            v["bucketName"] = serde_json::Value::String(b.to_string());
        }
        v
    }

    fn vend_self_ok() -> serde_json::Value {
        serde_json::json!({
            "credentials": {
                "accessKeyId": "ASIA",
                "secretAccessKey": "secret",
                "sessionToken": "tok"
            },
            "expiresAt": "2026-01-01T01:00:00Z"
        })
    }

    // (a) No bucket → ensure_personal_bucket_and_first_push provisions exactly once.
    #[tokio::test]
    async fn test_no_bucket_triggers_provision() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/entity/by-type/person"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "entities": [person_entity_json("prs_x", "user@example.com", None, "2026-01-01T00:00:00Z")]
            })))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/provision/bucket"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "bucketName": "hq-vault-prs-x",
                "kmsKeyId": "key-1"
            })))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/sts/vend-self"))
            .respond_with(ResponseTemplate::new(200).set_body_json(vend_self_ok()))
            .mount(&server)
            .await;

        let tmp_state = TempDir::new().unwrap();
        let tmp_hq = TempDir::new().unwrap();
        let tmp_home = TempDir::new().unwrap();
        let upload_counter = Arc::new(AtomicUsize::new(0));

        let result = {
            let _guard = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
            std::env::set_var("HQ_STATE_DIR", tmp_state.path());
            std::env::set_var("HOME", tmp_home.path());

            let app = tauri::test::mock_app();
            let handle = app.handle().clone();
            let vault = VaultClient::new(&server.uri(), "tok");
            let r = ensure_impl(&handle, &vault, tmp_hq.path(), Some(make_counter_uploader(upload_counter.clone()))).await;

            std::env::remove_var("HQ_STATE_DIR");
            std::env::remove_var("HOME");
            r
        };

        assert!(result.is_ok(), "expected Ok, got: {:?}", result);

        let reqs = server.received_requests().await.unwrap();
        let prov: Vec<_> = reqs.iter().filter(|r| r.url.path() == "/provision/bucket").collect();
        assert_eq!(prov.len(), 1, "provision must be called exactly once when no bucket; got {} calls", prov.len());
        assert_eq!(upload_counter.load(Ordering::SeqCst), 0, "no uploads from empty hq_root");
    }

    // (b) Bucket already present → provision is NOT called.
    #[tokio::test]
    async fn test_with_bucket_skips_provision() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/entity/by-type/person"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "entities": [person_entity_json("prs_x", "user@example.com", Some("hq-vault-prs-x"), "2026-01-01T00:00:00Z")]
            })))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/sts/vend-self"))
            .respond_with(ResponseTemplate::new(200).set_body_json(vend_self_ok()))
            .mount(&server)
            .await;

        let tmp_state = TempDir::new().unwrap();
        let tmp_hq = TempDir::new().unwrap();
        let tmp_home = TempDir::new().unwrap();
        let upload_counter = Arc::new(AtomicUsize::new(0));

        let result = {
            let _guard = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
            std::env::set_var("HQ_STATE_DIR", tmp_state.path());
            std::env::set_var("HOME", tmp_home.path());

            let app = tauri::test::mock_app();
            let handle = app.handle().clone();
            let vault = VaultClient::new(&server.uri(), "tok");
            let r = ensure_impl(&handle, &vault, tmp_hq.path(), Some(make_counter_uploader(upload_counter.clone()))).await;

            std::env::remove_var("HQ_STATE_DIR");
            std::env::remove_var("HOME");
            r
        };

        assert!(result.is_ok(), "expected Ok, got: {:?}", result);

        let reqs = server.received_requests().await.unwrap();
        let prov: Vec<_> = reqs.iter().filter(|r| r.url.path() == "/provision/bucket").collect();
        assert_eq!(prov.len(), 0, "provision must NOT be called when bucket_name is already set");
        assert_eq!(upload_counter.load(Ordering::SeqCst), 0, "no uploads from empty hq_root");
    }

    // (c) Personal vault scope is restricted to PERSONAL_VAULT_PATHS.
    //     docs/, modules/, packages/, root files, companies/ → all excluded.
    //     .claude/, knowledge/, policies/, projects/ → all included.
    #[tokio::test]
    async fn test_personal_vault_path_allowlist() {
        let tmp_state = TempDir::new().unwrap();
        let tmp_hq = TempDir::new().unwrap();
        let root = tmp_hq.path();

        // Allowlisted (must be uploaded)
        write_file(&root.join("knowledge/notes.md"), b"knowledge");
        write_file(&root.join("policies/auto-deploy.md"), b"policy");
        write_file(&root.join("projects/foo/prd.json"), b"prd");
        write_file(&root.join(".claude/skills/foo/SKILL.md"), b"skill");
        // NOT allowlisted (must be skipped)
        write_file(&root.join("README.md"), b"root readme");
        write_file(&root.join("docs/README.md"), b"docs");
        write_file(&root.join("modules/modules.yaml"), b"modules");
        write_file(&root.join("packages/foo/README.md"), b"packages");
        write_file(&root.join("companies/acme/file.md"), b"company");

        let calls = Arc::new(Mutex::new(vec![]));
        {
            let _guard = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
            std::env::set_var("HQ_STATE_DIR", tmp_state.path());
            let _ = run_personal_first_push(root, make_uploader(calls.clone()), |_, _, _| {}, |_, _| {}).await;
            std::env::remove_var("HQ_STATE_DIR");
        }

        let captured = calls.lock().unwrap();

        // Allowlisted prefixes must appear.
        for prefix in [".claude/", "knowledge/", "policies/", "projects/"] {
            assert!(
                captured.iter().any(|k| k.starts_with(prefix)),
                "{prefix} must be uploaded; got: {captured:?}",
            );
        }
        // Non-allowlisted entries must NOT appear.
        for forbidden in ["README.md", "docs/", "modules/", "packages/", "companies/"] {
            assert!(
                !captured.iter().any(|k| k.starts_with(forbidden)),
                "{forbidden} must be skipped; got: {captured:?}",
            );
        }
    }

    // ── is_personal_vault_path (pure helper) ─────────────────────────────

    #[test]
    fn test_is_personal_vault_path_allowlist() {
        // Allowlisted
        assert!(is_personal_vault_path("knowledge/foo.md"));
        assert!(is_personal_vault_path("policies/auto-deploy.md"));
        assert!(is_personal_vault_path("projects/foo/prd.json"));
        assert!(is_personal_vault_path(".claude/skills/foo/SKILL.md"));
        assert!(is_personal_vault_path(".claude/commands/x.md"));
        // Not allowlisted
        assert!(!is_personal_vault_path("README.md"), "root files excluded");
        assert!(!is_personal_vault_path("companies/acme/x.md"), "companies excluded");
        assert!(!is_personal_vault_path("modules/modules.yaml"));
        assert!(!is_personal_vault_path("packages/foo/README.md"));
        assert!(!is_personal_vault_path("scripts/run.sh"));
        assert!(!is_personal_vault_path(""));
        // Edge: a top-level entry NAMED knowledge.md (file, not dir) gets the
        // first segment "knowledge.md" — not "knowledge" — so excluded.
        assert!(!is_personal_vault_path("knowledge.md"), "string match must be exact segment");
    }

    // (d) Re-run with journal populated → zero PutObject calls.
    //     Uses an allowlisted path (knowledge/) — pre-allowlist this test
    //     used a root-level notes.md which is now excluded by design.
    #[tokio::test]
    async fn test_rerun_no_op_via_journal() {
        let tmp_state = TempDir::new().unwrap();
        let tmp_hq = TempDir::new().unwrap();
        let root = tmp_hq.path();

        write_file(&root.join("knowledge/notes.md"), b"stable content");

        {
            let _guard = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
            std::env::set_var("HQ_STATE_DIR", tmp_state.path());

            let calls1 = Arc::new(Mutex::new(vec![]));
            run_personal_first_push(root, make_uploader(calls1.clone()), |_, _, _| {}, |_, _| {})
                .await.unwrap();
            assert_eq!(calls1.lock().unwrap().len(), 1);

            let calls2 = Arc::new(Mutex::new(vec![]));
            let (uploaded, _) = run_personal_first_push(
                root,
                make_uploader(calls2.clone()),
                |_, _, _| {},
                |_, _| {},
            ).await.unwrap();

            std::env::remove_var("HQ_STATE_DIR");

            assert_eq!(uploaded, 0, "second run must upload nothing");
            assert!(calls2.lock().unwrap().is_empty(), "no PutObject calls on re-run");
        }
    }

    // (e) Multi-person → canonical pick is oldest created_at, regardless of list order.
    // Runs twice (reversed list order on second run); both vend_self calls must use prs_x.
    #[tokio::test]
    async fn test_multi_person_canonical_pick() {
        let server = MockServer::start().await;

        // Run 1 fallback and Run 2 response: [prs_x (oldest), prs_y (newer)]
        Mock::given(method("GET"))
            .and(path("/entity/by-type/person"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "entities": [
                    person_entity_json("prs_x", "oldest@example.com", Some("hq-vault-prs-x"), "2026-01-01T00:00:00Z"),
                    person_entity_json("prs_y", "newer@example.com",  Some("hq-vault-prs-y"), "2026-02-01T00:00:00Z"),
                ]
            })))
            .mount(&server)
            .await;

        // Run 1 response (higher priority, expires after 1 use): [prs_y, prs_x] — prs_y listed first
        Mock::given(method("GET"))
            .and(path("/entity/by-type/person"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "entities": [
                    person_entity_json("prs_y", "newer@example.com",  Some("hq-vault-prs-y"), "2026-02-01T00:00:00Z"),
                    person_entity_json("prs_x", "oldest@example.com", Some("hq-vault-prs-x"), "2026-01-01T00:00:00Z"),
                ]
            })))
            .up_to_n_times(1)
            .mount(&server)
            .await;

        Mock::given(method("POST"))
            .and(path("/sts/vend-self"))
            .respond_with(ResponseTemplate::new(200).set_body_json(vend_self_ok()))
            .mount(&server)
            .await;

        let tmp_state = TempDir::new().unwrap();
        let tmp_hq = TempDir::new().unwrap();
        let tmp_home = TempDir::new().unwrap();

        {
            let _guard = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
            std::env::set_var("HQ_STATE_DIR", tmp_state.path());
            std::env::set_var("HOME", tmp_home.path());

            let app = tauri::test::mock_app();
            let handle = app.handle().clone();
            let vault = VaultClient::new(&server.uri(), "tok");

            // Run 1: list = [prs_y, prs_x] → canonical sort picks prs_x (oldest)
            ensure_impl(&handle, &vault, tmp_hq.path(), Some(make_counter_uploader(Arc::new(AtomicUsize::new(0))))).await.unwrap();
            // Delete cache so Run 2 re-lists (reversed order)
            delete_cache();
            // Run 2: list = [prs_x, prs_y] → canonical sort still picks prs_x
            ensure_impl(&handle, &vault, tmp_hq.path(), Some(make_counter_uploader(Arc::new(AtomicUsize::new(0))))).await.unwrap();

            std::env::remove_var("HQ_STATE_DIR");
            std::env::remove_var("HOME");
        }

        let reqs = server.received_requests().await.unwrap();
        let vend_self_reqs: Vec<_> = reqs.iter().filter(|r| r.url.path() == "/sts/vend-self").collect();
        assert_eq!(vend_self_reqs.len(), 2, "vend_self must be called twice (once per run)");

        for req in &vend_self_reqs {
            let body: serde_json::Value = serde_json::from_slice(&req.body).unwrap_or_default();
            assert_eq!(
                body["personUid"],
                serde_json::json!("prs_x"),
                "canonical pick must always be prs_x (oldest); vend_self body: {body}"
            );
        }
    }

    // (f) vend_self routing: zero hits on /sts/vend-child, ≥1 on /sts/vend-self.
    #[tokio::test]
    async fn test_vend_self_routing_zero_vend_child_hits() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/entity/by-type/person"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "entities": [person_entity_json("prs_x", "user@example.com", Some("hq-vault-prs-x"), "2026-01-01T00:00:00Z")]
            })))
            .mount(&server)
            .await;
        // vend-child mock records calls (should get zero)
        Mock::given(method("POST"))
            .and(path("/sts/vend-child"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/sts/vend-self"))
            .respond_with(ResponseTemplate::new(200).set_body_json(vend_self_ok()))
            .mount(&server)
            .await;

        let tmp_state = TempDir::new().unwrap();
        let tmp_hq = TempDir::new().unwrap();
        let tmp_home = TempDir::new().unwrap();

        {
            let _guard = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
            std::env::set_var("HQ_STATE_DIR", tmp_state.path());
            std::env::set_var("HOME", tmp_home.path());

            let app = tauri::test::mock_app();
            let handle = app.handle().clone();
            let vault = VaultClient::new(&server.uri(), "tok");
            ensure_impl(&handle, &vault, tmp_hq.path(), Some(make_counter_uploader(Arc::new(AtomicUsize::new(0))))).await.unwrap();

            std::env::remove_var("HQ_STATE_DIR");
            std::env::remove_var("HOME");
        }

        let reqs = server.received_requests().await.unwrap();
        let vend_child: Vec<_> = reqs.iter().filter(|r| r.url.path() == "/sts/vend-child").collect();
        let vend_self: Vec<_> = reqs.iter().filter(|r| r.url.path() == "/sts/vend-self").collect();

        assert_eq!(vend_child.len(), 0, "vend_child must NOT be called from personal flow");
        assert!(vend_self.len() >= 1, "vend_self must be called at least once");
    }

    // (g) SELF_OWNERSHIP_MISMATCH → returns Err, emits event, zero upload calls.
    #[tokio::test]
    async fn test_self_ownership_mismatch_surfaces_as_err() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/entity/by-type/person"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "entities": [person_entity_json("prs_x", "user@example.com", Some("hq-vault-prs-x"), "2026-01-01T00:00:00Z")]
            })))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/sts/vend-self"))
            .respond_with(ResponseTemplate::new(403).set_body_json(serde_json::json!({
                "error": "ownership mismatch",
                "code": "SELF_OWNERSHIP_MISMATCH"
            })))
            .mount(&server)
            .await;

        let tmp_state = TempDir::new().unwrap();
        let tmp_hq = TempDir::new().unwrap();
        let tmp_home = TempDir::new().unwrap();
        let upload_counter = Arc::new(AtomicUsize::new(0));

        let mismatch_events: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(vec![]));
        let mismatch_events_clone = mismatch_events.clone();

        let result = {
            let _guard = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
            std::env::set_var("HQ_STATE_DIR", tmp_state.path());
            std::env::set_var("HOME", tmp_home.path());

            let app = tauri::test::mock_app();
            let handle = app.handle().clone();
            // Register event listener BEFORE invoking the function
            app.listen(
                EVENT_SYNC_PERSONAL_SKIPPED_OWNERSHIP_MISMATCH,
                move |e| {
                    mismatch_events_clone.lock().unwrap().push(e.payload().to_string());
                },
            );
            let vault = VaultClient::new(&server.uri(), "tok");
            let r = ensure_impl(
                &handle,
                &vault,
                tmp_hq.path(),
                Some(make_counter_uploader(upload_counter.clone())),
            ).await;

            std::env::remove_var("HQ_STATE_DIR");
            std::env::remove_var("HOME");
            r
        };

        // 1. Function returns Err
        assert!(result.is_err(), "expected Err from SELF_OWNERSHIP_MISMATCH");
        let err_msg = result.unwrap_err();
        assert!(
            err_msg.contains("SELF_OWNERSHIP_MISMATCH"),
            "error must mention SELF_OWNERSHIP_MISMATCH; got: {err_msg}"
        );

        // 2. sync:personal-skipped-ownership-mismatch event was emitted
        let evs = mismatch_events.lock().unwrap();
        assert_eq!(
            evs.len(),
            1,
            "mismatch event must be emitted exactly once; got: {:?}",
            *evs
        );

        // 3. Zero uploader calls (function aborted before reaching run_personal_first_push)
        assert_eq!(
            upload_counter.load(Ordering::SeqCst),
            0,
            "no uploads must happen after ownership mismatch"
        );
    }

    // Additional: journal path for "personal" slug is correct.
    #[test]
    fn test_personal_journal_path() {
        use crate::util::journal::journal_path;
        let _guard = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = TempDir::new().unwrap();
        std::env::set_var("HQ_STATE_DIR", tmp.path());
        let p = journal_path("personal").unwrap();
        std::env::remove_var("HQ_STATE_DIR");
        assert!(
            p.to_string_lossy().ends_with("sync-journal.personal.json"),
            "got: {}",
            p.display()
        );
    }
}
