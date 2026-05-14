//! Shell out to `hq cloud provision company <slug>` — the canonical cloud-
//! promotion subcommand that lives in `@indigoai-us/hq-cli`.
//!
//! The CLI is the single source of truth for: GET-then-POST entity idempotency,
//! atomic `companies/manifest.yaml` patch, atomic `companies/<slug>/.hq/config.json`
//! write, and initial `share()` sync. Both calling paths in this app
//! (`provision::provision_missing_companies` auto-flow and
//! `workspaces::connect_workspace_to_cloud` Connect button) delegate here so
//! the contract stays in one place — see
//! `workspace/reports/cloud-promote-architecture-2026-04-27.md` for the
//! consolidation rationale.
//!
//! ## Subprocess contract
//!
//! Argv:
//!
//! ```text
//! hq cloud provision company <slug> [--name "<name>"]
//! ```
//!
//! Stdout: a single JSON line conforming to `CliProvisionResult`. We capture
//! the entire stream and parse the LAST non-empty line — chalk colour codes
//! and progress chatter from the CLI go to stderr, but if anything ever
//! escapes to stdout before the result line we want to ignore it gracefully.
//!
//! Stderr: free-form progress lines prefixed by the CLI itself (e.g.
//! `[hq cloud provision] validated slug=acme`). We tee every line into
//! the persistent diagnostic log via `util::logfile::log("provision-cli", …)`
//! so a stuck or failed provision leaves breadcrumbs we can grep for.
//!
//! Exit codes:
//!   * `0` — success, JSON has `ok: true`, `initial_sync.ok: true`
//!   * `1` — vault auth/network/API error (no entity provisioned)
//!   * `2` — validation error (bad slug, manifest missing, dir missing, etc.)
//!   * `3` — entity provisioned + manifest patched + config written, but the
//!           initial sync failed. The JSON line on stdout still carries the
//!           `cloud_uid` so retries can resume.
//!
//! ## Why a fresh subprocess and not a library call
//!
//! The CLI lives in a separate npm package (`@indigoai-us/hq-cli`) and
//! depends on `@indigoai-us/hq-cloud` for the `share()` runner. Calling it
//! out-of-process keeps the Tauri/Rust binary free of any Node.js coupling
//! and lets the CLI evolve independently — the only contract we depend on
//! is the JSON shape on stdout and the exit-code mapping above.
//!
//! ## Why we don't fall back to direct vault calls
//!
//! The whole point of this refactor is single-source-of-truth. If the CLI
//! is unavailable (binary missing, npm not installed, etc.) we surface a
//! clear `CliProvisionError::Spawn` to the caller rather than silently
//! re-implementing the flow with `vault_client.rs`. The caller then logs
//! and the user sees the error in Connect diagnostics.
//!
//! `vault_client.rs` is retained for other callers (membership lookups,
//! telemetry, STS vending, etc.) — only the cloud-promote callers were
//! migrated.

use std::collections::VecDeque;
use std::path::Path;
use std::process::Stdio;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, BufReader};

use crate::util::hq_resolver::{self, HqInvocation};
use crate::util::logfile::log;
use crate::util::paths;

/// Last N stderr lines kept in memory so we can attach them to Sentry events.
/// Capped to keep payloads under Sentry's per-event size limits.
const STDERR_TAIL_CAP: usize = 50;

