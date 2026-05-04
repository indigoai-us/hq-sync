//! First-push: shell out to `hq sync push --creds-from-stdin --json` to
//! upload every local file under a company folder to S3 after provisioning.
//!
//! ## Why a subprocess and not direct S3 calls
//!
//! Before Option C3 of the cloud-promote consolidation, this file held a
//! 719-line independent S3 upload implementation (WalkDir + journal + retry
//! + per-file PUT). That duplicated `share()` from `@indigoai-us/hq-cloud`
//! line-for-line — every bug fix had to land in both places, and the two
//! implementations had subtly different ignore rules and conflict semantics.
//!
//! After C3, the canonical upload path is `hq sync push` (which uses
//! `share()` under the hood). AppBar still owns:
//!
//! * **STS-vending via `/sts/vend-child`** — preserves task-scoped audit
//!   traceability (`task_id` + `task_description` + `task_scope`) that the
//!   simpler `/sts/vend` used by `share()`'s default Cognito path doesn't
//!   carry. Two STS endpoints in production by design — the upload path is
//!   consolidated, the credential-vending path stays differentiated.
//! * **Tauri event emission** — the menubar UI subscribes to per-file
//!   progress and a terminal complete event. We translate from the CLI's
//!   stderr JSONL stream (`--json`) into these Tauri events 1:1.
//!
//! See `workspace/reports/cloud-promote-architecture-2026-04-27.md` and
//! the C3 PR description in `repos/private/hq-sync` for the full rationale.
//!
//! ## Subprocess contract
//!
//! Argv:
//!
//! ```text
//! hq sync push --creds-from-stdin --json --company <slug> --hq-root <path> <company_dir>
//! ```
//!
//! Stdin: a single JSON document conforming to `@indigoai-us/hq-cloud`'s
//! `EntityContext` shape (camelCase keys):
//!
//! ```json
//! {
//!   "uid": "cmp_...",
//!   "slug": "...",
//!   "bucketName": "hq-vault-...",
//!   "region": "us-east-1",
//!   "credentials": {
//!     "accessKeyId": "...",
//!     "secretAccessKey": "...",
//!     "sessionToken": "..."
//!   },
//!   "expiresAt": "2026-..."
//! }
//! ```
//!
//! Stderr (JSON Lines, one record per line):
//!
//! * `{"type":"plan", "filesToUpload": N, "bytesToUpload": N, ...}` — once at start
//! * `{"type":"progress", "path": "...", "bytes": N, "message"?: "..."}` — per uploaded file
//! * `{"type":"conflict", "path": "...", "direction":"push", "resolution": "..."}`
//! * `{"type":"error", "path": "...", "message": "..."}`
//! * `{"type":"complete", "filesUploaded": N, "bytesUploaded": N, "filesSkipped": N, "conflictPaths": [...], "aborted": bool}` — once at end
//! * `{"type":"fatal", "message": "..."}` — on terminal failure (instead of aborting silently)
//!
//! Exit codes:
//!
//! * `0` — success; `complete` event has been emitted with final counts
//! * `1` — terminal failure; `fatal` event sent to stderr first, OR an
//!   `aborted` complete event was emitted (conflict-strategy abort)
//!
//! ## Why we still vend ourselves vs. letting share() vend
//!
//! AppBar already has the STS infrastructure (`vend_child`, task scoping).
//! Switching to share()'s internal `/sts/vend` would silently drop the
//! task-scoped audit metadata. The `/sts/vend-child` endpoint exists
//! specifically for callers that want explicit task tracing, and AppBar
//! is exactly that caller.

use std::path::Path;
use std::process::Stdio;

use serde::{Deserialize, Serialize};
use tauri::Emitter;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

use crate::commands::provision::ProvisionedCompany;
use crate::commands::vault_client::{TaskScope, VaultClient, VendChildInput};
use crate::events::{
    SyncCompanyFirstPushCompleteEvent, SyncCompanyFirstPushProgressEvent,
    EVENT_SYNC_COMPANY_FIRST_PUSH_COMPLETE, EVENT_SYNC_COMPANY_FIRST_PUSH_PROGRESS,
};
use crate::util::hq_resolver::{self, HqInvocation};
use crate::util::logfile::log;
use crate::util::paths;

