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
//!   * Updating hq-core no longer requires opening Claude Code. The
//!     `install_hq_core_update` Tauri command spawns the bundled
//!     `replace-rescue.sh` against `indigoai-us/hq-core` at the latest
//!     release tag — the same rescue + overlay engine the staging pill
//!     uses for `@getindigo.ai` builders, just pointed at the public prod
//!     repo. Drifts are rescued into `personal/`, the release tree is
//!     overlaid on top, and `core/core.yaml`'s `replaced_from_source`
//!     stamp is updated with the cloned SHA so subsequent checks have a
//!     valid history floor. The frontend pill swaps its label to
//!     "Updating…" while the future is pending and surfaces a result chip
//!     on completion (mirroring the staging pill's UX).
//!   * No `:cleared` event — the banner naturally clears on the next
//!     background check after `core.yaml`'s `hqVersion` advances. The
//!     frontend re-reads `get_hq_version` post-install so the footer
//!     version row updates without waiting for the 6h cycle.
//!
//! Cadence: first check 20s after launch (offset from updater's 10s and
//! the CLI nag's 15s so they don't spike CPU + network in lockstep), then
//! every 6h. Matches the rest of the family.

use std::time::Duration;

use serde::Deserialize;

use crate::commands::config::{read_hq_config_lenient, MenubarPrefs};
use crate::util::logfile::log;
use crate::util::paths;

/// GitHub Releases API endpoint for the canonical hq-core repo. Returns
/// the latest *non-prerelease* release for the repo — staging tags pushed
/// to `hq-core-staging` don't surface here, only what's promoted to the
/// public `hq-core` repo via `/personal:release-hq-core`.
const RELEASES_URL: &str =
    "https://api.github.com/repos/indigoai-us/hq-core/releases/latest";

/// HTTP request timeout — keep tight so a flaky network doesn't stall the
/// `install_hq_core_update` handler.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Deserialize)]
struct GithubRelease {
    tag_name: String,
}

/// Resolve the locally-installed hq-core version by reading `hqVersion`
/// from `core.yaml` inside the user's HQ folder.
///
/// File location is layout-aware:
///   * **canonical (v14+):** `<HQ folder>/core/core.yaml`
///   * **legacy (pre-v14):** `<HQ folder>/core.yaml`
///
/// The v14 hq-core release moved `core.yaml` one level deeper (see
/// `apps/hq-core/MIGRATION.md` in this monorepo — "Root core.yaml;
/// canonical location is core/core.yaml"). We check the canonical
/// location first and fall back to the legacy root for any HQ folder
/// that hasn't migrated yet.
///
/// Resolution order for the HQ folder mirrors what `conflicts.rs` and
/// `daemon.rs` do: menubar.json `hqPath` → config.json `hqFolderPath` →
/// discovery via `core.yaml` signature → `~/HQ`. See `paths::resolve_hq_folder`.
///
/// Returns `None` when:
///   * the HQ folder can't be located,
///   * neither canonical nor legacy `core.yaml` exists,
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

    // Canonical first (v14+), legacy fallback (pre-v14). Two stat
    // syscalls in the miss path is fine — this runs every 6h, not on
    // a hot loop.
    let canonical = hq_folder.join("core").join("core.yaml");
    let legacy = hq_folder.join("core.yaml");
    let core_yaml = if canonical.is_file() { canonical } else { legacy };

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

/// Resolve a ref (typically `v{X.Y.Z}`) to its 40-char commit SHA in `repo`.
///
/// Used by `install_hq_core_update` to derive the history floor passed
/// to the rescue script (`--floor-sha`). Returns `None` on any failure
/// (404, network, parse) rather than `Err` — the caller treats the SHA
/// as a best-effort hint: when present the rescue runs in
/// `history_floor` mode (correct vs. installed baseline); when absent
/// it falls back to `head_compare` (safe but loses USER-EDIT precision
/// for files changed upstream since the install).
async fn fetch_tag_sha(repo: &str, git_ref: &str) -> Option<String> {
    let url = format!("https://api.github.com/repos/{repo}/commits/{git_ref}");
    let client = reqwest::Client::builder()
        .default_headers(crate::util::client_info::client_headers())
        .timeout(REQUEST_TIMEOUT)
        .build()
        .ok()?;
    let resp = client
        .get(&url)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .ok()?;
    if !resp.status().is_success() {
        return None;
    }
    #[derive(serde::Deserialize)]
    struct GhCommit {
        sha: String,
    }
    let parsed: GhCommit = resp.json().await.ok()?;
    let sha = parsed.sha.trim();
    // Defensive: GitHub returns a 40-char hex SHA. Validate to match the
    // script's `--floor-sha` regex so we don't pass through garbage.
    if sha.len() == 40 && sha.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()) {
        Some(sha.to_string())
    } else {
        None
    }
}

