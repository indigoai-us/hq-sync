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
//!
//! The `install_hq_cli_update` command runs the upgrade directly by
//! spawning `npm install -g @indigoai-us/hq-cli@latest` with the same
//! beefed-up PATH used elsewhere for child processes (`paths::child_path`).
//! On success it re-checks and emits a fresh `hq-cli-update:cleared` event;
//! on failure it returns stderr so the UI can fall back to the manual
//! copy-the-command flow (typical failure: EACCES against a system-prefix
//! npm that needs sudo).

use std::process::Command;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};

use crate::util::logfile::log;
use crate::util::paths;

/// npm package the menubar nags the user to keep current.
pub(crate) const HQ_CLI_PACKAGE: &str = "@indigoai-us/hq-cli@latest";

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

/// Resolve the installed `@indigoai-us/hq-cli` version. Returns `None`
/// when the CLI doesn't appear to be installed.
///
/// We deliberately do NOT trust `hq --version`. The CLI's `index.ts`
/// carries a hardcoded `.version("…")` string that has not been kept in
/// sync with the published npm version (same gotcha documented in
/// `util::hq_resolver`). Asking the binary would lie to us and either
/// over- or under-trigger the update banner.
///
/// Resolution order:
///   1. Locate npm via `paths::resolve_bin("npm")`. If we have an npm,
///      ask `npm root -g` for the global modules directory and read the
///      `version` field from the installed `package.json` directly.
///   2. Fall back to `hq --version` only if npm is unreachable (no node
///      toolchain at all). Worse than nothing, but still better than
///      returning None and silently disabling the nag for users who
///      have `hq` somehow but not `npm`.
pub fn get_local_version() -> Option<String> {
    let npm = paths::resolve_bin("npm");
    if npm != "npm" {
        if let Some(v) = read_installed_version(&npm, &paths::child_path()) {
            return Some(v);
        }
        // npm resolved but the package isn't installed anywhere it can see.
        // Don't fall through to `hq --version` — we'd just get the same
        // stale hardcoded string from a binary npm doesn't manage.
        return None;
    }

    // Last-ditch: no npm on PATH but the user might still have `hq` from
    // an unmanaged install. Accept its lying version rather than emit
    // nothing at all.
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

/// Build the argv for the global install. Factored out so the unit test
/// can lock the shape without spawning npm.
pub(crate) fn install_argv() -> [&'static str; 3] {
    ["install", "-g", HQ_CLI_PACKAGE]
}

/// Read the version field from the installed package.json inside the npm
/// global prefix. We do this instead of `hq --version` because the CLI's
/// `index.ts` carries a hardcoded `.version("5.5.0")`-style string that
/// has not been kept in sync with the published npm version (same gotcha
/// documented in `util::hq_resolver`). package.json is the canonical source.
///
/// `npm_bin` is the absolute path to the `npm` binary we just spawned; we
/// re-use it to ask `npm root -g` for the global modules directory so we
/// read the package.json from the *same* prefix `install -g` just wrote
/// to. Asking a different `npm` on PATH would read a different prefix.
fn read_installed_version(npm_bin: &str, path: &str) -> Option<String> {
    let out = Command::new(npm_bin)
        .args(["root", "-g"])
        .env("PATH", path)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let root = String::from_utf8(out.stdout).ok()?.trim().to_string();
    if root.is_empty() {
        return None;
    }
    let pkg_json = std::path::Path::new(&root)
        .join("@indigoai-us")
        .join("hq-cli")
        .join("package.json");
    let bytes = std::fs::read(&pkg_json).ok()?;
    let parsed: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    parsed
        .get("version")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

/// Tauri command — runs `npm install -g @indigoai-us/hq-cli@latest` in a
/// blocking task using the same child PATH as the runner (so node-shebanged
/// npm and its own subprocess lookups succeed under the launchd-minimal
/// PATH a Dock-launched menubar app inherits). On success we re-check and
/// emit `hq-cli-update:cleared` so the frontend banner can disappear without
/// waiting for the 6h background loop.
///
/// Failure mode is deliberate: we surface the npm stderr verbatim to the
/// caller. The most common one — `EACCES: permission denied, mkdir
/// '/usr/local/lib/node_modules/@indigoai-us'` — means the user's npm
/// prefix needs sudo. The UI falls back to the previous copy-the-command
/// path for that case rather than prompting for a password.
#[tauri::command]
pub async fn install_hq_cli_update(app: AppHandle) -> Result<HqCliUpdateInfo, String> {
    let npm = paths::resolve_bin("npm");
    let path = paths::child_path();
    let args = install_argv();
    log(
        "hq-cli-update",
        &format!("install: spawning {} {}", npm, args.join(" ")),
    );

    let npm_for_install = npm.clone();
    let path_for_install = path.clone();
    let output = tauri::async_runtime::spawn_blocking(move || {
        Command::new(&npm_for_install)
            .args(args)
            .env("PATH", path_for_install)
            .output()
    })
    .await
    .map_err(|e| format!("join blocking task: {e}"))?
    .map_err(|e| format!("spawn npm: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let detail = if stderr.is_empty() { stdout } else { stderr };
        log(
            "hq-cli-update",
            &format!("install failed (exit {:?}): {detail}", output.status.code()),
        );
        return Err(if detail.is_empty() {
            format!("npm install exited with status {:?}", output.status.code())
        } else {
            detail
        });
    }

    // npm exit 0 means the @latest tag is now installed at npm's global
    // prefix — by definition the user is current. We deliberately do NOT
    // run `hq --version` here: the CLI's hardcoded version string lags
    // the published npm version (see util::hq_resolver for the same
    // gotcha) so `hq --version` would still read the old number and the
    // banner would never clear.
    let latest = fetch_latest().await?;
    let npm_for_root = npm.clone();
    let path_for_root = path.clone();
    let installed = tauri::async_runtime::spawn_blocking(move || {
        read_installed_version(&npm_for_root, &path_for_root)
    })
    .await
    .ok()
    .flatten();
    // Prefer the package.json version we just installed; fall back to the
    // registry `latest` we already fetched (also accurate since `@latest`
    // was the install target).
    let local = installed.clone().or_else(|| Some(latest.clone()));
    log(
        "hq-cli-update",
        &format!(
            "install succeeded: installed_pkg={:?} latest={}",
            installed, latest
        ),
    );
    let info = HqCliUpdateInfo {
        local,
        latest: latest.clone(),
    };
    // Frontend uses this to drop the banner immediately on success.
    let _ = app.emit("hq-cli-update:cleared", &info);
    Ok(info)
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

    /// Lock the npm argv shape so a typo (e.g., dropping `-g`, renaming
    /// the package) can't ship a non-global or wrong-package install.
    #[test]
    fn install_argv_targets_global_hq_cli() {
        let argv = install_argv();
        assert_eq!(argv[0], "install");
        assert_eq!(argv[1], "-g");
        assert!(
            argv[2].starts_with("@indigoai-us/hq-cli@"),
            "package arg must target @indigoai-us/hq-cli; got {}",
            argv[2],
        );
        // The banner button is the "update to current" path — pin must
        // resolve to `latest`, not a hardcoded version that would rot.
        assert!(
            argv[2].ends_with("@latest"),
            "package arg must request @latest; got {}",
            argv[2],
        );
    }

    #[test]
    fn cmp_semver_missing_segments_default_to_zero() {
        // Don't panic on weird inputs — under-report rather than crash.
        assert_eq!(cmp_semver("5", "5.0.0"), Ordering::Equal);
        assert_eq!(cmp_semver("", "5.12.0"), Ordering::Less);
        assert_eq!(cmp_semver("not-a-version", "0.0.0"), Ordering::Equal);
    }
}
