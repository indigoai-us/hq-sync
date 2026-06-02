//! Staging-aware drift classification (@getindigo.ai builders only).
//!
//! The base drift check (`hq_core_drift.rs`) compares locked-scope local
//! files against hq-core at the *released* `v{hqVersion}` tag. For an HQ
//! builder that's noisy: most "drift" is just work already promoted into
//! the `hq-core-staging` pipeline (merged to `main`, or sitting in an open
//! PR) that simply hasn't been cut into a release + pulled back via
//! `/update-hq` yet. Those files diff against the old release but are fully
//! accounted for.
//!
//! This module cross-references each drifted file's local git-blob SHA
//! against the staging repo and tags it:
//!   * `staging-main`  — byte-identical to the file on staging `main`
//!   * `pr:{n}`        — byte-identical to the file at the head of open PR #n
//!   * `unaccounted`   — not present anywhere in the pipeline (the real
//!                        action item)
//!
//! Scope + safety:
//!   * Gated to `@getindigo.ai` (same email-claim gate as `meetings.rs` /
//!     `daemon.rs`). Ineligible users trigger zero staging API calls — the
//!     feature is completely dark and the public binary carries no private-
//!     repo reference (the repo is read from config, defaulting only for
//!     eligible users).
//!   * The private staging repo requires auth; we resolve a token from the
//!     local `gh` CLI (`gh auth token`, falling back to `~/.config/gh/
//!     hosts.yml`). No token / no access → feature dark.
//!   * Every failure path returns `None` (fail-quiet) so the base drift
//!     report always renders even when staging is unreachable.

use std::collections::{BTreeMap, BTreeSet};
use std::process::Command;

use serde::{Deserialize, Serialize};
use tauri::path::BaseDirectory;
use tauri::{AppHandle, Manager};

use crate::commands::config::{read_hq_config_lenient, MenubarPrefs};
use crate::util::client_info::client_headers;
use crate::util::logfile::log;
use crate::util::paths;

/// Phase-1 eligibility domain. Mirrors `meetings::ALLOWED_DOMAIN` and the
/// `daemon.rs` event-push gate — the leading `@` blocks look-alikes like
/// `forgetindigo.ai`.
const ALLOWED_DOMAIN: &str = "@getindigo.ai";

/// Default staging repo for eligible users. Never baked into config or the
/// public payload — only resolved at runtime once the email gate passes.
const DEFAULT_STAGING_REPO: &str = "indigoai-us/hq-core-staging";

const REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);

/// Where a drifted file's content was found in the promotion pipeline.
/// Serialized to a flat wire string so the Svelte side can render it without
/// a tagged-union switch: `"staging-main"`, `"pr:182"`, `"unaccounted"`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(into = "String", try_from = "String")]
pub enum StagingStatus {
    /// Local content matches the file on staging `main`.
    StagingMain,
    /// Local content matches the file at the head of this open PR.
    StagingPr(u32),
    /// Local content matches nothing in the staging pipeline.
    Unaccounted,
}

impl StagingStatus {
    pub fn to_wire(&self) -> String {
        match self {
            StagingStatus::StagingMain => "staging-main".to_string(),
            StagingStatus::StagingPr(n) => format!("pr:{n}"),
            StagingStatus::Unaccounted => "unaccounted".to_string(),
        }
    }

    pub fn from_wire(s: &str) -> Result<Self, String> {
        match s {
            "staging-main" => Ok(StagingStatus::StagingMain),
            "unaccounted" => Ok(StagingStatus::Unaccounted),
            other => {
                if let Some(num) = other.strip_prefix("pr:") {
                    num.parse::<u32>()
                        .map(StagingStatus::StagingPr)
                        .map_err(|_| format!("bad PR number in staging status: {other:?}"))
                } else {
                    Err(format!("unrecognized staging status: {other:?}"))
                }
            }
        }
    }
}

impl From<StagingStatus> for String {
    fn from(s: StagingStatus) -> String {
        s.to_wire()
    }
}

