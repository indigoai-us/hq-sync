//! Self-healing resolver for the `hq` CLI subprocess invocation.
//!
//! ## Why this exists
//!
//! AppBar shells out to `hq` from two places:
//!   * `commands::run_cli_provision` — Phase B's `hq cloud provision company`
//!   * `commands::first_push` — Option C3's `hq sync push --creds-from-stdin --json`
//!
//! Both contracts depend on flags that arrived in `@indigoai-us/hq-cli@5.7.0`
//! (the C3 push flags) and earlier versions (`cloud provision company` in
//! 5.6.0, `--skip-initial-sync` in 5.6.1). The Path A demote-to-local flow
//! shells out to `hq cloud demote company <slug> --force`, which arrived in
//! `@indigoai-us/hq-cli@5.10.0`. If the user's installed `hq` binary is
//! missing, on a stale PATH, or older than 5.10.0, the subprocess fails
//! with a cryptic error — "spawn ENOENT" or "unknown option
//! '--creds-from-stdin'" — that surfaces as a "Sync failed" toast in the
//! menubar with no actionable hint.
//!
//! ## What it does
//!
//! Runs two `--help` probes against the local `hq` once per AppBar
//! process: `hq sync push --help` must contain `--creds-from-stdin`
//! (5.7-era flag), AND `hq cloud --help` must list the `demote`
//! subcommand (5.10-era; required by Path A's tombstone branch).
//! If both pass, the local binary is used directly (fast path).
//! Otherwise we fall back to:
//!
//! ```text
//! npx -y --package=@indigoai-us/hq-cli@<HQ_CLI_NPM_RANGE> hq <args>
//! ```
//!
//! Same pattern as `commands::sync::HQ_CLOUD_VERSION`'s npx pin for
//! `hq-sync-runner`, just lazily probed instead of hardcoded — the local
//! binary stays the fast path when present and current.
//!
//! ## Why a capability probe, not `hq --version`
//!
//! `repos/public/hq/packages/hq-cli/src/index.ts` carries a hardcoded
//! `.version("5.5.0")` string that has not been kept in sync with the npm
//! package version (verified against published `hq-cli@5.7.0`). Probing
//! `hq --version` would lie to us. Asking "do the flags I need exist?"
//! is the only reliable signal.
//!
//! ## Lifecycle
//!
//! Cached in a `OnceLock` for the AppBar process lifetime. If the user
//! later upgrades hq-cli (`npm install -g @indigoai-us/hq-cli@latest`),
//! the next AppBar restart re-probes and picks up the local binary again.

use std::process::Command;
use std::sync::OnceLock;

use crate::util::logfile::log;
use crate::util::paths;

/// npm range used for the auto-fallback. Bump when AppBar starts depending
/// on a flag introduced in a newer hq-cli version.
///
/// Floor raised 5.7.0 → 5.10.0 to require `hq cloud demote company`, the
/// CLI subcommand Path A shells out to when an entity is `deleted=true`.
pub const HQ_CLI_NPM_RANGE: &str = "^5.10.0";

/// Cached invocation decision for the current process.
static HQ_INVOCATION: OnceLock<HqInvocation> = OnceLock::new();

/// How to spawn `hq` for the current AppBar process.
#[derive(Debug, Clone)]
pub enum HqInvocation {
    /// Local `hq` at this absolute path passed the C3 capability probe.
    Local(String),
    /// Local `hq` was missing or too old; route through `npx -y --package=...@<range> hq`.
    Npx,
}

impl HqInvocation {
    /// Build a `tokio::process::Command` for `hq <args>` according to the
    /// chosen invocation strategy. Caller appends args via `cmd.arg(...)`
    /// after this returns.
    pub fn command(&self) -> tokio::process::Command {
        match self {
            HqInvocation::Local(path) => tokio::process::Command::new(path),
            HqInvocation::Npx => {
                let mut cmd = tokio::process::Command::new("npx");
                cmd.args([
                    "-y",
                    "--package",
                    &format!("@indigoai-us/hq-cli@{HQ_CLI_NPM_RANGE}"),
                    "hq",
                ]);
                cmd
            }
        }
    }

    /// Human-readable label for log lines and diagnostic output.
    pub fn label(&self) -> String {
        match self {
            HqInvocation::Local(path) => format!("local:{path}"),
            HqInvocation::Npx => format!("npx:@indigoai-us/hq-cli@{HQ_CLI_NPM_RANGE}"),
        }
    }
}

/// Resolve the right way to invoke `hq`, caching the decision per-process.
/// Probes `hq sync push --help` synchronously the first time; subsequent
/// calls return the cached value with no overhead.
///
/// Logs the chosen invocation under the `hq-resolver` tag so a stuck
/// subprocess leaves a breadcrumb of which binary path it tried.
pub fn resolve_hq() -> HqInvocation {
    HQ_INVOCATION
        .get_or_init(|| {
            let chosen = probe();
            let label = chosen.label();
            log("hq-resolver", &format!("chose invocation: {label}"));
            // Breadcrumb for any subsequent Sentry event in the same scope —
            // tells us at-a-glance whether the user was on the locally-installed
            // CLI or the npx self-heal path. Critical when triaging
            // provision-cli failures: a stale local `hq` and a missing `npx`
            // produce different stderr signatures.
            sentry::add_breadcrumb(sentry::Breadcrumb {
                category: Some("hq-resolver".to_string()),
                message: Some(format!("chose invocation: {label}")),
                level: sentry::Level::Info,
                ..Default::default()
            });
            chosen
        })
        .clone()
}

