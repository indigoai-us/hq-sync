//! "Update available" check for the `@indigoai-us/hq-cli` npm package.
//!
//! Mirrors `updater.rs` (which handles the menubar app itself) but targets
//! the user's globally-installed `hq` CLI. The two are decoupled releases:
//! the menubar pins a runner range via `util::hq_resolver::HQ_CLI_NPM_RANGE`
//! and self-heals via `npx` when the local `hq` falls below the floor, but
//! we still want to nag the user to upgrade their installed CLI so the
//! npx-fallback hot path isn't permanent.
//!
//! Flow:
//!   1. Resolve `hq` via `util::paths::resolve_bin`. If we get the bare
//!      name "hq" back, the user doesn't have it installed — `local` is
//!      None and we emit nothing (no nag for "you don't have it").
//!   2. Run `hq --version` to read the installed version string.
//!   3. GET https://registry.npmjs.org/@indigoai-us/hq-cli/latest and
//!      pull the `version` field.
//!   4. Compare numerically. If latest > local, emit
//!      `hq-cli-update:available` with both versions.
//!
//! A background task fires the check 15s after launch (offset from the
//! app updater's 10s so they don't both spike CPU at the same moment),
//! then every 6h. The result is also exposed as the `check_hq_cli_update`
//! Tauri command for on-demand polls.

use std::process::Command;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};

use crate::util::logfile::log;
use crate::util::paths;

/// npm registry endpoint that returns the dist-tag `latest` manifest. Cheap,
/// cached by the registry CDN, and returns a tiny JSON document.
const REGISTRY_URL: &str = "https://registry.npmjs.org/@indigoai-us/hq-cli/latest";

/// HTTP request timeout — keep tight so a flaky network doesn't stall the
/// background loop.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

/// Offset from app launch before the first check fires. 15s vs. the app
/// updater's 10s so they don't spike CPU + network in lockstep on launch.
const INITIAL_DELAY: Duration = Duration::from_secs(15);

/// Re-check cadence. Matches `updater::setup_update_checker` (6h).
const CHECK_INTERVAL: Duration = Duration::from_secs(21600);

/// Payload emitted to the frontend and returned by `check_hq_cli_update`.
#[derive(Debug, Clone, Serialize)]
pub struct HqCliUpdateInfo {
    /// Locally-installed version (None if `hq` isn't on PATH).
    pub local: Option<String>,
    /// `latest` dist-tag from the npm registry.
    pub latest: String,
}

#[derive(Debug, Deserialize)]
struct NpmLatest {
    version: String,
}

/// Three-segment numeric semver compare ("X.Y.Z[-pre]"). Pre-release
/// suffixes are dropped before comparison since the npm `latest` tag is
/// always stable. Anything that fails to parse compares as zero — we'd
/// rather under-report an update than crash the checker.
pub(crate) fn cmp_semver(a: &str, b: &str) -> std::cmp::Ordering {
    fn parse(v: &str) -> (u64, u64, u64) {
        let core = v.split('-').next().unwrap_or(v);
        let mut parts = core.split('.');
        let major = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
        let minor = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
        let patch = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
        (major, minor, patch)
    }
    parse(a).cmp(&parse(b))
}

/// Read the installed `hq --version` output. Returns `None` when `hq`
/// isn't resolvable (bare name) or the subprocess fails / returns garbage.
pub fn get_local_version() -> Option<String> {
    let bin = paths::resolve_bin("hq");
    if bin == "hq" {
        return None;
    }
    let out = Command::new(&bin).arg("--version").output().ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8(out.stdout).ok()?;
    let line = s.lines().next()?.trim().to_string();
    // Tolerate "v5.12.1" too, just in case the CLI ever prefixes with `v`.
    let cleaned = line.trim_start_matches('v').trim();
    if cleaned.is_empty() {
        None
    } else {
        Some(cleaned.to_string())
    }
}

async fn fetch_latest() -> Result<String, String> {
    let client = reqwest::Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .build()
        .map_err(|e| format!("build client: {e}"))?;
    let resp = client
        .get(REGISTRY_URL)
        .send()
        .await
        .map_err(|e| format!("GET {REGISTRY_URL}: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("registry returned HTTP {}", resp.status()));
    }
    let parsed: NpmLatest = resp
        .json()
        .await
        .map_err(|e| format!("parse registry JSON: {e}"))?;
    Ok(parsed.version)
}

/// Perform one check. Returns `Some(info)` when an upgrade is available,
/// `None` when the user is already on the latest (or `hq` isn't installed
/// — we don't pester users who don't have the CLI).
pub async fn check_once(app: &AppHandle) -> Result<Option<HqCliUpdateInfo>, String> {
    let latest = fetch_latest().await?;
    let local = get_local_version();
    let update_available = match local.as_deref() {
        Some(l) => cmp_semver(l, &latest) == std::cmp::Ordering::Less,
        None => false,
    };
    log(
        "hq-cli-update",
        &format!(
            "check: local={:?} latest={} update_available={}",
            local, latest, update_available
        ),
    );
    if !update_available {
        return Ok(None);
    }
    let info = HqCliUpdateInfo { local, latest };
    let _ = app.emit("hq-cli-update:available", &info);
    Ok(Some(info))
}

/// Tauri command — synchronous one-shot check used by the tray
/// "Check for Updates" menu item and by the Settings panel.
#[tauri::command]
pub async fn check_hq_cli_update(app: AppHandle) -> Result<Option<HqCliUpdateInfo>, String> {
    check_once(&app).await
}

/// Background loop: first check 15s after launch, then every 6h.
/// Mirrors `updater::setup_update_checker`. Logs but does not propagate
/// errors — a flaky network shouldn't kill the loop.
pub fn setup_hq_cli_update_checker(app: &AppHandle) {
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(INITIAL_DELAY).await;
        loop {
            if let Err(e) = check_once(&handle).await {
                log("hq-cli-update", &format!("background check failed: {e}"));
            }
            tokio::time::sleep(CHECK_INTERVAL).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cmp::Ordering;

    #[test]
    fn cmp_semver_compares_numerically_not_lexically() {
        // The whole point of a custom comparator — string compare would
        // say "5.10.0" < "5.2.0" because '1' < '2'.
        assert_eq!(cmp_semver("5.10.0", "5.2.0"), Ordering::Greater);
        assert_eq!(cmp_semver("5.10.10", "5.10.2"), Ordering::Greater);
    }

    #[test]
    fn cmp_semver_equal_and_less() {
        assert_eq!(cmp_semver("5.11.0", "5.11.0"), Ordering::Equal);
        assert_eq!(cmp_semver("5.11.0", "5.12.0"), Ordering::Less);
        assert_eq!(cmp_semver("5.12.1", "5.12.2"), Ordering::Less);
    }

    #[test]
    fn cmp_semver_handles_prerelease_suffix() {
        // npm `latest` is stable, but tolerate the suffix instead of
        // returning "no update" when the user is on a -beta or -rc.
        assert_eq!(cmp_semver("5.12.0-beta.1", "5.12.0"), Ordering::Equal);
        assert_eq!(cmp_semver("5.11.0-rc.3", "5.12.0"), Ordering::Less);
    }

    #[test]
    fn cmp_semver_missing_segments_default_to_zero() {
        // Don't panic on weird inputs — under-report rather than crash.
        assert_eq!(cmp_semver("5", "5.0.0"), Ordering::Equal);
        assert_eq!(cmp_semver("", "5.12.0"), Ordering::Less);
        assert_eq!(cmp_semver("not-a-version", "0.0.0"), Ordering::Equal);
    }
}