// ── Subprocess wire format ────────────────────────────────────────────────────

/// Mirror of `@indigoai-us/hq-cloud`'s `EntityContext` type. We construct
/// this from a `vend_child` result and serialize to JSON for the CLI's
/// `--creds-from-stdin` flag. Field names are camelCase to match the JS side.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct EntityContextPayload {
    uid: String,
    slug: String,
    bucket_name: String,
    region: String,
    credentials: EntityCredentials,
    expires_at: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct EntityCredentials {
    access_key_id: String,
    secret_access_key: String,
    session_token: String,
}

/// One line from the CLI's `--json` stderr stream. We don't strictly model
/// every variant — the parent only cares about `progress`, `complete`, and
/// `fatal`. Other types (`plan`, `conflict`, `error`, future additions) are
/// tolerated and logged but don't drive UI state. Forward-compatibility
/// matters because the CLI ships independently and may add event types.
///
/// `#[serde(flatten)]` on `rest` captures every non-`type` field as a raw
/// JSON map, which we reach into for the specific fields we care about
/// without fragmenting this type into a per-variant enum.
#[derive(Debug, Clone, Deserialize)]
struct CliEvent {
    #[serde(rename = "type")]
    event_type: String,
    #[serde(flatten)]
    rest: serde_json::Map<String, serde_json::Value>,
}

// ── Public entry point ────────────────────────────────────────────────────────