/// Capture a provision-cli failure to Sentry with full diagnostic context.
/// Tags carry the slug + CLI invocation kind + exit code so we can slice
/// failures by stack/version. Extra carries the stderr tail (the most useful
/// signal — what the CLI was actually doing when it died).
///
/// `local-env` failures (npm cache permission, disk full, npm registry
/// unreachable / timeout) carry an additional `local_env_kind` tag so the
/// existing `provision_kind=network` vault-incident alert rule can be
/// tightened to exclude them — these are user-laptop problems, not platform
/// incidents.
fn report_provision_error(
    err: &CliProvisionError,
    slug: &str,
    invocation_label: &str,
    exit_code: Option<i32>,
    stderr_tail: &[String],
) {
    let kind = match err {
        CliProvisionError::Spawn(_) => "spawn",
        CliProvisionError::Validation(_) => "validation",
        CliProvisionError::Network(_) => "network",
        CliProvisionError::LocalEnv { .. } => "local-env",
        CliProvisionError::Sync { .. } => "sync",
        CliProvisionError::Other(_) => "other",
    };
    let local_env_kind: Option<&str> = match err {
        CliProvisionError::LocalEnv { kind, .. } => Some(kind),
        _ => None,
    };
    let stderr_blob = stderr_tail.join("\n");
    let exit_str = exit_code
        .map(|c| c.to_string())
        .unwrap_or_else(|| "signal/none".to_string());

    sentry::with_scope(
        |scope| {
            scope.set_tag("slug", slug);
            scope.set_tag("provision_kind", kind);
            if let Some(k) = local_env_kind {
                scope.set_tag("local_env_kind", k);
            }
            scope.set_tag("cli_invocation", invocation_label);
            scope.set_tag("exit_code", &exit_str);
            scope.set_extra("stderr_tail", stderr_blob.into());
        },
        || {
            sentry::capture_message(
                &format!("[provision-cli] {err}"),
                sentry::Level::Error,
            );
        },
    );
}

/// Inspect the captured stderr tail for unambiguous local-environment
/// failures and classify them. Returns `(kind, detail)` if matched.
///
/// `kind` is a stable identifier the frontend uses to render a kind-specific
/// "Fix in Claude Code" deep link (see `src/lib/copy-prompts.ts`). The
/// frontend parses the `Display` string
/// (`"local environment failure (<kind>): <detail>"`) to extract this, so
/// adding a new kind requires no IPC schema changes — just a new match arm
/// here and a new builder branch on the frontend side.
///
/// The patterns are deliberately narrow — they target failures that happen
/// *before* the CLI is loaded (most commonly when the resolver routes through
/// `npx -y --package=@indigoai-us/hq-cli@<range> hq`), where `npx` returns
/// exit code 1 for reasons that have nothing to do with vault. Mis-bucketing
/// these as `CliProvisionError::Network` produces false-positive
/// `provision_kind=network` Sentry alerts in `#hq-liveops` that look like
/// platform outages but are user-laptop fixes.
fn classify_local_env_failure(stderr_tail: &[String]) -> Option<(&'static str, String)> {
    let blob = stderr_tail.join("\n");

    // npm cache permission — `~/.npm/_cacache/...` owned by root from a prior
    // `sudo npm` run. Remedy is `sudo chown -R $(id -u):$(id -g) ~/.npm`.
    if blob.contains("npm error code EACCES")
        || blob.contains("npm ERR! code EACCES")
        || blob.contains("EACCES: permission denied")
    {
        let detail = first_matching_line(&blob, &["npm error path", "npm ERR! path", "EACCES"])
            .unwrap_or_else(|| "~/.npm cache contains root-owned files".to_string());
        return Some(("npm-cache-permission", detail));
    }

    // Disk full during package extraction.
    if blob.contains("npm error code ENOSPC")
        || blob.contains("npm ERR! code ENOSPC")
        || blob.contains("ENOSPC: no space left")
    {
        return Some((
            "disk-full",
            "no space left on device during npm package install".to_string(),
        ));
    }

    // npm registry DNS failure — captive portal, offline, custom-registry typo.
    if blob.contains("npm error code ENOTFOUND")
        || blob.contains("npm ERR! code ENOTFOUND")
        || blob.contains("getaddrinfo ENOTFOUND")
    {
        let detail = first_matching_line(&blob, &["ENOTFOUND", "getaddrinfo"])
            .unwrap_or_else(|| "could not resolve npm registry host".to_string());
        return Some(("npm-registry-unreachable", detail));
    }

    // npm registry TCP timeout — proxy / slow link / npmjs.org incident.
    if blob.contains("npm error code ETIMEDOUT")
        || blob.contains("npm ERR! code ETIMEDOUT")
        || blob.contains("connect ETIMEDOUT")
        || blob.contains("ESOCKETTIMEDOUT")
    {
        return Some((
            "npm-registry-timeout",
            "npm request to registry timed out".to_string(),
        ));
    }

    None
}