impl TryFrom<String> for StagingStatus {
    type Error = String;
    fn try_from(s: String) -> Result<Self, Self::Error> {
        StagingStatus::from_wire(&s)
    }
}

/// In-memory index of every blob SHA each path carries across the staging
/// pipeline. Built once per drift scan, queried per drifted file.
#[derive(Debug, Default)]
pub struct StagingIndex {
    /// path -> set of blob SHAs present on staging `main`.
    main: BTreeMap<String, BTreeSet<String>>,
    /// (PR number, path -> set of blob SHAs at PR head), kept sorted by
    /// number so classification picks the lowest matching PR deterministically.
    prs: Vec<(u32, BTreeMap<String, BTreeSet<String>>)>,
}

impl StagingIndex {
    /// Classify one drifted file. `main` wins over any PR (already merged →
    /// most "settled"); otherwise the lowest-numbered open PR that carries a
    /// byte-identical copy wins.
    pub fn classify(&self, path: &str, local_sha: &str) -> StagingStatus {
        if self
            .main
            .get(path)
            .is_some_and(|shas| shas.contains(local_sha))
        {
            return StagingStatus::StagingMain;
        }
        let mut prs_sorted: Vec<&(u32, BTreeMap<String, BTreeSet<String>>)> =
            self.prs.iter().collect();
        prs_sorted.sort_by_key(|(n, _)| *n);
        for (num, files) in prs_sorted {
            if files.get(path).is_some_and(|shas| shas.contains(local_sha)) {
                return StagingStatus::StagingPr(*num);
            }
        }
        StagingStatus::Unaccounted
    }
}

// ── Eligibility + token resolution ────────────────────────────────────────────

/// Pure email gate — public for unit testing. Case-insensitive suffix match.
pub fn is_eligible_email(email: Option<&str>) -> bool {
    match email {
        Some(s) if !s.is_empty() => s.trim().to_ascii_lowercase().ends_with(ALLOWED_DOMAIN),
        _ => false,
    }
}

/// Read the signed-in email from the locally-cached Cognito id_token. Same
/// reader the sync path + `event_push_eligible` use; any failure → None.
fn signed_in_email() -> Option<String> {
    crate::commands::cognito::read_tokens_from_file()
        .ok()
        .flatten()
        .and_then(|t| t.id_token)
        .and_then(|tok| crate::commands::cognito::decode_id_token_claims(&tok).ok())
        .and_then(|c| c.email)
}

/// Read `stagingChannel` from `~/.hq/menubar.json`. Returns `true` (channel
/// ON) when the field is missing or null — preserves the pre-toggle
/// behaviour for @indigo builders who haven't touched Settings. Explicit
/// `false` flips them to the prod release channel.
///
/// Non-@indigo users' value is read but has no effect: `is_eligible_email`
/// gates them out before `is_staging_channel_enabled` is consulted. The
/// field is only writable for @indigo users via the Settings toggle (which
/// is hidden behind the shared `meetings_feature_enabled` gate — same
/// @getindigo.ai predicate the share-notify section uses).
fn is_staging_channel_enabled() -> bool {
    let prefs: Option<MenubarPrefs> = paths::menubar_json_path()
        .ok()
        .filter(|p| p.exists())
        .and_then(|p| std::fs::read_to_string(&p).ok())
        .and_then(|s| serde_json::from_str(&s).ok());
    prefs.and_then(|p| p.staging_channel).unwrap_or(true)
}

/// True when `~/.hq/menubar.json` carries an explicit non-empty
/// `driftStagingRepo`. The toggle's intent is "indigo opt-out of the
/// DEFAULT staging channel"; a user who manually configured a custom
/// staging repo has already made an explicit choice that takes
/// precedence (Codex P2 review on PR #110). `resolve_channel` in
/// `hq_core_state.rs` already shortcircuits to Staging on this path
/// before reading the toggle, so the spawn-side gate needs the same
/// carve-out to stay consistent with what the popover renders.
fn has_explicit_staging_repo() -> bool {
    let prefs: Option<MenubarPrefs> = paths::menubar_json_path()
        .ok()
        .filter(|p| p.exists())
        .and_then(|p| std::fs::read_to_string(&p).ok())
        .and_then(|s| serde_json::from_str(&s).ok());
    prefs
        .and_then(|p| p.drift_staging_repo)
        .map(|s| !s.trim().is_empty())
        .unwrap_or(false)
}

