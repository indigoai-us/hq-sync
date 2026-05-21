//! Core-files drift check.
//!
//! Tells the user "your local hq-core files differ from what your installed
//! `hqVersion` shipped." Surfaces as a small `N drifted` pill on the HQ-
//! version footer row of the popover; clicking opens a detail window
//! (see `drift_detail.rs`) listing the modified / missing / added files
//! with per-row actions.
//!
//! Family alignment with the other two notifiers:
//!   * `updater.rs`            → menubar self-update
//!   * `hq_cli_update.rs`      → `@indigoai-us/hq-cli` npm nag
//!   * `hq_core_update.rs`     → "new hq-core release" nag (version-only)
//!   * `hq_core_drift.rs`      → this — file-content drift vs the version
//!                                the user is *currently* on (orthogonal to
//!                                the update nag: drift is "what changed
//!                                under you" vs. update is "what's newer
//!                                upstream").
//!
//! Source of truth for "upstream content":
//!   `GET https://api.github.com/repos/indigoai-us/hq-core/git/trees/v{hqVersion}?recursive=1`
//! returns the entire tree as one JSON, each blob carrying its git SHA-1
//! (`sha1("blob {len}\0{content}")`). We compute the same hash locally
//! for each file under the locked paths and compare — no content download
//! is needed unless the user later clicks Restore.
//!
//! Why git-trees instead of the tarball: one API call, no archive parsing,
//! no transitive crate weight (only +`sha1`). Locked-paths scope is
//! ~9 entries / few hundred files, well under the 7000-entry tree cap.
//!
//! Cadence: first check 30s after launch (offset from updater 10s / CLI
//! 15s / core-update 20s so we don't spike the loop), then every 6h.
//!
//! Scope: `rules.locked` from the user's local `core.yaml`. Reviewable
//! and auto-generated paths (`core/workers/registry.yaml`, `workspace/`,
//! `.claude/skills/`) are intentionally excluded — they're expected to
//! diverge.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use sha1::{Digest, Sha1};
use tauri::{AppHandle, Emitter};

use crate::commands::config::{read_hq_config_lenient, MenubarPrefs};
use crate::commands::hq_core_update::get_local_version;
use crate::util::logfile::log;
use crate::util::paths;

/// GitHub trees-API endpoint template for the hq-core repo. `{tag}` is
/// substituted at call time with the user's local `hqVersion`, e.g.
/// `v14.2.1`. The recursive=1 query expands the entire subtree in a
/// single response (capped at 7000 entries by GitHub — hq-core is
/// ~hundreds of files, comfortably under).
const TREES_URL: &str =
    "https://api.github.com/repos/indigoai-us/hq-core/git/trees/v{tag}?recursive=1";

/// Raw-content fetch endpoint for restore actions. Uses the codeload
/// CDN (raw.githubusercontent.com) rather than the API to avoid burning
/// the 60/hr rate limit on bulk restores. `{tag}` and `{path}` are
/// substituted at call time.
const RAW_URL: &str =
    "https://raw.githubusercontent.com/indigoai-us/hq-core/v{tag}/{path}";

const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
const INITIAL_DELAY: Duration = Duration::from_secs(30);
const CHECK_INTERVAL: Duration = Duration::from_secs(21600); // 6h

/// One drift entry. `git_sha_upstream` is `None` for ADDED (no upstream
/// counterpart); `git_sha_local` is `None` for MISSING (no local copy).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DriftEntry {
    /// Path relative to the HQ folder (e.g. `core/policies/foo.md`).
    pub path: String,
    /// File size in bytes — local for modified/added, upstream for missing.
    pub size: u64,
    /// Local git-blob SHA-1, or None when the file is missing locally.
    pub git_sha_local: Option<String>,
    /// Upstream git-blob SHA-1, or None when the path is locally-added
    /// (not part of upstream at the current hqVersion).
    pub git_sha_upstream: Option<String>,
}

