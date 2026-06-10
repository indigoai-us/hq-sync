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

use crate::commands::vault_client::{EntityInfo, VaultClient, VaultClientError, VendSelfInput};
use crate::util::logfile::log;
use crate::events::{
    SyncPersonalFirstPushCompleteEvent, SyncPersonalFirstPushProgressEvent, SyncPersonalFirstPushScanEvent,
    SyncPersonalFirstPushSkippedEvent, SyncPersonalProvisionedEvent,
    SyncPersonalSkippedOwnershipMismatchEvent,
    EVENT_SYNC_PERSONAL_FIRST_PUSH_COMPLETE, EVENT_SYNC_PERSONAL_FIRST_PUSH_PROGRESS,
    EVENT_SYNC_PERSONAL_FIRST_PUSH_SCAN,
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

// ── Personal-vault path exclusion list ────────────────────────────────────────

/// Top-level directories under `hq_root/` that the personal vault MUST NOT
/// sync. Everything else (root files, `.claude/`, `knowledge/`, `modules/`,
/// `core/`, hidden dotfile dirs like `.codex/`, etc.) is included subject
/// to the IgnoreFilter (`.gitignore` + `.hqignore`).
///
/// Rationale per exclusion:
///   - `companies/`: synced separately by the runner's per-membership fanout;
///     do not double-write into the personal vault.
///   - `workspace/`, `repos/`: per user directive — heavy local-only content
///     (cloned remotes, session threads) that should not live in the personal
///     vault.
///   - `.git/`: a git repo's internal state is large, opaque, and useless
///     after sync — gitignore alone doesn't cover `.git/` because it's the
///     repo itself, not a tracked path.
///
/// Note: `core/`, `data/`, and `personal/` were previously excluded but are
/// now INCLUDED (user directive 2026-05-13). `core/` ships the hq-core
/// scaffold (policies/, settings/, skills/, workers/, the rules manifest at
/// core/core.yaml) — real project content the box needs. `data/` and
/// `personal/` carry per-user data and policies/hooks/skills that the user
/// expects to follow them across machines. The hq-root identity marker
/// `core.yaml` (distinct from `core/core.yaml`) is filtered separately
/// downstream by the anchored `/core.yaml` DEFAULT_IGNORES rule in
/// `@indigoai-us/hq-cloud`.
///
/// Mirror this constant in `@indigoai-us/hq-cloud`'s sync-runner so push
/// behaviour from the Node runner matches the Rust first-push.
pub(crate) const PERSONAL_VAULT_EXCLUDED_TOP_LEVEL: &[&str] = &[
    ".git",
    "companies",
    "repos",
    "workspace",
];

/// True when a relative path (relative to hq_root, forward-slash separators)
/// is part of the personal vault — i.e. its top-level segment is NOT in
/// `PERSONAL_VAULT_EXCLUDED_TOP_LEVEL`. Empty paths return false (no top
/// segment to check).
/// "Preparing sync…" pre-pass: walk every push-side target, hash each file,
/// and compare against the journal to count exactly how many UPLOADS the
/// runner will emit. The runner only fires `progress` events for actual
/// transfers (skipped files are silent), so this count IS the bar's
/// denominator for the upload phase.
///
/// Pull-side downloads aren't counted here yet — that requires an S3 LIST
/// per bucket (vend STS + paginated list). For the common steady-state
/// case (everything matches the journal), pull-side downloads = 0, so this
/// count is exact. For first-time syncs (empty journal, empty bucket),
/// downloads also = 0 (nothing remote to pull). Mid-life syncs with
/// out-of-band changes may have a small under-count; the UI's honest-
/// fallback caption switches from "X of Y" to bare "X transferred" once
/// cumulative exceeds the estimate.
///
/// Cost: one full local walk + sha256 per file (matches what the runner
/// will do anyway). For 13K files that's ~1-3s of disk I/O. Steady-state
/// folder = mostly cached pages, much faster.
pub(crate) fn count_files_to_transfer(hq_root: &Path, company_slugs: &[String]) -> u64 {
    let filter = match crate::util::ignore::IgnoreFilter::for_hq_root(hq_root) {
        Ok(f) => f,
        Err(_) => return 0,
    };

    let mut to_upload: u64 = 0;

    // ── Personal allowlist (.claude, knowledge, policies, projects) ───────
    let personal_journal = crate::util::journal::read_journal("personal")
        .unwrap_or_default();
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
        if file_needs_upload(entry.path(), &rel, &personal_journal) {
            to_upload += 1;
        }
    }

    // ── Each company folder ───────────────────────────────────────────────
    for slug in company_slugs {
        let dir = hq_root.join("companies").join(slug);
        if !dir.is_dir() {
            continue;
        }
        let company_journal = crate::util::journal::read_journal(slug)
            .unwrap_or_default();
        // Remote keys are company-relative (e.g. "knowledge/foo.md"), not
        // hq-root-relative. The runner's share() strips companies/{slug}/
        // from the absolute path before journaling.
        for entry in WalkDir::new(&dir).into_iter().filter_map(|e| e.ok()) {
            if !entry.file_type().is_file() {
                continue;
            }
            if !filter.should_sync(entry.path()) {
                continue;
            }
            let rel_to_company = match entry.path().strip_prefix(&dir) {
                Ok(r) => r.to_string_lossy().replace('\\', "/"),
                Err(_) => continue,
            };
            if file_needs_upload(entry.path(), &rel_to_company, &company_journal) {
                to_upload += 1;
            }
        }
    }

    to_upload
}