/// Resolve the staging repo (`owner/name`) to classify against:
///   1. explicit `driftStagingRepo` in `~/.hq/menubar.json` (any team can
///      point this at their own staging repo), else
///   2. the Indigo default — but only for an eligible (`@getindigo.ai`) user.
/// Returns `None` for ineligible users with no explicit config (feature dark).
fn resolve_staging_repo(eligible: bool) -> Option<String> {
    let prefs: Option<MenubarPrefs> = paths::menubar_json_path()
        .ok()
        .filter(|p| p.exists())
        .and_then(|p| std::fs::read_to_string(&p).ok())
        .and_then(|s| serde_json::from_str(&s).ok());
    if let Some(repo) = prefs.and_then(|p| p.drift_staging_repo) {
        let trimmed = repo.trim().to_string();
        if !trimmed.is_empty() {
            return Some(trimmed);
        }
    }
    if eligible {
        Some(DEFAULT_STAGING_REPO.to_string())
    } else {
        None
    }
}

/// Resolve a GitHub token via the local `gh` CLI, falling back to parsing
/// `~/.config/gh/hosts.yml`. Returns `None` (feature dark) on any failure.
///
/// Crate-public: `hq_core_update::install_hq_core_update` reuses this to
/// pass `GH_TOKEN` through to the rescue script when available. The public
/// hq-core repo doesn't strictly require a token, but threading one through
/// dodges the anonymous-clone rate limit (60/h) for users who already have
/// `gh` configured locally.
pub(crate) fn resolve_gh_token() -> Option<String> {
    // 1. `gh auth token` — the canonical, always-fresh source.
    let gh = paths::resolve_bin("gh");
    if let Ok(output) = Command::new(&gh).args(["auth", "token"]).output() {
        if output.status.success() {
            let tok = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !tok.is_empty() {
                return Some(tok);
            }
        }
    }
    // 2. Fallback: parse oauth_token for github.com out of hosts.yml. Covers
    //    setups where `gh` isn't on the Tauri app's minimal PATH but the
    //    config file is present.
    let hosts = dirs::home_dir()?
        .join(".config")
        .join("gh")
        .join("hosts.yml");
    let bytes = std::fs::read(&hosts).ok()?;
    let parsed: serde_yaml::Value = serde_yaml::from_slice(&bytes).ok()?;
    let tok = parsed
        .get("github.com")
        .and_then(|h| h.get("oauth_token"))
        .and_then(|t| t.as_str())
        .map(|s| s.to_string())?;
    if tok.is_empty() {
        None
    } else {
        Some(tok)
    }
}

// ── GitHub API shapes ──────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct GhTreesResponse {
    #[serde(default)]
    tree: Vec<GhTreeEntry>,
    #[serde(default)]
    truncated: bool,
}

#[derive(Debug, Deserialize)]
struct GhTreeEntry {
    path: String,
    #[serde(rename = "type")]
    kind: String,
    sha: String,
}

#[derive(Debug, Deserialize)]
struct GhPull {
    number: u32,
}

#[derive(Debug, Deserialize)]
struct GhPullFile {
    filename: String,
    sha: String,
}

// ── Fetch ────────────────────────────────────────────────────────────────────

fn authed_client(token: &str) -> Result<reqwest::Client, String> {
    let mut headers = client_headers();
    let bearer = format!("Bearer {token}");
    if let Ok(v) = reqwest::header::HeaderValue::from_str(&bearer) {
        headers.insert(reqwest::header::AUTHORIZATION, v);
    }
    headers.insert(
        reqwest::header::ACCEPT,
        reqwest::header::HeaderValue::from_static("application/vnd.github+json"),
    );
    reqwest::Client::builder()
        .default_headers(headers)
        .timeout(REQUEST_TIMEOUT)
        .build()
        .map_err(|e| format!("build staging client: {e}"))
}