/// Drift summary emitted to the frontend. `count` is the total of the
/// three lists, used by the popover footer pill.
///
/// `Deserialize` is required (not just `Serialize`) because the same struct
/// round-trips through the renderer when the user clicks the drift pill:
/// frontend `invoke('open_drift_detail', { report })` passes the cached
/// payload back to Rust, which stashes it in `PendingDrift` for the detail
/// window's `*_ready` handshake. Round-tripping the report (vs. re-fetching
/// from GitHub) avoids burning a second API call on every pill click and
/// guarantees the detail window's count matches the pill's exactly.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DriftReport {
    /// Total number of drifted files (modified + missing + added).
    pub count: usize,
    /// Files present in both but with differing content.
    pub modified: Vec<DriftEntry>,
    /// Files in upstream but not local (deleted or never synced).
    pub missing: Vec<DriftEntry>,
    /// Files local under a locked-path prefix but absent upstream
    /// (locally-added under a scope that shouldn't be added to).
    pub added: Vec<DriftEntry>,
    /// ISO-8601 timestamp of when the scan ran.
    pub scanned_at: String,
    /// The hqVersion the report was computed against. Mirrored so the
    /// frontend can detect a stale cached report after `/update-hq` runs.
    pub hq_version: String,
}

/// GitHub trees-API response shape. We only care about `tree[].path` and
/// `tree[].sha`; the rest is ignored.
#[derive(Debug, Deserialize)]
struct GhTreesResponse {
    tree: Vec<GhTreeEntry>,
    /// True when the recursive listing was truncated at the 7000-entry
    /// cap. Logged as a warning if hit — should never happen for hq-core
    /// at current size, but if it does the drift count is a lower bound.
    #[serde(default)]
    truncated: bool,
}

#[derive(Debug, Deserialize)]
struct GhTreeEntry {
    path: String,
    /// `"blob"` for files, `"tree"` for directories. We only diff blobs.
    #[serde(rename = "type")]
    kind: String,
    sha: String,
    #[serde(default)]
    size: Option<u64>,
}

/// Read `rules.locked` from the user's local `core.yaml`. Returns an empty
/// vec if the file is missing/unparseable — same fail-quiet posture as
/// `get_local_version` (don't pester users without a working HQ).
///
/// Returns relative paths exactly as they appear in `core.yaml` (e.g.
/// `.claude/CLAUDE.md`, `core/policies/`). Trailing-slash convention
/// indicates "this directory and everything under it"; leaf paths are
/// single-file scopes.
fn read_locked_paths(hq_folder: &Path) -> Vec<String> {
    let canonical = hq_folder.join("core").join("core.yaml");
    let legacy = hq_folder.join("core.yaml");
    let core_yaml = if canonical.is_file() { canonical } else { legacy };
    let Ok(bytes) = std::fs::read(&core_yaml) else {
        return Vec::new();
    };
    let Ok(parsed) = serde_yaml::from_slice::<serde_yaml::Value>(&bytes) else {
        return Vec::new();
    };
    let Some(locked) = parsed.get("rules").and_then(|r| r.get("locked")) else {
        return Vec::new();
    };
    let Some(arr) = locked.as_sequence() else {
        return Vec::new();
    };
    arr.iter()
        .filter_map(|v| v.as_str().map(|s| s.to_string()))
        .collect()
}

/// True iff the given upstream path (from the GitHub trees response)
/// falls under one of the locked-path prefixes. Trailing-slash entries
/// are dir-prefix matches; non-slash entries are exact-path matches.
fn path_in_locked_scope(path: &str, locked: &[String]) -> bool {
    for scope in locked {
        if let Some(prefix) = scope.strip_suffix('/') {
            // Directory scope: prefix-match on the dir + '/' so
            // `core/policies/` doesn't match `core/policies-extra.md`.
            if path == prefix || path.starts_with(&format!("{}/", prefix)) {
                return true;
            }
        } else if path == scope {
            return true;
        }
    }
    false
}