/// True iff the file's current sha256 differs from its journal entry (or has
/// no journal entry). Mirrors `share.ts` skipUnchanged logic. Hashing errors
/// (missing file, permission denied) are treated as "needs upload" to err on
/// the side of including it — the runner will hit the same error and surface
/// it cleanly.
fn file_needs_upload(
    abs_path: &Path,
    journal_key: &str,
    journal: &crate::util::journal::SyncJournal,
) -> bool {
    let contents = match std::fs::read(abs_path) {
        Ok(c) => c,
        Err(_) => return true,
    };
    let hash = format!("{:x}", Sha256::digest(&contents));
    match journal.files.get(journal_key) {
        Some(entry) => entry.hash != hash,
        None => true,
    }
}

pub(crate) fn is_personal_vault_path(rel: &str) -> bool {
    // companies/manifest.yaml is the routing source-of-truth (which slugs
    // exist, which are cloud-backed) — included in the personal-vault
    // scope despite the parent `companies/` top-level exclusion. Mirrors
    // the TS `computePersonalVaultPaths` special-case shipped in
    // @indigoai-us/hq-cloud@5.39.0 so the Rust first-push and Node
    // steady-state push agree on whether manifest.yaml belongs in the
    // personal vault.
    if rel == "companies/manifest.yaml" {
        return true;
    }
    let top = rel.split('/').next().unwrap_or("");
    if top.is_empty() {
        return false;
    }
    !PERSONAL_VAULT_EXCLUDED_TOP_LEVEL.contains(&top)
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

/// Returns ASCII-only `(metadata key, value)` pairs to stamp on every
/// PutObject during personal first-push. Mirrors `buildAuthorMetadata` in
/// packages/hq-cloud/src/s3.ts and hq-console/src/lib/s3-vault.ts so the
/// Vault tab's CREATED BY column resolves uniformly across upload paths.
/// Returns an empty Vec when no Cognito tokens are cached or claims are
/// unparseable — uploads still succeed, the column just stays "—".
fn build_personal_author_metadata() -> Vec<(String, String)> {
    let mut meta = Vec::with_capacity(3);
    let claims = match crate::commands::cognito::read_tokens_from_file() {
        Ok(Some(tokens)) => tokens
            .id_token
            .as_deref()
            .and_then(|t| crate::commands::cognito::decode_id_token_claims(t).ok()),
        _ => None,
    };
    if let Some(c) = claims {
        if let Some(sub) = c.sub.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            if sub.bytes().all(|b| (0x20..=0x7E).contains(&b)) {
                meta.push(("created-by-sub".to_string(), sub.to_string()));
            }
        }
        if let Some(email) = c.email.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            if email.bytes().all(|b| (0x20..=0x7E).contains(&b)) {
                meta.push(("created-by".to_string(), email.to_string()));
            }
        }
    }
    let created_at = Utc::now().to_rfc3339();
    if created_at.bytes().all(|b| (0x20..=0x7E).contains(&b)) {
        meta.push(("created-at".to_string(), created_at));
    }
    meta
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
pub(crate) async fn run_personal_first_push<C, P, S>(
    hq_root: &Path,
    uploader: UploaderFn,
    on_scan: C,
    on_progress: P,
    on_skip: S,
) -> Result<(usize, usize), String>
where
    C: Fn(usize, usize, Option<String>),
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

    let walk_total = file_paths.len();
    let mut uploaded = 0usize;
    let mut skipped = 0usize;
    let mut journal = read_journal("personal")?;
    let now = Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);

    // Phase A — scan: hash every in-scope file against the journal to build
    // the upload plan. `on_scan` is a liveness signal carrying walk totals;
    // the popover's "x of N files" denominator comes from `on_progress`
    // below, which only ever carries the plan (changed-file) size — feeding
    // the walk total there made a 1-file delta read "x of 2,877 files".
    let mut upload_err: Option<String> = None;
    let mut plan: Vec<(PathBuf, String)> = Vec::new();
    'scan: for (i, abs) in file_paths.into_iter().enumerate() {
        let rel_key = match abs.strip_prefix(hq_root) {
            Ok(p) => p.to_string_lossy().replace('\\', "/"),
            Err(e) => {
                upload_err = Some(format!("path strip error: {e}"));
                break 'scan;
            }
        };

        on_scan(i, walk_total, Some(rel_key.clone()));

        if !IgnoreFilter::within_size_limit(&abs) {
            on_skip(rel_key.clone(), "exceeds 50MB limit".into());
            skipped += 1;
            continue;
        }

        let contents = match std::fs::read(&abs) {
            Ok(c) => c,
            Err(e) => {
                upload_err = Some(format!("{}: {e}", abs.display()));
                break 'scan;
            }
        };
        let digest = Sha256::digest(&contents);
        let sha256_hex = format!("{:x}", digest);

        if let Some(entry) = journal.files.get(&rel_key) {
            if entry.hash == sha256_hex {
                skipped += 1;
                continue;
            }
        }

        plan.push((abs, rel_key));
    }
    on_scan(walk_total, walk_total, None);

    // Phase B — upload exactly the plan. Files are re-read here rather than
    // carried from the scan: holding the whole changed set in memory would
    // pin the entire vault on a true first push. Re-hashing keeps the
    // journal entry honest if a file changed between phases.
    if upload_err.is_none() {
        let plan_total = plan.len();
        'upload: for (i, (abs, rel_key)) in plan.into_iter().enumerate() {
            on_progress(i, plan_total, Some(rel_key.clone()));

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
        on_progress(plan_total, plan_total, None);
    }

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

