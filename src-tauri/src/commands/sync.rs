//! Tauri commands for spawning and cancelling `hq-sync-runner --companies`.
//!
//! Uses [`crate::commands::process`] for subprocess lifecycle (spawn, stream,
//! SIGTERM→SIGKILL). Emits typed sync events to the Svelte renderer.
//!
//! Phase 7 (ADR-0001, 2026-04-19): switched from `hq sync --json` (never
//! shipped) to `hq-sync-runner --companies`. The runner is the canonical
//! machine-targeted entrypoint from `@indigoai-us/hq-cloud` ≥5.1.0 — ndjson is
//! the default and only output mode. See:
//!   packages/hq-cloud/src/bin/sync-runner.ts
//!
//! ## Binary resolution: `npx` (not a global install)
//!
//! We spawn `npx -y --package=@indigoai-us/hq-cloud@<ver> hq-sync-runner ...`
//! instead of requiring `hq-sync-runner` to be on PATH. This keeps the
//! install story simple: the HQ Sync DMG needs Node.js on the machine
//! (already enforced by the installer's deps step) and nothing else — the
//! runner is downloaded into npx's on-disk cache (`~/.npm/_npx/`) on first
//! use and reused forever after.
//!
//! **Why not a global `npm install -g`?** Tried it twice; both times a
//! later UX-polish pass decided "hq-cloud isn't really a prereq" and
//! removed it from the installer's DEPS list, re-breaking every fresh
//! install. Putting the dependency at the spawn site (this file) means
//! there's no separate list to forget. See PRs #9 / #15 in hq-installer.
//!
//! **Version pinning:** `HQ_CLOUD_VERSION` below is authoritative. Bumping
//! it ships a new runner to users on their next sync (npx sees a new
//! cache key, downloads once, caches for steady state). See
//! `commands::prewarm` for the on-startup background fetch that keeps the
//! user's first-click-Sync-Now latency near zero after a version bump.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::SecondsFormat;
use tauri::{AppHandle, Emitter};

use crate::commands::cognito;
use crate::commands::config::{ensure_machine_id, HqConfig, MenubarPrefs};
use crate::commands::vault_client::VaultClient;
use crate::commands::process::{
    cancel_process_impl, deregister_process, is_registered, run_process_impl, try_register_handle,
    ProcessEvent, SpawnArgs,
};
use crate::commands::status::{journal_for_sync_complete, write_journal};
use crate::events::{
    SyncCompanyProvisionedEvent, SyncCompleteEvent, SyncErrorEvent, SyncEvent,
    EVENT_SYNC_ALL_COMPLETE, EVENT_SYNC_AUTH_ERROR, EVENT_SYNC_COMPANY_PROVISIONED,
    EVENT_SYNC_COMPLETE, EVENT_SYNC_ERROR, EVENT_SYNC_FANOUT_PLAN, EVENT_SYNC_PROGRESS,
    EVENT_SYNC_SETUP_NEEDED,
};
use crate::util::paths;

// ─────────────────────────────────────────────────────────────────────────────
// Per-run aggregated counters
// ─────────────────────────────────────────────────────────────────────────────

/// Aggregated counters across a single sync run.
///
/// A fresh instance is created per `start_sync` invocation, so totals are
/// scoped to the run — no reset needed between runs. Per-company `Complete`
/// events contribute via `accumulate`; the `AllComplete` handler reads the
/// final totals to build the journal.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RunTotals {
    pub conflicts: u32,
}

impl RunTotals {
    /// Update totals from a single event. Only `Complete` events contribute;
    /// others are ignored. Saturates at `u32::MAX` to avoid panics.
    pub fn accumulate(&mut self, event: &SyncEvent) {
        if let SyncEvent::Complete(c) = event {
            self.conflicts = self.conflicts.saturating_add(c.conflicts);
        }
    }
}

/// Singleton handle — only one sync at a time.
const SYNC_HANDLE: &str = "hq-sync";

/// Hard timeout for a sync run (10 minutes).
const SYNC_TIMEOUT: Duration = Duration::from_secs(600);

/// SIGKILL delay after SIGTERM on cancel.
const SIGKILL_DELAY: Duration = Duration::from_secs(5);

/// Pinned version of `@indigoai-us/hq-cloud` that ships `hq-sync-runner`.
///
/// Bumping this cuts a new npx cache key, so every user's next sync
/// fetches the new runner once, then reuses the cache. The
/// `commands::prewarm` task fires this same fetch on app startup so the
/// fetch happens in the background rather than during the user's first
/// click of "Sync Now".
pub const HQ_CLOUD_VERSION: &str = "5.2.0";

/// Package name for the runner. Used by both the spawn site below and the
/// startup prewarm. Paired with `HQ_CLOUD_VERSION` to form the full
/// `npx --package=<pkg>@<ver>` argument.
pub const HQ_CLOUD_PACKAGE: &str = "@indigoai-us/hq-cloud";

/// Bin name shipped by `HQ_CLOUD_PACKAGE` (per its package.json `bin` entry).
/// npx needs this separately from the package because the bin name does
/// not match the package name.
pub const RUNNER_BIN: &str = "hq-sync-runner";

// ─────────────────────────────────────────────────────────────────────────────
// Config resolution (inline — avoids calling async Tauri command)
// ─────────────────────────────────────────────────────────────────────────────