/// Compute git's blob SHA-1 for a byte slice. Git's content-addressable
/// blob hash is `sha1("blob " + content_length_decimal + "\0" + content)`
/// — matches what the GitHub trees API returns as `sha` for blob entries.
fn git_blob_sha(content: &[u8]) -> String {
    let mut hasher = Sha1::new();
    hasher.update(format!("blob {}\0", content.len()).as_bytes());
    hasher.update(content);
    let digest = hasher.finalize();
    let mut hex = String::with_capacity(40);
    for byte in digest.iter() {
        hex.push_str(&format!("{:02x}", byte));
    }
    hex
}

/// Walk the local HQ folder, restricted to the locked-path scopes, and
/// return a map of relative-path → (sha1, size). Walks files directly
/// for leaf scopes; uses walkdir for directory scopes. Symlinks are
/// followed but never escape the HQ root.
fn walk_local_under_scope(hq_folder: &Path, locked: &[String]) -> BTreeMap<String, (String, u64)> {
    let mut out = BTreeMap::new();
    for scope in locked {
        let (rel, is_dir) = if let Some(prefix) = scope.strip_suffix('/') {
            (prefix.to_string(), true)
        } else {
            (scope.clone(), false)
        };
        let abs = hq_folder.join(&rel);
        if is_dir {
            if !abs.is_dir() {
                continue;
            }
            for entry in walkdir::WalkDir::new(&abs)
                .follow_links(false)
                .into_iter()
                .filter_map(|e| e.ok())
            {
                if !entry.file_type().is_file() {
                    continue;
                }
                let Ok(rel_path) = entry.path().strip_prefix(hq_folder) else {
                    continue;
                };
                let Ok(content) = std::fs::read(entry.path()) else {
                    continue;
                };
                let size = content.len() as u64;
                let sha = git_blob_sha(&content);
                let rel_str = rel_path.to_string_lossy().replace('\\', "/");
                out.insert(rel_str, (sha, size));
            }
        } else if abs.is_file() {
            if let Ok(content) = std::fs::read(&abs) {
                let size = content.len() as u64;
                let sha = git_blob_sha(&content);
                out.insert(rel.clone(), (sha, size));
            }
        }
    }
    out
}

async fn fetch_upstream_tree(hq_version: &str) -> Result<BTreeMap<String, (String, u64)>, String> {
    let url = TREES_URL.replace("{tag}", hq_version);
    let client = reqwest::Client::builder()
        .default_headers(crate::util::client_info::client_headers())
        .timeout(REQUEST_TIMEOUT)
        .build()
        .map_err(|e| format!("build client: {e}"))?;
    let resp = client
        .get(&url)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|e| format!("GET {url}: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("GitHub API returned HTTP {}", resp.status()));
    }
    let parsed: GhTreesResponse = resp
        .json()
        .await
        .map_err(|e| format!("parse trees JSON: {e}"))?;
    if parsed.truncated {
        log(
            "hq-core-drift",
            "WARNING: GitHub trees response truncated — drift count is a lower bound",
        );
    }
    let mut out = BTreeMap::new();
    for entry in parsed.tree {
        if entry.kind != "blob" {
            continue;
        }
        out.insert(entry.path, (entry.sha, entry.size.unwrap_or(0)));
    }
    Ok(out)
}

