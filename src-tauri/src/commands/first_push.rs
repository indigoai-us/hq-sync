//! First-push: upload every local file under a company folder to S3 after provisioning.
//!
//! `first_push_company` is the public entry point; it vends STS credentials, builds an S3
//! client, and delegates to the testable inner `run_first_push` function.

use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::future::Future;
use std::sync::Arc;

use base64::Engine as _;
use bytes::Bytes;
use chrono::Utc;
use sha2::{Digest, Sha256};
use walkdir::WalkDir;

use aws_credential_types::Credentials;
use aws_sdk_s3::config::{Builder as S3ConfigBuilder, Region};
use aws_sdk_s3::primitives::ByteStream;
use tauri::Emitter;

use crate::commands::provision::ProvisionedCompany;
use crate::commands::vault_client::{TaskScope, VaultClient, VendChildInput};
use crate::events::{
    SyncCompanyFirstPushCompleteEvent, SyncCompanyFirstPushProgressEvent,
    EVENT_SYNC_COMPANY_FIRST_PUSH_COMPLETE, EVENT_SYNC_COMPANY_FIRST_PUSH_PROGRESS,
};
use crate::util::ignore::IgnoreFilter;
use crate::util::journal::{read_journal, write_journal, Direction, JournalEntry};

// ── Types ─────────────────────────────────────────────────────────────────────

pub type BoxFuture<T> = Pin<Box<dyn Future<Output = T> + Send>>;

#[derive(Debug)]
pub enum UploadOutcome {
    Ok,
    Transient(String),
    Permanent(String),
}

// ── Retry helper ──────────────────────────────────────────────────────────────

async fn upload_with_retry<F>(
    key: &str,
    data: Bytes,
    sha256_hex: &str,
    uploader: &F,
) -> Result<(), String>
where
    F: Fn(String, Bytes, String) -> BoxFuture<UploadOutcome> + Send + Sync,
{
    const MAX_ATTEMPTS: usize = 3;
    const DELAY_MS: [u64; 2] = [1000, 3000];

    let mut last_err = String::new();
    for attempt in 0..MAX_ATTEMPTS {
        if attempt > 0 {
            #[cfg(not(test))]
            tokio::time::sleep(std::time::Duration::from_millis(DELAY_MS[attempt - 1])).await;
        }
        // Bytes::clone is a cheap ref-count increment — no buffer copy on retry.
        match uploader(key.to_string(), data.clone(), sha256_hex.to_string()).await {
            UploadOutcome::Ok => return Ok(()),
            UploadOutcome::Transient(e) => last_err = e,
            UploadOutcome::Permanent(e) => return Err(format!("permanent upload error: {e}")),
        }
    }
    Err(format!("upload '{key}' failed after {MAX_ATTEMPTS} attempts: {last_err}"))
}

// ── Core algorithm ────────────────────────────────────────────────────────────