/// Run an initial push for `company`: vend STS creds → spawn `hq sync push` →
/// re-emit per-file progress + final-complete events through Tauri so the
/// menubar UI sees the same stream it did under the pre-C3 implementation.
///
/// On success, emits `EVENT_SYNC_COMPANY_FIRST_PUSH_COMPLETE` with final
/// upload/skip counts and returns `Ok(())`. On failure (subprocess crash,
/// non-zero exit, or `fatal` event), returns `Err(message)`; the caller
/// (`sync.rs::run_sync_now`) is responsible for surfacing that to the UI.
pub async fn first_push_company(
    app: &tauri::AppHandle,
    vault: &VaultClient,
    hq_root: &Path,
    company: &ProvisionedCompany,
) -> Result<(), String> {
    // Step 1: Vend STS creds via /sts/vend-child. UNCHANGED from the pre-C3
    // implementation — preserves task-scoped audit (task_id + description +
    // scope) that share()'s simpler /sts/vend doesn't carry. 15-min TTL is
    // well above typical first-push runtime so the subprocess never has to
    // worry about refresh; share() with a pre-vended context does NOT
    // attempt to refresh (no Cognito token to re-vend with).
    let vend_result = vault
        .vend_child(&VendChildInput {
            company_uid: company.uid.clone(),
            task_id: ulid::Ulid::new().to_string(),
            task_description: "hq-sync first-push".to_string(),
            task_scope: TaskScope {
                allowed_prefixes: vec!["".to_string()],
                allowed_actions: Some(vec!["read".to_string(), "write".to_string()]),
            },
            duration_seconds: Some(900),
        })
        .await
        .map_err(|e| format!("vend_child for {}: {e}", company.uid))?;

    // Step 2: Build the EntityContext payload that share() consumes via
    // --creds-from-stdin. Region is hard-coded to us-east-1 for the same
    // reason the pre-C3 build_s3_client did: the vault Lambda always
    // provisions buckets there today. Multi-region would need a region
    // field on ProvisionedCompany (or a vend_child response field) and
    // careful wiring through both AppBar and share().
    let payload = EntityContextPayload {
        uid: company.uid.clone(),
        slug: company.slug.clone(),
        bucket_name: company.bucket_name.clone(),
        region: "us-east-1".to_string(),
        credentials: EntityCredentials {
            access_key_id: vend_result.credentials.access_key_id,
            secret_access_key: vend_result.credentials.secret_access_key,
            session_token: vend_result.credentials.session_token,
        },
        expires_at: vend_result.expires_at,
    };
    let payload_json = serde_json::to_string(&payload)
        .map_err(|e| format!("serialize EntityContext: {e}"))?;

    // Step 3: Spawn `hq sync push --creds-from-stdin --json ...`.
    //
    // `hq_resolver::resolve_hq()` decides whether to invoke a local `hq`
    // binary or fall back to `npx -y --package=@indigoai-us/hq-cli@<range>
    // hq` (range pinned by `HQ_CLI_NPM_RANGE`) based on a one-time
    // capability probe (looks for the --creds-from-stdin flag in
    // `hq sync push --help`). This makes the subprocess self-healing when
    // the user's local `hq` is missing or older than the floor — the
    // contract still works, just with a one-time npx cold-start cost.
    let invocation: HqInvocation = hq_resolver::resolve_hq();
    let path_env = paths::child_path();
    let company_dir = hq_root.join("companies").join(&company.slug);

    log(
        "first-push-cli",
        &format!(
            "spawn ({}): hq sync push --creds-from-stdin --json --company {} --hq-root {} {}",
            invocation.label(),
            company.slug,
            hq_root.display(),
            company_dir.display(),
        ),
    );

    let mut cmd = invocation.command();
    cmd.arg("sync")
        .arg("push")
        .arg("--creds-from-stdin")
        .arg("--json")
        .arg("--company")
        .arg(&company.slug)
        .arg("--hq-root")
        .arg(hq_root.as_os_str())
        .arg(company_dir.as_os_str())
        .env("PATH", &path_env)
        .stdin(Stdio::piped())
        // share()'s default human output goes to stdout — in --json mode all
        // events go to stderr, and stdout carries nothing useful. Discarding
        // it avoids burning a kernel buffer on output we'd ignore anyway.
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        // Without kill_on_drop a panic / cancellation in the caller would
        // leak an orphan `hq` subprocess — the user has no UI to see or
        // kill it. Same posture as run_cli_provision.
        .kill_on_drop(true);

    let mut child = cmd
        .spawn()
        .map_err(|e| format!("spawn `hq sync push` ({}): {e}", invocation.label()))?;

    // Step 4: Pipe payload JSON to the child's stdin, then close stdin so
    // the CLI's `for await (chunk of process.stdin)` loop terminates and
    // the credentials are parsed.
    {
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| "child stdin pipe missing".to_string())?;
        stdin
            .write_all(payload_json.as_bytes())
            .await
            .map_err(|e| format!("write child stdin: {e}"))?;
        stdin.flush().await.ok();
        // dropped here → close
    }

    // Step 5: Stream stderr line-by-line. Each line is either:
    //   * a JSON event (parse + dispatch to Tauri events)
    //   * free-form text (log to diagnostic file, ignore for UI)
    //
    // We read sequentially before calling wait() because there's only one
    // pipe to drain (stdout is /dev/null). Once stderr closes (child exits)
    // next_line() returns None and we fall through to wait().
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| "child stderr pipe missing".to_string())?;
    let mut reader = BufReader::new(stderr).lines();

    let mut total_files: usize = 0;
    let mut files_done: usize = 0;
    let mut files_uploaded: usize = 0;
    let mut files_skipped: usize = 0;
    let mut last_fatal: Option<String> = None;
    let mut saw_complete = false;
    let mut aborted = false;

    while let Ok(Some(line)) = reader.next_line().await {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        // Tolerate non-JSON lines (e.g. shell warnings, accidental println
        // from a future CLI version) — log and continue rather than killing
        // the stream.
        let event: CliEvent = match serde_json::from_str(trimmed) {
            Ok(e) => e,
            Err(_) => {
                log("first-push-cli", &format!("(non-json) {trimmed}"));
                continue;
            }
        };

        match event.event_type.as_str() {
            "plan" => {
                total_files = event
                    .rest
                    .get("filesToUpload")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0) as usize;
                log(
                    "first-push-cli",
                    &format!("plan: filesToUpload={total_files}"),
                );
            }
            "progress" => {
                files_done += 1;
                let path = event
                    .rest
                    .get("path")
                    .and_then(|v| v.as_str())
                    .map(String::from);
                let _ = app.emit(
                    EVENT_SYNC_COMPANY_FIRST_PUSH_PROGRESS,
                    SyncCompanyFirstPushProgressEvent {
                        company_uid: company.uid.clone(),
                        company_slug: company.slug.clone(),
                        files_done,
                        files_total: total_files,
                        current_file: path,
                    },
                );
            }
            "complete" => {
                files_uploaded = event
                    .rest
                    .get("filesUploaded")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0) as usize;
                files_skipped = event
                    .rest
                    .get("filesSkipped")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0) as usize;
                aborted = event
                    .rest
                    .get("aborted")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                saw_complete = true;
                log(
                    "first-push-cli",
                    &format!(
                        "complete: uploaded={files_uploaded} skipped={files_skipped} aborted={aborted}"
                    ),
                );
            }
            "fatal" => {
                let msg = event
                    .rest
                    .get("message")
                    .and_then(|v| v.as_str())
                    .unwrap_or("(no message)")
                    .to_string();
                log("first-push-cli", &format!("fatal: {msg}"));
                last_fatal = Some(msg);
            }
            // `error` is per-file (already-retried, then skipped); `conflict`
            // is per-file (already resolved). Neither kills the run — log
            // for forensics and let the loop continue.
            other => {
                log(
                    "first-push-cli",
                    &format!("event type={other}: {trimmed}"),
                );
            }
        }
    }

    // Step 6: Wait for exit and reconcile.
    let status = child
        .wait()
        .await
        .map_err(|e| format!("wait child: {e}"))?;

    log(
        "first-push-cli",
        &format!(
            "exit code={:?}, saw_complete={saw_complete}, aborted={aborted}, slug={}",
            status.code(),
            company.slug,
        ),
    );

    if !status.success() {
        let msg = last_fatal.unwrap_or_else(|| {
            format!(
                "hq sync push exited with status {} for slug={}",
                status.code().unwrap_or(-1),
                company.slug,
            )
        });
        return Err(msg);
    }

    if aborted {
        return Err(format!(
            "hq sync push aborted for slug={} (uploaded={files_uploaded}, skipped={files_skipped})",
            company.slug,
        ));
    }

    if !saw_complete {
        // Process exited 0 without emitting a `complete` event. That
        // shouldn't happen with the current CLI but is plausible if a
        // future CLI version crashes after share() returns. Surface as
        // an error rather than silently emitting a complete event with
        // (0, 0) counts that would mislead the UI.
        return Err(format!(
            "hq sync push exited 0 without `complete` event for slug={}",
            company.slug,
        ));
    }

    // Emit the terminal Tauri event the menubar listens for.
    let _ = app.emit(
        EVENT_SYNC_COMPANY_FIRST_PUSH_COMPLETE,
        SyncCompanyFirstPushCompleteEvent {
            company_uid: company.uid.clone(),
            company_slug: company.slug.clone(),
            files_uploaded,
            files_skipped,
        },
    );

    Ok(())
}