/// Run one drift check. Returns `Ok(None)` when we can't compute a
/// report (no hqVersion, network failure) — same fail-quiet posture as
/// the other notifiers.
pub async fn check_once(app: &AppHandle) -> Result<Option<DriftReport>, String> {
    let Some(hq_version) = get_local_version() else {
        return Ok(None);
    };

    let menubar_prefs: Option<MenubarPrefs> = paths::menubar_json_path()
        .ok()
        .filter(|p| p.exists())
        .and_then(|p| std::fs::read_to_string(&p).ok())
        .and_then(|s| serde_json::from_str(&s).ok());
    let config = read_hq_config_lenient().ok().flatten();
    let hq_folder = paths::resolve_hq_folder(
        config.as_ref().and_then(|c| c.hq_folder_path.as_deref()),
        menubar_prefs.as_ref().and_then(|p| p.hq_path.as_deref()),
    );

    let locked = read_locked_paths(&hq_folder);
    if locked.is_empty() {
        return Ok(None);
    }

    let upstream = fetch_upstream_tree(&hq_version).await?;
    let local = walk_local_under_scope(&hq_folder, &locked);

    let upstream_in_scope: BTreeMap<String, (String, u64)> = upstream
        .into_iter()
        .filter(|(path, _)| path_in_locked_scope(path, &locked))
        .collect();

    let upstream_paths: BTreeSet<&String> = upstream_in_scope.keys().collect();
    let local_paths: BTreeSet<&String> = local.keys().collect();

    let mut modified = Vec::new();
    let mut missing = Vec::new();
    let mut added = Vec::new();

    for path in upstream_paths.intersection(&local_paths) {
        let (sha_up, size_up) = &upstream_in_scope[*path];
        let (sha_local, size_local) = &local[*path];
        if sha_up != sha_local {
            modified.push(DriftEntry {
                path: (*path).clone(),
                size: *size_local,
                git_sha_local: Some(sha_local.clone()),
                git_sha_upstream: Some(sha_up.clone()),
            });
            // size_up is unused for modified — we already report local size.
            let _ = size_up;
        }
    }
    for path in upstream_paths.difference(&local_paths) {
        let (sha_up, size_up) = &upstream_in_scope[*path];
        missing.push(DriftEntry {
            path: (*path).clone(),
            size: *size_up,
            git_sha_local: None,
            git_sha_upstream: Some(sha_up.clone()),
        });
    }
    for path in local_paths.difference(&upstream_paths) {
        let (sha_local, size_local) = &local[*path];
        added.push(DriftEntry {
            path: (*path).clone(),
            size: *size_local,
            git_sha_local: Some(sha_local.clone()),
            git_sha_upstream: None,
        });
    }

    let count = modified.len() + missing.len() + added.len();
    let report = DriftReport {
        count,
        modified,
        missing,
        added,
        scanned_at: chrono::Utc::now().to_rfc3339(),
        hq_version,
    };

    log(
        "hq-core-drift",
        &format!(
            "check: hq_version={} count={} (modified={}, missing={}, added={})",
            report.hq_version,
            report.count,
            report.modified.len(),
            report.missing.len(),
            report.added.len()
        ),
    );

    // Always emit so the frontend can update from a stale count back to
    // zero when the user has since reconciled. Unlike the update nag
    // which only emits on the unhappy path, drift state can swing in
    // both directions on a re-check.
    let _ = app.emit("hq-core-drift:available", &report);
    Ok(Some(report))
}

/// Tauri command — synchronous one-shot drift check. Used by the popover
/// pill's "Refresh" affordance and by the detail window on mount.
#[tauri::command]
pub async fn check_hq_core_drift(app: AppHandle) -> Result<Option<DriftReport>, String> {
    check_once(&app).await
}

/// Background loop: first check 30s after launch (offset from updater
/// 10s / CLI 15s / core-update 20s), then every 6h. Logs but does not
/// propagate errors — a flaky network shouldn't kill the loop.
pub fn setup_hq_core_drift_checker(app: &AppHandle) {
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(INITIAL_DELAY).await;
        loop {
            if let Err(e) = check_once(&handle).await {
                log("hq-core-drift", &format!("background check failed: {e}"));
            }
            tokio::time::sleep(CHECK_INTERVAL).await;
        }
    });
}