/// Fetch the staging `main` tree as path -> {sha}.
async fn fetch_main_tree(
    client: &reqwest::Client,
    repo: &str,
) -> Result<BTreeMap<String, BTreeSet<String>>, String> {
    let url = format!("https://api.github.com/repos/{repo}/git/trees/main?recursive=1");
    let resp = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("GET {url}: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("staging main tree HTTP {}", resp.status()));
    }
    let parsed: GhTreesResponse = resp
        .json()
        .await
        .map_err(|e| format!("parse staging tree JSON: {e}"))?;
    if parsed.truncated {
        log(
            "hq-core-staging",
            "WARNING: staging main tree truncated — classification is a lower bound",
        );
    }
    let mut out: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for entry in parsed.tree {
        if entry.kind == "blob" {
            out.entry(entry.path).or_default().insert(entry.sha);
        }
    }
    Ok(out)
}

/// List open PR numbers (paginated).
async fn fetch_open_pr_numbers(client: &reqwest::Client, repo: &str) -> Result<Vec<u32>, String> {
    let mut numbers = Vec::new();
    let mut page = 1u32;
    loop {
        let url = format!(
            "https://api.github.com/repos/{repo}/pulls?state=open&per_page=100&page={page}"
        );
        let resp = client
            .get(&url)
            .send()
            .await
            .map_err(|e| format!("GET {url}: {e}"))?;
        if !resp.status().is_success() {
            return Err(format!("staging pulls list HTTP {}", resp.status()));
        }
        let pulls: Vec<GhPull> = resp
            .json()
            .await
            .map_err(|e| format!("parse pulls JSON: {e}"))?;
        let n = pulls.len();
        numbers.extend(pulls.into_iter().map(|p| p.number));
        if n < 100 {
            break;
        }
        page += 1;
    }
    Ok(numbers)
}

/// Fetch one PR's changed files as path -> {sha} (paginated).
async fn fetch_pr_files(
    client: &reqwest::Client,
    repo: &str,
    pr: u32,
) -> Result<BTreeMap<String, BTreeSet<String>>, String> {
    let mut out: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut page = 1u32;
    loop {
        let url = format!(
            "https://api.github.com/repos/{repo}/pulls/{pr}/files?per_page=100&page={page}"
        );
        let resp = client
            .get(&url)
            .send()
            .await
            .map_err(|e| format!("GET {url}: {e}"))?;
        if !resp.status().is_success() {
            return Err(format!("staging PR #{pr} files HTTP {}", resp.status()));
        }
        let files: Vec<GhPullFile> = resp
            .json()
            .await
            .map_err(|e| format!("parse PR #{pr} files JSON: {e}"))?;
        let n = files.len();
        for f in files {
            out.entry(f.filename).or_default().insert(f.sha);
        }
        if n < 100 {
            break;
        }
        page += 1;
    }
    Ok(out)
}

/// Build the full staging index if and only if the current user is eligible
/// and a token + repo resolve. Returns `None` (feature dark / fail-quiet) on
/// any miss — callers treat that as "no classification available".
pub async fn build_index_if_eligible() -> Option<StagingIndex> {
    let eligible = is_eligible_email(signed_in_email().as_deref());
    let repo = resolve_staging_repo(eligible)?;
    // An explicit non-Indigo repo is honoured even for non-@getindigo.ai
    // users (any team can adopt the workflow); the Indigo default is gated.
    if !eligible && repo == DEFAULT_STAGING_REPO {
        return None;
    }
    let token = resolve_gh_token()?;
    let client = match authed_client(&token) {
        Ok(c) => c,
        Err(e) => {
            log("hq-core-staging", &format!("client build failed: {e}"));
            return None;
        }
    };

    let main = match fetch_main_tree(&client, &repo).await {
        Ok(m) => m,
        Err(e) => {
            log("hq-core-staging", &format!("main tree fetch failed: {e}"));
            return None;
        }
    };

    let mut prs = Vec::new();
    match fetch_open_pr_numbers(&client, &repo).await {
        Ok(numbers) => {
            for pr in numbers {
                match fetch_pr_files(&client, &repo, pr).await {
                    Ok(files) => prs.push((pr, files)),
                    Err(e) => log(
                        "hq-core-staging",
                        &format!("PR #{pr} files fetch failed (skipping): {e}"),
                    ),
                }
            }
        }
        Err(e) => log(
            "hq-core-staging",
            &format!("open-PR list fetch failed (main-only classification): {e}"),
        ),
    }

    log(
        "hq-core-staging",
        &format!(
            "index built: repo={repo} main_paths={} open_prs={}",
            main.len(),
            prs.len()
        ),
    );
    Some(StagingIndex { main, prs })
}

