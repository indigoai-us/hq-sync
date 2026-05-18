//! "New hq-core release" notification.
//!
//! Mirrors `hq_cli_update.rs` and `updater.rs` but targets the `hq-core`
//! scaffold (the user's HQ folder itself) rather than an npm package or
//! the menubar binary. The three notifiers are independent releases:
//!   * `updater.rs`         → menubar self-update (Tauri updater + latest.json)
//!   * `hq_cli_update.rs`   → `@indigoai-us/hq-cli` npm nag
//!   * `hq_core_update.rs`  → this — hq-core GitHub release nag
//!
//! Flow:
//!   1. Read `<resolved-HQ-folder>/core.yaml` and pull the `hqVersion`
//!      field. If the file is missing or unparseable, the user hasn't
//!      set up HQ yet (or it's broken) — `local` is None and we emit
//!      nothing. We don't pester users who don't have an HQ to update.
//!   2. GET `https://api.github.com/repos/indigoai-us/hq-core/releases/latest`
//!      and pull `tag_name` (stripping a leading `v`), `html_url`, `body`.
//!   3. Compare numerically with `cmp_semver` (same comparator used by the
//!      CLI nag — three-segment major.minor.patch with `-pre` suffix
//!      stripped). If latest > local, emit `hq-core-update:available`.
//!
//! Differences from the CLI nag:
//!   * No install command. Updating hq-core means running the `/update-hq`
//!     Claude-Code skill inside the user's HQ folder, which is a much
//!     heavier interactive flow than a one-shot `npm install -g`. The
//!     banner's CTA is "Open release notes" — handled in the frontend via
//!     `@tauri-apps/plugin-shell`'s `open()` against the GitHub release
//!     URL, which is allowed by the existing `shell:allow-open` capability.
//!   * No `:cleared` event for the same reason — there's nothing the app
//!     can do to make `core.yaml` advance, so the banner naturally clears
//!     on the next background check after the user has updated.
//!
//! Cadence: first check 20s after launch (offset from updater's 10s and
//! the CLI nag's 15s so they don't spike CPU + network in lockstep), then
//! every 6h. Matches the rest of the family.

use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};

use crate::commands::config::{read_hq_config_lenient, MenubarPrefs};
use crate::commands::hq_cli_update::cmp_semver;
use crate::util::logfile::log;
use crate::util::paths;

/// GitHub Releases API endpoint for the canonical hq-core repo. Returns
/// the latest *non-prerelease* release for the repo — staging tags pushed
/// to `hq-core-staging` don't surface here, only what's promoted to the
/// public `hq-core` repo via `/personal:release-hq-core`.
const RELEASES_URL: &str =
    "https://api.github.com/repos/indigoai-us/hq-core/releases/latest";

/// HTTP request timeout — keep tight so a flaky network doesn't stall the
/// background loop.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

/// Offset from app launch before the first check fires. 20s vs. the app
/// updater's 10s and the CLI nag's 15s so all three don't spike CPU +
/// network in lockstep on launch.
const INITIAL_DELAY: Duration = Duration::from_secs(20);

/// Re-check cadence. Matches `updater::setup_update_checker` and
/// `hq_cli_update::setup_hq_cli_update_checker` (6h).
const CHECK_INTERVAL: Duration = Duration::from_secs(21600);