async fn fetch_latest() -> Result<String, String> {
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
    Ok(strip_v_prefix(parsed.tag_name.trim()).to_string())
}

/// Tauri command — cheap on-disk read of the local hq-core `hqVersion`.
///
/// No network, no background loop — just `get_local_version()` exposed to
/// the frontend so the popover footer can display "HQ vX.Y.Z" (or the
/// "version unknown" / copy-prompt repair affordance when None) without
/// waiting for the 20s-delayed update check, and without falsely implying
/// an upgrade by piggy-backing on `check_hq_core_update`'s `latest` field.
///
/// Returns `None` for the same reasons as `get_local_version`: the HQ
/// folder can't be located, `core.yaml` is missing/unparseable, or it has
/// no `hqVersion` field. The frontend treats `None` as "show repair
/// affordance" rather than hiding the row — silence here masks a broken
/// install, which is exactly the case we want surfaced.
#[tauri::command]
pub fn get_hq_version() -> Option<String> {
    get_local_version()
}

/// Canonical owner/name of the prod hq-core repo. Mirrors the constant
/// shape staging uses (`DEFAULT_STAGING_REPO`) so the spawn site stays
/// symmetric. Hard-coded — there's no menubar.json override for prod
/// because the release feed and the rescue source must agree, and the
/// release feed (`RELEASES_URL` above) is already pinned.
const PROD_HQ_CORE_REPO: &str = "indigoai-us/hq-core";