/// Resolve the HQ folder path by reading config.json and menubar.json directly.
fn resolve_hq_folder_path() -> Result<String, String> {
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
        let contents = std::fs::read_to_string(&config_path)
            .map_err(|e| format!("Failed to read config.json: {}", e))?;
        Some(
            serde_json::from_str(&contents)
                .map_err(|e| format!("Failed to parse config.json: {}", e))?,
        )
    } else {
        None
    };

    let hq_folder = paths::resolve_hq_folder(
        config
            .as_ref()
            .and_then(|c| c.hq_folder_path.as_deref()),
        menubar_prefs
            .as_ref()
            .and_then(|p| p.hq_path.as_deref()),
    );

    Ok(hq_folder.to_string_lossy().to_string())
}

/// Resolve the vault API URL. Precedence (highest to lowest):
///   1. `HQ_VAULT_API_URL` env var — dev/test override.
///   2. `~/.hq/config.json` `vault_api_url` field — legacy installer-provisioned
///      setups continue to work without migration. Read errors fall through
///      to the default rather than aborting (the file may be partial/stale).
///   3. Hardcoded canonical hq-dev URL — lets create-hq users (and anyone
///      with `companies/{slug}/company.yaml: { cloud: true }` but no global
///      config) run hq-sync directly. `provision_missing_companies` then
///      walks the YAMLs and writes per-company `.hq/config.json` files
///      itself, so the global config.json is no longer required.
///
/// See hq-pro ADR-0003 for the canonical-stage rationale.
fn resolve_vault_api_url() -> Result<String, String> {
    const DEFAULT_VAULT_API_URL: &str =
        "https://ky8cgbl4yh.execute-api.us-east-1.amazonaws.com";

    if let Ok(url) = std::env::var("HQ_VAULT_API_URL") {
        if !url.is_empty() {
            return Ok(url);
        }
    }

    let config_path = paths::config_json_path()?;
    if config_path.exists() {
        if let Ok(contents) = std::fs::read_to_string(&config_path) {
            if let Ok(config) = serde_json::from_str::<HqConfig>(&contents) {
                return Ok(config.vault_api_url);
            }
        }
    }

    Ok(DEFAULT_VAULT_API_URL.to_string())
}

/// Testable core: given a pre-fetched token result and a refresh function,
/// return a fresh access token (refreshing if expired).
///
/// The `tokens = refreshed;` reassignment is the critical line that routes the
/// returned token through the refreshed struct — removing it causes the function
/// to return the stale access_token. `test_start_sync_jwt_fetch_uses_refreshed_token`
/// asserts this.
async fn resolve_jwt_impl<F, Fut>(
    tokens_result: Result<Option<cognito::CognitoTokens>, String>,
    refresh_fn: F,
) -> Result<String, String>
where
    F: FnOnce(String) -> Fut,
    Fut: std::future::Future<Output = Result<cognito::CognitoTokens, String>>,
{
    let mut tokens = tokens_result?
        .ok_or_else(|| "Not signed in — please complete setup first".to_string())?;
    if cognito::is_expired(&tokens) {
        let refreshed = refresh_fn(tokens.refresh_token).await?;
        tokens = refreshed;
    }
    Ok(tokens.access_token)
}

/// Fetch the current JWT from the on-disk token cache, refreshing if expired.
pub async fn resolve_jwt() -> Result<String, String> {
    let tokens_result = cognito::get_tokens().await;
    resolve_jwt_impl(tokens_result, |rt| async move {
        cognito::refresh_access_token(&rt).await
    })
    .await
}

// ─────────────────────────────────────────────────────────────────────────────
// SpawnArgs builder (testable)
// ─────────────────────────────────────────────────────────────────────────────