/// Helper for `classify_local_env_failure` — pull a line of the blob that
/// contains one of the needles, trimmed. Needles are tried in order, so the
/// caller can express priority by ordering: e.g. `["npm error path", "EACCES"]`
/// prefers the path line over the bare error-code line. Returns None when no
/// needle matches so the caller can fall back to a generic detail string.
fn first_matching_line(blob: &str, needles: &[&str]) -> Option<String> {
    for n in needles {
        for line in blob.lines() {
            if line.contains(n) {
                return Some(line.trim().to_string());
            }
        }
    }
    None
}

// ── Public types ─────────────────────────────────────────────────────────────

/// Per-step sync result inside `CliProvisionResult`. Mirrors the CLI's
/// `initial_sync` field — `ok: false` means the entity was provisioned but the
/// follow-up `share()` call failed (exit code 3).
///
/// Every field is optional because the CLI's TS interface declares them all
/// optional too: a happy-path run carries `ok` + counts; a failed run carries
/// `ok: false` + `error`; and a `--skip-initial-sync` run (always used by
/// AppBar) carries only `skipped: true`. Treating any of these as required
/// caused serde to reject the skip payload silently and surface as
/// "exit 0 but no JSON line on stdout".
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CliInitialSync {
    #[serde(default)]
    pub ok: Option<bool>,
    #[serde(default)]
    pub files_uploaded: Option<u64>,
    #[serde(default)]
    pub bytes_uploaded: Option<u64>,
    #[serde(default)]
    pub error: Option<String>,
    /// True when the CLI was invoked with `--skip-initial-sync`. AppBar passes
    /// this on every call because it owns its own STS-credentialed upload
    /// pipeline (`first_push_company` + Tauri progress events).
    #[serde(default)]
    pub skipped: Option<bool>,
}

/// Parsed JSON result emitted on stdout by `hq cloud provision company`.
///
/// Field names match the CLI's `ProvisionResult` interface (snake_case JSON,
/// not camelCase) — see
/// `repos/public/hq/packages/hq-cli/src/commands/cloud-provision.ts`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CliProvisionResult {
    pub ok: bool,
    pub company_slug: String,
    pub cloud_uid: String,
    pub bucket_name: String,
    pub vault_api_url: String,
    /// Some entities have no KMS key — never assume non-null.
    #[serde(default)]
    pub kms_key_id: Option<String>,
    pub created_entity: bool,
    pub manifest_patched: bool,
    pub config_written: bool,
    pub initial_sync: CliInitialSync,
}

/// Typed error surface for `run_cli_provision`. Mapped from the CLI's
/// documented exit codes (1=vault, 2=validation, 3=sync) plus our local
/// failure modes (spawn / IO / non-JSON output).
#[derive(Debug)]
pub enum CliProvisionError {
    /// Failed to spawn `hq` — binary not on PATH or exec error. The user is
    /// missing the CLI; surface a clear "install hq" message.
    Spawn(String),
    /// Exit code 2 — bad slug, missing manifest entry, archived company, etc.
    /// Caller should NOT retry; the user must fix the input.
    Validation(String),
    /// Exit code 1 — vault HTTP / network / auth failure. Retryable.
    Network(String),
    /// Exit code 1 *before* the CLI even started — `npx` / `npm` failed
    /// during the pre-launch package fetch. These look identical to vault
    /// failures from the exit code alone, but the captured `stderr_tail`
    /// carries an unambiguous npm-error blob. Surfaced as a distinct variant
    /// so Sentry routing + the popover can offer kind-specific remediation
    /// instead of a generic "vault error" message.
    ///
    /// `kind` is one of: `"npm-cache-permission"`, `"disk-full"`,
    /// `"npm-registry-unreachable"`, `"npm-registry-timeout"` — and these
    /// strings are part of the IPC contract with the frontend's
    /// `OpenInClaudeCodeButton` / `copy-prompts.ts` registry.
    LocalEnv { kind: &'static str, detail: String },
    /// Exit code 3 — entity created, manifest patched, config written, but
    /// the initial `share()` upload failed. The CLI's stdout still emits a
    /// `CliProvisionResult` with `cloud_uid` populated, so callers can
    /// retry the sync separately. We carry the partial result for them.
    Sync {
        message: String,
        partial: Option<CliProvisionResult>,
    },
    /// Anything we can't classify — non-zero exit code outside [1,2,3], or
    /// stdout that didn't contain a parseable JSON line, or IO mid-stream.
    Other(String),
}

impl std::fmt::Display for CliProvisionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Spawn(m) => write!(f, "spawn `hq` failed: {m}"),
            Self::Validation(m) => write!(f, "validation error from `hq cloud provision`: {m}"),
            Self::Network(m) => write!(f, "vault/network error from `hq cloud provision`: {m}"),
            // The exact prefix `"local environment failure (<kind>): "` is
            // part of the IPC contract — the frontend regex-parses the kind
            // out of this string to render the right "Fix in Claude Code"
            // button. Don't reword it without updating
            // `src/lib/copy-prompts.ts::parseLocalEnvFailure`.
            Self::LocalEnv { kind, detail } => {
                write!(f, "local environment failure ({kind}): {detail}")
            }
            Self::Sync { message, .. } => {
                write!(f, "initial sync failed after entity provisioned: {message}")
            }
            Self::Other(m) => write!(f, "`hq cloud provision` failed: {m}"),
        }
    }
}

