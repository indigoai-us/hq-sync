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
//! **Version selection:** `HQ_CLOUD_VERSION` below is authoritative. It is
//! a tilde-prefixed semver range (`~MAJOR.MINOR.0`) — npx resolves it to
//! the newest published patch in that minor line at spawn time. So
//! patch-only bug fixes ship to users on their next sync without a Rust
//! rebuild, while bumping the minor line (e.g. `~5.19.0` → `~5.20.0`) is
//! the deliberate "ship a new behavior set" lever and still requires an
//! HQ Sync release. See `commands::prewarm` for the on-startup background
//! fetch that keeps first-click-Sync-Now latency near zero after either
//! kind of bump.

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
    SyncAllCompleteEvent, SyncCompanyProvisionedEvent, SyncCompleteEvent, SyncErrorEvent,
    SyncEvent, EVENT_SYNC_ALL_COMPLETE, EVENT_SYNC_AUTH_ERROR, EVENT_SYNC_COMPANY_PROVISIONED,
    EVENT_SYNC_COMPLETE, EVENT_SYNC_DELETE_REFUSED_STALE_ETAG, EVENT_SYNC_ERROR,
    EVENT_SYNC_FANOUT_PLAN, EVENT_SYNC_NEW_FILES, EVENT_SYNC_PLAN, EVENT_SYNC_PROGRESS,
    EVENT_SYNC_SETUP_NEEDED,
};
use crate::util::logfile::log;
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
    /// Set true when the runner emits AllComplete. Used by the Exit handler
    /// to detect "runner exited without ever finishing the protocol" — e.g.
    /// when it bails on `setup-needed` before reaching the fanout — so we
    /// can emit a synthetic AllComplete and unblock the UI from a stuck
    /// "syncing" state.
    pub all_complete_seen: bool,
}

impl RunTotals {
    /// Update totals from a single event. `Complete` events contribute to
    /// counters; `AllComplete` flips the seen-flag. Saturates on overflow.
    pub fn accumulate(&mut self, event: &SyncEvent) {
        match event {
            SyncEvent::Complete(c) => {
                self.conflicts = self.conflicts.saturating_add(c.conflicts);
            }
            SyncEvent::AllComplete(_) => {
                self.all_complete_seen = true;
            }
            _ => {}
        }
    }
}

/// Singleton handle — only one sync at a time.
const SYNC_HANDLE: &str = "hq-sync";

/// Hard timeout for a sync run (1 hour).
const SYNC_TIMEOUT: Duration = Duration::from_secs(3600);

/// SIGKILL delay after SIGTERM on cancel.
const SIGKILL_DELAY: Duration = Duration::from_secs(5);