/// Build the SpawnArgs for `npx … hq-sync-runner --companies`.
///
/// The command line we spawn looks like:
/// ```text
/// npx -y --package=@indigoai-us/hq-cloud@5.1.11 hq-sync-runner \
///   --companies --direction both --on-conflict abort --hq-root <path>
/// ```
///
/// npx flags:
/// - `-y` / `--yes` — auto-confirm the "Need to install the following
///   packages — Ok to proceed?" prompt. Without this, npx blocks on stdin
///   (our Tauri subprocess has no interactive stdin → hang).
/// - `--package=<pkg>@<ver>` — tells npx which package provides the bin,
///   since the bin name (`hq-sync-runner`) doesn't match the package
///   name (`@indigoai-us/hq-cloud`). The `@<ver>` pin makes the cache
///   key deterministic: same pin → same cache hit → no redownload.
///
/// Runner flags:
/// - `--companies` — fan out to every membership the caller has
/// - `--direction both` — bidirectional sync: push local changes first,
///   then pull remote. Added in hq-cloud 5.1.11. Runner default is `pull`
///   for back-compat; the menubar explicitly opts into `both` so a single
///   "Sync Now" click broadcasts local edits AND pulls remote updates.
/// - `--on-conflict abort` — V1 policy; conflicts surface as `aborted: true` on
///   the per-company `complete` event. Interactive resolution is a follow-up
///   (the runner protocol doesn't emit per-file conflict events).
/// - `--hq-root <path>` — local HQ directory
///
/// `HQ_ROOT` is also set in the child env as defense-in-depth (matches the
/// pre-Phase-7 pattern).
pub fn build_sync_spawn_args(hq_folder_path: &str) -> SpawnArgs {
    let mut env = HashMap::new();
    env.insert("HQ_ROOT".to_string(), hq_folder_path.to_string());
    // The runner is a Node script with `#!/usr/bin/env node`, and npx itself
    // is `#!/usr/bin/env node`. Without a real PATH, `env` can't find node on
    // Dock-launched apps and either process exits with code 127. See
    // `paths::child_path`.
    env.insert("PATH".to_string(), paths::child_path());

    SpawnArgs {
        // Resolve npx via known install prefixes + login-shell PATH fallback.
        // See `paths::resolve_bin` — GUI-launched Tauri apps get a minimal
        // launchd PATH and would otherwise fail with os error 2 on `npx`
        // (which lives in /opt/homebrew/bin or ~/.npm-global/bin, not in
        // /usr/bin). npx is part of npm, which is a listed installer prereq.
        cmd: paths::resolve_bin("npx"),
        args: vec![
            "-y".to_string(),
            format!("--package={}@{}", HQ_CLOUD_PACKAGE, HQ_CLOUD_VERSION),
            RUNNER_BIN.to_string(),
            "--companies".to_string(),
            "--direction".to_string(),
            "both".to_string(),
            "--on-conflict".to_string(),
            "abort".to_string(),
            "--hq-root".to_string(),
            hq_folder_path.to_string(),
        ],
        cwd: None,
        env: Some(env),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// ndjson line handler (testable)
// ─────────────────────────────────────────────────────────────────────────────

/// Returns `true` when a per-company error indicates the company has not been
/// provisioned on S3 yet.
///
/// Only per-company sentinel errors (`path == "(company)"`) are eligible; file-
/// level errors on real paths are never entity-not-found and must surface normally.
///
/// Match logic is deliberately narrow to avoid swallowing auth / STS errors
/// whose HTTP bodies can also contain generic "not found" substrings:
/// - `"no bucket provisioned"` is an exact phrase unique to the vault guard.
/// - For HTTP-404 paths we require **both** `"entity"` and `"not found"` so
///   that `"Token not found"`, `"Session not found"`, etc. are excluded.
fn is_entity_not_yet_provisioned(err: &SyncErrorEvent) -> bool {
    if err.path != "(company)" {
        return false;
    }
    let msg = err.message.to_lowercase();
    msg.contains("no bucket provisioned")
        || (msg.contains("entity") && msg.contains("not found"))
}

/// Classifies a per-company error event. Returns `Some(SyncCompleteEvent)` when
/// the error represents a company not yet provisioned on S3 (empty-sync
/// semantics), or `None` when the error should surface normally.
///
/// The `None`-company case (discovery-phase errors) always returns `None` so
/// those errors are never silently swallowed.
///
/// TODO: The durable fix belongs in `hq-cloud/src/context.ts` (`resolveEntityContext`)
/// so all consumers of hq-sync-runner get the correct behaviour without
/// pattern-matching on error strings across a process boundary.
fn classify_error_event(payload: &SyncErrorEvent) -> Option<SyncCompleteEvent> {
    let company = payload.company.as_deref()?;
    if !is_entity_not_yet_provisioned(payload) {
        return None;
    }
    Some(SyncCompleteEvent {
        company: company.to_string(),
        files_downloaded: 0,
        bytes_downloaded: 0,
        files_skipped: 0,
        conflicts: 0,
        aborted: false,
    })
}

/// Parse a single ndjson line and emit the corresponding Tauri event.
/// Unknown/malformed lines are silently skipped (logged in debug builds).
///
/// Per-company `Complete` events also accumulate into `totals`. On
/// `all-complete`, the aggregated totals are persisted to
/// `{hq_folder}/.hq-sync-journal.json` so `get_sync_status` surfaces a real
/// `lastSyncAt` and conflict count instead of "never" / zero.
fn handle_sync_line(app: &AppHandle, hq_folder: &str, totals: &Mutex<RunTotals>, line: &str) {
    // The runner can emit blank lines at process teardown. Skip those cheaply
    // rather than logging a parse error.
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return;
    }

    let event: SyncEvent = match serde_json::from_str(trimmed) {
        Ok(e) => e,
        Err(_e) => {
            #[cfg(debug_assertions)]
            eprintln!("[sync] skipping unparseable line: {} | line: {}", _e, trimmed);
            return;
        }
    };

    // Accumulate per-run counters before emitting. Poisoned locks shouldn't
    // happen in practice (no panics while the mutex is held), but we recover
    // by using the inner value rather than crashing the sync thread.
    {
        let mut t = totals.lock().unwrap_or_else(|e| e.into_inner());
        t.accumulate(&event);
    }

    // Unit struct variants (SetupNeeded) serialize to `()` when emitted via
    // Tauri's `emit(...)` — the frontend gets the event name and an empty
    // payload, which is exactly what we want for a "caller has no person
    // entity" signal.
    let result = match &event {
        SyncEvent::SetupNeeded => app.emit(EVENT_SYNC_SETUP_NEEDED, ()),
        SyncEvent::AuthError(payload) => app.emit(EVENT_SYNC_AUTH_ERROR, payload.clone()),
        SyncEvent::FanoutPlan(payload) => app.emit(EVENT_SYNC_FANOUT_PLAN, payload.clone()),
        SyncEvent::Progress(payload) => app.emit(EVENT_SYNC_PROGRESS, payload.clone()),
        SyncEvent::Error(payload) => {
            // `classify_error_event` is the test-covered classification boundary;
            // the dispatch logic here (Some → COMPLETE, None → ERROR) is intentionally
            // kept to these two lines so it is visually auditable without a harness.
            if let Some(complete_event) = classify_error_event(payload) {
                #[cfg(debug_assertions)]
                eprintln!(
                    "[sync] company '{}' not yet on S3 — treating as empty sync: {}",
                    complete_event.company, payload.message
                );
                // Synthetic completes are excluded from RunTotals by design:
                // all fields are zero so accumulate would be a no-op today, and
                // these companies have no real files to count.
                app.emit(EVENT_SYNC_COMPLETE, complete_event)
            } else {
                app.emit(EVENT_SYNC_ERROR, payload.clone())
            }
        }
        SyncEvent::Complete(payload) => app.emit(EVENT_SYNC_COMPLETE, payload.clone()),
        SyncEvent::AllComplete(payload) => {
            // Persist summary journal before emitting — the frontend's
            // SyncStats refresh reads this file on popover mount.
            let conflicts = totals.lock().unwrap_or_else(|e| e.into_inner()).conflicts;
            let now_iso = chrono::Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);
            let journal = journal_for_sync_complete(&now_iso, conflicts);
            if let Err(_e) = write_journal(hq_folder, &journal) {
                #[cfg(debug_assertions)]
                eprintln!("[sync] failed to write journal: {}", _e);
            }
            app.emit(EVENT_SYNC_ALL_COMPLETE, payload.clone())
        }
    };

    if let Err(_e) = result {
        #[cfg(debug_assertions)]
        eprintln!("[sync] failed to emit event: {}", _e);
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tauri commands
// ─────────────────────────────────────────────────────────────────────────────

/// Spawn `hq-sync-runner --companies` as a child process.
///
/// - Only one sync can run at a time (singleton handle).
/// - Emits typed sync events (see `events.rs`) to the Svelte renderer as
///   ndjson lines arrive.
/// - Hard timeout of 10 minutes; the sync is cancelled if it exceeds this.
///
/// Returns the handle string on success (always `"hq-sync"`).
#[tauri::command]
pub async fn start_sync(app: AppHandle) -> Result<String, String> {
    #[cfg(debug_assertions)]
    eprintln!("[sync] start_sync invoked");

    // Atomically check-and-register to prevent concurrent syncs (TOCTOU-safe)
    if !try_register_handle(SYNC_HANDLE) {
        #[cfg(debug_assertions)]
        eprintln!("[sync] BAIL: already running");
        return Err("Sync is already running".to_string());
    }

    // Best-effort machineId bootstrap — log on failure but do not abort sync.
    if let Err(e) = ensure_machine_id() {
        eprintln!("ensure_machine_id failed: {e}");
    }

    // Resolve HQ folder — deregister on failure so future syncs aren't blocked
    let hq_folder_path = match resolve_hq_folder_path() {
        Ok(p) => p,
        Err(e) => {
            #[cfg(debug_assertions)]
            eprintln!("[sync] BAIL: resolve_hq_folder_path failed: {}", e);
            deregister_process(SYNC_HANDLE);
            return Err(e);
        }
    };

    // Resolve vault URL from ~/.hq/config.json
    let vault_api_url = match resolve_vault_api_url() {
        Ok(u) => u,
        Err(e) => {
            deregister_process(SYNC_HANDLE);
            return Err(e);
        }
    };

    // Fetch (and if needed refresh) the Cognito JWT
    let jwt = match resolve_jwt().await {
        Ok(j) => j,
        Err(e) => {
            deregister_process(SYNC_HANDLE);
            return Err(e);
        }
    };

    // Provision any cloud: true companies that haven't been provisioned yet
    let vault = VaultClient::new(&vault_api_url, &jwt);
    let companies = match crate::commands::provision::provision_missing_companies(
        &std::path::PathBuf::from(&hq_folder_path),
        &vault,
        &vault_api_url,
    )
    .await
    {
        Ok(c) => c,
        Err(e) => {
            deregister_process(SYNC_HANDLE);
            return Err(e);
        }
    };
    for company in &companies {
        if let Err(_e) = app.emit(
            EVENT_SYNC_COMPANY_PROVISIONED,
            SyncCompanyProvisionedEvent {
                company_uid: company.uid.clone(),
                company_slug: company.slug.clone(),
                bucket_name: company.bucket_name.clone(),
            },
        ) {
            #[cfg(debug_assertions)]
            eprintln!("[sync] failed to emit company-provisioned: {}", _e);
        }
        // First-push: upload every local file for the newly-provisioned company.
        if let Err(e) = crate::commands::first_push::first_push_company(
            &app,
            &vault,
            &std::path::PathBuf::from(&hq_folder_path),
            company,
        )
        .await
        {
            #[cfg(debug_assertions)]
            eprintln!("[sync] first_push failed for {}: {}", company.slug, e);
            let _ = app.emit(
                crate::events::EVENT_SYNC_COMPANY_FIRST_PUSH_FAILED,
                crate::events::SyncCompanyFirstPushFailedEvent {
                    company_uid: company.uid.clone(),
                    company_slug: company.slug.clone(),
                    error: e,
                },
            );
        }
    }

    // Personal first-push: provision + upload personal HQ files via /sts/vend-self.
    if let Err(e) = crate::commands::personal::ensure_personal_bucket_and_first_push(
        &app,
        &vault,
        &std::path::PathBuf::from(&hq_folder_path),
    )
    .await
    {
        #[cfg(debug_assertions)]
        eprintln!("[sync] personal first-push failed: {}", e);
        let _ = app.emit(
            EVENT_SYNC_ERROR,
            SyncErrorEvent {
                company: None,
                path: "personal".to_string(),
                message: format!("personal first-push failed: {e}"),
            },
        );
    }

    let spawn_args = build_sync_spawn_args(&hq_folder_path);
    #[cfg(debug_assertions)]
    eprintln!(
        "[sync] about to spawn: cmd={} args={:?} hq_root={}",
        spawn_args.cmd, spawn_args.args, hq_folder_path
    );

    // Timeout watchdog — cancels sync after SYNC_TIMEOUT
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(SYNC_TIMEOUT).await;
        if is_registered(SYNC_HANDLE) {
            #[cfg(debug_assertions)]
            eprintln!("[sync] timeout reached, cancelling");
            cancel_process_impl(SYNC_HANDLE, SIGKILL_DELAY);
        }
    });

    // Background task: run the subprocess and stream events.
    // run_process_impl is a blocking sync function (mpsc::Receiver iteration +
    // child.wait()), so it must run on a dedicated OS thread via spawn_blocking,
    // not on a tokio worker thread.
    let app_bg = app.clone();
    let hq_folder_for_handler = hq_folder_path.clone();
    // Fresh totals per run — no reset needed between runs.
    let totals: Arc<Mutex<RunTotals>> = Arc::new(Mutex::new(RunTotals::default()));
    tauri::async_runtime::spawn_blocking(move || {
        #[cfg(debug_assertions)]
        eprintln!("[sync] bg task: entering run_process_impl");
        let result = run_process_impl(SYNC_HANDLE, &spawn_args, |event| match event {
            ProcessEvent::Stdout(line) => {
                #[cfg(debug_assertions)]
                eprintln!("[sync stdout] {}", line);
                handle_sync_line(&app_bg, &hq_folder_for_handler, &totals, &line);
            }
            ProcessEvent::Stderr(_line) => {
                #[cfg(debug_assertions)]
                eprintln!("[sync stderr] {}", _line);
            }
            ProcessEvent::Exit { code, success } => {
                // The runner exits 0 for recoverable conditions (setup-needed,
                // auth-error) — those surface as ndjson events before exit, so
                // the frontend already knows. A non-zero exit means the runner
                // bailed before emitting a useful protocol stream.
                if !success {
                    let _ = app_bg.emit(
                        EVENT_SYNC_ERROR,
                        crate::events::SyncErrorEvent {
                            company: None,
                            path: "(runner)".to_string(),
                            message: format!(
                                "hq-sync-runner exited with code {}",
                                code.map(|c| c.to_string())
                                    .unwrap_or_else(|| "unknown".to_string())
                            ),
                        },
                    );
                }
            }
        });

        if let Err(e) = result {
            let _ = app_bg.emit(
                EVENT_SYNC_ERROR,
                crate::events::SyncErrorEvent {
                    company: None,
                    path: "(spawn)".to_string(),
                    message: e,
                },
            );
        }
    });

    Ok(SYNC_HANDLE.to_string())
}