// ── Sync-point provenance & full replace-rescue ──────────────────────────────
//
// The scripts/replace-rescue.sh rescue script (renamed from
// `replace-from-staging-rescue.sh` in v0.1.104) stamps
// `replaced_from_source.last_sync_sha` into `core/core.yaml` after every
// successful default-mode run. The menubar surfaces an "Update from staging"
// pill (only for `@getindigo.ai` users) when:
//   * the stamp is missing entirely (never synced), OR
//   * the stamped SHA is older than staging `main`'s HEAD SHA.
//
// Clicking the pill invokes `run_replace_from_staging`, which spawns the
// bundled bash script against the resolved HQ folder. The script handles
// drift rescue, history-aware skip gate, carve-outs, and the post-overlay
// stamp write — see scripts/replace-rescue.sh for the full algorithm.

/// Output from spawning the rescue script. Wired to the frontend so the
/// popover can show success / failure + the last few lines of script output
/// without dumping the whole 700-line scan log into the popover.
#[derive(Debug, Clone, Serialize)]
pub struct RescueRunResult {
    /// Process exit code. `0` is success; non-zero is a script-side error.
    pub exit_code: i32,
    /// Last ~40 lines of combined stdout+stderr — enough for the trailing
    /// summary block without flooding the IPC bridge.
    pub log_tail: String,
    /// Full log file path on disk for `Open in Finder` / debug.
    pub log_path: String,
}

/// Resolve the user's HQ folder using the same 4-tier resolver the rest of
/// the app uses (menubar.json → config.json → discovery → ~/HQ).
///
/// Crate-public: shared with `hq_core_update::install_hq_core_update` so the
/// prod-update spawn path doesn't re-implement the resolver tree.
pub(crate) fn resolve_hq_folder() -> std::path::PathBuf {
    let menubar_prefs: Option<MenubarPrefs> = paths::menubar_json_path()
        .ok()
        .filter(|p| p.exists())
        .and_then(|p| std::fs::read_to_string(&p).ok())
        .and_then(|s| serde_json::from_str(&s).ok());
    let config = read_hq_config_lenient().ok().flatten();
    paths::resolve_hq_folder(
        config.as_ref().and_then(|c| c.hq_folder_path.as_deref()),
        menubar_prefs.as_ref().and_then(|p| p.hq_path.as_deref()),
    )
}