/// Provision the caller's person entity by reading Cognito idToken claims
/// (sub, name/email) and POST'ing to /entity. Used when `list_entities_by_type`
/// returns empty — i.e. a brand-new account that has never synced before.
/// Mirrors `vault-client.ts::ensureMyPersonEntity` so the auto-create path is
/// identical in shape to the runner's claim-dance path.
///
/// Return semantics:
///   * `Ok(Some(entity))` — created (or recovered an already-existing) person.
///   * `Ok(None)` — the person already exists server-side (HTTP 409) but could
///     not be resolved this cycle. This is BENIGN: the `hq-sync-runner` that
///     follows owns the personal vault, so the caller should skip personal
///     first-push quietly rather than surface a `sync:error`. Mirrors the TS
///     runner's claim-dance, which tolerates an already-provisioned person
///     ("claim-dance skipped — …") instead of treating it as a sync failure.
///   * `Err(..)` — a REAL failure (5xx, network, auth, malformed token). These
///     are NOT 409s and stay loud so genuine first-push breakage is reported.
pub(crate) async fn create_person_entity_from_cognito(
    vault: &VaultClient,
) -> Result<Option<EntityInfo>, String> {
    let tokens = crate::commands::cognito::read_tokens_from_file()?
        .ok_or_else(|| "no cached cognito tokens — sign in first".to_string())?;
    let id_token = tokens
        .id_token
        .as_deref()
        .ok_or_else(|| "cognito tokens missing id_token field".to_string())?;
    let claims = crate::commands::cognito::decode_id_token_claims(id_token)?;
    let owner_sub = claims
        .sub
        .clone()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "id_token has no `sub` claim".to_string())?;
    let display_name = claims.display_name();
    if display_name.is_empty() {
        return Err("id_token has no name/given_name/family_name/email — can't derive a display name".into());
    }
    // Slugify: lower, [^a-z0-9]→'-', trim leading/trailing '-', cap at 63 chars.
    // Fallback if slug is empty: "user-<last 8 of sub, lowercased>".
    let mut slug: String = display_name
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    while slug.contains("--") {
        slug = slug.replace("--", "-");
    }
    let slug = slug.trim_matches('-');
    let slug = if slug.is_empty() {
        let last8: String = owner_sub.chars().rev().take(8).collect::<String>()
            .chars().rev().collect::<String>().to_lowercase();
        format!("user-{last8}")
    } else {
        let mut s = slug.to_string();
        s.truncate(63);
        s
    };

    log("personal", &format!("auto-create person entity: slug={slug} name={display_name}"));
    match vault
        .create_entity(&crate::commands::vault_client::CreateEntityInput {
            entity_type: "person".into(),
            slug: slug.clone(),
            name: display_name,
            email: claims.email.clone(),
            owner_uid: Some(owner_sub),
        })
        .await
    {
        Ok(entity) => Ok(Some(entity)),
        // 409 = the person entity already exists. We only reach create at all
        // when `list_entities_by_type("person")` returned empty, so an
        // already-exists here means the list/create views disagreed this cycle
        // (e.g. eventual-consistency or scoping skew). Recover the existing row
        // by slug so first-push can still proceed; if it can't be resolved,
        // return None so the caller skips quietly. Either way this is benign and
        // must NOT surface as a user-facing "personal first-push failed" error.
        Err(VaultClientError::Http { status: 409, .. }) => {
            log(
                "personal",
                &format!("person entity already exists (409) — recovering by slug={slug}"),
            );
            match vault.find_entity_by_slug("person", &slug).await {
                Ok(Some(existing)) => Ok(Some(existing)),
                Ok(None) => {
                    log(
                        "personal",
                        &format!("person entity exists (409) but not resolvable by slug={slug} — skipping personal first-push (runner will handle)"),
                    );
                    Ok(None)
                }
                Err(e) => {
                    // Recovery lookup itself failed transiently. The person
                    // still exists (we got a 409), so this remains benign —
                    // skip quietly rather than report an error.
                    log(
                        "personal",
                        &format!("person-entity recovery lookup failed after 409: {e} — skipping personal first-push"),
                    );
                    Ok(None)
                }
            }
        }
        Err(e) => Err(format!("create person entity: {e}")),
    }
}