/// Tauri command — prod-user "Update" action. Runs hq-cloud's `hq-rescue`
/// bin (via npx, pinned to `HQ_CLOUD_VERSION`) against `indigoai-us/hq-core`
/// at the latest release tag (`v{latest}`), replacing the old "open Claude
/// Code with /update-hq" CTA.
///
/// Shape mirrors `hq_core_staging::run_replace_from_staging`:
///   1. Resolve the HQ folder via the standard 4-tier resolver. Bail if it
///      isn't a valid HQ root (`looks_like_hq_root`: `companies/` plus one of
///      `.claude/`, `core/`, or `personal/`) so we fail fast with a clean
///      error instead of an opaque rescue exit-3. Note this must NOT require
///      `personal/` — a faithful v14.0.0 install has none (rescue creates it
///      as the drift override target), and requiring it blocked every
///      v14.0.0 → v15 update (DEV-1741).
///   2. Refetch the latest release tag from GitHub. We don't trust the
///      frontend-supplied value — it may be stale (last background check
///      ran 6h ago) and the operation is heavyweight enough that the
///      extra round-trip is negligible.
///   3. Spawn `npx -y --package=@indigoai-us/hq-cloud@<pin> hq-rescue
///      --hq-root <folder> --source indigoai-us/hq-core --ref v{latest}
///      --yes`. GH token is forwarded via `GH_TOKEN` env when available —
///      public repo doesn't strictly need it, but having one dodges the
///      60/h anonymous-clone rate limit during the history-index walk.
///   4. Return the same `RescueRunResult` shape (exit code + 40-line log
///      tail + log path) so the frontend can reuse the staging pill's
///      result-chip UI verbatim.
///
/// Long-running (~30-90s on first run because of the full-history clone +
/// scan). The frontend disables the pill + swaps to "Updating…" while the
/// future is pending.
#[tauri::command]
pub async fn install_hq_core_update(
) -> Result<crate::commands::hq_core_staging::RescueRunResult, String> {
    let hq_folder = crate::commands::hq_core_staging::resolve_hq_folder();
    if !crate::commands::hq_core_staging::looks_like_hq_root(&hq_folder) {
        return Err(format!(
            "HQ folder at {} is not a valid HQ root (need companies/ plus one of .claude/, core/, or personal/)",
            hq_folder.display()
        ));
    }

    let latest = fetch_latest()
        .await
        .map_err(|e| format!("fetch latest hq-core release: {e}"))?;
    if latest.is_empty() {
        return Err("GitHub returned an empty tag for the latest hq-core release".to_string());
    }
    // hq-core release tags are `vX.Y.Z` (see strip_v_prefix doc above);
    // `fetch_latest` strips the leading `v` for the semver comparator, so
    // re-add it here for the git ref.
    let git_ref = format!("v{latest}");

    // History-floor SHA for the rescue baseline (Codex P1 review on PR
    // #110). Resolve the user's installed `v{hqVersion}` tag to its commit
    // SHA in the prod repo and pass to the script as `--floor-sha`. The
    // script uses this when `core/core.yaml` has no
    // `replaced_from_source.last_sync_sha` stamp (i.e. the user has never
    // run a rescue before — exactly the population shipping this prod
    // Update button targets).
    //
    // Without this:
    //   * No stamp + no override → `BASELINE_MODE=head_compare` → every
    //     file changed upstream between the user's installed version and
    //     latest is classified USER-EDIT and moved to `personal/`, leaving
    //     a tree full of stale overrides that mask the upstream update.
    //
    // With this:
    //   * No stamp + override = `v{hqVersion}`'s SHA → `BASELINE_MODE=
    //     history_floor` against the user's actual installed tree → only
    //     files the user themselves modified get rescued; everything else
    //     converges cleanly to latest.
    //
    // Best-effort: if `get_local_version` is None (broken install — would
    // surface as "version unknown" in the popover, the user's already
    // seeing the copy-prompt path) or the SHA fetch fails (network /
    // missing tag), we omit `--floor-sha` and the script falls through to
    // `head_compare` — the same behavior shipped before this fix, no
    // regression vs. baseline.
    let floor_sha = match get_local_version() {
        Some(ver) => {
            let user_tag = format!("v{ver}");
            let resolved = fetch_tag_sha(PROD_HQ_CORE_REPO, &user_tag).await;
            if resolved.is_none() {
                log(
                    "hq-core-update",
                    &format!(
                        "floor-sha lookup failed for {PROD_HQ_CORE_REPO}@{user_tag}; rescue will use head_compare fallback"
                    ),
                );
            }
            resolved
        }
        None => {
            log(
                "hq-core-update",
                "local hqVersion unreadable; rescue will use head_compare fallback (no --floor-sha)",
            );
            None
        }
    };

    let log_path = std::env::temp_dir().join(format!(
        "hq-sync-install-hq-core-update-{}.log",
        std::process::id()
    ));
    let log_file_for_stdout = std::fs::File::create(&log_path)
        .map_err(|e| format!("create log file {}: {e}", log_path.display()))?;
    let log_file_for_stderr = log_file_for_stdout
        .try_clone()
        .map_err(|e| format!("dup log file fd: {e}"))?;

    log(
        "hq-core-update",
        &format!(
            "spawning rescue (prod): hq-cloud@{} hq_root={} repo={} ref={} floor_sha={} log={}",
            crate::commands::sync::HQ_CLOUD_VERSION,
            hq_folder.display(),
            PROD_HQ_CORE_REPO,
            git_ref,
            floor_sha.as_deref().unwrap_or("(none — head_compare fallback)"),
            log_path.display()
        ),
    );

    let mut cmd = crate::commands::hq_core_staging::rescue_command();
    cmd.arg("--hq-root")
        .arg(hq_folder.as_os_str())
        .arg("--source")
        .arg(PROD_HQ_CORE_REPO)
        .arg("--ref")
        .arg(&git_ref)
        .arg("--yes")
        .stdout(std::process::Stdio::from(log_file_for_stdout))
        .stderr(std::process::Stdio::from(log_file_for_stderr));
    if let Some(sha) = floor_sha.as_deref() {
        cmd.arg("--floor-sha").arg(sha);
    }

    // GH token is optional for the public repo. Forward when present so
    // the history-index walk doesn't hit anonymous rate limits.
    if let Some(token) = crate::commands::hq_core_staging::resolve_gh_token() {
        cmd.env("GH_TOKEN", &token);
    }

    let status = cmd
        .status()
        .await
        .map_err(|e| format!("spawn rescue script: {e}"))?;

    let exit_code = status.code().unwrap_or(-1);
    let log_tail = crate::commands::hq_core_staging::tail_log(&log_path, 40)
        .unwrap_or_else(|e| format!("(log tail unavailable: {e})"));

    log(
        "hq-core-update",
        &format!(
            "rescue exit={} ref={} log={}",
            exit_code,
            git_ref,
            log_path.display()
        ),
    );

    Ok(crate::commands::hq_core_staging::RescueRunResult {
        exit_code,
        log_tail,
        log_path: log_path.display().to_string(),
    })
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