/// Resolve the rescue script. Tries (in order):
///   1. Bundled `Resources/_up_/scripts/` — the happy path for any
///      packaged install whose Tauri auto-update refreshed the resource
///      dir along with the executable.
///   2. Dev-fallback `scripts/...` relative to cwd — covers `cargo run`
///      and `tauri dev` from a repo checkout.
///   3. **Live-fetch fallback** — download `scripts/replace-rescue.sh`
///      from `indigoai-us/hq-sync` at the matching version tag (with
///      `main` as a secondary fallback) into
///      `~/.hq/cache/hq-sync/scripts/replace-rescue-v{app_version}.sh`
///      and return the cached path. Closes the gap where the Tauri
///      updater bumped the executable but left a pre-rename copy in
///      `Resources/_up_/scripts/` (observed between v0.1.106 and
///      v0.1.107 around the `replace-from-staging-rescue.sh` →
///      `replace-rescue.sh` rename in commit cebf307).
///
/// Async because of step 3 — the network fetch can't run on the UI thread
/// without blocking it for the request duration.
///
/// Crate-public: `hq_core_update::install_hq_core_update` reuses this to
/// spawn the same rescue script against the released hq-core repo (replaces
/// the old "open Claude Code with /update-hq" CTA for prod users).
pub(crate) async fn resolve_rescue_script(app: &AppHandle) -> Result<std::path::PathBuf, String> {
    // New name first (v0.1.104+), old name as fallback so a stale resource
    // dir mid-rebuild (or any pre-rename hq-sync.app still on disk that
    // somehow re-invokes this code path) still resolves cleanly. Both
    // names point at the same script; the rename was purely cosmetic.
    let candidates = [
        "_up_/scripts/replace-rescue.sh",
        "scripts/replace-rescue.sh",
        "_up_/scripts/replace-from-staging-rescue.sh",
        "scripts/replace-from-staging-rescue.sh",
    ];
    for rel in candidates {
        if let Ok(p) = app.path().resolve(rel, BaseDirectory::Resource) {
            if p.is_file() {
                return Ok(p);
            }
        }
    }
    // Dev fallback: the cwd may be the repo root when running `cargo run`
    // directly without going through `tauri dev`. Try the new name first.
    let cwd = std::env::current_dir().ok();
    for filename in ["replace-rescue.sh", "replace-from-staging-rescue.sh"] {
        if let Some(p) = cwd.as_ref().map(|c| c.join("scripts").join(filename)) {
            if p.is_file() {
                return Ok(p);
            }
        }
    }
    // Live-fetch fallback — the bundle was stale. Download the matching
    // version from GitHub into the per-user cache and return that.
    //
    // The fetcher classifies the response into the three variants
    // `ensure_cached_rescue_script` requires (see rescue_script_cache::
    // FetchOutcome): only a definitive 404 authorises a fallback from
    // the tagged URL to `main`. Any other HTTP status (5xx, 403, etc.)
    // or transport-layer failure (DNS, TLS, timeout) returns
    // TransientError, which the caller surfaces verbatim instead of
    // silently caching `main` content under the running version key.
    let app_version = app.package_info().version.to_string();
    let home = dirs::home_dir();
    let fetcher = |url: String| async move {
        use crate::commands::rescue_script_cache::FetchOutcome;
        let client = match reqwest::Client::builder()
            .default_headers(crate::util::client_info::client_headers())
            .timeout(std::time::Duration::from_secs(15))
            .build()
        {
            Ok(c) => c,
            Err(e) => return FetchOutcome::TransientError(format!("build http client: {e}")),
        };
        let resp = match client.get(&url).send().await {
            Ok(r) => r,
            Err(e) => return FetchOutcome::TransientError(format!("GET {url}: {e}")),
        };
        let status = resp.status();
        if status == reqwest::StatusCode::NOT_FOUND {
            return FetchOutcome::NotFound;
        }
        if !status.is_success() {
            return FetchOutcome::TransientError(format!("GET {url}: HTTP {status}"));
        }
        match resp.bytes().await {
            Ok(b) => FetchOutcome::Body(b.to_vec()),
            Err(e) => FetchOutcome::TransientError(format!("read body for {url}: {e}")),
        }
    };
    match crate::commands::rescue_script_cache::ensure_cached_rescue_script(
        home.as_deref(),
        &app_version,
        fetcher,
    )
    .await
    {
        Ok((path, source)) => {
            log(
                "hq-core-staging",
                &format!(
                    "rescue script resolved via {source:?} at {}",
                    path.display()
                ),
            );
            Ok(path)
        }
        Err(e) => Err(format!(
            "replace-rescue.sh not found in resource dir (looked at: {:?}); \
             live-fetch fallback also failed: {e}",
            candidates
        )),
    }
}

