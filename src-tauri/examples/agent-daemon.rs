//! Headless agent daemon binary — US-010.
//!
//! Runs the work-routing loop for a roster agent (`agt_` identity) on
//! Linux/Outpost hosts without the Tauri menubar.
//!
//! # Required environment variables
//!
//!   WORK_MESH_ENABLED=true    feature gate — daemon exits immediately if absent
//!   AGENT_ACCESS_TOKEN        Cognito access token for the agt_ identity
//!   VAULT_API_URL             hq-pro API base, e.g. https://api.hq.getindigo.ai
//!
//! # Optional
//!
//!   AGENT_HQ_FOLDER           working directory passed to spawned claude sessions
//!
//! # Usage
//!
//!   cargo build --example agent-daemon --release
//!   WORK_MESH_ENABLED=true \
//!   AGENT_ACCESS_TOKEN=<token> \
//!   VAULT_API_URL=https://api.hq.getindigo.ai \
//!   ./target/release/examples/agent-daemon
//!
//! The daemon runs until killed. A process manager (systemd, supervisor) should
//! restart it on exit. Rotate `AGENT_ACCESS_TOKEN` before expiry (Cognito
//! tokens expire in 1 hour by default; replace the env var and restart).

// Include the headless implementation as a local module.
// Lives in examples/ (not src/bin/) to avoid Tauri's bundler walking
// src/bin/*.rs and trying to copy unbundled binaries into the .app
// (same pattern as emit-sample-journal.rs).
// `headless.rs` is self-contained (no `crate::` imports) so the `#[path]`
// approach works cleanly from an `[[example]]` in the same package.
#[path = "../src/commands/work_daemon/headless.rs"]
mod headless;

fn main() {
    match headless::HeadlessConfig::from_env() {
        Ok(config) => {
            headless::run_headless(config);
        }
        Err(e) => {
            eprintln!("[agent-daemon] AGENT_DAEMON_CONFIG_FAIL {e}");
            eprintln!(
                "[agent-daemon] Required env: AGENT_ACCESS_TOKEN, VAULT_API_URL\n\
                 [agent-daemon] Optional env: AGENT_HQ_FOLDER, WORK_MESH_ENABLED=true"
            );
            std::process::exit(1);
        }
    }
}