/// Walk `{hq_root}/companies/{company_slug}/`, upload files that pass the ignore
/// filter and aren't already in the journal, return (files_uploaded, files_skipped).
///
/// `uploader` is injectable so tests can mock S3 without a real AWS connection.
pub(crate) async fn run_first_push<F, P, S>(
    hq_root: &Path,
    company_slug: &str,
    uploader: F,
    on_progress: P,
    on_skip: S,
) -> Result<(usize, usize), String>
where
    F: Fn(String, Bytes, String) -> BoxFuture<UploadOutcome> + Send + Sync,
    P: Fn(usize, usize, Option<String>),
    S: Fn(String, String),
{
    let filter = IgnoreFilter::for_hq_root(hq_root)?;
    let company_dir = hq_root.join("companies").join(company_slug);
    if !company_dir.exists() {
        return Ok((0, 0));
    }

    // Phase 1: collect eligible paths
    let mut file_paths: Vec<PathBuf> = Vec::new();
    for entry in WalkDir::new(&company_dir).into_iter().filter_map(|e| e.ok()) {
        if !entry.file_type().is_file() {
            continue;
        }
        let abs = entry.path().to_path_buf();
        if !filter.should_sync(&abs) {
            continue;
        }
        file_paths.push(abs);
    }

    let total = file_paths.len();
    let mut uploaded = 0usize;
    let mut skipped = 0usize;
    let mut journal = read_journal(company_slug)?;
    let now = Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);

    // Phase 2: upload or skip each file.
    // On any failure, break out of the loop so the journal is always flushed on the
    // way out — preserving entries for files already successfully uploaded this run
    // (load-bearing idempotency invariant, plan.md:1337).
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

        // Oversize guard
        if !IgnoreFilter::within_size_limit(&abs) {
            on_skip(rel_key.clone(), "exceeds 50MB limit".into());
            skipped += 1;
            continue;
        }

        // Read and hash
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

        // Journal idempotency check
        if let Some(entry) = journal.files.get(&rel_key) {
            if entry.hash == sha256_hex {
                skipped += 1;
                continue;
            }
        }

        // Upload with retry — break on error so the finally-block below can flush.
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
        // Flush after every successful upload so a mid-stream failure never
        // discards previously-uploaded entries (idempotency invariant).
        write_journal(company_slug, &journal)?;
        uploaded += 1;
    }

    // Terminal progress event — UI consumers see done == total.
    on_progress(total, total, None);

    // Always flush the final journal state on the way out, even on error, so
    // any partial progress is persisted for the next run.
    journal.last_sync = now;
    let _ = write_journal(company_slug, &journal);

    if let Some(e) = upload_err {
        return Err(e);
    }

    Ok((uploaded, skipped))
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
        "hq-sync-first-push",
    );
    // Hard-coded to us-east-1 because the vault Lambda always provisions buckets
    // in us-east-1 today. If multi-region tenants are added, ProvisionedCompany
    // will need a `region` field and this must be wired through. TODO: track in
    // the follow-up for regulatory-tenant work.
    let config = S3ConfigBuilder::new()
        .credentials_provider(creds)
        .region(Region::new("us-east-1"))
        .build();
    aws_sdk_s3::Client::from_conf(config)
}

// ── Public entry point ────────────────────────────────────────────────────────

