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
//! 5.6.0, `--skip-initial-sync` in 5.6.1). If the user's installed `hq`
//! binary is missing, on a stale PATH, or older than 5.7.0, the subprocess
//! fails with a cryptic error — "spawn ENOENT" or "unknown option
//! '--creds-from-stdin'" — that surfaces as a "Sync failed" toast in the
//! menubar with no actionable hint.
//!
//! ## What it does
//!
//! Probes the local `hq sync push --help` output once per AppBar process
//! and looks for `--creds-from-stdin` in stdout. If present, the local
//! binary supports the C3 contract and we use it directly (fast path).
//! Otherwise we fall back to:
//!
//! ```text
//! npx -y --package=@indigoai-us/hq-cli@^5.7.0 hq <args>
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
pub const HQ_CLI_NPM_RANGE: &str = "^5.7.0";

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
            log("hq-resolver", &format!("chose invocation: {}", chosen.label()));
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

    if capability_probe_passes(&local) {
        HqInvocation::Local(local)
    } else {
        log(
            "hq-resolver",
            &format!(
                "local `hq` at {local} failed C3 capability probe (missing --creds-from-stdin); \
                 falling back to npx. Hint: `npm install -g @indigoai-us/hq-cli@latest` to upgrade.",
            ),
        );
        HqInvocation::Npx
    }
}

/// True iff the local `hq` at `bin` supports the C3 subprocess contract.
/// Probes `hq sync push --help` and looks for `--creds-from-stdin` in
/// stdout — the canonical capability signal for this AppBar build.
///
/// Capability-based check is more reliable than parsing `hq --version`
/// because index.ts has a stale hardcoded version string that lies about
/// the actual installed capabilities.
///
/// ~100-300ms cold cost (one-time per AppBar startup).
fn capability_probe_passes(bin: &str) -> bool {
    let output = match Command::new(bin)
        .args(["sync", "push", "--help"])
        // Inherit PATH from parent so node-shebanged `hq` can find `node`.
        .env("PATH", paths::child_path())
        .output()
    {
        Ok(o) => o,
        Err(_) => return false,
    };
    if !output.status.success() {
        return false;
    }
    // Help text goes to stdout; we look for the literal flag name. If a
    // future commander version changes the formatting, this still matches
    // because the flag name itself is stable across formatting variants.
    let stdout = String::from_utf8_lossy(&output.stdout);
    stdout.contains("--creds-from-stdin")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A bin path that doesn't exist must produce a `false` probe result
    /// (not a panic, not a hang). This is the primary `Local → Npx`
    /// fallback trigger and we lock the contract here.
    #[test]
    fn capability_probe_rejects_nonexistent_binary() {
        assert!(!capability_probe_passes(
            "/nonexistent/path/to/hq-binary-xyz-123",
        ));
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