/// Semver range for `@indigoai-us/hq-cloud` that ships `hq-sync-runner`.
///
/// Format is npm's `package-spec` — a tilde-prefixed minor floor
/// (`~MAJOR.MINOR.0`) selects the *minor line* but lets patches flow
/// automatically: `~5.19.0` resolves to the newest published `5.19.x` at
/// spawn time. Bumping the minor (e.g. to `~5.20.0`) is the deliberate
/// "select a new line" lever; patch-only fixes (5.18.1, 5.18.2, …) ship
/// to users automatically on their next sync without a Rust rebuild.
///
/// npx resolves the range at each spawn (the resolved version becomes
/// the on-disk cache key under `~/.npm/_npx/`), so a freshly published
/// patch causes a single re-fetch then steady-state cache reuse — same
/// shape as an exact-version bump, just driven by the registry instead
/// of source. The `commands::prewarm` task fires this same fetch on app
/// startup so the cost lands in the background rather than during the
/// user's first click of "Sync Now".
///
/// 5.19.x switches the sync runner's slug resolution to the per-user
/// namespace endpoint (`/entity/check-slug/me` → `entity.get(uid)`).
/// On 2026-05-15 hq-pro#67 went live and flipped the legacy global
/// `/entity/by-slug/{type}/{slug}` to `requireUnique: true` semantics:
/// any slug shared across tenants now returns HTTP 409
/// `SlugNotUniqueError` instead of silently resolving to the caller's
/// own entity. The 5.18.x runner still calls the global endpoint, so
/// `~5.18.0` clients keep working only until the first cross-tenant
/// slug collision in prod — `~5.19.0` is the minimum pin that stays
/// correct under the new server semantics. See indigoai-us/hq-cloud#3
/// + indigoai-us/hq-pro#67.
///
/// 5.18.x adds two corrections downstream of the 5.17.x reconciliation
/// fixes. 5.18.0 made symlinks round-trip as first-class entries (zero-
/// knowledge target, ETag-distinguishable from same-content regular
/// files), instead of silently dereferencing top-level symlinks or
/// dropping nested ones during walk. 5.18.1 filters S3 directory-marker
/// objects (0-byte, key ends in `/`) at `listRemoteFiles` so neither
/// the pull-planner (`hashFile` on existing local dir → EISDIR "read")
/// nor `downloadFile` (`writeFileSync` on trailing-slash path → EISDIR
/// "open") ever sees them — closes the regression introduced in 5.13.0
/// when the 0-byte filter was widened to admit legitimate `.gitkeep`
/// placeholders. See indigoai-us/hq-cloud#2 (5.18.0) + #4 (5.18.1) for
/// the wire-format details.
///
/// 5.17.x earlier shipped the journal-direction + ignore-filter guard
/// on `propagateDeletes` (defaults to `"owned-only"`). The 5.15.x line
/// still followed the legacy "delete every journal entry whose local
/// file is missing" semantics, which would erase peer uploads when the
/// first menubar sync ran on a behind machine and would erase legacy/
/// filtered paths when the local hqRoot's ignore filter rejected them.
/// See indigoai-us/hq#142 + the 2026-05-14 incident report.
///
/// 5.24.0 (2026-05-21) ships two related fixes, both motivated by a
/// real incident where a user's personal vault accumulated ~2,600 zombie
/// objects (~1,700 from old HQ layouts + ~900 from conflict-mirror
/// pollution + 196 cross-scope `companies/{slug}/**` leaks).
///   - Conflict-mirror exclusion in the push walker AND delete plan:
///     `*.conflict-<ISO>-<hash>.<ext>` files never round-trip to S3.
///     Active for ALL policies, not just the new default. Stops new
///     litter accretion immediately.
///   - `currency-gated` delete-propagation policy (opt-in in 5.24,
///     scheduled default in 5.25). Per-file HEAD + ETag verification
///     before any local-delete propagates. Strictly safer than
///     `owned-only` because it lets files arriving via `/update-hq`
///     (direction:"down") be cleanly deleted by the device that wrote
///     them, as long as no other device touched them since.
///   - Plus: new `filesTombstoned` / `filesRefusedStale` counters on
///     ShareResult, a `delete-refused-stale-etag` event variant, and
///     the `HQ_SYNC_DELETE_POLICY=currency-gated|owned-only|all` env
///     override honored by `sync-runner`.
/// See indigoai-us/hq-cloud#14 + 2026-05-21 reconcile incident report.
///
/// 5.25.0 (2026-05-21) ships two more fixes building on 5.24:
///   - PERSONAL_VAULT_DEFAULT_EXCLUSIONS — a hard exclusion list applied
///     when personalMode is true, complementing the existing top-level
///     filter (PERSONAL_VAULT_EXCLUDED_TOP_LEVEL) and the ephemeral
///     conflict-mirror filter (EPHEMERAL_PATH_PATTERN). Categories:
///     secrets (.env, .env.*, .mcp.json), machine-local state (.beads/,
///     .obsidian/, .vercel/, .cache_*), update-flow scratch (output/,
///     _legacy-*), pre-5.24 conflict mirror dir (.hq-conflicts/), OS /
///     build cruft (.DS_Store, node_modules/, dist/, .next/, build/).
///     Wired into both the upload walk and the delete-plan walk so
///     existing journaled entries matching a new exclusion get orphaned
///     (no DELETE issued). Emits a single `personal-vault-out-of-policy`
///     event per share() call when count > 0.
///   - Default delete policy flipped owned-only -> currency-gated (the
///     5.24-promised flip after the soak window). Rollback knob:
///     HQ_SYNC_DELETE_POLICY=owned-only.
///   - New CLI flag `--skip-personal` + env `HQ_SYNC_SKIP_PERSONAL=1`
///     drops the personal target from the --companies fanout. Used by
///     the menubar's "Sync personal vault" Settings toggle.
/// See indigoai-us/hq-cloud#15.
///
/// 5.26.0 (2026-05-22) adds the event-driven push watcher (`--event-push`,
/// gated to @getindigo.ai in the menubar; default poll-only otherwise).
/// 5.27.0 (2026-05-22) fixes the watcher never firing for `--companies`
/// edits: the runner no longer forces `personalMode: true` (which excluded
/// the `companies/` subtree it actually syncs), and the chokidar `ignored`
/// predicate no longer prunes ancestor dirs of allowlisted leaves on its
/// stat-less descent probe. Without this, instant sync silently fell back
/// to the 10-minute poll. The `~5.26` -> `~5.27` bump is required to pick it
/// up (tilde ranges don't cross the minor boundary).
/// 5.28.0 (2026-05-22) replaces the watcher's per-directory chokidar watch
/// with a SINGLE recursive `fs.watch` on macOS (FSEvents). chokidar 4 dropped
/// `fsevents`, so it watched via kqueue at ~1 fd per path (~11,600 fds over a
/// real HQ tree) — which EMFILEs under the default soft `ulimit -n` (256) and
/// silently kills the watcher. After: 1 OS handle for the whole tree. The
/// `~5.27` -> `~5.28` bump is required to pick it up.
/// 5.29.0 (2026-05-22) stamps `direction` ("up"/"down") on per-file progress
/// events so the menubar's Recent Changes activity log can label each file
/// uploaded vs downloaded. The `~5.28` -> `~5.29` bump is required to pick it up.
/// 5.30.0 (2026-05-22) fixes the personal-vault journal-slug collision: the
/// personal-vault fanout slot and a real `companies/personal` company both
/// resolved to journal slug `personal`, sharing one journal — so the company's
/// whole-tree delete-plan tombstoned the vault's hq-root keys every cycle and
/// the vault re-uploaded them (~190 `.claude/skills/*` files churned per sync).
/// 5.30 reserves a distinct vault journal slug (with a one-time seed migration).
/// The `~5.29` -> `~5.30` bump is required to pick it up.
/// 5.31.0 (2026-05-22) returns the downloaded object's S3 `created-by` metadata
/// and stamps it as `author` on download `progress` events, so the Recent
/// Changes activity log can attribute downloaded files to whoever uploaded
/// them. The `~5.30` -> `~5.31` bump is required to pick it up.
/// 5.32.0 (2026-05-23) extends sync to cloud:false companies via the
/// personal-vault fanout slot — the menubar can sync local-only company
/// trees through the same engine without registering them as cloud-backed.
/// 5.33.0 (2026-05-23) closes the original conflict-loop incident: lifts
/// machine-id provisioning into hq-cloud (4-tier resolver with SHA-1 hex
/// normalization for non-hex tier-1/tier-3 sources) and widens
/// `EPHEMERAL_PATH_PATTERN` to accept the `unknown` sentinel and
/// extensionless originals. Pre-5.33 Lightsail outposts stamped
/// `-unknown` short tokens that the share filter then refused, looping
/// `.conflict-*` litter through S3 forever. See indigoai-us/hq-cloud#23.
/// 5.34.0 (2026-05-24) ships the 10-bug cross-machine sync cleanup
/// (indigoai-us/hq-cloud#24) — three of those bugs directly destroy the
/// menubar's user-facing promises:
///   - Bug #9: cross-machine deletes now propagate via journal-vs-LIST
///     diff + HEAD-verify scope guard. Pre-5.34 the pull walker had no
///     tombstone-consumption mechanism, so the menubar's "drifted files"
///     count never zeroed (root cause of the operator's
///     `sync-app-is-still-showing-24-drifted-files-after-update` project).
///   - Bug #7: first-time-upload-with-cloud-collision now writes a
///     mirror instead of silently overwriting peer content. Pre-fix two
///     open laptops editing the same file before either synced silently
///     destroyed the slower-to-sync side with no conflict event → no tray
///     badge → invisible data loss.
///   - Bug #10: dir-vs-file `(local-file, cloud-dir)` no longer throws
///     `ENOTDIR` and aborts the whole company sync. Pre-fix one stale
///     path wedged auto-sync indefinitely.
/// Plus #1/#6/#8 (`.hq/` leak channel), #2 (pull-side ephemeral filter),
/// #3 (conflictPaths dedup), #4 (dir-vs-file warning), #5 (file mode
/// preserved across sync). Codex P1/P2 follow-ups: HEAD-verify STS scope,
/// EACCES journal retention, local-edit-vs-remote-delete race detection
/// for both files and symlinks, strict-octal `hq-mode` parse.
/// 5.35.0 (2026-05-24) adds `.claude/state/` + `.claude/audit/` to
/// `DEFAULT_IGNORES`. Those directories are session-/host-scoped by
/// design and were the dominant source of conflict mirrors in the 5.34.0
/// live cross-machine test (~25 of 30 mirrors traced directly there).
/// See indigoai-us/hq-cloud#25.
/// 5.36.0 (2026-05-24) ships two sync speedups: lstat fast-path skips
/// SHA-256 when `(size, mtimeMs)` match the journal baseline (~5–10× on
/// no-op syncs — most syncs) and a bounded-parallel transfer pool
/// (default 16, knob `HQ_SYNC_TRANSFER_CONCURRENCY`) for uploads +
/// downloads (4–8× on transfer-heavy syncs). Codex P1 follow-ups
/// serialize the interactive conflict prompt under the pool and drain
/// in-flight transfers on worker error. See indigoai-us/hq-cloud#26.
/// 5.37.0 (2026-05-25) ships file mtime + birthtime preservation across
/// sync. Push stamps source-side `lstat.mtimeMs` (and `birthtimeMs` when
/// the filesystem supports it AND differs from mtime) into S3 metadata
/// as `hq-mtime` / `hq-btime`. Pull applies the stamped value via
/// `utimesSync` after the byte write, falling back to write-time when
/// metadata is absent (back-compat). Symlinks skipped on both sides.
/// Composes cleanly with the 5.36 fast-path — the journal records the
/// post-utimes mtime, so the next sync correctly skips re-hashing. Codex
/// P2 widened the accepted mtime domain to include `0` (Unix epoch,
/// reproducible-builds clamp value) and negative epochs (pre-1970). See
/// indigoai-us/hq-cloud#27. The `~5.31` -> `~5.37` bump is required to
/// pick the whole chain up.
///
/// **5.38.0 (bulk-asymmetry circuit-breaker)** — `computeDeletePlan` now
/// refuses to convert >=10% / >=10-abs of in-scope journal entries into
/// remote `DeleteObject` calls when their local files have gone missing
/// all at once (moved hqRoot, partial restore, fresh clone over inherited
/// `~/.hq/`, unmounted volume, accidental `rm -rf`). Closes the failure
/// mode behind the 2026-05-25 indigo vault mass-delete (269 signals/ +
/// 290 sources/ delete-markers in one afternoon). Bypass paths preserved:
/// `HQ_SYNC_DELETE_BULK_OVERRIDE=1` env or `propagateDeletePolicy: "all"`.
/// See indigoai-us/hq-cloud#28.
pub const HQ_CLOUD_VERSION: &str = "~5.48.3";

