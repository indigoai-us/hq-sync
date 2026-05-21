//! "New hq-core-staging release" notification — staging channel.
//!
//! Companion to `hq_core_update.rs`. Same shape, same comparator, same
//! 6-hour cadence — only two differences:
//!
//! 1. **Target repo:** `indigoai-us/hq-core-staging` instead of `hq-core`.
//!    Staging gets a fresh `v<hqVersion>-beta.<N>` tag pushed by the repo's
//!    own `auto-beta-release.yml` workflow on every push to main, so this
//!    surface exists specifically to let operators on the staging channel
//!    pick up those bleeding-edge betas without waiting for the public
//!    `hq-core` promotion.
//!
//! 2. **Feature-flagged:** every entry point checks the
//!    `staging_update_channel` field in `~/.hq/menubar.json` and no-ops
//!    when it's missing or false. Default off — operators have to flip
//!    the Settings toggle on to start receiving these notifications. This
//!    is what makes the toggle itself the feature flag: a user who never
//!    enabled it will never see a staging-update event fire.
//!
//! Flow when the flag is on:
//!   1. Read local `hqVersion` from `core/core.yaml` (same `get_local_version`
//!      helper as the prod notifier — kept distinct here to avoid pulling a
//!      cross-module re-export through a third call site).
//!   2. GET `/repos/indigoai-us/hq-core-staging/releases/latest`. GitHub's
//!      `/releases/latest` endpoint skips pre-releases by default, so we
//!      ask for the *all-releases* feed and take the first entry instead
//!      (`/releases?per_page=1`) — that's what surfaces the freshest
//!      auto-beta tag, which is precisely what this channel is for.
//!   3. Compare numerically with `cmp_semver` (same comparator used by the
//!      CLI nag and the prod notifier). If latest > local, emit
//!      `hq-core-staging-update:available`.
//!
//! Differences from the prod notifier:
//!   * No CTA URL — the frontend's update flow for this channel is the
//!     `apply_hq_core_staging` Tauri command, not "open release notes in
//!     the browser".
//!   * Background loop is offset by 30s vs. prod's 20s so they don't spike
//!     at the exact same moment when the toggle is on. (Same posture as
//!     the rest of the update-checker family — staggered startup.)

use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};

use crate::commands::config::MenubarPrefs;
use crate::commands::hq_cli_update::cmp_semver;
use crate::commands::hq_core_update::get_local_version;
use crate::util::logfile::log;
use crate::util::paths;

/// All-releases feed for hq-core-staging. We DON'T use `/releases/latest`
/// here because GitHub silently filters out pre-release tags from that
/// endpoint, and the staging channel exists *specifically* to surface
/// `*-beta.*` pre-releases. `per_page=1` returns just the most recently
/// published release of any flavour.
const RELEASES_URL: &str =
    "https://api.github.com/repos/indigoai-us/hq-core-staging/releases?per_page=1";

/// HTTP request timeout. Matches the prod notifier.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

/// First check fires 30s after launch (vs. updater's 10s, CLI nag's 15s,
/// hq-core's 20s) so the four notifier loops don't fire in lockstep on
/// startup.
const INITIAL_DELAY: Duration = Duration::from_secs(30);

/// Re-check cadence — 6h, matching the rest of the notifier family.
const CHECK_INTERVAL: Duration = Duration::from_secs(21600);

/// Payload emitted to the frontend + returned by `check_hq_core_staging_update`.
#[derive(Debug, Clone, Serialize)]
pub struct HqCoreStagingUpdateInfo {
    /// Locally-installed `hqVersion` (None when no working HQ install — the
    /// staging notifier short-circuits in that case for the same reason the
    /// prod notifier does: don't pester users without a working HQ).
    pub local: Option<String>,
    /// Latest staging release tag, `v` prefix preserved so the frontend can
    /// render it verbatim. Use `latest_semver` for the comparison-ready
    /// form (no leading v).
    pub latest_tag: String,
    /// `latest_tag` with any leading `v` stripped — the comparison-ready
    /// semver string. Provided alongside `latest_tag` so the frontend
    /// doesn't have to re-derive it.
    pub latest_semver: String,
}

#[derive(Debug, Deserialize)]
struct GithubRelease {
    tag_name: String,
}