/// Cancel a running sync.
///
/// Sends SIGTERM to the process group. If the process doesn't exit within 5
/// seconds, SIGKILL is sent.
///
/// Returns `true` if a sync was running and cancellation was initiated.
#[tauri::command]
pub fn cancel_sync() -> bool {
    cancel_process_impl(SYNC_HANDLE, SIGKILL_DELAY)
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::cognito::CognitoTokens;

    // ── resolve_jwt_impl ─────────────────────────────────────────────────────────

    fn make_tokens(access: &str, refresh: &str, expires_at: i64) -> CognitoTokens {
        CognitoTokens {
            access_token: access.to_string(),
            id_token: None,
            refresh_token: refresh.to_string(),
            expires_at,
        }
    }

    /// The `tokens = refreshed;` reassignment is critical: without it the function
    /// returns the stale access_token even after a successful refresh.
    #[tokio::test]
    async fn test_start_sync_jwt_fetch_uses_refreshed_token() {
        let expired = make_tokens("EXPIRED_ACCESS", "REFRESH_TOKEN", 0); // expires_at=0 → is_expired==true
        let fresh = make_tokens("FRESH_ACCESS", "REFRESH_TOKEN", i64::MAX);

        let result = resolve_jwt_impl(Ok(Some(expired)), |_rt| async move { Ok(fresh) })
            .await
            .unwrap();

        assert_eq!(
            result, "FRESH_ACCESS",
            "resolve_jwt must return the refreshed access_token, not the expired one"
        );
    }

    #[tokio::test]
    async fn test_resolve_jwt_impl_no_refresh_when_not_expired() {
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;
        let valid = make_tokens("VALID_ACCESS", "REFRESH_TOKEN", now_ms + 600_000);

        let result = resolve_jwt_impl(Ok(Some(valid)), |_rt| async move {
            panic!("refresh_fn must not be called when token is valid")
        })
        .await
        .unwrap();

        assert_eq!(result, "VALID_ACCESS");
    }

    #[tokio::test]
    async fn test_resolve_jwt_impl_none_tokens_returns_err() {
        let result = resolve_jwt_impl(
            Ok(None),
            |_rt| async move { panic!("should not reach refresh") },
        )
        .await;
        assert!(result.is_err());
    }

    #[test]
    fn test_build_sync_spawn_args_cmd() {
        let args = build_sync_spawn_args("/Users/test/HQ");
        // `resolve_bin` may return an absolute path (e.g.
        // `/opt/homebrew/bin/npx`) on a dev box with npm installed, or the
        // bare name on a CI box without it. Either way, the trailing file
        // component must be `npx`.
        assert!(
            args.cmd == "npx" || args.cmd.ends_with("/npx"),
            "expected cmd to be `npx` or `*/npx`, got `{}`",
            args.cmd
        );
    }

    #[test]
    fn test_build_sync_spawn_args_flags() {
        let args = build_sync_spawn_args("/Users/test/HQ");
        assert_eq!(
            args.args,
            vec![
                "-y".to_string(),
                format!("--package={}@{}", HQ_CLOUD_PACKAGE, HQ_CLOUD_VERSION),
                RUNNER_BIN.to_string(),
                "--companies".to_string(),
                "--direction".to_string(),
                "both".to_string(),
                "--on-conflict".to_string(),
                "abort".to_string(),
                "--hq-root".to_string(),
                "/Users/test/HQ".to_string(),
            ]
        );
    }

    /// Sync Now is bidirectional — the spawn must opt into `--direction both`.
    /// Guards against a future refactor silently dropping back to pull-only.
    #[test]
    fn test_build_sync_spawn_args_opts_into_direction_both() {
        let args = build_sync_spawn_args("/tmp");
        let joined = args.args.join(" ");
        assert!(
            joined.contains("--direction both"),
            "spawn args must include `--direction both`: {:?}",
            args.args,
        );
    }

    /// Guards against the regression that broke fresh installs twice: the
    /// runner is ONLY available via this npx invocation. If a future refactor
    /// decides to drop the `--package=` arg, every sync fails with "npm
    /// package `hq-sync-runner` not found". This test makes that failure
    /// obvious in CI, not at runtime on users' machines.
    #[test]
    fn test_build_sync_spawn_args_pins_hq_cloud_package() {
        let args = build_sync_spawn_args("/tmp");
        let expected_pin = format!("--package={}@{}", HQ_CLOUD_PACKAGE, HQ_CLOUD_VERSION);
        assert!(
            args.args.contains(&expected_pin),
            "spawn args must pin the hq-cloud package (missing `{}`): {:?}",
            expected_pin,
            args.args,
        );
        assert!(
            args.args.contains(&"-y".to_string()),
            "spawn args must include `-y` so npx doesn't block on stdin: {:?}",
            args.args,
        );
        assert!(
            args.args.contains(&RUNNER_BIN.to_string()),
            "spawn args must invoke `{}` after the package pin: {:?}",
            RUNNER_BIN,
            args.args,
        );
    }

    #[test]
    fn test_build_sync_spawn_args_env_sets_hq_root() {
        let args = build_sync_spawn_args("/Users/test/HQ");
        let env = args.env.unwrap();
        assert_eq!(env.get("HQ_ROOT"), Some(&"/Users/test/HQ".to_string()));
        assert_eq!(env.len(), 2);
    }

    #[test]
    fn test_build_sync_spawn_args_env_sets_path_with_homebrew() {
        let args = build_sync_spawn_args("/tmp");
        let env = args.env.unwrap();
        let path = env.get("PATH").expect("PATH must be set so shebang can find node");
        // Must include homebrew so `#!/usr/bin/env node` resolves on Dock launches.
        assert!(path.contains("/opt/homebrew/bin"), "PATH missing /opt/homebrew/bin: {}", path);
    }

    #[test]
    fn test_build_sync_spawn_args_no_cwd() {
        let args = build_sync_spawn_args("/any/path");
        assert!(args.cwd.is_none());
    }

    #[test]
    fn test_parse_progress_ndjson() {
        let line = r#"{"type":"progress","company":"indigo","path":"docs/a.md","bytes":42}"#;
        let event: SyncEvent = serde_json::from_str(line).unwrap();
        match event {
            SyncEvent::Progress(p) => {
                assert_eq!(p.company, "indigo");
                assert_eq!(p.path, "docs/a.md");
                assert_eq!(p.bytes, 42);
                assert_eq!(p.message, None);
            }
            _ => panic!("Expected Progress event"),
        }
    }

    #[test]
    fn test_parse_setup_needed_ndjson() {
        let line = r#"{"type":"setup-needed"}"#;
        let event: SyncEvent = serde_json::from_str(line).unwrap();
        assert_eq!(event, SyncEvent::SetupNeeded);
    }

    #[test]
    fn test_parse_auth_error_ndjson() {
        let line = r#"{"type":"auth-error","message":"Token expired"}"#;
        let event: SyncEvent = serde_json::from_str(line).unwrap();
        match event {
            SyncEvent::AuthError(e) => assert_eq!(e.message, "Token expired"),
            _ => panic!("Expected AuthError event"),
        }
    }

    #[test]
    fn test_parse_fanout_plan_ndjson() {
        let line = r#"{"type":"fanout-plan","companies":[{"uid":"cmp_1","slug":"indigo"}]}"#;
        let event: SyncEvent = serde_json::from_str(line).unwrap();
        match event {
            SyncEvent::FanoutPlan(p) => {
                assert_eq!(p.companies.len(), 1);
                assert_eq!(p.companies[0].slug, "indigo");
            }
            _ => panic!("Expected FanoutPlan event"),
        }
    }

    #[test]
    fn test_parse_error_ndjson() {
        let line = r#"{"type":"error","company":"indigo","path":"docs/x.md","message":"Access denied"}"#;
        let event: SyncEvent = serde_json::from_str(line).unwrap();
        match event {
            SyncEvent::Error(e) => {
                assert_eq!(e.company, Some("indigo".to_string()));
                assert_eq!(e.path, "docs/x.md");
                assert_eq!(e.message, "Access denied");
            }
            _ => panic!("Expected Error event"),
        }
    }

    #[test]
    fn test_parse_complete_ndjson() {
        let line = r#"{"type":"complete","company":"indigo","filesDownloaded":7,"bytesDownloaded":204800,"filesSkipped":1,"conflicts":0,"aborted":false}"#;
        let event: SyncEvent = serde_json::from_str(line).unwrap();
        match event {
            SyncEvent::Complete(c) => {
                assert_eq!(c.company, "indigo");
                assert_eq!(c.files_downloaded, 7);
                assert_eq!(c.bytes_downloaded, 204800);
                assert!(!c.aborted);
            }
            _ => panic!("Expected Complete event"),
        }
    }

    #[test]
    fn test_parse_all_complete_ndjson() {
        let line = r#"{"type":"all-complete","companiesAttempted":2,"filesDownloaded":10,"bytesDownloaded":999,"errors":[]}"#;
        let event: SyncEvent = serde_json::from_str(line).unwrap();
        match event {
            SyncEvent::AllComplete(a) => {
                assert_eq!(a.companies_attempted, 2);
                assert!(a.errors.is_empty());
            }
            _ => panic!("Expected AllComplete event"),
        }
    }

    #[test]
    fn test_unknown_event_type_skipped() {
        let line = r#"{"type":"metrics","cpu":50}"#;
        let result: Result<SyncEvent, _> = serde_json::from_str(line);
        assert!(result.is_err(), "Unknown type should fail to parse");
    }

    #[test]
    fn test_malformed_json_skipped() {
        let line = "not json at all";
        let result: Result<SyncEvent, _> = serde_json::from_str(line);
        assert!(result.is_err());
    }

    #[test]
    fn test_empty_line_skipped() {
        let line = "";
        let result: Result<SyncEvent, _> = serde_json::from_str(line);
        assert!(result.is_err());
    }

    #[test]
    fn test_sync_handle_constant() {
        assert_eq!(SYNC_HANDLE, "hq-sync");
    }

    #[test]
    fn test_runner_bin_constant() {
        assert_eq!(RUNNER_BIN, "hq-sync-runner");
    }

    #[test]
    fn test_hq_cloud_package_constant() {
        assert_eq!(HQ_CLOUD_PACKAGE, "@indigoai-us/hq-cloud");
    }

    /// Belt-and-braces: fail loudly if someone pastes a non-semver string
    /// into the version const. npx tolerates a lot, but "latest" / "*" /
    /// empty would defeat the whole point of cache pinning and make first
    /// sync a roulette wheel.
    #[test]
    fn test_hq_cloud_version_is_pinned_semver() {
        assert!(
            !HQ_CLOUD_VERSION.is_empty(),
            "HQ_CLOUD_VERSION must not be empty"
        );
        assert_ne!(
            HQ_CLOUD_VERSION, "latest",
            "HQ_CLOUD_VERSION must be a pinned semver, not `latest`"
        );
        // Rough semver shape: three dot-separated numeric segments.
        let parts: Vec<&str> = HQ_CLOUD_VERSION.split('.').collect();
        assert_eq!(
            parts.len(),
            3,
            "HQ_CLOUD_VERSION should look like MAJOR.MINOR.PATCH, got `{}`",
            HQ_CLOUD_VERSION
        );
        for part in &parts {
            assert!(
                part.chars().all(|c| c.is_ascii_digit()),
                "HQ_CLOUD_VERSION segment `{}` is not a number — got `{}`",
                part,
                HQ_CLOUD_VERSION
            );
        }
    }

    // ── RunTotals ────────────────────────────────────────────────────────

    use crate::events::{SyncAllCompleteEvent, SyncCompleteEvent, SyncProgressEvent};

    fn complete(company: &str, conflicts: u32, aborted: bool) -> SyncEvent {
        SyncEvent::Complete(SyncCompleteEvent {
            company: company.to_string(),
            files_downloaded: 0,
            bytes_downloaded: 0,
            files_skipped: 0,
            conflicts,
            aborted,
        })
    }

    #[test]
    fn test_run_totals_default_is_zero() {
        let t = RunTotals::default();
        assert_eq!(t.conflicts, 0);
    }

    #[test]
    fn test_accumulate_ignores_setup_needed() {
        let mut t = RunTotals::default();
        t.accumulate(&SyncEvent::SetupNeeded);
        assert_eq!(t.conflicts, 0);
    }

    #[test]
    fn test_accumulate_ignores_progress() {
        let mut t = RunTotals::default();
        t.accumulate(&SyncEvent::Progress(SyncProgressEvent {
            company: "x".to_string(),
            path: "y".to_string(),
            bytes: 0,
            message: None,
        }));
        assert_eq!(t.conflicts, 0);
    }

    #[test]
    fn test_accumulate_ignores_all_complete() {
        let mut t = RunTotals { conflicts: 4 };
        t.accumulate(&SyncEvent::AllComplete(SyncAllCompleteEvent {
            companies_attempted: 1,
            files_downloaded: 0,
            bytes_downloaded: 0,
            errors: vec![],
        }));
        // AllComplete is the signal to read, not accumulate — totals unchanged.
        assert_eq!(t.conflicts, 4);
    }

    #[test]
    fn test_accumulate_sums_conflicts_across_completes() {
        let mut t = RunTotals::default();
        t.accumulate(&complete("a", 3, false));
        t.accumulate(&complete("b", 2, true)); // aborted companies still contribute
        assert_eq!(t.conflicts, 5);
    }

    #[test]
    fn test_accumulate_zero_conflicts_is_noop() {
        let mut t = RunTotals { conflicts: 10 };
        t.accumulate(&complete("a", 0, false));
        assert_eq!(t.conflicts, 10);
    }

    #[test]
    fn test_accumulate_saturates_on_overflow() {
        let mut t = RunTotals {
            conflicts: u32::MAX,
        };
        t.accumulate(&complete("a", 1, false));
        assert_eq!(t.conflicts, u32::MAX);
    }

    // ── is_entity_not_yet_provisioned ────────────────────────────────────────

    fn make_company_error(company: Option<&str>, path: &str, message: &str) -> SyncErrorEvent {
        SyncErrorEvent {
            company: company.map(str::to_string),
            path: path.to_string(),
            message: message.to_string(),
        }
    }

    #[test]
    fn test_not_provisioned_404_not_found_in_message() {
        let err = make_company_error(
            Some("acme"),
            "(company)",
            "Failed to fetch entity cmp_01ABC: 404 company/entity not found",
        );
        assert!(is_entity_not_yet_provisioned(&err));
    }

    #[test]
    fn test_not_provisioned_no_bucket() {
        let err = make_company_error(
            Some("newco"),
            "(company)",
            "Entity cmp_01ABC (newco) has no bucket provisioned. Run VLT-2 bucket provisioning first.",
        );
        assert!(is_entity_not_yet_provisioned(&err));
    }

    #[test]
    fn test_not_provisioned_case_insensitive() {
        // Both "entity" and "not found" must be present; case-insensitive.
        let err = make_company_error(Some("acme"), "(company)", "Entity cmp_XYZ NOT FOUND");
        assert!(is_entity_not_yet_provisioned(&err));
    }

    #[test]
    fn test_not_provisioned_generic_not_found_excluded() {
        // "not found" without "entity" must NOT match — protects against auth
        // errors like "Token not found" or "Session not found".
        let err = make_company_error(Some("acme"), "(company)", "Token not found");
        assert!(!is_entity_not_yet_provisioned(&err));
    }

    #[test]
    fn test_not_provisioned_file_level_error_excluded() {
        // File-level errors on real paths must not be swallowed.
        let err = make_company_error(
            Some("acme"),
            "docs/secret.md",
            "not found",
        );
        assert!(!is_entity_not_yet_provisioned(&err));
    }

    #[test]
    fn test_not_provisioned_different_company_error_not_matched() {
        // A real per-company failure (e.g. STS 500) must surface as an error.
        let err = make_company_error(
            Some("acme"),
            "(company)",
            "STS vend failed for cmp_01ABC: 500 Internal Server Error",
        );
        assert!(!is_entity_not_yet_provisioned(&err));
    }

    #[test]
    fn test_not_provisioned_discovery_error_still_matches_predicate() {
        // The predicate checks only path + message; it has no knowledge of company.
        // A None-company error can still match the predicate — the caller
        // (classify_error_event) is responsible for the None guard.
        let err = make_company_error(
            None,
            "(company)",
            "Failed to fetch entity cmp_01ABC: 404 company/entity not found",
        );
        assert!(is_entity_not_yet_provisioned(&err));
    }

    // ── classify_error_event ─────────────────────────────────────────────────

    #[test]
    fn test_classify_error_event_not_provisioned_returns_complete() {
        // Entity 404: must convert to a zero-files SyncCompleteEvent.
        let err = make_company_error(
            Some("acme"),
            "(company)",
            "Failed to fetch entity cmp_01ABC: 404 company/entity not found",
        );
        let result = classify_error_event(&err);
        assert!(result.is_some());
        let complete = result.unwrap();
        assert_eq!(complete.company, "acme");
        assert_eq!(complete.files_downloaded, 0);
        assert_eq!(complete.bytes_downloaded, 0);
        assert_eq!(complete.files_skipped, 0);
        assert_eq!(complete.conflicts, 0);
        assert!(!complete.aborted);
    }

    #[test]
    fn test_classify_error_event_none_company_passes_through() {
        // Discovery-phase error (no company): must NOT be converted — return None.
        let err = make_company_error(
            None,
            "(company)",
            "Failed to fetch entity cmp_01ABC: 404 company/entity not found",
        );
        assert!(classify_error_event(&err).is_none());
    }

    #[test]
    fn test_classify_error_event_real_error_passes_through() {
        // A real per-company failure (STS 500): must NOT be converted — return None.
        let err = make_company_error(
            Some("acme"),
            "(company)",
            "STS vend failed for cmp_01ABC: 500 Internal Server Error",
        );
        assert!(classify_error_event(&err).is_none());
    }

    #[test]
    fn test_classify_error_event_no_bucket_returns_complete() {
        // "no bucket provisioned" path also converts correctly.
        let err = make_company_error(
            Some("newco"),
            "(company)",
            "Entity cmp_01ABC (newco) has no bucket provisioned. Run VLT-2 bucket provisioning first.",
        );
        let result = classify_error_event(&err);
        assert!(result.is_some());
        assert_eq!(result.unwrap().company, "newco");
    }
}