impl std::error::Error for CliProvisionError {}

impl From<CliProvisionError> for String {
    fn from(e: CliProvisionError) -> String {
        e.to_string()
    }
}

// ── Public entry point ───────────────────────────────────────────────────────

/// Spawn `hq cloud provision company <slug> [--name <name>] --hq-root <root>`
/// and parse the JSON result.
///
/// * `slug` — company slug (must match a top-level key under `.companies` in
///   `companies/manifest.yaml`). The CLI rejects `"personal"` itself.
/// * `display_name` — optional human-readable name forwarded as `--name`.
///   Falls back to the CLI's default (the slug) when None.
/// * `hq_root` — absolute path to the user's HQ folder. Forwarded as
///   `--hq-root <path>` AND set as the subprocess `current_dir`. Without
///   this the CLI defaults `--hq-root` to `~/hq` and bails with
///   `companies/manifest.yaml not found at /Users/<u>/hq/companies/manifest.yaml`
///   for any user whose HQ folder isn't at the lowercase default — exit 2
///   silently propagates back to the menubar with no UI feedback.
///
/// Stderr lines are tee'd into `~/.hq/logs/hq-sync.log` under the
/// `provision-cli` tag so a hung or failed provision leaves a trail.
///
/// On success the parsed `CliProvisionResult` is returned with
/// `result.ok == true` and `result.initial_sync.ok == true`. Exit code 3
/// (initial-sync failure after entity creation) is surfaced as
/// `CliProvisionError::Sync` carrying the partial result so the caller can
/// still record the `cloud_uid` and let the user retry the sync separately.
pub async fn run_cli_provision(
    slug: &str,
    display_name: Option<&str>,
    hq_root: &Path,
) -> Result<CliProvisionResult, CliProvisionError> {
    // `hq_resolver::resolve_hq()` self-heals when the user's local `hq`
    // is missing or older than the pinned floor (HQ_CLI_NPM_RANGE) by
    // routing through `npx -y --package=@indigoai-us/hq-cli@<range> hq`.
    // The capability probe is shared with first_push and cached for the
    // AppBar process lifetime, so this call is free after the first
    // invocation.
    //
    // The pinned range covers the cloud-provision flags this command needs
    // (`--skip-initial-sync` shipped in 5.6.1, `cloud provision company`
    // shipped in 5.6.0), so the resolver's choice is safe here.
    let invocation: HqInvocation = hq_resolver::resolve_hq();
    let path_env = paths::child_path();

    log(
        "provision-cli",
        &format!(
            "spawn ({}): hq cloud provision company {slug} --hq-root {}{}",
            invocation.label(),
            hq_root.display(),
            display_name
                .map(|n| format!(" --name {n:?}"))
                .unwrap_or_default()
        ),
    );

    let mut cmd = invocation.command();
    cmd.arg("cloud")
        .arg("provision")
        .arg("company")
        .arg(slug)
        // AppBar always opts out of the CLI's post-provision share() — our own
        // first_push_company runs with STS-vended per-company creds + Tauri
        // progress events. Pre-C3 this comment said "would otherwise upload
        // twice"; post-C3 we still want to keep this flag so the CLI doesn't
        // perform a Cognito-credentialed upload before AppBar's vend-child
        // upload runs (the two would race and produce different journal
        // states).
        .arg("--skip-initial-sync")
        // Pass the resolved HQ folder explicitly. The CLI defaults
        // `--hq-root` to `~/hq` (lowercase) — if the user's HQ folder is
        // anywhere else (e.g. `~/Documents/HQ`), the CLI exits 2 with
        // "companies/manifest.yaml not found at ..." and the menubar shows
        // nothing. `current_dir` is set to the same path as belt-and-
        // suspenders for any future code path that reads cwd-relative.
        .arg("--hq-root")
        .arg(hq_root)
        .current_dir(hq_root)
        .env("PATH", &path_env)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    if let Some(name) = display_name {
        cmd.arg("--name").arg(name);
    }

    // `kill_on_drop` ensures a panic / cancellation in the caller doesn't
    // leave an orphaned `hq` subprocess — we'd rather lose progress than
    // leak processes the user has no UI to kill.
    cmd.kill_on_drop(true);

    let invocation_label = invocation.label();
    let mut child = cmd.spawn().map_err(|e| {
        let err = CliProvisionError::Spawn(format!("{invocation_label}: {e}"));
        report_provision_error(&err, slug, &invocation_label, None, &[]);
        err
    })?;

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| CliProvisionError::Other("child stdout pipe missing".to_string()))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| CliProvisionError::Other("child stderr pipe missing".to_string()))?;

    // Stream stderr line-by-line into the diagnostic log AND a bounded
    // ring buffer. The ring buffer survives past the subprocess and gets
    // attached to Sentry events on error so we can see what the CLI was
    // doing right before it died.
    let stderr_buffer: Arc<Mutex<VecDeque<String>>> =
        Arc::new(Mutex::new(VecDeque::with_capacity(STDERR_TAIL_CAP)));
    let stderr_buffer_for_task = Arc::clone(&stderr_buffer);
    let stderr_task = tokio::spawn(async move {
        let mut reader = BufReader::new(stderr).lines();
        while let Ok(Some(line)) = reader.next_line().await {
            log("provision-cli", &line);
            if let Ok(mut buf) = stderr_buffer_for_task.lock() {
                if buf.len() == STDERR_TAIL_CAP {
                    buf.pop_front();
                }
                buf.push_back(line);
            }
        }
    });

    // Stream stdout line-by-line into a buffer. Final result is the last
    // non-empty line — anything else (warnings, accidental println) is
    // tolerated rather than treated as a parse failure.
    let stdout_task = tokio::spawn(async move {
        let mut reader = BufReader::new(stdout).lines();
        let mut lines: Vec<String> = Vec::new();
        while let Ok(Some(line)) = reader.next_line().await {
            lines.push(line);
        }
        lines
    });

    let status = child
        .wait()
        .await
        .map_err(|e| CliProvisionError::Other(format!("wait child: {e}")))?;

    // Drain both readers — these complete once the child closes the pipes,
    // which happens on exit. `wait()` already returned, so they should be
    // ready immediately.
    let lines = stdout_task
        .await
        .map_err(|e| CliProvisionError::Other(format!("stdout reader join: {e}")))?;
    if let Err(e) = stderr_task.await {
        // Stderr reader join failure shouldn't fail the call — we still have
        // a status code and stdout. Log and continue.
        log(
            "provision-cli",
            &format!("stderr reader task join failed (non-fatal): {e}"),
        );
    }

    let last_json_line = lines
        .iter()
        .rev()
        .find(|l| !l.trim().is_empty())
        .map(|s| s.as_str());

    let parse_result: Option<Result<CliProvisionResult, serde_json::Error>> =
        last_json_line.map(serde_json::from_str::<CliProvisionResult>);
    let parsed: Option<CliProvisionResult> = parse_result
        .as_ref()
        .and_then(|r| r.as_ref().ok())
        .cloned();

    let exit_code = status.code();
    log(
        "provision-cli",
        &format!(
            "exit code={:?}, parsed_json={}, slug={slug}",
            exit_code,
            parsed.is_some(),
        ),
    );

    // Differentiate the failure modes for the exit-0 path so the error message
    // points at the actual cause: missing stdout vs. parse failure. The previous
    // single message ("no JSON line on stdout") falsely accused the CLI when
    // the real culprit was a Rust↔TS schema drift (e.g. CliInitialSync requiring
    // `ok` while the CLI emitted `{ skipped: true }`).
    let exit0_err = || -> CliProvisionError {
        match (last_json_line, parse_result.as_ref()) {
            (None, _) => CliProvisionError::Other(format!(
                "exit 0 but no output on stdout for slug={slug}"
            )),
            (Some(line), Some(Err(e))) => CliProvisionError::Other(format!(
                "exit 0 but stdout JSON failed to parse for slug={slug}: {e} (last_line={line:?})"
            )),
            (Some(_), _) => CliProvisionError::Other(format!(
                "exit 0 but no JSON line on stdout for slug={slug} (last_line={last_json_line:?})"
            )),
        }
    };

    let stderr_tail: Vec<String> = stderr_buffer
        .lock()
        .map(|buf| buf.iter().cloned().collect())
        .unwrap_or_default();

    let result: Result<CliProvisionResult, CliProvisionError> = match exit_code {
        Some(0) => parsed.ok_or_else(exit0_err),
        // Exit 1 is overloaded: the CLI documents it as "vault auth/network",
        // but `npx` (the pre-launch wrapper from `hq_resolver` when the user
        // has no local `hq` or a stale one) also returns 1 for npm cache
        // EACCES, ENOSPC, ENOTFOUND, ETIMEDOUT — failures that happen before
        // the CLI is even loaded. Run the local-env classifier on the stderr
        // tail first; only fall through to `Network` if nothing matched.
        Some(1) => match classify_local_env_failure(&stderr_tail) {
            Some((env_kind, detail)) => Err(CliProvisionError::LocalEnv {
                kind: env_kind,
                detail,
            }),
            None => Err(CliProvisionError::Network(format!(
                "exit 1 (vault) — see ~/.hq/logs/hq-sync.log [provision-cli] for slug={slug}"
            ))),
        },
        Some(2) => Err(CliProvisionError::Validation(format!(
            "exit 2 (validation) — see ~/.hq/logs/hq-sync.log [provision-cli] for slug={slug}"
        ))),
        Some(3) => Err(CliProvisionError::Sync {
            message: format!(
                "exit 3 (initial sync) — entity provisioned but upload failed; see ~/.hq/logs/hq-sync.log for slug={slug}"
            ),
            partial: parsed,
        }),
        Some(other) => Err(CliProvisionError::Other(format!(
            "unexpected exit code {other} for slug={slug}"
        ))),
        None => Err(CliProvisionError::Other(format!(
            "child terminated by signal (no exit code) for slug={slug}"
        ))),
    };

    if let Err(ref err) = result {
        report_provision_error(err, slug, &invocation_label, exit_code, &stderr_tail);
    }
    result
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// The struct must accept the exact JSON shape documented in
    /// `cloud-provision.ts::ProvisionResult`. Locks the CLI ↔ Rust contract.
    #[test]
    fn deserialize_success_payload() {
        let line = json!({
            "ok": true,
            "company_slug": "indigo",
            "cloud_uid": "cmp_01H123",
            "bucket_name": "hq-vault-cmp-01H123",
            "vault_api_url": "https://vault.example.com",
            "kms_key_id": "key-abc",
            "created_entity": true,
            "manifest_patched": true,
            "config_written": true,
            "initial_sync": {
                "ok": true,
                "files_uploaded": 42,
                "bytes_uploaded": 123456
            }
        })
        .to_string();
        let r: CliProvisionResult = serde_json::from_str(&line).unwrap();
        assert!(r.ok);
        assert_eq!(r.cloud_uid, "cmp_01H123");
        assert_eq!(r.bucket_name, "hq-vault-cmp-01H123");
        assert_eq!(r.kms_key_id.as_deref(), Some("key-abc"));
        assert_eq!(r.initial_sync.ok, Some(true));
        assert_eq!(r.initial_sync.files_uploaded, Some(42));
    }

    /// The CLI emits `kms_key_id: null` when the entity has no KMS key —
    /// must round-trip cleanly into Option<String>::None rather than erroring.
    #[test]
    fn deserialize_null_kms_key() {
        let line = json!({
            "ok": true,
            "company_slug": "acme",
            "cloud_uid": "cmp_x",
            "bucket_name": "hq-vault-cmp-x",
            "vault_api_url": "https://v",
            "kms_key_id": null,
            "created_entity": false,
            "manifest_patched": true,
            "config_written": true,
            "initial_sync": { "ok": true }
        })
        .to_string();
        let r: CliProvisionResult = serde_json::from_str(&line).unwrap();
        assert!(r.kms_key_id.is_none());
        assert_eq!(r.initial_sync.files_uploaded, None);
    }

    /// Partial-success payload (exit 3) — `initial_sync.ok: false` with an
    /// error message. Used by callers to record the cloud_uid and skip the
    /// follow-up sync gracefully.
    #[test]
    fn deserialize_exit3_partial_payload() {
        let line = json!({
            "ok": false,
            "company_slug": "acme",
            "cloud_uid": "cmp_partial",
            "bucket_name": "hq-vault-cmp-partial",
            "vault_api_url": "https://v",
            "kms_key_id": null,
            "created_entity": true,
            "manifest_patched": true,
            "config_written": true,
            "initial_sync": { "ok": false, "error": "S3 PutObject failed: timeout" }
        })
        .to_string();
        let r: CliProvisionResult = serde_json::from_str(&line).unwrap();
        assert!(!r.ok);
        assert_eq!(r.cloud_uid, "cmp_partial");
        assert_eq!(r.initial_sync.ok, Some(false));
        assert_eq!(
            r.initial_sync.error.as_deref(),
            Some("S3 PutObject failed: timeout"),
        );
    }

    /// AppBar always invokes the CLI with `--skip-initial-sync`, in which case
    /// the CLI emits `initial_sync: { skipped: true }` (no `ok` field). The
    /// Rust struct used to require `ok: bool`, so this payload silently failed
    /// to deserialize and the caller surfaced "exit 0 but no JSON line on
    /// stdout" — the actual stdout was fine, the parser was wrong. Lock the
    /// contract here so it can't regress.
    #[test]
    fn deserialize_skip_initial_sync_payload() {
        let line = json!({
            "ok": true,
            "company_slug": "bug2-verify",
            "cloud_uid": "cmp_01KQSR92SNH",
            "bucket_name": "hq-vault-cmp-01kqsr92snh21n8nba2r77zaqk",
            "vault_api_url": "https://v",
            "kms_key_id": null,
            "created_entity": true,
            "manifest_patched": true,
            "config_written": true,
            "initial_sync": { "skipped": true }
        })
        .to_string();
        let r: CliProvisionResult = serde_json::from_str(&line).unwrap();
        assert!(r.ok);
        assert_eq!(r.initial_sync.skipped, Some(true));
        assert_eq!(r.initial_sync.ok, None);
        assert_eq!(r.initial_sync.files_uploaded, None);
    }

    #[test]
    fn error_display_smoke() {
        let e = CliProvisionError::Validation("bad slug".to_string());
        assert!(e.to_string().contains("validation"));
        let e = CliProvisionError::Network("503".to_string());
        assert!(e.to_string().contains("network"));
        let e = CliProvisionError::Sync {
            message: "timeout".to_string(),
            partial: None,
        };
        assert!(e.to_string().contains("initial sync"));
    }

    /// The Display string for `LocalEnv` is part of the IPC contract — the
    /// Svelte frontend parses `local environment failure (<kind>): <detail>`
    /// to extract `kind` for the `OpenInClaudeCodeButton`. Locking the prefix
    /// here so a casual reword can't silently break the popover.
    #[test]
    fn local_env_display_contract_is_parseable() {
        let e = CliProvisionError::LocalEnv {
            kind: "npm-cache-permission",
            detail: "~/.npm cache contains root-owned files".to_string(),
        };
        let s = e.to_string();
        assert!(
            s.starts_with("local environment failure ("),
            "Display must start with the parseable prefix; got {s:?}",
        );
        assert!(s.contains("(npm-cache-permission)"));
        assert!(s.contains("root-owned"));
    }

    /// Classifier: npm cache EACCES (the canonical example from the
    /// HQ-SYNC-WEB-E Sentry alert that motivated this code). Must classify
    /// regardless of which npm flavour text is in stderr (modern "npm error"
    /// vs. legacy "npm ERR!" prefix) and must surface the cache path detail
    /// when present so the popover prompt names the right directory.
    #[test]
    fn classify_npm_eacces_modern() {
        let tail: Vec<String> = vec![
            "npm error code EACCES".to_string(),
            "npm error syscall open".to_string(),
            "npm error path /Users/alice/.npm/_cacache/index-v5/aa/bb/foo".to_string(),
            "npm error errno EACCES".to_string(),
        ];
        let (kind, detail) = classify_local_env_failure(&tail).expect("must classify");
        assert_eq!(kind, "npm-cache-permission");
        assert!(
            detail.contains("/Users/alice/.npm"),
            "detail should carry the offending cache path; got {detail:?}",
        );
    }

    #[test]
    fn classify_npm_eacces_legacy() {
        let tail: Vec<String> = vec![
            "npm ERR! code EACCES".to_string(),
            "npm ERR! path /Users/bob/.npm/_cacache/x".to_string(),
        ];
        let (kind, _detail) = classify_local_env_failure(&tail).expect("must classify");
        assert_eq!(kind, "npm-cache-permission");
    }

    #[test]
    fn classify_disk_full() {
        let tail: Vec<String> = vec![
            "npm error code ENOSPC".to_string(),
            "ENOSPC: no space left on device, write".to_string(),
        ];
        let (kind, _detail) = classify_local_env_failure(&tail).expect("must classify");
        assert_eq!(kind, "disk-full");
    }

    #[test]
    fn classify_registry_unreachable() {
        let tail: Vec<String> = vec![
            "npm error code ENOTFOUND".to_string(),
            "npm error errno ENOTFOUND".to_string(),
            "npm error network getaddrinfo ENOTFOUND registry.npmjs.org".to_string(),
        ];
        let (kind, detail) = classify_local_env_failure(&tail).expect("must classify");
        assert_eq!(kind, "npm-registry-unreachable");
        assert!(detail.contains("registry.npmjs.org") || detail.contains("ENOTFOUND"));
    }

    #[test]
    fn classify_registry_timeout() {
        let tail: Vec<String> = vec![
            "npm error code ETIMEDOUT".to_string(),
            "connect ETIMEDOUT 104.16.0.0:443".to_string(),
        ];
        let (kind, _detail) = classify_local_env_failure(&tail).expect("must classify");
        assert_eq!(kind, "npm-registry-timeout");
    }

    /// Vault-flavoured stderr (genuine CLI 5xx) must NOT classify as
    /// local-env — otherwise the alert routing flips the wrong way and real
    /// vault incidents go quiet. This is the inverse of the original bug.
    #[test]
    fn classify_vault_error_returns_none() {
        let tail: Vec<String> = vec![
            "[hq cloud provision] vault POST /v1/entity returned 503".to_string(),
            "Error: vault unreachable after 3 retries".to_string(),
        ];
        assert!(classify_local_env_failure(&tail).is_none());
    }

    /// Empty stderr — defensive: the classifier must never panic on a tail
    /// that's empty or carries only whitespace. Returning None lets the
    /// caller fall back to the generic Network mapping.
    #[test]
    fn classify_empty_stderr_returns_none() {
        assert!(classify_local_env_failure(&[]).is_none());
        let tail: Vec<String> = vec!["".to_string(), "  ".to_string()];
        assert!(classify_local_env_failure(&tail).is_none());
    }

    /// `From<CliProvisionError> for String` lets callers `?`-propagate into
    /// Tauri commands whose error type is `String`. Smoke-test the conversion.
    #[test]
    fn into_string_for_tauri_command() {
        let e = CliProvisionError::Spawn("not on PATH".to_string());
        let s: String = e.into();
        assert!(s.contains("spawn"));
        assert!(s.contains("not on PATH"));
    }
}