/// True when the user has opted into the staging update channel via
/// `~/.hq/menubar.json#stagingUpdateChannel`. The Settings UI toggle IS
/// the feature flag — see `MenubarPrefs::staging_update_channel`.
fn is_staging_channel_enabled() -> bool {
    let path = match paths::menubar_json_path() {
        Ok(p) => p,
        Err(_) => return false,
    };
    if !path.exists() {
        return false;
    }
    let contents = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(_) => return false,
    };
    serde_json::from_str::<MenubarPrefs>(&contents)
        .ok()
        .and_then(|p| p.staging_update_channel)
        .unwrap_or(false)
}

fn strip_v_prefix(s: &str) -> &str {
    s.strip_prefix('v').unwrap_or(s)
}

async fn fetch_latest_tag() -> Result<String, String> {
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
    let releases: Vec<GithubRelease> = resp
        .json()
        .await
        .map_err(|e| format!("parse GitHub releases JSON: {e}"))?;
    let first = releases.into_iter().next().ok_or_else(|| {
        "no releases yet on indigoai-us/hq-core-staging (auto-beta workflow hasn't pushed a tag)"
            .to_string()
    })?;
    Ok(first.tag_name.trim().to_string())
}

/// Perform one check. Returns `Some(info)` when an upgrade is available,
/// `None` when the user is already on the latest (or the feature flag is
/// off, or `core.yaml` isn't readable).
pub async fn check_once(app: &AppHandle) -> Result<Option<HqCoreStagingUpdateInfo>, String> {
    // FLAG GATE: silent no-op when the staging channel toggle is off.
    // Returns Ok(None) (not an error) so the background loop and any
    // manual check_hq_core_staging_update call from the Settings UI
    // can call this without paying network cost while disabled.
    if !is_staging_channel_enabled() {
        return Ok(None);
    }

    let latest_tag = fetch_latest_tag().await?;
    let latest_semver = strip_v_prefix(&latest_tag).to_string();
    let local = get_local_version();
    let update_available = match local.as_deref() {
        Some(l) => cmp_semver(l, &latest_semver) == std::cmp::Ordering::Less,
        None => false,
    };
    log(
        "hq-core-staging-update",
        &format!(
            "check: local={:?} latest={} update_available={}",
            local, latest_tag, update_available
        ),
    );
    if !update_available {
        return Ok(None);
    }
    let info = HqCoreStagingUpdateInfo {
        local,
        latest_tag,
        latest_semver,
    };
    let _ = app.emit("hq-core-staging-update:available", &info);
    Ok(Some(info))
}

/// Tauri command — one-shot manual check from the Settings panel.
#[tauri::command]
pub async fn check_hq_core_staging_update(
    app: AppHandle,
) -> Result<Option<HqCoreStagingUpdateInfo>, String> {
    check_once(&app).await
}

/// Background loop: first check 30s after launch, then every 6h. Internally
/// flag-gated — when the user toggles the channel off, the loop continues
/// to tick but `check_once` short-circuits before fetching anything, so
/// the cost is one menubar.json stat every 6h. Cheaper than tearing down
/// and re-spawning the task on toggle flip.
pub fn setup_hq_core_staging_update_checker(app: &AppHandle) {
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(INITIAL_DELAY).await;
        loop {
            if let Err(e) = check_once(&handle).await {
                log(
                    "hq-core-staging-update",
                    &format!("background check failed: {e}"),
                );
            }
            tokio::time::sleep(CHECK_INTERVAL).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_v_prefix_preserves_beta_suffix() {
        // Staging tags always carry the `-beta.<N>` suffix produced by the
        // auto-beta workflow. The strip must touch only the leading 'v'.
        assert_eq!(strip_v_prefix("v14.2.1-beta.3"), "14.2.1-beta.3");
        assert_eq!(strip_v_prefix("14.2.1-beta.3"), "14.2.1-beta.3");
        assert_eq!(strip_v_prefix(""), "");
    }

    #[test]
    fn is_staging_channel_enabled_returns_false_when_menubar_json_missing() {
        // No panic, no I/O error escape — the flag-gate is intentionally
        // forgiving so a fresh install with no menubar.json yet doesn't
        // spam the network.
        let _ = is_staging_channel_enabled();
    }
}