fn probe() -> HqInvocation {
    let local = paths::resolve_bin("hq");
    // resolve_bin returns the bare "hq" string when it can't find the
    // binary anywhere — that's our signal to skip the probe entirely
    // and fall straight to npx. Avoids spawning a process that's
    // guaranteed to ENOENT.
    if local == "hq" {
        log(
            "hq-resolver",
            "local `hq` not found by resolve_bin; falling back to npx",
        );
        return HqInvocation::Npx;
    }

    let (creds_ok, demote_ok) = (
        capability_probe_creds_from_stdin(&local),
        capability_probe_cloud_demote(&local),
    );
    if creds_ok && demote_ok {
        HqInvocation::Local(local)
    } else {
        let missing = match (creds_ok, demote_ok) {
            (false, false) => "--creds-from-stdin AND `cloud demote` subcommand",
            (false, true) => "--creds-from-stdin",
            (true, false) => "`cloud demote` subcommand",
            (true, true) => unreachable!(),
        };
        log(
            "hq-resolver",
            &format!(
                "local `hq` at {local} failed capability probe (missing {missing}); \
                 falling back to npx. Hint: `npm install -g @indigoai-us/hq-cli@latest` to upgrade.",
            ),
        );
        HqInvocation::Npx
    }
}

/// Common subprocess wrapper for capability probes. Spawns `bin <args>`,
/// returns stdout on success, `None` on spawn-error or non-zero exit.
/// Shared between the probes so a future probe addition is one helper call.
fn run_help(bin: &str, args: &[&str]) -> Option<String> {
    let output = Command::new(bin)
        .args(args)
        // Inherit PATH from parent so node-shebanged `hq` can find `node`.
        .env("PATH", paths::child_path())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// True iff the local `hq` at `bin` supports the C3 subprocess contract.
/// Probes `hq sync push --help` and looks for `--creds-from-stdin` in
/// stdout — shipped in `@indigoai-us/hq-cli@5.7.0`.
///
/// Capability-based check is more reliable than parsing `hq --version`
/// because index.ts has a stale hardcoded version string that lies about
/// the actual installed capabilities.
///
/// ~100-300ms cold cost (one-time per AppBar startup).
fn capability_probe_creds_from_stdin(bin: &str) -> bool {
    run_help(bin, &["sync", "push", "--help"])
        .map(|s| s.contains("--creds-from-stdin"))
        .unwrap_or(false)
}

/// True iff the local `hq` at `bin` supports `cloud demote company`,
/// shipped in `@indigoai-us/hq-cli@5.10.0`. Probes `hq cloud --help` and
/// looks for the literal `demote` subcommand line.
///
/// Required because `--creds-from-stdin` alone is a 5.7-era capability —
/// a 5.7–5.9 local binary passes the C3 probe but lacks the demote
/// subcommand AppBar's Path A relies on. Without this probe, those
/// users would silently get `error: unknown command 'demote'` instead
/// of the npx self-heal.
fn capability_probe_cloud_demote(bin: &str) -> bool {
    run_help(bin, &["cloud", "--help"])
        .map(|s| s.contains("demote"))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A bin path that doesn't exist must produce a `false` probe result
    /// (not a panic, not a hang) for both probes. This is the primary
    /// `Local → Npx` fallback trigger and we lock the contract here.
    #[test]
    fn capability_probes_reject_nonexistent_binary() {
        let bogus = "/nonexistent/path/to/hq-binary-xyz-123";
        assert!(!capability_probe_creds_from_stdin(bogus));
        assert!(!capability_probe_cloud_demote(bogus));
    }

    /// The Npx invocation must build the exact npx argv shape the
    /// existing `hq-sync-runner` pin uses (`-y --package=PKG@RANGE BIN`).
    /// If this drifts, the spawned subprocess won't find `hq`.
    #[test]
    fn npx_invocation_builds_correct_argv() {
        let invocation = HqInvocation::Npx;
        let cmd = invocation.command();
        let std_cmd = cmd.as_std();
        assert_eq!(std_cmd.get_program(), "npx");
        let args: Vec<&str> = std_cmd
            .get_args()
            .map(|a| a.to_str().unwrap_or(""))
            .collect();
        assert_eq!(args[0], "-y");
        assert_eq!(args[1], "--package");
        assert!(
            args[2].starts_with("@indigoai-us/hq-cli@"),
            "package arg must start with @indigoai-us/hq-cli@; got {}",
            args[2],
        );
        assert!(
            args[2].contains(HQ_CLI_NPM_RANGE),
            "package arg must contain the pinned range {HQ_CLI_NPM_RANGE}; got {}",
            args[2],
        );
        assert_eq!(args[3], "hq");
    }

    /// The Local invocation must use the absolute path verbatim (no
    /// extra args). Caller appends sync/push/--creds-from-stdin/etc.
    #[test]
    fn local_invocation_uses_path_directly() {
        let invocation = HqInvocation::Local("/usr/local/bin/hq".to_string());
        let cmd = invocation.command();
        let std_cmd = cmd.as_std();
        assert_eq!(std_cmd.get_program(), "/usr/local/bin/hq");
        assert_eq!(std_cmd.get_args().count(), 0);
    }

    /// Labels must be human-readable for the logfile breadcrumb.
    #[test]
    fn label_contains_useful_info() {
        let local = HqInvocation::Local("/opt/homebrew/bin/hq".to_string());
        assert!(local.label().contains("/opt/homebrew/bin/hq"));
        let npx = HqInvocation::Npx;
        assert!(npx.label().contains("npx"));
        assert!(npx.label().contains("@indigoai-us/hq-cli"));
    }
}