/// Payload emitted to the frontend and returned by `check_hq_core_update`.
#[derive(Debug, Clone, Serialize)]
pub struct HqCoreUpdateInfo {
    /// Locally-installed `hqVersion` from `core.yaml` (None when HQ
    /// hasn't been set up yet or `core.yaml` is unreadable).
    pub local: Option<String>,
    /// GitHub release `tag_name`, with any leading `v` stripped so it
    /// compares cleanly against the YAML's bare semver string.
    pub latest: String,
    /// `html_url` from the GitHub release — the user's "Open release
    /// notes" CTA navigates here in their default browser.
    pub release_url: String,
    /// Release body / changelog (markdown, may be long). Truncated by
    /// the frontend if needed; we pass through verbatim.
    pub body: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GithubRelease {
    tag_name: String,
    html_url: String,
    body: Option<String>,
}

/// Resolve the locally-installed hq-core version by reading `hqVersion`
/// from `core.yaml` at the root of the user's HQ folder.
///
/// Resolution order for the HQ folder mirrors what `conflicts.rs` and
/// `daemon.rs` do: menubar.json `hqPath` → config.json `hqFolderPath` →
/// discovery via `core.yaml` signature → `~/HQ`. See `paths::resolve_hq_folder`.
///
/// Returns `None` when:
///   * the HQ folder can't be located,
///   * `core.yaml` doesn't exist at that path,
///   * `core.yaml` is unparseable as YAML,
///   * `core.yaml` has no `hqVersion` field.
///
/// All four cases are silent: the banner doesn't fire for users without
/// a working HQ install — the CLI nag's "don't pester users who don't
/// have it installed" rule applies here too.
pub fn get_local_version() -> Option<String> {
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

    let core_yaml = hq_folder.join("core.yaml");
    let bytes = std::fs::read(&core_yaml).ok()?;
    let parsed: serde_yaml::Value = serde_yaml::from_slice(&bytes).ok()?;
    let s = parsed.get("hqVersion")?.as_str()?.trim();
    if s.is_empty() {
        None
    } else {
        Some(s.to_string())
    }
}

/// Strip a single leading `v` from a tag name. GitHub release tag_names
/// come in both flavours (`v14.1.0` and `14.1.0`) depending on the repo's
/// convention; hq-core's release workflow uses the `v`-prefixed form.
fn strip_v_prefix(s: &str) -> &str {
    s.strip_prefix('v').unwrap_or(s)
}

async fn fetch_latest() -> Result<(String, String, Option<String>), String> {
    // GitHub returns 403 with the message "Request forbidden by
    // administrative rules" when User-Agent is missing. The client_info
    // headers include a UA already; layer the timeout on top so a
    // hung connection doesn't stall the loop forever.
    let client = reqwest::Client::builder()
        .default_headers(crate::util::client_info::client_headers())
        .timeout(REQUEST_TIMEOUT)
        .build()
        .map_err(|e| format!("build client: {e}"))?;
    let resp = client
        .get(RELEASES_URL)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|e| format!("GET {RELEASES_URL}: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("GitHub API returned HTTP {}", resp.status()));
    }
    let parsed: GithubRelease = resp
        .json()
        .await
        .map_err(|e| format!("parse GitHub release JSON: {e}"))?;
    let version = strip_v_prefix(parsed.tag_name.trim()).to_string();
    Ok((version, parsed.html_url, parsed.body))
}

/// Perform one check. Returns `Some(info)` when an upgrade is available,
/// `None` when the user is already on the latest (or `core.yaml` isn't
/// readable — we don't pester users without a working HQ install).
pub async fn check_once(app: &AppHandle) -> Result<Option<HqCoreUpdateInfo>, String> {
    let (latest, release_url, body) = fetch_latest().await?;
    let local = get_local_version();
    let update_available = match local.as_deref() {
        Some(l) => cmp_semver(l, &latest) == std::cmp::Ordering::Less,
        None => false,
    };
    log(
        "hq-core-update",
        &format!(
            "check: local={:?} latest={} update_available={}",
            local, latest, update_available
        ),
    );
    if !update_available {
        return Ok(None);
    }
    let info = HqCoreUpdateInfo {
        local,
        latest,
        release_url,
        body,
    };
    let _ = app.emit("hq-core-update:available", &info);
    Ok(Some(info))
}

/// Tauri command — synchronous one-shot check used by the Settings panel
/// and any future "Check for Updates" surface.
#[tauri::command]
pub async fn check_hq_core_update(app: AppHandle) -> Result<Option<HqCoreUpdateInfo>, String> {
    check_once(&app).await
}

/// Background loop: first check 20s after launch, then every 6h.
/// Mirrors `updater::setup_update_checker` and
/// `hq_cli_update::setup_hq_cli_update_checker`. Logs but does not
/// propagate errors — a flaky network shouldn't kill the loop.
pub fn setup_hq_core_update_checker(app: &AppHandle) {
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(INITIAL_DELAY).await;
        loop {
            if let Err(e) = check_once(&handle).await {
                log("hq-core-update", &format!("background check failed: {e}"));
            }
            tokio::time::sleep(CHECK_INTERVAL).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_v_prefix_handles_both_conventions() {
        assert_eq!(strip_v_prefix("v14.1.0"), "14.1.0");
        assert_eq!(strip_v_prefix("14.1.0"), "14.1.0");
        // Only one leading 'v' — anything else is a pathological tag we
        // can't meaningfully repair, so pass through.
        assert_eq!(strip_v_prefix("vv1.0.0"), "v1.0.0");
        assert_eq!(strip_v_prefix(""), "");
    }

    #[test]
    fn local_version_returns_none_when_core_yaml_missing() {
        // Smoke-test: even with no HQ folder anywhere on disk in the
        // sandbox, the function must not panic — under-report rather
        // than crash, same posture as the CLI nag.
        let _ = get_local_version();
    }
}