/// Tauri command — run the rescue script against the resolved HQ folder.
/// Eligibility is enforced by `resolve_staging_repo`: ineligible users with
/// no `driftStagingRepo` override resolve to `None` and get rejected here.
/// Ineligible users WITH an explicit override are allowed through — they
/// opted in by configuring the menubar pref, and the same path lets the
/// pill render for them in `check_staging_replace_available`. Keeping the
/// two entry points consistent avoids the "visible action that always
/// errors" pattern Codex flagged.
///
/// Long-running (~30-90s on first run because of the full-history clone +
/// scan). The frontend should show a spinner and disable the pill while
/// the future is pending.
#[tauri::command]
pub async fn run_replace_from_staging(app: AppHandle) -> Result<RescueRunResult, String> {
    // Settings toggle: @indigo user opted out of the DEFAULT staging
    // channel. The pill should already be hidden when the toggle is
    // off, so reaching this command means the frontend is stale or a
    // custom caller is invoking us — refuse the spawn unless the user
    // also carved out an explicit `driftStagingRepo` override.
    //
    // The carve-out matters because `resolve_channel` treats an
    // explicit repo as Staging *regardless* of the toggle. Without
    // this guard a user with a custom staging repo would see the
    // popover render Staging + click the pill + hit "staging channel
    // disabled" — exactly the inconsistency Codex flagged in the
    // round-5 review on PR #110.
    if !is_staging_channel_enabled() && !has_explicit_staging_repo() {
        return Err(
            "staging channel disabled in Settings — re-enable to run replace-from-staging"
                .to_string(),
        );
    }
    let eligible = is_eligible_email(signed_in_email().as_deref());
    let repo = resolve_staging_repo(eligible).ok_or_else(|| {
        "no staging repo resolved (set driftStagingRepo in ~/.hq/menubar.json, or sign in with an @getindigo.ai account)".to_string()
    })?;
    let token = resolve_gh_token()
        .ok_or_else(|| "no GitHub token available (gh auth token failed)".to_string())?;
    let hq_folder = resolve_hq_folder();
    if !hq_folder.join("companies").is_dir() || !hq_folder.join("personal").is_dir() {
        return Err(format!(
            "HQ folder at {} is missing companies/ or personal/ (not a valid HQ root)",
            hq_folder.display()
        ));
    }
    let script = resolve_rescue_script(&app).await?;

    // Stream the combined output to a per-invocation log file so the user
    // can `tail -f` it during the multi-minute scan and so we have a
    // post-mortem on failures. The popover gets a 40-line tail.
    let log_path = std::env::temp_dir().join(format!(
        "hq-sync-replace-from-staging-{}.log",
        std::process::id()
    ));
    let log_file_for_stdout = std::fs::File::create(&log_path)
        .map_err(|e| format!("create log file {}: {e}", log_path.display()))?;
    let log_file_for_stderr = log_file_for_stdout
        .try_clone()
        .map_err(|e| format!("dup log file fd: {e}"))?;

    log(
        "hq-core-staging",
        &format!(
            "spawning rescue: script={} hq_root={} repo={} log={}",
            script.display(),
            hq_folder.display(),
            repo,
            log_path.display()
        ),
    );

    // bash <script> --hq-root <folder> --source <repo> --yes
    // Token is passed via env (never in argv — argv shows up in `ps`).
    let bash = paths::resolve_bin("bash");
    let mut cmd = tokio::process::Command::new(&bash);
    cmd.arg(script.as_os_str())
        .arg("--hq-root")
        .arg(hq_folder.as_os_str())
        .arg("--source")
        .arg(&repo)
        .arg("--yes")
        .env("GH_TOKEN", &token)
        .stdout(std::process::Stdio::from(log_file_for_stdout))
        .stderr(std::process::Stdio::from(log_file_for_stderr));

    let status = cmd
        .status()
        .await
        .map_err(|e| format!("spawn rescue script: {e}"))?;

    let exit_code = status.code().unwrap_or(-1);
    let log_tail =
        tail_log(&log_path, 40).unwrap_or_else(|e| format!("(log tail unavailable: {e})"));

    log(
        "hq-core-staging",
        &format!("rescue exit={} log={}", exit_code, log_path.display()),
    );

    Ok(RescueRunResult {
        exit_code,
        log_tail,
        log_path: log_path.display().to_string(),
    })
}