pub async fn first_push_company(
    app: &tauri::AppHandle,
    vault: &VaultClient,
    hq_root: &Path,
    company: &ProvisionedCompany,
) -> Result<(), String> {
    // Vend task-scoped STS creds for this company
    let vend_result = vault
        .vend_child(&VendChildInput {
            company_uid: company.uid.clone(),
            task_id: ulid::Ulid::new().to_string(),
            task_description: "hq-sync first-push".to_string(),
            task_scope: TaskScope {
                allowed_prefixes: vec!["".to_string()],
                allowed_actions: Some(vec!["read".to_string(), "write".to_string()]),
            },
            duration_seconds: None,
        })
        .await
        .map_err(|e| format!("vend_child for {}: {e}", company.uid))?;

    let s3 = Arc::new(build_s3_client(
        &vend_result.credentials.access_key_id,
        &vend_result.credentials.secret_access_key,
        &vend_result.credentials.session_token,
    ));
    let bucket = company.bucket_name.clone();

    let app_ref = app.clone();
    let uid = company.uid.clone();
    let slug = company.slug.clone();
    let uid_p = uid.clone();
    let slug_p = slug.clone();

    let uploader = {
        let s3 = s3.clone();
        let bucket = bucket.clone();
        move |key: String, data: Bytes, sha256_hex: String| -> BoxFuture<UploadOutcome> {
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
        }
    };

    let (files_uploaded, files_skipped) = run_first_push(
        hq_root,
        &company.slug,
        uploader,
        move |done, total, file| {
            let _ = app_ref.emit(
                EVENT_SYNC_COMPANY_FIRST_PUSH_PROGRESS,
                SyncCompanyFirstPushProgressEvent {
                    company_uid: uid_p.clone(),
                    company_slug: slug_p.clone(),
                    files_done: done,
                    files_total: total,
                    current_file: file,
                },
            );
        },
        |_key, _reason| {
            // Oversize skips are counted in files_skipped; no separate event emitted
        },
    )
    .await?;

    let _ = app.emit(
        EVENT_SYNC_COMPANY_FIRST_PUSH_COMPLETE,
        SyncCompanyFirstPushCompleteEvent {
            company_uid: uid.clone(),
            company_slug: slug.clone(),
            files_uploaded,
            files_skipped,
        },
    );

    Ok(())
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::test_support::ENV_MUTEX;
    use std::sync::Mutex;
    use tempfile::TempDir;

    fn make_uploader(
        calls: Arc<Mutex<Vec<String>>>,
    ) -> impl Fn(String, Bytes, String) -> BoxFuture<UploadOutcome> + Send + Sync {
        move |key: String, _data: Bytes, _sha256: String| -> BoxFuture<UploadOutcome> {
            calls.lock().unwrap().push(key);
            Box::pin(async { UploadOutcome::Ok })
        }
    }

    fn company_dir(root: &Path, slug: &str) -> PathBuf {
        let dir = root.join("companies").join(slug);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_file(path: &Path, content: &[u8]) {
        if let Some(p) = path.parent() {
            std::fs::create_dir_all(p).unwrap();
        }
        std::fs::write(path, content).unwrap();
    }

    // (a) Files in ignored dirs (.git/, node_modules/) are NOT uploaded.
    #[tokio::test]
    async fn test_ignored_files_not_uploaded() {
        let tmp_state = TempDir::new().unwrap();
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        let co_dir = company_dir(root, "acme");

        write_file(&co_dir.join("README.md"), b"hello");
        write_file(&co_dir.join(".git/config"), b"git config");
        write_file(&co_dir.join("node_modules/pkg/index.js"), b"js");

        let calls = Arc::new(Mutex::new(vec![]));
        let uploader = make_uploader(calls.clone());

        {
            let _guard = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
            std::env::set_var("HQ_STATE_DIR", tmp_state.path());
            let (uploaded, _skipped) =
                run_first_push(root, "acme", uploader, |_, _, _| {}, |_, _| {})
                    .await
                    .unwrap();
            std::env::remove_var("HQ_STATE_DIR");

            assert_eq!(uploaded, 1, "only README.md should be uploaded");
            let captured = calls.lock().unwrap();
            assert_eq!(captured.len(), 1);
            assert!(
                captured[0].ends_with("README.md"),
                "expected README.md; got {:?}",
                captured[0]
            );
        }
    }

    #[tokio::test]
    async fn test_settings_excluded_target_company_dirs_uploaded() {
        let tmp_state = TempDir::new().unwrap();
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        let co_dir = company_dir(root, "_test");

        write_file(&co_dir.join("settings/secret.txt"), b"do not upload");
        write_file(&co_dir.join("policies/p.md"), b"policy");
        write_file(&co_dir.join("projects/x/prd.json"), b"{}");
        write_file(&co_dir.join("knowledge/k.md"), b"knowledge");
        write_file(&co_dir.join(".claude/commands/c.md"), b"command");

        let calls = Arc::new(Mutex::new(vec![]));
        let uploader = make_uploader(calls.clone());

        {
            let _guard = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
            std::env::set_var("HQ_STATE_DIR", tmp_state.path());
            let (uploaded, skipped) =
                run_first_push(root, "_test", uploader, |_, _, _| {}, |_, _| {})
                    .await
                    .unwrap();
            std::env::remove_var("HQ_STATE_DIR");

            assert_eq!(uploaded, 4, "only the four target files should upload");
            assert_eq!(
                skipped, 0,
                "ignored settings/ files are not counted as size skips"
            );

            let captured = calls.lock().unwrap();
            assert_eq!(captured.len(), 4);
            assert!(
                !captured.iter().any(|k| k.contains("/settings/")),
                "settings files must never reach the uploader: {:?}",
                captured
            );
            for expected in [
                "companies/_test/policies/p.md",
                "companies/_test/projects/x/prd.json",
                "companies/_test/knowledge/k.md",
                "companies/_test/.claude/commands/c.md",
            ] {
                assert!(
                    captured.iter().any(|k| k == expected),
                    "expected upload key {expected}; got {:?}",
                    captured
                );
            }
        }
    }

    // (b) Oversize file triggers on_skip callback; is NOT uploaded.
    #[tokio::test]
    async fn test_oversize_file_skipped() {
        let tmp_state = TempDir::new().unwrap();
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        let co_dir = company_dir(root, "bigco");

        // Create a sparse file > 50 MB (no actual disk space used)
        let big_path = co_dir.join("huge.bin");
        {
            let f = std::fs::File::create(&big_path).unwrap();
            f.set_len(50 * 1024 * 1024 + 1).unwrap();
        }
        write_file(&co_dir.join("small.txt"), b"tiny");

        let calls = Arc::new(Mutex::new(vec![]));
        let uploader = make_uploader(calls.clone());
        let skipped_keys = Arc::new(Mutex::new(vec![]));
        let sk = skipped_keys.clone();

        {
            let _guard = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
            std::env::set_var("HQ_STATE_DIR", tmp_state.path());
            let (_uploaded, skipped) = run_first_push(
                root,
                "bigco",
                uploader,
                |_, _, _| {},
                move |key, _reason| {
                    sk.lock().unwrap().push(key);
                },
            )
            .await
            .unwrap();
            std::env::remove_var("HQ_STATE_DIR");

            assert_eq!(skipped, 1, "huge.bin should be counted as skipped");
            let sk_list = skipped_keys.lock().unwrap();
            assert_eq!(sk_list.len(), 1);
            assert!(sk_list[0].ends_with("huge.bin"), "skip key: {:?}", sk_list[0]);
            let captured = calls.lock().unwrap();
            assert_eq!(captured.len(), 1, "small.txt should have been uploaded");
        }
    }

    // (c) Successful upload writes a journal entry with correct hash.
    //     Also asserts the terminal progress event fires with done == total.
    #[tokio::test]
    async fn test_upload_writes_journal_entry() {
        let tmp_state = TempDir::new().unwrap();
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        let co_dir = company_dir(root, "newco");
        let content = b"Hello, world!";
        write_file(&co_dir.join("README.md"), content);

        let calls = Arc::new(Mutex::new(vec![]));
        let uploader = make_uploader(calls.clone());
        let progress_events = Arc::new(Mutex::new(vec![]));
        let pe = progress_events.clone();

        {
            let _guard = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
            std::env::set_var("HQ_STATE_DIR", tmp_state.path());
            let (uploaded, skipped) =
                run_first_push(root, "newco", uploader, move |done, total, file| {
                    pe.lock().unwrap().push((done, total, file));
                }, |_, _| {})
                    .await
                    .unwrap();

            assert_eq!(uploaded, 1);
            assert_eq!(skipped, 0);

            let journal = crate::util::journal::read_journal("newco").unwrap();
            std::env::remove_var("HQ_STATE_DIR");

            let key = "companies/newco/README.md";
            assert!(journal.files.contains_key(key), "journal missing key {key}");
            let entry = &journal.files[key];
            let expected_hash = format!("{:x}", Sha256::digest(content));
            assert_eq!(entry.hash, expected_hash);
            assert_eq!(entry.size, content.len() as u64);

            // Terminal progress event: last event must have done == total.
            let events = progress_events.lock().unwrap();
            let last = events.last().expect("at least one progress event");
            assert_eq!(last.0, last.1, "terminal progress event must have done == total");
        }
    }

    // (d) Retry-on-transient: uploader returns Transient for first 2 calls, Ok on 3rd.
    #[tokio::test]
    async fn test_retry_on_transient_error() {
        let tmp_state = TempDir::new().unwrap();
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        let co_dir = company_dir(root, "retryco");
        write_file(&co_dir.join("doc.md"), b"retry me");

        let attempt_count = Arc::new(Mutex::new(0u32));
        let ac = attempt_count.clone();
        let uploader =
            move |_key: String, _data: Bytes, _sha256: String| -> BoxFuture<UploadOutcome> {
                let ac = ac.clone();
                Box::pin(async move {
                    let mut count = ac.lock().unwrap();
                    *count += 1;
                    if *count < 3 {
                        UploadOutcome::Transient(format!("503 attempt {}", *count))
                    } else {
                        UploadOutcome::Ok
                    }
                })
            };

        {
            let _guard = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
            std::env::set_var("HQ_STATE_DIR", tmp_state.path());
            let (uploaded, _) =
                run_first_push(root, "retryco", uploader, |_, _, _| {}, |_, _| {})
                    .await
                    .unwrap();
            std::env::remove_var("HQ_STATE_DIR");

            assert_eq!(uploaded, 1, "file must be uploaded after retries");
            assert_eq!(
                *attempt_count.lock().unwrap(),
                3,
                "must have attempted 3 times"
            );
        }
    }

    // (e) Re-run with journal already populated → zero PutObject calls (idempotency).
    #[tokio::test]
    async fn test_rerun_skips_all_if_journal_matches() {
        let tmp_state = TempDir::new().unwrap();
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        let co_dir = company_dir(root, "idempco");
        let content = b"stable content";
        write_file(&co_dir.join("file.md"), content);

        {
            let _guard = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
            std::env::set_var("HQ_STATE_DIR", tmp_state.path());

            // First run populates the journal
            let calls1 = Arc::new(Mutex::new(vec![]));
            run_first_push(
                root,
                "idempco",
                make_uploader(calls1.clone()),
                |_, _, _| {},
                |_, _| {},
            )
            .await
            .unwrap();
            assert_eq!(calls1.lock().unwrap().len(), 1);

            // Second run: same hash → zero uploads
            let calls2 = Arc::new(Mutex::new(vec![]));
            let (uploaded2, _) = run_first_push(
                root,
                "idempco",
                make_uploader(calls2.clone()),
                |_, _, _| {},
                |_, _| {},
            )
            .await
            .unwrap();

            std::env::remove_var("HQ_STATE_DIR");

            assert_eq!(uploaded2, 0, "second run must upload nothing");
            assert!(
                calls2.lock().unwrap().is_empty(),
                "no PutObject calls expected on re-run"
            );
        }
    }

    // (f) Partial failure: uploader returns Ok for the first file it sees,
    //     Permanent for the second. The journal must contain the first file's
    //     entry, and a re-run must NOT re-upload it (idempotency after
    //     mid-stream failure).
    #[tokio::test]
    async fn test_partial_failure_persists_prior_entries() {
        let tmp_state = TempDir::new().unwrap();
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        let co_dir = company_dir(root, "partco");

        write_file(&co_dir.join("file1.txt"), b"first file");
        write_file(&co_dir.join("file2.txt"), b"second file");

        let call_count = Arc::new(Mutex::new(0u32));
        let first_uploaded_key: Arc<Mutex<String>> = Arc::new(Mutex::new(String::new()));

        let cc = call_count.clone();
        let fuk = first_uploaded_key.clone();
        let uploader = move |key: String, _data: Bytes, _sha256: String| -> BoxFuture<UploadOutcome> {
            let cc = cc.clone();
            let fuk = fuk.clone();
            Box::pin(async move {
                let mut n = cc.lock().unwrap();
                *n += 1;
                if *n == 1 {
                    *fuk.lock().unwrap() = key;
                    UploadOutcome::Ok
                } else {
                    UploadOutcome::Permanent("403 IAM propagation".into())
                }
            })
        };

        {
            let _guard = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
            std::env::set_var("HQ_STATE_DIR", tmp_state.path());

            // First run: first-visited file uploads ok, second fails permanently.
            let result = run_first_push(root, "partco", uploader, |_, _, _| {}, |_, _| {}).await;
            assert!(result.is_err(), "expected Err on permanent upload failure");

            let first_key = first_uploaded_key.lock().unwrap().clone();
            assert!(!first_key.is_empty(), "uploader must have been called at least once");

            // Journal must contain the first-uploaded entry despite the error.
            let journal = crate::util::journal::read_journal("partco").unwrap();
            assert!(
                journal.files.contains_key(&first_key),
                "journal must contain first-uploaded entry after partial failure; got keys: {:?}",
                journal.files.keys().collect::<Vec<_>>()
            );

            // Re-run must skip the first-uploaded file (already in journal).
            let rerun_calls = Arc::new(Mutex::new(vec![]));
            let rc = rerun_calls.clone();
            let rerun_uploader = move |key: String, _data: Bytes, _sha256: String| -> BoxFuture<UploadOutcome> {
                let rc = rc.clone();
                Box::pin(async move {
                    rc.lock().unwrap().push(key);
                    UploadOutcome::Ok
                })
            };
            let _ = run_first_push(root, "partco", rerun_uploader, |_, _, _| {}, |_, _| {}).await;

            std::env::remove_var("HQ_STATE_DIR");

            // Re-run must NOT have re-uploaded the first file.
            let rerun_keys = rerun_calls.lock().unwrap();
            assert!(
                !rerun_keys.iter().any(|k| *k == first_key),
                "re-run must not re-upload already-journaled file {first_key}; got: {:?}",
                rerun_keys
            );
        }
    }
}