/// Package name for the runner. Used by both the spawn site below and the
/// startup prewarm. Paired with `HQ_CLOUD_VERSION` to form the full
/// `npx --package=<pkg>@<ver>` argument.
pub const HQ_CLOUD_PACKAGE: &str = "@indigoai-us/hq-cloud";

/// Bin name shipped by `HQ_CLOUD_PACKAGE` (per its package.json `bin` entry).
/// npx needs this separately from the package because the bin name does
/// not match the package name.
pub const RUNNER_BIN: &str = "hq-sync-runner";

// ─────────────────────────────────────────────────────────────────────────────
// Error reporting
// ─────────────────────────────────────────────────────────────────────────────

/// Render a process termination as a human-readable string. When `code` is
/// `Some(N)`, the process called `exit(N)`. When `signal` is `Some(N)`, the
/// OS killed it with that signal — name it (SIGKILL=9, SIGTERM=15, SIGSEGV=11,
/// SIGBUS=10, SIGABRT=6) so "code unknown" no longer hides whether the runner
/// was OOM-killed vs crashed vs cancelled.
fn describe_exit(code: Option<i32>, signal: Option<i32>) -> String {
    if let Some(c) = code {
        return format!("with code {}", c);
    }
    match signal {
        Some(9) => "killed by SIGKILL (likely OOM or force-quit)".into(),
        Some(15) => "killed by SIGTERM (cancelled)".into(),
        Some(11) => "crashed with SIGSEGV (segfault)".into(),
        Some(10) => "crashed with SIGBUS".into(),
        Some(6) => "aborted with SIGABRT".into(),
        Some(2) => "killed by SIGINT".into(),
        Some(1) => "killed by SIGHUP".into(),
        Some(n) => format!("killed by signal {}", n),
        None => "with code unknown".into(),
    }
}

/// Emit a `sync:error` Tauri event AND capture the message to Sentry.
///
/// Used at exactly one call site today: the runner non-zero exit handler
/// in `start_sync`'s background task. By the time we reach that site, the
/// runner's stderr breadcrumbs have already accumulated on the Sentry
/// scope (see `ProcessEvent::Stderr` arm), so the captured event ships
/// with a trail of "what the runner was doing right before it died".
///
/// Other emit sites (`personal first-push`, runner-emitted ndjson `error`
/// events on stdout, `run_process_impl` spawn failures) intentionally
/// only call `app.emit(...)` — see the comments at each site for why.
/// In short: those failure modes either happen before the runner is up
/// (no breadcrumbs to attach) or are per-file errors that don't terminate
/// the run. If they prove to be recurring silent failures, add an explicit
/// `report_sync_error(...)` call at the relevant site.
///
/// History: prior to this helper, the `hq-sync-runner exited with code …`
/// path surfaced in the UI but never reached Sentry, so `#hq-alerts` was
/// silent during prod sync failures. See the broader silent-prod-error
/// fix for hq-onboarding (Cognito `invalid_client`) for the incident
/// context.
fn report_sync_error(app: &AppHandle, payload: SyncErrorEvent) -> tauri::Result<()> {
    sentry::with_scope(
        |scope| {
            if let Some(c) = &payload.company {
                scope.set_tag("company", c);
            }
            scope.set_tag("path", &payload.path);
        },
        || {
            sentry::capture_message(
                &format!("[sync] {}", payload.message),
                sentry::Level::Error,
            );
        },
    );
    app.emit(EVENT_SYNC_ERROR, payload)
}

// ─────────────────────────────────────────────────────────────────────────────
// Config resolution (inline — avoids calling async Tauri command)
// ─────────────────────────────────────────────────────────────────────────────