/// Read the last N lines of a log file. Pure stdlib so we don't pull in
/// another dep just for tailing. Reads the whole file into memory — fine
/// for our use (rescue logs are < 100 KB even in the worst case).
///
/// Crate-public so `hq_core_update::install_hq_core_update` can surface the
/// same trailing-log feedback chip the staging pill uses.
pub(crate) fn tail_log(path: &std::path::Path, n_lines: usize) -> Result<String, String> {
    let content =
        std::fs::read_to_string(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    let lines: Vec<&str> = content.lines().collect();
    let start = lines.len().saturating_sub(n_lines);
    Ok(lines[start..].join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn idx() -> StagingIndex {
        let mut main = BTreeMap::new();
        main.insert(
            "a.md".to_string(),
            BTreeSet::from(["sha-main-a".to_string()]),
        );

        let mut pr182 = BTreeMap::new();
        pr182.insert(
            "b.md".to_string(),
            BTreeSet::from(["sha-182-b".to_string()]),
        );
        pr182.insert(
            "shared.md".to_string(),
            BTreeSet::from(["sha-shared".to_string()]),
        );

        let mut pr183 = BTreeMap::new();
        pr183.insert(
            "c.md".to_string(),
            BTreeSet::from(["sha-183-c".to_string()]),
        );
        pr183.insert(
            "shared.md".to_string(),
            BTreeSet::from(["sha-shared".to_string()]),
        );

        StagingIndex {
            main,
            prs: vec![(183, pr183), (182, pr182)],
        }
    }

    #[test]
    fn classify_main_match() {
        assert_eq!(
            idx().classify("a.md", "sha-main-a"),
            StagingStatus::StagingMain
        );
    }

    #[test]
    fn classify_pr_match() {
        assert_eq!(
            idx().classify("b.md", "sha-182-b"),
            StagingStatus::StagingPr(182)
        );
        assert_eq!(
            idx().classify("c.md", "sha-183-c"),
            StagingStatus::StagingPr(183)
        );
    }

    #[test]
    fn classify_lowest_pr_when_multiple() {
        // `shared.md` with `sha-shared` exists in both 182 and 183 (inserted
        // 183-first) — the lower number must win deterministically.
        assert_eq!(
            idx().classify("shared.md", "sha-shared"),
            StagingStatus::StagingPr(182)
        );
    }

    #[test]
    fn classify_unaccounted() {
        assert_eq!(
            idx().classify("a.md", "different-sha"),
            StagingStatus::Unaccounted
        );
        assert_eq!(
            idx().classify("missing.md", "whatever"),
            StagingStatus::Unaccounted
        );
    }

    #[test]
    fn email_gate_allows_indigo() {
        assert!(is_eligible_email(Some("corey@getindigo.ai")));
        assert!(is_eligible_email(Some("Corey@GetIndigo.ai")));
    }

    #[test]
    fn email_gate_blocks_lookalike_and_empty() {
        assert!(!is_eligible_email(Some("attacker@forgetindigo.ai")));
        assert!(!is_eligible_email(Some("someone@example.com")));
        assert!(!is_eligible_email(Some("")));
        assert!(!is_eligible_email(None));
    }

    #[test]
    fn staging_status_wire_round_trip() {
        for s in [
            StagingStatus::StagingMain,
            StagingStatus::StagingPr(182),
            StagingStatus::Unaccounted,
        ] {
            let wire = s.to_wire();
            assert_eq!(StagingStatus::from_wire(&wire).unwrap(), s);
        }
        assert_eq!(StagingStatus::StagingMain.to_wire(), "staging-main");
        assert_eq!(StagingStatus::StagingPr(182).to_wire(), "pr:182");
        assert_eq!(StagingStatus::Unaccounted.to_wire(), "unaccounted");
        assert!(StagingStatus::from_wire("garbage").is_err());
        assert!(StagingStatus::from_wire("pr:notanum").is_err());
    }

    #[test]
    fn serde_round_trip_through_json() {
        let s = StagingStatus::StagingPr(182);
        let json = serde_json::to_string(&s).unwrap();
        assert_eq!(json, "\"pr:182\"");
        let back: StagingStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(back, s);
    }
}
