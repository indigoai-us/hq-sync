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
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::path::BaseDirectory;
use tauri::{AppHandle, Emitter, Manager};

use crate::commands::config::{read_hq_config_lenient, MenubarPrefs};
use crate::commands::hq_core_drift::{
    excluded_scope_paths, is_conflict_artifact, path_in_excluded_scope, path_in_locked_scope,
    read_locked_paths, walk_local_under_scope, DriftEntry, DriftReport,
};
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
        let mut prs_sorted: Vec<&(u32, BTreeMap<String, BTreeSet<String>>)> = self.prs.iter().collect();
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
fn resolve_gh_token() -> Option<String> {
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
    let hosts = dirs::home_dir()?.join(".config").join("gh").join("hosts.yml");
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
    /// Git file mode. `100644`/`100755` = regular file, `120000` = symlink,
    /// `040000` = tree, `160000` = submodule. Drift code filters out
    /// `120000` so symlinks aren't compared against local content (the
    /// blob is the target-path string, not the target's content — local
    /// would always look "modified" even when identical).
    #[serde(default)]
    mode: Option<String>,
    /// Blob size in bytes; absent for non-blob entries (trees/commits/
    /// submodules) and tolerated as `None` so the JSON parses cleanly.
    /// Used by the staging-drift calculator's DriftEntry size column;
    /// the PR-classifier index path ignores it.
    #[serde(default)]
    size: Option<u64>,
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

// ── Sync-point provenance & full replace-from-staging ─────────────────────────
//
// The scripts/replace-from-staging-rescue.sh rescue script stamps
// `replaced_from_staging.last_sync_sha` into `core/core.yaml` after every
// successful default-mode run. The menubar surfaces an "Update from staging"
// pill (only for `@getindigo.ai` users) when:
//   * the stamp is missing entirely (never synced), OR
//   * the stamped SHA is older than staging `main`'s HEAD SHA.
//
// Clicking the pill invokes `run_replace_from_staging`, which spawns the
// bundled bash script against the resolved HQ folder. The script handles
// drift rescue, history-aware skip gate, carve-outs, and the post-overlay
// stamp write — see scripts/replace-from-staging-rescue.sh for the full
// algorithm.

/// What the menubar needs to decide whether to show the "Update from staging"
/// pill and what to label it with. Serialized to the frontend as JSON.
#[derive(Debug, Clone, Serialize)]
pub struct StagingReplaceInfo {
    /// True when the user should be offered an update. False means either the
    /// local stamp matches `main`'s HEAD, or the user is ineligible and the
    /// feature is dark.
    pub available: bool,
    /// `replaced_from_staging.last_sync_sha` from local `core/core.yaml`.
    /// `None` if the stamp is missing (never synced via this script).
    pub local_sha: Option<String>,
    /// HEAD SHA of staging `main` at check time.
    pub latest_sha: String,
    /// First 7 chars of `latest_sha`, for use in pill labels like
    /// `"Update from staging (b02eeb4)"`.
    pub latest_short: String,
    /// `owner/name` form of the repo being checked. Useful for tooltip text.
    pub repo: String,
}

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

#[derive(Debug, Deserialize)]
struct GhCommit {
    sha: String,
}

#[derive(Debug, Deserialize)]
struct LocalCoreYaml {
    #[serde(default)]
    replaced_from_staging: Option<LocalReplacedFromStaging>,
}

#[derive(Debug, Deserialize)]
struct LocalReplacedFromStaging {
    #[serde(default)]
    source: Option<String>,
    #[serde(default)]
    last_sync_sha: Option<String>,
}

/// Resolve the user's HQ folder using the same 4-tier resolver the rest of
/// the app uses (menubar.json → config.json → discovery → ~/HQ).
fn resolve_hq_folder() -> std::path::PathBuf {
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

/// Read `replaced_from_staging` from local `core/core.yaml`. Falls back to
/// `core.yaml` (pre-v14 layout) the same way `hq_core_update.rs` does.
/// Returns the SHA only if the recorded `source` matches `expected_source`
/// (different sources can't be meaningfully compared).
fn local_last_sync_sha(expected_source: &str) -> Option<String> {
    let hq_folder = resolve_hq_folder();
    let canonical = hq_folder.join("core").join("core.yaml");
    let legacy = hq_folder.join("core.yaml");
    let core_yaml = if canonical.is_file() { canonical } else { legacy };

    let bytes = std::fs::read(&core_yaml).ok()?;
    let parsed: LocalCoreYaml = serde_yaml::from_slice(&bytes).ok()?;
    let rfs = parsed.replaced_from_staging?;
    // Only honour the stamp when the recorded source matches what we're
    // about to compare against. A stamp from a different fork tells us
    // nothing about how far ahead `indigoai-us/hq-core-staging` is.
    if rfs.source.as_deref() != Some(expected_source) {
        return None;
    }
    rfs.last_sync_sha
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Fetch staging `main`'s HEAD commit SHA via the GitHub API. One round-trip,
/// returns the 40-char SHA.
async fn fetch_staging_main_sha(
    client: &reqwest::Client,
    repo: &str,
) -> Result<String, String> {
    let url = format!("https://api.github.com/repos/{repo}/commits/main");
    let resp = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("GET {url}: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("staging main commit HTTP {}", resp.status()));
    }
    let parsed: GhCommit = resp
        .json()
        .await
        .map_err(|e| format!("parse staging main commit JSON: {e}"))?;
    let sha = parsed.sha.trim().to_string();
    if sha.len() < 40 {
        return Err(format!("unexpected SHA length from GH API: {sha:?}"));
    }
    Ok(sha)
}

/// Tauri command — decide whether to show the "Update from staging" pill.
/// Returns `None` (feature dark / silent) when:
///   * user is not `@getindigo.ai`,
///   * no GH token is available,
///   * GH API is unreachable.
/// Returns `Some(StagingReplaceInfo)` otherwise; `available=false` means
/// local is in sync with staging `main`.
#[tauri::command]
pub async fn check_staging_replace_available() -> Option<StagingReplaceInfo> {
    let eligible = is_eligible_email(signed_in_email().as_deref());
    let repo = resolve_staging_repo(eligible)?;
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

    let latest_sha = match fetch_staging_main_sha(&client, &repo).await {
        Ok(sha) => sha,
        Err(e) => {
            log("hq-core-staging", &format!("main HEAD fetch failed: {e}"));
            return None;
        }
    };

    let local_sha = local_last_sync_sha(&repo);
    let available = match local_sha.as_deref() {
        Some(local) => local != latest_sha,
        None => true, // no stamp -> never synced via the script -> offer update
    };

    let latest_short = latest_sha.chars().take(7).collect::<String>();
    log(
        "hq-core-staging",
        &format!(
            "replace-from-staging check: repo={repo} local={:?} latest={} available={}",
            local_sha, latest_short, available
        ),
    );
    Some(StagingReplaceInfo {
        available,
        local_sha,
        latest_sha,
        latest_short,
        repo,
    })
}

/// Resolve the bundled rescue script via Tauri's resource API. In dev
/// (`cargo run` / `tauri dev`) the script ships at `_up_/scripts/...` under
/// the resource dir (Tauri rewrites `../scripts/...` to `_up_/scripts/...`
/// during resource staging). In packaged builds the bundler places it in
/// the .app's `Resources/` directory under the same relative path.
fn resolve_rescue_script(app: &AppHandle) -> Result<std::path::PathBuf, String> {
    let candidates = [
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
    // Last-resort dev fallback: the cwd may be the repo root when
    // running `cargo run` directly without going through `tauri dev`.
    let cwd_fallback = std::env::current_dir()
        .ok()
        .map(|c| c.join("scripts").join("replace-from-staging-rescue.sh"));
    if let Some(p) = cwd_fallback {
        if p.is_file() {
            return Ok(p);
        }
    }
    Err(format!(
        "replace-from-staging-rescue.sh not found in resource dir (looked at: {:?})",
        candidates
    ))
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
    let script = resolve_rescue_script(&app)?;

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
    let log_tail = tail_log(&log_path, 40).unwrap_or_else(|e| format!("(log tail unavailable: {e})"));

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
fn tail_log(path: &std::path::Path, n_lines: usize) -> Result<String, String> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| format!("read {}: {e}", path.display()))?;
    let lines: Vec<&str> = content.lines().collect();
    let start = lines.len().saturating_sub(n_lines);
    Ok(lines[start..].join("\n"))
}

// ── Staging-aware drift count ────────────────────────────────────────────────
//
// Parallel to `hq_core_drift::check_once` but compares the user's locked-scope
// files against staging `main` instead of the released `v{hqVersion}` tag.
// `@getindigo.ai`-only — non-eligible users see release drift (the existing
// pill) unchanged.
//
// Reuses the same data model (`DriftReport` + `DriftEntry`) so the frontend
// can plug the staging report into the same pill + detail-window plumbing
// without forking the UI. Emits on `hq-core-staging-drift:available` (distinct
// event so App.svelte can route to a separate state slot).
//
// Method (lifted from hq_core_drift):
//   1. `GET /repos/{repo}/git/trees/main?recursive=1` — entire staging tree
//      in one JSON, each blob carries its git SHA-1.
//   2. Walk local under `rules.locked` scopes via `walk_local_under_scope`.
//   3. Set-difference upstream-vs-local to produce modified/missing/added.
//
// Cost: one authed API call + a local walk over ~hundreds of files. Cheap
// enough to run every 6h on the same offset cadence as the release-drift
// background loop. NO full clone — this is a count pill, not the rescue.

const STAGING_DRIFT_INITIAL_DELAY: Duration = Duration::from_secs(40);
// 30 min — faster than the 6h family default. The staging channel moves
// many times per day for active @getindigo.ai builders, so a stale pill is
// a near-constant footgun. One authed trees-API call per cycle (no clone,
// no walk over the whole HQ tree — just `rules.locked` scope) keeps the
// cost negligible vs. the release-drift checker's hourly rhythm.
const STAGING_DRIFT_CHECK_INTERVAL: Duration = Duration::from_secs(1800);

/// `GET /repos/:owner/:repo/git/trees/main?recursive=1` shaped for drift
/// computation: path → (blob_sha, size). The hq_core_staging `fetch_main_tree`
/// (used by the PR-classifier) returns a different shape — path → SetOf<sha>
/// — because the classifier indexes EVERY blob ever at that path across
/// `main` + open PRs. For drift we only need the current `main` SHA + size.
async fn fetch_staging_main_tree_for_drift(
    client: &reqwest::Client,
    repo: &str,
) -> Result<BTreeMap<String, (String, u64)>, String> {
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
            "hq-core-staging-drift",
            "WARNING: staging main tree truncated — drift count is a lower bound",
        );
    }
    let mut out: BTreeMap<String, (String, u64)> = BTreeMap::new();
    for entry in parsed.tree {
        if entry.kind != "blob" {
            continue;
        }
        // Skip symlinks (mode `120000`) — see field comment on GhTreeEntry.
        if entry.mode.as_deref() == Some("120000") {
            continue;
        }
        // `size` is None for unusual entries (submodules, etc.); 0 is a
        // safe placeholder — the value only feeds the detail window's
        // display column.
        let size = entry.size.unwrap_or(0);
        out.insert(entry.path, (entry.sha, size));
    }
    Ok(out)
}

// `GhTreesResponse` and `GhTreeEntry` already live above for the PR-classifier
// build_index path; they include `size` even though that field is unused
// there. The drift fetch above reuses both via the same `Deserialize` impls.
//
// `size` IS used here, so make sure the trees-API entry shape carries it.
// (Defined further up — see `struct GhTreeEntry`.)
//
// NOTE: the existing GhTreeEntry currently has no `size` field — add one
// below as part of this patch.

/// Run one staging-drift check. Returns `Ok(None)` for the same fail-quiet
/// cases as the release-drift counterpart, plus the eligibility / token
/// gates from the rest of `hq_core_staging`.
pub async fn check_staging_drift_once(app: &AppHandle) -> Result<Option<DriftReport>, String> {
    let eligible = is_eligible_email(signed_in_email().as_deref());
    let Some(repo) = resolve_staging_repo(eligible) else {
        return Ok(None);
    };
    if !eligible && repo == DEFAULT_STAGING_REPO {
        return Ok(None);
    }
    let Some(token) = resolve_gh_token() else {
        return Ok(None);
    };
    let client = match authed_client(&token) {
        Ok(c) => c,
        Err(e) => {
            log("hq-core-staging-drift", &format!("client build failed: {e}"));
            return Ok(None);
        }
    };

    let hq_folder = resolve_hq_folder();
    let locked = read_locked_paths(&hq_folder);
    if locked.is_empty() {
        return Ok(None);
    }

    // The "hq_version" field in DriftReport is mirrored back to the renderer
    // so the detail window can label "drift vs <ref>". For staging drift we
    // overload it with the source@ref shorthand — the existing renderer
    // doesn't parse it, just displays as a tag.
    let report_ref_label = format!("{repo}@main");

    let upstream = match fetch_staging_main_tree_for_drift(&client, &repo).await {
        Ok(m) => m,
        Err(e) => {
            log(
                "hq-core-staging-drift",
                &format!("main tree fetch failed: {e}"),
            );
            return Ok(None);
        }
    };
    let excluded = excluded_scope_paths();
    let local = walk_local_under_scope(&hq_folder, &locked);

    let upstream_in_scope: BTreeMap<String, (String, u64)> = upstream
        .into_iter()
        .filter(|(path, _)| path_in_locked_scope(path, &locked))
        .filter(|(path, _)| !path_in_excluded_scope(path, &excluded))
        .collect();

    let local: BTreeMap<String, (String, u64)> = local
        .into_iter()
        .filter(|(path, _)| !path_in_excluded_scope(path, &excluded))
        .filter(|(path, _)| !is_conflict_artifact(path))
        .collect();

    let upstream_paths: BTreeSet<&String> = upstream_in_scope.keys().collect();
    let local_paths: BTreeSet<&String> = local.keys().collect();

    let mut modified = Vec::new();
    let mut missing = Vec::new();
    let mut added = Vec::new();

    for path in upstream_paths.intersection(&local_paths) {
        let (sha_up, _size_up) = &upstream_in_scope[*path];
        let (sha_local, size_local) = &local[*path];
        if sha_up != sha_local {
            modified.push(DriftEntry {
                path: (*path).clone(),
                size: *size_local,
                git_sha_local: Some(sha_local.clone()),
                git_sha_upstream: Some(sha_up.clone()),
                staging_status: None,
            });
        }
    }
    for path in upstream_paths.difference(&local_paths) {
        let (sha_up, size_up) = &upstream_in_scope[*path];
        missing.push(DriftEntry {
            path: (*path).clone(),
            size: *size_up,
            git_sha_local: None,
            git_sha_upstream: Some(sha_up.clone()),
            staging_status: None,
        });
    }
    for path in local_paths.difference(&upstream_paths) {
        let (sha_local, size_local) = &local[*path];
        added.push(DriftEntry {
            path: (*path).clone(),
            size: *size_local,
            git_sha_local: Some(sha_local.clone()),
            git_sha_upstream: None,
            staging_status: None,
        });
    }

    let count = modified.len() + missing.len() + added.len();
    let report = DriftReport {
        count,
        modified,
        missing,
        added,
        scanned_at: chrono::Utc::now().to_rfc3339(),
        hq_version: report_ref_label,
    };

    log(
        "hq-core-staging-drift",
        &format!(
            "check: repo={} count={} (modified={}, missing={}, added={})",
            repo,
            report.count,
            report.modified.len(),
            report.missing.len(),
            report.added.len()
        ),
    );

    // Always emit so the frontend can swing the count back to zero on
    // re-check after a rescue run (same posture as hq-core-drift). The
    // event name is distinct so App.svelte can route to its own state slot
    // without entangling with the release-drift listener.
    let _ = app.emit("hq-core-staging-drift:available", &report);

    // Keep the detail window in sync if it's open. Same pattern as
    // hq_core_drift — emit_to no-ops when the window doesn't exist.
    if let Some(state) = app.try_state::<crate::commands::drift_detail::PendingDrift>() {
        *state.0.lock().unwrap() = Some(report.clone());
    }
    let _ = app.emit_to(
        crate::commands::drift_detail::WINDOW_LABEL,
        "drift:report",
        &report,
    );

    Ok(Some(report))
}

/// Tauri command — synchronous one-shot staging-drift check. Mirrors
/// `hq_core_drift::check_hq_core_drift`. Returns `None` for ineligible
/// users / dark feature so the frontend can route conditionally without
/// distinguishing failure modes.
#[tauri::command]
pub async fn check_staging_drift(app: AppHandle) -> Result<Option<DriftReport>, String> {
    check_staging_drift_once(&app).await
}

/// Background loop: first check 40s after launch (offset from the
/// release-drift checker's 30s), then every 6h. Logs but doesn't
/// propagate errors — a flaky network or expired token shouldn't kill
/// the loop, and a future re-check can succeed once the user runs
/// `gh auth login` etc.
pub fn setup_staging_drift_checker(app: &AppHandle) {
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(STAGING_DRIFT_INITIAL_DELAY).await;
        loop {
            if let Err(e) = check_staging_drift_once(&handle).await {
                log(
                    "hq-core-staging-drift",
                    &format!("background check failed: {e}"),
                );
            }
            tokio::time::sleep(STAGING_DRIFT_CHECK_INTERVAL).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn idx() -> StagingIndex {
        let mut main = BTreeMap::new();
        main.insert("a.md".to_string(), BTreeSet::from(["sha-main-a".to_string()]));

        let mut pr182 = BTreeMap::new();
        pr182.insert("b.md".to_string(), BTreeSet::from(["sha-182-b".to_string()]));
        pr182.insert("shared.md".to_string(), BTreeSet::from(["sha-shared".to_string()]));

        let mut pr183 = BTreeMap::new();
        pr183.insert("c.md".to_string(), BTreeSet::from(["sha-183-c".to_string()]));
        pr183.insert("shared.md".to_string(), BTreeSet::from(["sha-shared".to_string()]));

        StagingIndex {
            main,
            prs: vec![(183, pr183), (182, pr182)],
        }
    }

    #[test]
    fn classify_main_match() {
        assert_eq!(idx().classify("a.md", "sha-main-a"), StagingStatus::StagingMain);
    }

    #[test]
    fn classify_pr_match() {
        assert_eq!(idx().classify("b.md", "sha-182-b"), StagingStatus::StagingPr(182));
        assert_eq!(idx().classify("c.md", "sha-183-c"), StagingStatus::StagingPr(183));
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
        assert_eq!(idx().classify("a.md", "different-sha"), StagingStatus::Unaccounted);
        assert_eq!(idx().classify("missing.md", "whatever"), StagingStatus::Unaccounted);
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