/// Resolve the HQ folder path by reading config.json and menubar.json directly.
fn resolve_hq_folder_path() -> Result<String, String> {
    let menubar_path = paths::menubar_json_path()?;

    let menubar_prefs: Option<MenubarPrefs> = if menubar_path.exists() {
        std::fs::read_to_string(&menubar_path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
    } else {
        None
    };

    // Shared lenient reader: parse failures fall through to menubar/discovery,
    // but real IO errors still propagate as Err. Uniform across all four
    // `resolve_hq_folder_path` duplicates.
    let config = crate::commands::config::read_hq_config_lenient()?;

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
pub(crate) fn resolve_vault_api_url() -> Result<String, String> {
    const DEFAULT_VAULT_API_URL: &str = "https://hqapi.getindigo.ai";

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
/// npx -y --package=@indigoai-us/hq-cloud@~5.19.0 hq-sync-runner \
///   --companies --direction both --on-conflict keep --hq-root <path>
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
/// - `--on-conflict keep` — preserve local edits when a divergent file is
///   detected, instead of aborting the company-wide sync. With `abort`, a
///   single conflicting file halted every other file's progress. `keep`
///   keeps the user's local copy as-is and continues syncing the rest.
/// - `--hq-root <path>` — local HQ directory
///
/// `HQ_ROOT` is also set in the child env as defense-in-depth (matches the
/// pre-Phase-7 pattern).
///
/// `personal_sync_enabled` toggles the personal-vault target in the fanout.
/// When false, `--skip-personal` is appended so the spawned runner's
/// `resolveSkipPersonal()` drops the personal slot. Sourced from
/// `MenubarPrefs.personal_sync_enabled` (defaults to true in get_settings).
pub fn build_sync_spawn_args(hq_folder_path: &str, personal_sync_enabled: bool) -> SpawnArgs {
    let mut env = HashMap::new();
    env.insert("HQ_ROOT".to_string(), hq_folder_path.to_string());
    // The runner is a Node script with `#!/usr/bin/env node`, and npx itself
    // is `#!/usr/bin/env node`. Without a real PATH, `env` can't find node on
    // Dock-launched apps and either process exits with code 127. See
    // `paths::child_path`.
    env.insert("PATH".to_string(), paths::child_path());

    let mut args = vec![
        "-y".to_string(),
        format!("--package={}@{}", HQ_CLOUD_PACKAGE, HQ_CLOUD_VERSION),
        RUNNER_BIN.to_string(),
        "--companies".to_string(),
        "--direction".to_string(),
        "both".to_string(),
        "--on-conflict".to_string(),
        "keep".to_string(),
        "--hq-root".to_string(),
        hq_folder_path.to_string(),
    ];
    if !personal_sync_enabled {
        // Append rather than insert mid-args so reading the joined command
        // line in logs / Sentry tags is predictable (toggle state shows at
        // the end, after the canonical args).
        args.push("--skip-personal".to_string());
    }

    SpawnArgs {
        // Resolve npx via known install prefixes + login-shell PATH fallback.
        // See `paths::resolve_bin` — GUI-launched Tauri apps get a minimal
        // launchd PATH and would otherwise fail with os error 2 on `npx`
        // (which lives in /opt/homebrew/bin or ~/.npm-global/bin, not in
        // /usr/bin). npx is part of npm, which is a listed installer prereq.
        cmd: paths::resolve_bin("npx"),
        args,
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
        // Synthetic complete for a not-yet-provisioned company: nothing was
        // ever on remote, nothing was journaled, so tombstone + refused-
        // stale counts are zero by construction. Use None (Option<u32>)
        // rather than Some(0) so the wire shape matches what a pre-5.24
        // runner would emit — keeps the renderer's "is this field
        // populated?" branch the cleaner one.
        files_tombstoned: None,
        files_refused_stale: None,
    })
}

/// Parse a single ndjson line and emit the corresponding Tauri event.
/// Unknown/malformed lines are silently skipped (logged in debug builds).
///
/// Per-company `Complete` events also accumulate into `totals`. On
/// `all-complete`, the aggregated totals are persisted to
/// `{hq_folder}/.hq-sync-journal.json` so `get_sync_status` surfaces a real
/// `lastSyncAt` and conflict count instead of "never" / zero.
fn handle_sync_line(app: &AppHandle, hq_folder: &str, totals: &Mutex<RunTotals>, jwt: &str, line: &str) {
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
        // Per-company / per-direction Stage-1 totals from `hq-sync-runner`
        // (≥hq-cloud@5.5.0). Forwarded to the Svelte frontend so it can
        // refine the progress denominator established by EVENT_SYNC_TOTALS
        // before any per-file Progress events arrive. When connected to an
        // older runner that doesn't emit Plan, this branch is simply never
        // taken — the existing TOTALS-based denominator stays authoritative.
        SyncEvent::Plan(payload) => app.emit(EVENT_SYNC_PLAN, payload.clone()),
        SyncEvent::Progress(payload) => {
            // Record into the session activity log (uploaded/downloaded with a
            // timestamp) and live-append to the Recent Changes window if open.
            crate::commands::activity::record_progress(app, payload);
            app.emit(EVENT_SYNC_PROGRESS, payload.clone())
        }
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
                // Per-file ndjson `error` events from the runner. These are
                // *not* captured to Sentry here — the runner-level error
                // (likely visible in stderr breadcrumbs) will surface via the
                // `report_sync_error` capture at the non-zero-exit site below
                // if the run terminates because of these. Per-file errors that
                // co-exist with a clean exit (`success=true, errors[] in
                // all-complete`) are intentionally renderer-only.
                app.emit(EVENT_SYNC_ERROR, payload.clone())
            }
        }
        SyncEvent::Complete(payload) => app.emit(EVENT_SYNC_COMPLETE, payload.clone()),
        // hq-cloud ≥5.24.0. Emitted only by the `currency-gated` policy;
        // pre-5.24 runners silently never emit this and the branch is dead.
        // Forward to the renderer as a warning row — the file was kept on
        // remote because peer drift or a missing journal etag made the
        // delete unsafe to propagate.
        SyncEvent::DeleteRefusedStaleEtag(payload) => {
            app.emit(EVENT_SYNC_DELETE_REFUSED_STALE_ETAG, payload.clone())
        }
        SyncEvent::NewFiles(payload) => {
            // Reconcile into the activity log: mark these paths as "added" (vs
            // the default "updated") and back-fill author from `addedBy` where
            // the per-file progress event carried none. Lands after the rows'
            // progress events, so this back-fills + re-emits to the open window.
            crate::commands::activity::record_new_files(app, payload);
            app.emit(EVENT_SYNC_NEW_FILES, payload.clone())
        }
        SyncEvent::AllComplete(payload) => {
            // Persist summary journal before emitting — the frontend's
            // SyncStats refresh reads this file on popover mount.
            let conflicts = totals.lock().unwrap_or_else(|e| e.into_inner()).conflicts;
            let now_iso = chrono::Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);
            let journal = journal_for_sync_complete(&now_iso, conflicts);
            if let Err(_e) = write_journal(hq_folder, &journal) {
                log("sync", &format!("failed to write journal: {_e}"));
                #[cfg(debug_assertions)]
                eprintln!("[sync] failed to write journal: {}", _e);
            }
            log("sync", &format!("all-complete (conflicts={conflicts})"));
            // Mirror the HQ folder into its own git repo (if any) so the
            // sync also captures a versioned snapshot. Fire-and-forget;
            // never blocks the AllComplete handler.
            crate::commands::git_mirror::spawn_mirror_after_sync(hq_folder);
            let emit_result = app.emit(EVENT_SYNC_ALL_COMPLETE, payload.clone());
            let app_clone = app.clone();
            let hq = hq_folder.to_string();
            let jwt_owned = jwt.to_string();
            tauri::async_runtime::spawn(async move {
                let _ = crate::commands::telemetry::send_telemetry_if_opted_in(
                    &app_clone, &hq, &jwt_owned,
                ).await;
            });
            // Reconcile manifest with on-disk reality. The runner downloads
            // cloud-only companies into `companies/{slug}/` as a side effect of
            // file writes — the manifest needs to learn about those folders so
            // they don't render as "Cloud Only" forever after. Best-effort and
            // fire-and-forget; failures are logged but don't surface to the UI.
            let hq_for_reconcile = hq_folder.to_string();
            let jwt_for_reconcile = jwt.to_string();
            tauri::async_runtime::spawn(async move {
                let vault_url = match crate::commands::sync::resolve_vault_api_url() {
                    Ok(u) => u,
                    Err(e) => {
                        log("sync", &format!("reconcile skipped: vault url: {e}"));
                        return;
                    }
                };
                let vault = crate::commands::vault_client::VaultClient::new(
                    &vault_url,
                    &jwt_for_reconcile,
                );
                match crate::commands::workspaces::reconcile_manifest_after_sync(
                    std::path::Path::new(&hq_for_reconcile),
                    &vault,
                ).await {
                    Ok(0) => {} // nothing new — common case, stay quiet
                    Ok(n) => log("sync", &format!("reconcile: added {n} new manifest entries")),
                    Err(e) => log("sync", &format!("reconcile failed (non-fatal): {e}")),
                }
            });
            emit_result
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
/// - Hard timeout of 1 hour; the sync is cancelled if it exceeds this.
///
/// Returns the handle string on success (always `"hq-sync"`).
#[tauri::command]
pub async fn start_sync(app: AppHandle) -> Result<String, String> {
    log("sync", "start_sync invoked");
    #[cfg(debug_assertions)]
    eprintln!("[sync] start_sync invoked");

    // Atomically check-and-register to prevent concurrent syncs (TOCTOU-safe)
    if !try_register_handle(SYNC_HANDLE) {
        log("sync", "BAIL: already running");
        #[cfg(debug_assertions)]
        eprintln!("[sync] BAIL: already running");
        return Err("Sync is already running".to_string());
    }

    // Best-effort machineId bootstrap — log on failure but do not abort sync.
    if let Err(e) = ensure_machine_id() {
        log("sync", &format!("ensure_machine_id failed: {e}"));
        eprintln!("ensure_machine_id failed: {e}");
    }

    // Resolve HQ folder — deregister on failure so future syncs aren't blocked
    let hq_folder_path = match resolve_hq_folder_path() {
        Ok(p) => {
            log("sync", &format!("hq_folder resolved: {p}"));
            p
        }
        Err(e) => {
            log("sync", &format!("BAIL: resolve_hq_folder_path failed: {e}"));
            #[cfg(debug_assertions)]
            eprintln!("[sync] BAIL: resolve_hq_folder_path failed: {}", e);
            deregister_process(SYNC_HANDLE);
            return Err(e);
        }
    };

    // Resolve the personal-sync toggle ONCE for the duration of this sync
    // run — same flag drives (a) whether we run the personal first-push pass
    // and (b) whether `--skip-personal` gets appended to the spawned runner.
    // Defaults to true (preserve pre-5.25 behavior) when get_settings fails,
    // since a stale-prefs read shouldn't accidentally disable a feature the
    // user expects to be on. The setting can be flipped at any time from
    // Settings; next sync picks it up on the next read here.
    let personal_sync_enabled: bool = match crate::commands::settings::get_settings().await {
        Ok(prefs) => prefs.personal_sync_enabled.unwrap_or(true),
        Err(e) => {
            log("sync", &format!("get_settings failed; assuming personal_sync_enabled=true: {e}"));
            true
        }
    };
    log("sync", &format!("personal_sync_enabled={}", personal_sync_enabled));

    // Resolve vault URL from ~/.hq/config.json
    let vault_api_url = match resolve_vault_api_url() {
        Ok(u) => {
            log("sync", &format!("vault_api_url resolved: {u}"));
            u
        }
        Err(e) => {
            log("sync", &format!("BAIL: resolve_vault_api_url failed: {e}"));
            deregister_process(SYNC_HANDLE);
            return Err(e);
        }
    };

    // Fetch (and if needed refresh) the Cognito JWT
    let jwt = match resolve_jwt().await {
        Ok(j) => {
            log("sync", "jwt resolved");
            j
        }
        Err(e) => {
            log("sync", &format!("BAIL: resolve_jwt failed: {e}"));
            deregister_process(SYNC_HANDLE);
            return Err(e);
        }
    };

    // "Preparing sync…" — walk every push-side target, hash each file,
    // compare to journal, and count the ACTUAL number of uploads the
    // runner will perform. The runner only emits `progress` events for
    // transfers (not skips), so this count is the real denominator.
    //
    // Pull-side downloads aren't counted here yet (would need an S3 LIST
    // per bucket). For steady-state syncs the journal already tells the
    // runner there's nothing to download → 0. For first syncs the bucket
    // is empty → 0. Mid-life out-of-band changes may slightly under-count;
    // the UI's honest fallback handles overshoot gracefully.
    {
        let prep_root = std::path::PathBuf::from(&hq_folder_path);
        let (local_companies, _) =
            crate::commands::workspaces::discover_local_companies(&prep_root);
        let slugs: Vec<String> = local_companies.iter().map(|e| e.slug.clone()).collect();
        let prep_start = std::time::Instant::now();
        let to_transfer =
            crate::commands::personal::count_files_to_transfer(&prep_root, &slugs);
        let elapsed = prep_start.elapsed().as_millis();
        log("sync", &format!("preparing: {to_transfer} files to transfer ({elapsed}ms)"));
        let _ = app.emit(
            crate::events::EVENT_SYNC_TOTALS,
            serde_json::json!({ "totalFiles": to_transfer }),
        );
    }

    // Provision any cloud: true companies that haven't been provisioned yet
    log("sync", "phase: provision_missing_companies");
    let vault = VaultClient::new(&vault_api_url, &jwt);
    let companies = match crate::commands::provision::provision_missing_companies(
        &std::path::PathBuf::from(&hq_folder_path),
        &vault,
        &vault_api_url,
    )
    .await
    {
        Ok(c) => {
            log(
                "sync",
                &format!(
                    "provisioned {} new companies: {:?}",
                    c.len(),
                    c.iter().map(|x| &x.slug).collect::<Vec<_>>()
                ),
            );
            c
        }
        Err(e) => {
            log("sync", &format!("BAIL: provision_missing_companies failed: {e}"));
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
            log("sync", &format!("failed to emit company-provisioned: {_e}"));
            #[cfg(debug_assertions)]
            eprintln!("[sync] failed to emit company-provisioned: {}", _e);
        }
        // First-push: upload every local file for the newly-provisioned company.
        log("sync", &format!("phase: first_push {}", company.slug));
        if let Err(e) = crate::commands::first_push::first_push_company(
            &app,
            &vault,
            &std::path::PathBuf::from(&hq_folder_path),
            company,
        )
        .await
        {
            log("sync", &format!("first_push failed for {}: {e}", company.slug));
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
    // Skipped entirely when the user has flipped off "Sync personal vault" —
    // running it anyway would auto-provision a bucket the user explicitly
    // doesn't want populated, then upload everything just for the runner to
    // immediately re-walk the same tree with `--skip-personal`.
    if personal_sync_enabled {
        log("sync", "phase: personal first-push");
        if let Err(e) = crate::commands::personal::ensure_personal_bucket_and_first_push(
            &app,
            &vault,
            &std::path::PathBuf::from(&hq_folder_path),
        )
        .await
        {
            log("sync", &format!("personal first-push failed: {e}"));
            #[cfg(debug_assertions)]
            eprintln!("[sync] personal first-push failed: {}", e);
            // NOT captured to Sentry: personal first-push happens before the
            // runner spawns, so it has no stderr breadcrumb context, and the
            // exit-time `report_sync_error` capture below won't fire because we
            // continue past this and let the runner take over. If this path ever
            // becomes a recurring silent failure, add an explicit capture here.
            let _ = app.emit(
                EVENT_SYNC_ERROR,
                SyncErrorEvent {
                    company: None,
                    path: "personal".to_string(),
                    message: format!("personal first-push failed: {e}"),
                },
            );
        }
    } else {
        log("sync", "phase: personal first-push skipped (personal_sync_enabled=false)");
    }

    let spawn_args = build_sync_spawn_args(&hq_folder_path, personal_sync_enabled);
    log(
        "sync",
        &format!(
            "about to spawn: cmd={} args={:?} hq_root={}",
            spawn_args.cmd, spawn_args.args, hq_folder_path
        ),
    );
    #[cfg(debug_assertions)]
    eprintln!(
        "[sync] about to spawn: cmd={} args={:?} hq_root={}",
        spawn_args.cmd, spawn_args.args, hq_folder_path
    );

    // Timeout watchdog — cancels sync after SYNC_TIMEOUT
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(SYNC_TIMEOUT).await;
        if is_registered(SYNC_HANDLE) {
            log("sync", "timeout reached, cancelling");
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
    let jwt_for_handler = jwt.clone();
    // Fresh totals per run — no reset needed between runs.
    let totals: Arc<Mutex<RunTotals>> = Arc::new(Mutex::new(RunTotals::default()));
    tauri::async_runtime::spawn_blocking(move || {
        log("sync", "bg task: entering run_process_impl");
        #[cfg(debug_assertions)]
        eprintln!("[sync] bg task: entering run_process_impl");
        let result = run_process_impl(SYNC_HANDLE, &spawn_args, |event| match event {
            ProcessEvent::Stdout(line) => {
                // Always mirror runner stdout to the log file — this is the
                // ndjson protocol stream and the only durable record of what
                // the runner did. The eprintln! is dev-only / verbose.
                log("runner.stdout", &line);
                #[cfg(debug_assertions)]
                eprintln!("[sync stdout] {}", line);
                handle_sync_line(&app_bg, &hq_folder_for_handler, &totals, &jwt_for_handler, &line);
            }
            ProcessEvent::Stderr(line) => {
                // Always log runner stderr — when sync gets stuck this is the
                // most likely place the cause shows up (npx download retry,
                // node uncaught exception, runner panic, etc.).
                log("runner.stderr", &line);
                // Catch-all error pipeline: every runner stderr line becomes
                // a Sentry breadcrumb attached to the current scope. If the
                // runner exits non-zero, the `report_sync_error` capture at
                // the exit site below will publish a single Sentry event with
                // these breadcrumbs as the trail of "what the runner was
                // doing right before it died". This is the design intent —
                // breadcrumbs accumulate noise for free, exit-time capture
                // converts that into a single alertable issue with context.
                //
                // PROTOCOL NOTE (2026-04-25): the runner currently emits
                // structured per-file error events on STDOUT as ndjson. Once
                // the runner is updated to emit errors on STDERR (planned
                // protocol change in @indigoai-us/hq-cloud), each runner
                // error becomes a breadcrumb here automatically — no Tauri
                // changes required.
                sentry::add_breadcrumb(sentry::Breadcrumb {
                    category: Some("runner.stderr".into()),
                    level: sentry::Level::Warning,
                    message: Some(line.clone()),
                    ..Default::default()
                });
                #[cfg(debug_assertions)]
                eprintln!("[sync stderr] {}", line);
            }
            ProcessEvent::Exit {
                code,
                signal,
                success,
            } => {
                let exit_desc = describe_exit(code, signal);
                log("sync", &format!("runner exited: success={} {}", success, exit_desc));
                // The runner exits 0 for recoverable conditions (setup-needed,
                // auth-error) — those surface as ndjson events before exit, so
                // the frontend already knows. A non-zero exit means the runner
                // bailed before emitting a useful protocol stream.
                if !success {
                    let _ = report_sync_error(
                        &app_bg,
                        crate::events::SyncErrorEvent {
                            company: None,
                            path: "(runner)".to_string(),
                            message: format!("hq-sync-runner exited {}", exit_desc),
                        },
                    );
                } else {
                    // Successful exit but no AllComplete observed (e.g.
                    // runner bailed on setup-needed for a brand-new account
                    // with no companies yet). Emit a synthetic AllComplete
                    // so the UI returns to idle and the local sync-state.json
                    // gets stamped with "just now" — otherwise the popover
                    // sits in "syncing" forever and the top SyncStats card
                    // shows "never" while the personal first-push (which DID
                    // run) updated everything else.
                    let saw = totals
                        .lock()
                        .map(|t| t.all_complete_seen)
                        .unwrap_or(false);
                    if !saw {
                        log("sync", "runner exited without AllComplete — synthesizing");
                        let synthetic = SyncEvent::AllComplete(SyncAllCompleteEvent {
                            companies_attempted: 0,
                            files_downloaded: 0,
                            bytes_downloaded: 0,
                            errors: Vec::new(),
                        });
                        let line = serde_json::to_string(&synthetic)
                            .unwrap_or_else(|_| "{}".to_string());
                        handle_sync_line(
                            &app_bg,
                            &hq_folder_for_handler,
                            &totals,
                            &jwt_for_handler,
                            &line,
                        );
                    }
                }
            }
        });

        if let Err(e) = result {
            log("sync", &format!("run_process_impl error: {e}"));
            // NOT captured to Sentry: spawn failures happen before the runner
            // produces any stderr/stdout, so there's nothing for the catch-all
            // breadcrumb listener to attach. If `npx` repeatedly fails to
            // resolve `@indigoai-us/hq-cloud@<ver>` and this path becomes the
            // silent failure mode, add an explicit capture here.
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

    // ── describe_exit ────────────────────────────────────────────────────────────

    #[test]
    fn describe_exit_with_normal_exit_code() {
        assert_eq!(describe_exit(Some(0), None), "with code 0");
        assert_eq!(describe_exit(Some(1), None), "with code 1");
        assert_eq!(describe_exit(Some(127), None), "with code 127");
    }

    #[test]
    fn describe_exit_names_well_known_signals() {
        assert!(describe_exit(None, Some(9)).contains("SIGKILL"));
        assert!(describe_exit(None, Some(15)).contains("SIGTERM"));
        assert!(describe_exit(None, Some(11)).contains("SIGSEGV"));
        assert!(describe_exit(None, Some(10)).contains("SIGBUS"));
        assert!(describe_exit(None, Some(6)).contains("SIGABRT"));
        assert!(describe_exit(None, Some(2)).contains("SIGINT"));
        assert!(describe_exit(None, Some(1)).contains("SIGHUP"));
    }

    #[test]
    fn describe_exit_falls_back_to_signal_number() {
        assert_eq!(describe_exit(None, Some(31)), "killed by signal 31");
    }

    #[test]
    fn describe_exit_with_neither_returns_unknown() {
        assert_eq!(describe_exit(None, None), "with code unknown");
    }

    #[test]
    fn describe_exit_prefers_code_over_signal() {
        // Should never happen in practice (POSIX is XOR), but be defensive.
        assert_eq!(describe_exit(Some(42), Some(9)), "with code 42");
    }

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
        let args = build_sync_spawn_args("/Users/test/HQ", true);
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
        let args = build_sync_spawn_args("/Users/test/HQ", true);
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
                "keep".to_string(),
                "--hq-root".to_string(),
                "/Users/test/HQ".to_string(),
            ]
        );
    }

    /// Personal-sync toggle ON (default) must NOT include `--skip-personal`.
    /// Pinning the negative explicitly so a future regression that toggles
    /// the flag in the wrong direction (e.g. inverted check) surfaces here.
    #[test]
    fn test_build_sync_spawn_args_omits_skip_personal_when_enabled() {
        let args = build_sync_spawn_args("/Users/test/HQ", true);
        assert!(
            !args.args.iter().any(|a| a == "--skip-personal"),
            "expected NO `--skip-personal` when personal_sync_enabled=true, got: {:?}",
            args.args
        );
    }

    /// Personal-sync toggle OFF appends `--skip-personal` at the end so the
    /// spawned hq-sync-runner drops the personal slot from its fanout plan
    /// (resolveSkipPersonal in sync-runner.ts treats the flag as truthy via
    /// the parsed-args path, equivalent to HQ_SYNC_SKIP_PERSONAL=1).
    #[test]
    fn test_build_sync_spawn_args_appends_skip_personal_when_disabled() {
        let args = build_sync_spawn_args("/Users/test/HQ", false);
        assert_eq!(
            args.args.last().map(String::as_str),
            Some("--skip-personal"),
            "expected `--skip-personal` as last arg when personal_sync_enabled=false, got: {:?}",
            args.args
        );
        // The canonical args must still be present in the same order — the
        // toggle should ONLY append, not reorder or omit anything.
        assert!(args.args.contains(&"--companies".to_string()));
        assert!(args.args.contains(&"--direction".to_string()));
        assert!(args.args.contains(&"both".to_string()));
    }

    /// Sync Now must use `--on-conflict keep` so a divergent local file
    /// preserves the user's edits instead of aborting the company-wide sync.
    /// Regressing to `abort` would cause a single conflicting file to halt
    /// every other file's progress on the affected company.
    #[test]
    fn test_build_sync_spawn_args_on_conflict_is_keep() {
        let args = build_sync_spawn_args("/tmp", true);
        let joined = args.args.join(" ");
        assert!(
            joined.contains("--on-conflict keep"),
            "spawn args must include `--on-conflict keep`: {:?}",
            args.args,
        );
    }

    /// Sync Now is bidirectional — the spawn must opt into `--direction both`.
    /// Guards against a future refactor silently dropping back to pull-only.
    #[test]
    fn test_build_sync_spawn_args_opts_into_direction_both() {
        let args = build_sync_spawn_args("/tmp", true);
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
        let args = build_sync_spawn_args("/tmp", true);
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
        let args = build_sync_spawn_args("/Users/test/HQ", true);
        let env = args.env.unwrap();
        assert_eq!(env.get("HQ_ROOT"), Some(&"/Users/test/HQ".to_string()));
        assert_eq!(env.len(), 2);
    }

    #[test]
    fn test_build_sync_spawn_args_env_sets_path_with_homebrew() {
        let args = build_sync_spawn_args("/tmp", true);
        let env = args.env.unwrap();
        let path = env.get("PATH").expect("PATH must be set so shebang can find node");
        // Must include homebrew so `#!/usr/bin/env node` resolves on Dock launches.
        assert!(path.contains("/opt/homebrew/bin"), "PATH missing /opt/homebrew/bin: {}", path);
    }

    #[test]
    fn test_build_sync_spawn_args_no_cwd() {
        let args = build_sync_spawn_args("/any/path", true);
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

    /// Stage-1 plan event from hq-cloud@5.5.0 runner. Forwarded to the
    /// frontend as `sync:plan` so the menubar can refine the progress
    /// denominator before any per-file events arrive.
    #[test]
    fn test_parse_plan_ndjson() {
        let line = r#"{"type":"plan","company":"indigo","filesToDownload":7,"bytesToDownload":4096,"filesToUpload":2,"bytesToUpload":1024,"filesToSkip":3,"filesToConflict":1}"#;
        let event: SyncEvent = serde_json::from_str(line).unwrap();
        match event {
            SyncEvent::Plan(p) => {
                assert_eq!(p.company, "indigo");
                assert_eq!(p.files_to_download, 7);
                assert_eq!(p.bytes_to_download, 4096);
                assert_eq!(p.files_to_upload, 2);
                assert_eq!(p.bytes_to_upload, 1024);
                assert_eq!(p.files_to_skip, 3);
                assert_eq!(p.files_to_conflict, 1);
            }
            _ => panic!("Expected Plan event"),
        }
    }

    /// A pull-only plan (push counts zero) must still parse cleanly.
    /// Mirrors what `sync()` emits in pull-only direction.
    #[test]
    fn test_parse_plan_ndjson_pull_only() {
        let line = r#"{"type":"plan","company":"indigo","filesToDownload":5,"bytesToDownload":2048,"filesToUpload":0,"bytesToUpload":0,"filesToSkip":0,"filesToConflict":0}"#;
        let event: SyncEvent = serde_json::from_str(line).unwrap();
        match event {
            SyncEvent::Plan(p) => {
                assert_eq!(p.files_to_download, 5);
                assert_eq!(p.files_to_upload, 0);
            }
            _ => panic!("Expected Plan event"),
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
    fn test_parse_new_files_ndjson() {
        let line = r#"{"type":"new-files","company":"indigo","files":[{"path":"docs/new.md","bytes":1024,"addedBy":"stefan@example.com"},{"path":"docs/other.md","bytes":512}]}"#;
        let event: SyncEvent = serde_json::from_str(line).unwrap();
        match event {
            SyncEvent::NewFiles(nf) => {
                assert_eq!(nf.company, "indigo");
                assert_eq!(nf.files.len(), 2);
                assert_eq!(nf.files[0].path, "docs/new.md");
                assert_eq!(nf.files[0].bytes, 1024);
                assert_eq!(nf.files[0].added_by, Some("stefan@example.com".to_string()));
                assert_eq!(nf.files[1].path, "docs/other.md");
                assert_eq!(nf.files[1].bytes, 512);
                assert_eq!(nf.files[1].added_by, None);
            }
            _ => panic!("Expected NewFiles event"),
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

    /// Belt-and-braces: fail loudly if someone pastes an unbounded range
    /// into the version const. The canonical shape is `~MAJOR.MINOR.PATCH`
    /// (tilde-prefixed minor floor — auto-applies patches, not minors).
    /// A bare `MAJOR.MINOR.PATCH` is grandfathered in for callers that
    /// genuinely want an exact pin. `latest` / `*` / empty are rejected:
    /// they defeat the deliberate minor-line selection and make first
    /// sync a roulette wheel.
    #[test]
    fn test_hq_cloud_version_is_pinned_semver() {
        assert!(
            !HQ_CLOUD_VERSION.is_empty(),
            "HQ_CLOUD_VERSION must not be empty"
        );
        assert_ne!(
            HQ_CLOUD_VERSION, "latest",
            "HQ_CLOUD_VERSION must select a minor line, not `latest`"
        );
        assert_ne!(
            HQ_CLOUD_VERSION, "*",
            "HQ_CLOUD_VERSION must select a minor line, not `*`"
        );

        // Strip a leading semver-range prefix (`~` for patch-float, `^`
        // for minor-float) before validating the M.m.p shape. Anything
        // else in the prefix slot fails fast.
        let core = match HQ_CLOUD_VERSION.as_bytes().first() {
            Some(b'~') | Some(b'^') => &HQ_CLOUD_VERSION[1..],
            Some(b) if b.is_ascii_digit() => HQ_CLOUD_VERSION,
            _ => panic!(
                "HQ_CLOUD_VERSION must start with `~`, `^`, or a digit — got `{}`",
                HQ_CLOUD_VERSION
            ),
        };

        // Rough semver shape: three dot-separated numeric segments.
        let parts: Vec<&str> = core.split('.').collect();
        assert_eq!(
            parts.len(),
            3,
            "HQ_CLOUD_VERSION core should look like MAJOR.MINOR.PATCH, got `{}` (full `{}`)",
            core,
            HQ_CLOUD_VERSION
        );
        for part in &parts {
            assert!(
                !part.is_empty() && part.chars().all(|c| c.is_ascii_digit()),
                "HQ_CLOUD_VERSION segment `{}` is not a number — got `{}`",
                part,
                HQ_CLOUD_VERSION
            );
        }
    }

    /// Positive coverage for the tilde-range pattern that ships patch
    /// fixes automatically. If the const ever drifts off this shape,
    /// callers reading `HQ_CLOUD_VERSION` as a "semver range" string
    /// (e.g. the docs, the prewarm log lines) will go stale silently.
    #[test]
    fn test_hq_cloud_version_floats_patch_within_minor() {
        assert!(
            HQ_CLOUD_VERSION.starts_with('~'),
            "HQ_CLOUD_VERSION should be a tilde range so patches auto-apply, \
             got `{}`. Use `~MAJOR.MINOR.0` (e.g. `~5.19.0`). If you genuinely \
             need an exact pin, also update this test.",
            HQ_CLOUD_VERSION
        );
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
            files_tombstoned: None,
            files_refused_stale: None,
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
            direction: None,
            deleted: None,
            author: None,
        }));
        assert_eq!(t.conflicts, 0);
    }

    #[test]
    fn test_accumulate_ignores_all_complete() {
        let mut t = RunTotals {
            conflicts: 4,
            ..Default::default()
        };
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
        let mut t = RunTotals {
            conflicts: 10,
            ..Default::default()
        };
        t.accumulate(&complete("a", 0, false));
        assert_eq!(t.conflicts, 10);
    }

    #[test]
    fn test_accumulate_saturates_on_overflow() {
        let mut t = RunTotals {
            conflicts: u32::MAX,
            ..Default::default()
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