// ── Person resolution: cache → list+provision (no recursion) ─────────────────

/// Resolves (person_uid, bucket_name) using the local cache when valid, falling
/// back to a vault list + canonical sort + provision call if needed.
///
/// Cache validation uses `validate_cache_via_list` exclusively — the by-slug
/// route expects a Cognito sub / human identifier, not a UID like `prs_01HX...`.
/// On transient vault errors the cached data is used optimistically.
/// Returns `Ok(Some((person_uid, bucket_name)))` once resolved. Returns
/// `Ok(None)` when the person entity already exists but can't be resolved this
/// cycle (benign 409 — see `create_person_entity_from_cognito`); the caller
/// skips personal first-push quietly in that case.
async fn resolve_or_provision<R: tauri::Runtime + 'static>(
    app: &tauri::AppHandle<R>,
    vault: &VaultClient,
) -> Result<Option<(String, String)>, String> {
    if let Some(cache) = read_cache() {
        match validate_cache_via_list(vault, &cache).await {
            Ok(true) => return Ok(Some((cache.person_uid, cache.bucket_name))),
            Ok(false) => {
                // Entity confirmed absent from vault — invalidate cache
                delete_cache();
            }
            Err(_) => {
                // Transient error (5xx, network) — proceed optimistically with cached data
                return Ok(Some((cache.person_uid, cache.bucket_name)));
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
    let mut pick = match sorted.into_iter().next() {
        Some(p) => p,
        None => {
            // First sync for a brand-new account: no person entity exists yet.
            // Auto-create one from the cached Cognito idToken claims (sub for
            // owner, name/given+family/email for displayName). This replaces
            // the old bail ("no person entity for caller") that used to leave
            // the user stuck — they had to do the setup dance externally.
            // After creation the rest of provisioning continues as for any
            // existing entity (provision_bucket, cache, return).
            //
            // `Ok(None)` here means the person already exists server-side (409)
            // but couldn't be resolved — propagate the benign skip upward.
            match crate::commands::personal::create_person_entity_from_cognito(vault).await? {
                Some(entity) => entity,
                None => return Ok(None),
            }
        }
    };

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

    Ok(Some((pick.uid, resolved_bucket)))
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
    let (person_uid, bucket_name) = match resolve_or_provision(app, vault).await? {
        Some(p) => p,
        None => {
            // Benign: the person entity already exists but isn't resolvable this
            // cycle (HTTP 409). Skip personal first-push quietly — the
            // hq-sync-runner that follows owns the personal vault. Emit a
            // diagnostic skip event (no frontend error surface) and return Ok so
            // the user never sees a spurious "personal first-push failed".
            let _ = app.emit(
                EVENT_SYNC_PERSONAL_FIRST_PUSH_SKIPPED,
                SyncPersonalFirstPushSkippedEvent {
                    person_uid: String::new(),
                    path: "personal".to_string(),
                    reason: "person-entity-already-exists".to_string(),
                },
            );
            log(
                "personal",
                "personal first-push skipped — person entity already exists (benign 409)",
            );
            return Ok(());
        }
    };

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
            // Resolve uploader identity from the cached Cognito id token. The
            // hq-console Vault tab's CREATED BY column reads
            // S3 user metadata `created-by` / `created-by-sub` set on PutObject;
            // without these the column reads "—" for every personal-vault row.
            // ASCII-filter both fields to match `buildAuthorMetadata` in
            // packages/hq-cloud/src/s3.ts and hq-console/src/lib/s3-vault.ts.
            let author_meta = build_personal_author_metadata();
            Arc::new(move |key: String, data: Bytes, sha256_hex: String| -> BoxFuture<UploadOutcome> {
                let s3 = s3.clone();
                let bucket = bucket.clone();
                let author_meta = author_meta.clone();
                Box::pin(async move {
                    let sha256_b64 = base64::engine::general_purpose::STANDARD
                        .encode(hex_to_bytes(&sha256_hex));
                    let mut req = s3
                        .put_object()
                        .bucket(&bucket)
                        .key(&key)
                        .body(ByteStream::from(data))
                        .checksum_sha256(sha256_b64);
                    for (k, v) in author_meta.iter() {
                        req = req.metadata(k, v);
                    }
                    match req.send().await {
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

    let app_scan = app.clone();
    let person_uid_scan = person_uid.clone();
    let app_progress = app.clone();
    let person_uid_progress = person_uid.clone();
    let app_skip = app.clone();
    let person_uid_skip = person_uid.clone();
    let puid_complete = person_uid.clone();

    let (files_uploaded, files_skipped) = run_personal_first_push(
        hq_root,
        uploader,
        move |scanned, total, file| {
            let _ = app_scan.emit(
                EVENT_SYNC_PERSONAL_FIRST_PUSH_SCAN,
                SyncPersonalFirstPushScanEvent {
                    person_uid: person_uid_scan.clone(),
                    files_scanned: scanned,
                    files_total: total,
                    current_file: file,
                },
            );
        },
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

    /// Writes a `~/.hq/cognito-tokens.json` (under the test's `HOME`) whose
    /// id_token decodes to the given `sub` + `name`, so the auto-create person
    /// path can derive a slug. With name="Test User" the derived slug is
    /// "test-user". Returns the synthetic id_token for reference.
    fn write_cognito_tokens(home: &Path, sub: &str, name: &str) -> String {
        use base64::engine::general_purpose::URL_SAFE_NO_PAD;
        use base64::Engine as _;
        let payload = serde_json::json!({ "sub": sub, "name": name }).to_string();
        let b64 = URL_SAFE_NO_PAD.encode(payload.as_bytes());
        let id_token = format!("hdr.{b64}.sig");
        let json = serde_json::json!({
            "accessToken": "atok",
            "idToken": id_token,
            "refreshToken": "rtok",
            "expiresAt": 9_999_999_999_999i64,
        });
        let dir = home.join(".hq");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("cognito-tokens.json"),
            serde_json::to_vec(&json).unwrap(),
        )
        .unwrap();
        id_token
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

    // (c) Personal vault scope is now defined by exclusion (the inverse of
    //     the old PERSONAL_VAULT_PATHS allowlist). Everything except
    //     companies/, .git/, repos/, workspace/ is included (user directive
    //     2026-05-13: data/ + personal/ also now part of the personal vault).
    //     is included, subject to .gitignore/.hqignore.
    #[tokio::test]
    async fn test_personal_vault_path_exclusion() {
        let tmp_state = TempDir::new().unwrap();
        let tmp_hq = TempDir::new().unwrap();
        let root = tmp_hq.path();

        // Included (must be uploaded)
        write_file(&root.join("knowledge/notes.md"), b"knowledge");
        write_file(&root.join("policies/auto-deploy.md"), b"policy");
        write_file(&root.join("projects/foo/prd.json"), b"prd");
        write_file(&root.join(".claude/skills/foo/SKILL.md"), b"skill");
        write_file(&root.join("README.md"), b"root readme");
        write_file(&root.join("docs/README.md"), b"docs");
        // `modules/modules.yaml` is in DEFAULT_IGNORES (local resolution
        // state); test a different file under modules/ to validate the
        // dir itself is now included by the personal-vault rules.
        write_file(&root.join("modules/somepkg/README.md"), b"modules-content");
        write_file(&root.join("packages/foo/README.md"), b"packages");
        write_file(&root.join(".codex/state.json"), b"codex");
        // `core/` is now included (user directive 2026-05-13). Real-world
        // contents under `core/` include policies/, settings/, skills/,
        // workers/, plus the scaffold rules at core/core.yaml.
        write_file(&root.join("core/policies/auto-deploy.md"), b"core-policy");
        write_file(&root.join("core/core.yaml"), b"version: 1\nhqVersion: 15.0.7\n");
        // `data/` and `personal/` are now also part of the personal vault
        // (user directive 2026-05-13). They were previously local-only.
        write_file(&root.join("data/repos.yaml"), b"data-content");
        write_file(&root.join("personal/policies/wait-for-ci.md"), b"personal-policy");
        // Excluded (must be skipped)
        write_file(&root.join("companies/acme/file.md"), b"company");
        write_file(&root.join("repos/foo/README.md"), b"repos");
        write_file(&root.join("workspace/threads/T-1.md"), b"workspace");

        let calls = Arc::new(Mutex::new(vec![]));
        {
            let _guard = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
            std::env::set_var("HQ_STATE_DIR", tmp_state.path());
            let _ = run_personal_first_push(root, make_uploader(calls.clone()), |_, _, _| {}, |_, _, _| {}, |_, _| {}).await;
            std::env::remove_var("HQ_STATE_DIR");
        }

        let captured = calls.lock().unwrap();

        // Included prefixes must appear.
        for included in [".claude/", "knowledge/", "policies/", "projects/", "docs/", "modules/somepkg/", "packages/", ".codex/", "core/policies/", "core/core.yaml", "data/", "personal/", "README.md"] {
            assert!(
                captured.iter().any(|k| k.starts_with(included) || k.as_str() == included),
                "{included} must be uploaded; got: {captured:?}",
            );
        }
        // Excluded entries must NOT appear.
        for forbidden in ["companies/", "repos/", "workspace/"] {
            assert!(
                !captured.iter().any(|k| k.starts_with(forbidden)),
                "{forbidden} must be skipped; got: {captured:?}",
            );
        }
    }

    // ── is_personal_vault_path (pure helper) ─────────────────────────────

    #[test]
    fn test_is_personal_vault_path_exclusion() {
        // Included — historically allowlisted entries still in.
        assert!(is_personal_vault_path("knowledge/foo.md"));
        assert!(is_personal_vault_path("policies/auto-deploy.md"));
        assert!(is_personal_vault_path("projects/foo/prd.json"));
        assert!(is_personal_vault_path(".claude/skills/foo/SKILL.md"));
        assert!(is_personal_vault_path(".claude/commands/x.md"));
        // Included — newly permitted under exclusion semantics.
        assert!(is_personal_vault_path("README.md"), "root files now included");
        assert!(is_personal_vault_path("modules/modules.yaml"));
        assert!(is_personal_vault_path("packages/foo/README.md"));
        assert!(is_personal_vault_path("scripts/run.sh"));
        assert!(is_personal_vault_path(".codex/state.json"));
        assert!(is_personal_vault_path(".agents/runs/x.json"));
        assert!(is_personal_vault_path("knowledge.md"), "single-segment root file is a top-level itself");
        // `core/` re-included 2026-05-13 — it ships the hq-core scaffold
        // (policies/, settings/, skills/, workers/, the rules manifest at
        // core/core.yaml). The hq-root `core.yaml` identity marker is
        // filtered separately downstream by the anchored `/core.yaml`
        // DEFAULT_IGNORES rule in `@indigoai-us/hq-cloud`.
        assert!(is_personal_vault_path("core/policies/foo.md"), "core/ is part of the personal vault");
        assert!(is_personal_vault_path("core/core.yaml"), "core/core.yaml is the scaffold definition (synced)");
        // `data/` and `personal/` are now part of the personal vault
        // (user directive 2026-05-13). Were previously excluded.
        assert!(is_personal_vault_path("data/db.sqlite"), "data/ now in personal vault");
        assert!(is_personal_vault_path("personal/notes.md"), "personal/ now in personal vault");
        // Excluded — top-level dir is in the exclusion list.
        assert!(!is_personal_vault_path("companies/acme/x.md"), "companies handled by per-membership fanout");
        assert!(!is_personal_vault_path("repos/foo/README.md"), "repos/ have their own remotes");
        assert!(!is_personal_vault_path("workspace/threads/T-1.md"), "workspace/ is local session state");
        assert!(!is_personal_vault_path(".git/HEAD"), ".git/ is never synced");
        // companies/manifest.yaml is the ONE special-case: routing
        // source-of-truth, included despite the companies/ exclusion.
        // Mirrors hq-cloud@5.39.0 computePersonalVaultPaths.
        assert!(
            is_personal_vault_path("companies/manifest.yaml"),
            "manifest.yaml special-cased — routing source-of-truth, included in personal vault",
        );
        // Anti-test: ONLY manifest.yaml gets the bypass; other companies/
        // root files stay excluded.
        assert!(
            !is_personal_vault_path("companies/README.md"),
            "only manifest.yaml is special-cased — other companies/ root files stay excluded",
        );
        assert!(
            !is_personal_vault_path("companies/manifest.yml"),
            "exact filename match — .yml variant stays excluded",
        );
        // Empty input still false (no top segment to evaluate).
        assert!(!is_personal_vault_path(""));
    }

    // Regression: upload-phase progress totals must reflect the CHANGED-file
    // plan, not the walk total. Pre-fix, on_progress fired once per file
    // EXAMINED with total = every personal-vault file, so a re-run that only
    // needed to move 1 of 2,877 files showed "x of 2,877 files" in the
    // popover. The walker now scans first (on_scan, walk totals) and emits
    // on_progress only for files in the upload plan, with the plan size as
    // the denominator.
    #[tokio::test]
    async fn test_progress_total_is_changed_count_not_walk_count() {
        let tmp_state = TempDir::new().unwrap();
        let tmp_hq = TempDir::new().unwrap();
        let root = tmp_hq.path();

        write_file(&root.join("knowledge/a.md"), b"alpha");
        write_file(&root.join("knowledge/b.md"), b"bravo");
        write_file(&root.join("knowledge/c.md"), b"charlie");

        {
            let _guard = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
            std::env::set_var("HQ_STATE_DIR", tmp_state.path());

            // First run: all 3 are new → plan total 3.
            let progress1: Arc<Mutex<Vec<(usize, usize, Option<String>)>>> =
                Arc::new(Mutex::new(vec![]));
            let p1 = progress1.clone();
            run_personal_first_push(
                root,
                make_uploader(Arc::new(Mutex::new(vec![]))),
                |_, _, _| {},
                move |done, total, file| p1.lock().unwrap().push((done, total, file)),
                |_, _| {},
            )
            .await
            .unwrap();
            assert!(
                progress1.lock().unwrap().iter().all(|(_, total, _)| *total == 3),
                "first run: every progress event must carry plan total 3; got {:?}",
                progress1.lock().unwrap(),
            );

            // Touch ONE file. Re-run: walk still sees 3 files, but the plan
            // is 1 — progress must say "of 1", never "of 3".
            write_file(&root.join("knowledge/b.md"), b"bravo-changed");
            let scans: Arc<Mutex<Vec<(usize, usize)>>> = Arc::new(Mutex::new(vec![]));
            let s2 = scans.clone();
            let progress2: Arc<Mutex<Vec<(usize, usize, Option<String>)>>> =
                Arc::new(Mutex::new(vec![]));
            let p2 = progress2.clone();
            let (uploaded, _) = run_personal_first_push(
                root,
                make_uploader(Arc::new(Mutex::new(vec![]))),
                move |done, total, _| s2.lock().unwrap().push((done, total)),
                move |done, total, file| p2.lock().unwrap().push((done, total, file)),
                |_, _| {},
            )
            .await
            .unwrap();

            std::env::remove_var("HQ_STATE_DIR");

            assert_eq!(uploaded, 1, "only the touched file uploads");
            let prog = progress2.lock().unwrap();
            assert!(
                prog.iter().all(|(_, total, _)| *total == 1),
                "progress denominator must be the changed count (1), not the walk count (3); got {prog:?}",
            );
            assert!(
                prog.iter()
                    .filter_map(|(_, _, f)| f.as_deref())
                    .all(|f| f == "knowledge/b.md"),
                "progress must only fire for planned uploads; got {prog:?}",
            );
            // Scan liveness still reports walk totals (3) — separate channel.
            assert!(
                scans.lock().unwrap().iter().all(|(_, total)| *total == 3),
                "scan events carry the walk total; got {:?}",
                scans.lock().unwrap(),
            );
        }
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
            run_personal_first_push(root, make_uploader(calls1.clone()), |_, _, _| {}, |_, _, _| {}, |_, _| {})
                .await.unwrap();
            assert_eq!(calls1.lock().unwrap().len(), 1);

            let calls2 = Arc::new(Mutex::new(vec![]));
            let (uploaded, _) = run_personal_first_push(
                root,
                make_uploader(calls2.clone()),
                |_, _, _| {},
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

    // (h) Regression (feedback_dd73b772 / feedback_b5bd30ee): the person entity
    //     is already provisioned, but the list comes back empty this cycle, so
    //     resolve_or_provision reaches create → server returns 409. The fix
    //     recovers the existing entity by slug and resolves normally — the
    //     benign already-exists must NOT surface as a "personal first-push
    //     failed" error every sync.
    #[tokio::test]
    async fn test_create_409_recovered_by_slug_is_not_an_error() {
        let server = MockServer::start().await;

        // list returns empty → forces the create path
        Mock::given(method("GET"))
            .and(path("/entity/by-type/person"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({ "entities": [] })))
            .mount(&server)
            .await;
        // create → 409 (already exists)
        Mock::given(method("POST"))
            .and(path("/entity"))
            .respond_with(ResponseTemplate::new(409).set_body_json(serde_json::json!({ "error": "already exists" })))
            .mount(&server)
            .await;
        // recovery by slug returns the existing entity (bucket present → no provision)
        Mock::given(method("GET"))
            .and(path("/entity/by-slug/person/test-user"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "entity": person_entity_json("prs_existing", "test-user", Some("hq-vault-prs-existing"), "2026-01-01T00:00:00Z")
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
            write_cognito_tokens(tmp_home.path(), "sub-123", "Test User");

            let app = tauri::test::mock_app();
            let handle = app.handle().clone();
            let vault = VaultClient::new(&server.uri(), "tok");
            let r = ensure_impl(&handle, &vault, tmp_hq.path(), Some(make_counter_uploader(upload_counter.clone()))).await;

            std::env::remove_var("HQ_STATE_DIR");
            std::env::remove_var("HOME");
            r
        };

        assert!(
            result.is_ok(),
            "benign 409 (recovered by slug) must NOT surface as an error; got: {:?}",
            result
        );

        // vend_self ran against the recovered entity → first-push proceeded.
        let reqs = server.received_requests().await.unwrap();
        let vend: Vec<_> = reqs.iter().filter(|r| r.url.path() == "/sts/vend-self").collect();
        assert_eq!(vend.len(), 1, "vend_self must run once against the recovered entity");
    }

    // (i) Person exists (409) but is not resolvable by slug → benign skip:
    //     ensure_impl returns Ok(()), emits a personal-first-push-skipped
    //     diagnostic event, performs zero uploads, and never reaches vend_self.
    #[tokio::test]
    async fn test_create_409_unresolvable_skips_quietly() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/entity/by-type/person"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({ "entities": [] })))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/entity"))
            .respond_with(ResponseTemplate::new(409).set_body_json(serde_json::json!({ "error": "already exists" })))
            .mount(&server)
            .await;
        // recovery by slug 404s → unresolvable
        Mock::given(method("GET"))
            .and(path("/entity/by-slug/person/test-user"))
            .respond_with(ResponseTemplate::new(404).set_body_json(serde_json::json!({ "error": "not found" })))
            .mount(&server)
            .await;

        let tmp_state = TempDir::new().unwrap();
        let tmp_hq = TempDir::new().unwrap();
        let tmp_home = TempDir::new().unwrap();
        let upload_counter = Arc::new(AtomicUsize::new(0));
        let skip_events: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(vec![]));
        let skip_events_clone = skip_events.clone();

        let result = {
            let _guard = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
            std::env::set_var("HQ_STATE_DIR", tmp_state.path());
            std::env::set_var("HOME", tmp_home.path());
            write_cognito_tokens(tmp_home.path(), "sub-123", "Test User");

            let app = tauri::test::mock_app();
            let handle = app.handle().clone();
            app.listen(EVENT_SYNC_PERSONAL_FIRST_PUSH_SKIPPED, move |e| {
                skip_events_clone.lock().unwrap().push(e.payload().to_string());
            });
            let vault = VaultClient::new(&server.uri(), "tok");
            let r = ensure_impl(&handle, &vault, tmp_hq.path(), Some(make_counter_uploader(upload_counter.clone()))).await;

            std::env::remove_var("HQ_STATE_DIR");
            std::env::remove_var("HOME");
            r
        };

        assert!(
            result.is_ok(),
            "unresolvable benign 409 must NOT surface as an error; got: {:?}",
            result
        );
        assert_eq!(
            upload_counter.load(Ordering::SeqCst),
            0,
            "no uploads when personal first-push is skipped"
        );
        let evs = skip_events.lock().unwrap();
        assert!(
            evs.iter().any(|p| p.contains("person-entity-already-exists")),
            "a personal-first-push-skipped event with the benign reason must be emitted; got: {:?}",
            *evs
        );
        let reqs = server.received_requests().await.unwrap();
        let vend: Vec<_> = reqs.iter().filter(|r| r.url.path() == "/sts/vend-self").collect();
        assert_eq!(vend.len(), 0, "vend_self must not run on the skip path");
    }

    // (j) A REAL create failure (5xx) is NOT a benign 409 — it must still
    //     surface loudly as an Err so genuine first-push breakage is reported.
    #[tokio::test]
    async fn test_create_5xx_still_surfaces_as_err() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/entity/by-type/person"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({ "entities": [] })))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/entity"))
            .respond_with(ResponseTemplate::new(500).set_body_json(serde_json::json!({ "error": "boom" })))
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
            write_cognito_tokens(tmp_home.path(), "sub-123", "Test User");

            let app = tauri::test::mock_app();
            let handle = app.handle().clone();
            let vault = VaultClient::new(&server.uri(), "tok");
            let r = ensure_impl(&handle, &vault, tmp_hq.path(), Some(make_counter_uploader(upload_counter.clone()))).await;

            std::env::remove_var("HQ_STATE_DIR");
            std::env::remove_var("HOME");
            r
        };

        assert!(
            result.is_err(),
            "a 5xx create failure must surface as Err (loud), not be swallowed"
        );
        let msg = result.unwrap_err();
        assert!(
            msg.contains("create person entity"),
            "error should identify the create failure; got: {msg}"
        );
        assert_eq!(
            upload_counter.load(Ordering::SeqCst),
            0,
            "no uploads on a hard create failure"
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