/// Tauri command — restore a single file from upstream by overwriting
/// the local copy with the content at `v{hqVersion}` on GitHub.
///
/// Safety:
///   * `path` is constrained to live under one of `rules.locked` (the
///     same scopes the drift scan considers). Anything else returns
///     `Err` without touching disk — guards against a renderer being
///     tricked into requesting arbitrary writes.
///   * The local file is created if missing (Missing case) and
///     overwritten if present (Modified case).
///   * Added-only paths cannot be "restored" because there is no
///     upstream to restore from — callers should not invoke this for
///     them, and the function rejects with `Err("not in upstream scope")`.
///   * The fetched content is hash-verified against the expected
///     upstream blob SHA when provided, so a CDN poisoning or wrong-tag
///     fetch surfaces as an error instead of a silent bad write.
#[tauri::command]
pub async fn restore_from_upstream(
    path: String,
    expected_upstream_sha: Option<String>,
) -> Result<(), String> {
    let Some(hq_version) = get_local_version() else {
        return Err("hqVersion not detectable — cannot resolve upstream tag".into());
    };

    let menubar_prefs: Option<MenubarPrefs> = paths::menubar_json_path()
        .ok()
        .filter(|p| p.exists())
        .and_then(|p| std::fs::read_to_string(&p).ok())
        .and_then(|s| serde_json::from_str(&s).ok());
    let config = read_hq_config_lenient().ok().flatten();
    let hq_folder = paths::resolve_hq_folder(
        config.as_ref().and_then(|c| c.hq_folder_path.as_deref()),
        menubar_prefs.as_ref().and_then(|p| p.hq_path.as_deref()),
    );

    let locked = read_locked_paths(&hq_folder);
    if !path_in_locked_scope(&path, &locked) {
        return Err(format!("path {path:?} not in locked-path scope"));
    }

    // Guard against absolute paths, ".." traversal, or weird shapes.
    let normalised = Path::new(&path);
    if normalised.is_absolute() || normalised.components().any(|c| matches!(c, std::path::Component::ParentDir)) {
        return Err(format!("path {path:?} is not a safe relative path"));
    }

    let url = RAW_URL
        .replace("{tag}", &hq_version)
        .replace("{path}", &path);
    let client = reqwest::Client::builder()
        .default_headers(crate::util::client_info::client_headers())
        .timeout(REQUEST_TIMEOUT)
        .build()
        .map_err(|e| format!("build client: {e}"))?;
    let resp = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("GET {url}: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("upstream returned HTTP {}", resp.status()));
    }
    let bytes = resp
        .bytes()
        .await
        .map_err(|e| format!("read upstream body: {e}"))?;

    if let Some(expected) = expected_upstream_sha {
        let actual = git_blob_sha(&bytes);
        if actual != expected {
            return Err(format!(
                "upstream content sha mismatch: expected {expected}, got {actual} — refusing to write"
            ));
        }
    }

    let target: PathBuf = hq_folder.join(&path);
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("create parent dir for {target:?}: {e}"))?;
    }
    std::fs::write(&target, &bytes).map_err(|e| format!("write {target:?}: {e}"))?;
    log(
        "hq-core-drift",
        &format!("restored {} bytes to {}", bytes.len(), path),
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn git_blob_sha_matches_git_format() {
        // Empty blob's SHA-1 is the canonical e69de29bb2d1d6434b8b29ae775ad8c2e48c5391.
        let empty = git_blob_sha(b"");
        assert_eq!(empty, "e69de29bb2d1d6434b8b29ae775ad8c2e48c5391");

        // "hello\n" blob's SHA-1 is ce013625030ba8dba906f756967f9e9ca394464a.
        let hello = git_blob_sha(b"hello\n");
        assert_eq!(hello, "ce013625030ba8dba906f756967f9e9ca394464a");
    }

    #[test]
    fn locked_scope_matches_dir_prefix_and_exact_file() {
        let locked = vec![
            "core/policies/".to_string(),
            ".claude/CLAUDE.md".to_string(),
        ];
        // Dir-prefix matches.
        assert!(path_in_locked_scope("core/policies/foo.md", &locked));
        assert!(path_in_locked_scope("core/policies/sub/bar.md", &locked));
        assert!(path_in_locked_scope("core/policies", &locked));
        // Dir-prefix must not bleed onto sibling paths.
        assert!(!path_in_locked_scope("core/policies-extra.md", &locked));
        // Exact-file match.
        assert!(path_in_locked_scope(".claude/CLAUDE.md", &locked));
        // Anchored — substring of an exact-file scope shouldn't match.
        assert!(!path_in_locked_scope(".claude/CLAUDE.md.bak", &locked));
        // Out of scope.
        assert!(!path_in_locked_scope("workspace/threads/foo.json", &locked));
    }
}