// ── Tests ─────────────────────────────────────────────────────────────────────
//
// Pre-C3 this file shipped ~378 lines of tests covering the upload mechanics
// (retry-on-transient, journal idempotency, IgnoreFilter integration,
// oversize skip). All of that logic moved to `share()` in
// `@indigoai-us/hq-cloud` and is tested there (see
// `repos/public/hq/packages/hq-cloud/src/cli/share.test.ts`).
//
// What's left to test in this file is narrow: the wrapper logic — the JSON
// shape we send on stdin (cross-language contract) and the JSON shape we
// expect on stderr (forward-compat against future CLI versions).

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// The EntityContext payload must serialize with camelCase keys to
    /// match `@indigoai-us/hq-cloud`'s `EntityContext` interface — that's
    /// the cross-language contract the CLI relies on. If this test fails,
    /// `share()` will reject the stdin JSON and first-push will break.
    #[test]
    fn entity_context_serializes_camel_case() {
        let payload = EntityContextPayload {
            uid: "cmp_01H".to_string(),
            slug: "acme".to_string(),
            bucket_name: "hq-vault-cmp-01h".to_string(),
            region: "us-east-1".to_string(),
            credentials: EntityCredentials {
                access_key_id: "ASIA".to_string(),
                secret_access_key: "secret".to_string(),
                session_token: "token".to_string(),
            },
            expires_at: "2026-04-27T22:00:00Z".to_string(),
        };
        let json_str = serde_json::to_string(&payload).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();

        // Top-level keys: every snake_case field must serialize to camelCase.
        assert_eq!(parsed["uid"], "cmp_01H");
        assert_eq!(parsed["slug"], "acme");
        assert_eq!(parsed["bucketName"], "hq-vault-cmp-01h");
        assert_eq!(parsed["region"], "us-east-1");
        assert_eq!(parsed["expiresAt"], "2026-04-27T22:00:00Z");
        // Nested credentials: same rule.
        assert_eq!(parsed["credentials"]["accessKeyId"], "ASIA");
        assert_eq!(parsed["credentials"]["secretAccessKey"], "secret");
        assert_eq!(parsed["credentials"]["sessionToken"], "token");
        // Anti-test: the snake_case keys must NOT be present (would mean
        // serde rename_all isn't actually applied).
        assert!(parsed.get("bucket_name").is_none());
        assert!(parsed["credentials"].get("access_key_id").is_none());
    }

    /// `complete` event roundtrips into our `CliEvent` decoder and yields
    /// the fields we read (filesUploaded, filesSkipped, aborted). Locks the
    /// CLI ↔ Rust contract for the only event that determines success.
    #[test]
    fn cli_complete_event_round_trip() {
        let line = json!({
            "type": "complete",
            "filesUploaded": 42,
            "bytesUploaded": 12345,
            "filesSkipped": 7,
            "conflictPaths": ["a.md", "b.md"],
            "aborted": false
        })
        .to_string();
        let event: CliEvent = serde_json::from_str(&line).unwrap();
        assert_eq!(event.event_type, "complete");
        assert_eq!(event.rest.get("filesUploaded").and_then(|v| v.as_u64()), Some(42));
        assert_eq!(event.rest.get("filesSkipped").and_then(|v| v.as_u64()), Some(7));
        assert_eq!(event.rest.get("aborted").and_then(|v| v.as_bool()), Some(false));
    }

    /// `progress` event carries the `path` we surface to the UI as
    /// `current_file`. Locks the field name (`path`, not `file` or `key`).
    #[test]
    fn cli_progress_event_round_trip() {
        let line = json!({
            "type": "progress",
            "path": "knowledge/readme.md",
            "bytes": 2048
        })
        .to_string();
        let event: CliEvent = serde_json::from_str(&line).unwrap();
        assert_eq!(event.event_type, "progress");
        assert_eq!(
            event.rest.get("path").and_then(|v| v.as_str()),
            Some("knowledge/readme.md"),
        );
    }

    /// `fatal` carries a `message` we surface as the Err string when the
    /// subprocess exits non-zero. Locks the field name.
    #[test]
    fn cli_fatal_event_round_trip() {
        let line = json!({
            "type": "fatal",
            "message": "vault auth expired mid-run"
        })
        .to_string();
        let event: CliEvent = serde_json::from_str(&line).unwrap();
        assert_eq!(event.event_type, "fatal");
        assert_eq!(
            event.rest.get("message").and_then(|v| v.as_str()),
            Some("vault auth expired mid-run"),
        );
    }

    /// Forward-compat: an unknown event type must parse cleanly so the
    /// stream-reading loop can log-and-continue instead of crashing.
    /// Without this, a future CLI version that adds (say) `{"type":"warn"}`
    /// would silently break first-push for users on older AppBar builds.
    #[test]
    fn cli_unknown_event_type_parses_cleanly() {
        let line = json!({
            "type": "warn",
            "code": "FUTURE_FEATURE",
            "message": "hq-cli vN+1 emits a new event"
        })
        .to_string();
        let event: CliEvent = serde_json::from_str(&line).unwrap();
        assert_eq!(event.event_type, "warn");
        // The unknown fields are captured in `rest` for logging.
        assert!(event.rest.contains_key("code"));
        assert!(event.rest.contains_key("message"));
    }
}
