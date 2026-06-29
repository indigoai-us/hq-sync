//! User-facing "Report a problem" pathway.
//!
//! When a surface is genuinely blocked (e.g. the Meetings agenda has failed to
//! refresh for several poll cycles and is stuck on the last synced view), the
//! UI offers a one-click "Report a problem" button. Rather than inventing a
//! parallel reporting channel, that funnels into the canonical HQ feedback
//! pathway — the `hq feedback` CLI — exactly as the `/hq-bug` skill does:
//! write the body to a temp file and submit via `--body-file` so a multi-line
//! body never has to survive shell quoting.
//!
//! Design mirrors `packages.rs`: resolve the `hq` binary + the user's HQ
//! folder, hand the child the enriched PATH (node-shebang fix), shell out, and
//! relay a plain Ok/Err the window can toast.

use std::path::PathBuf;

use tokio::process::Command;

use crate::commands::config::{read_hq_config_lenient, MenubarPrefs};
use crate::util::logfile::log;
use crate::util::paths;

/// Resolve the HQ folder the same way the rest of the CLI-spawning commands do
/// (menubar override → legacy config path → discovery). Copied from
/// `packages.rs` to keep this module self-contained.
fn resolve_hq_folder() -> PathBuf {
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

/// Submit a bug report via `hq feedback bug --title <title> --body-file <file>`.
///
/// The body is written to a temp file and removed afterwards (best-effort), so
/// a multi-line body with arbitrary characters never has to be shell-escaped.
/// Returns `Ok(())` on a clean exit; any spawn/exit failure is surfaced as an
/// `Err(String)` so the window can fall back to telling the user to run
/// `/hq-bug` themselves.
#[tauri::command]
pub async fn submit_bug_report(title: String, body: String) -> Result<(), String> {
    let hq = paths::resolve_bin("hq");
    let folder = resolve_hq_folder();

    let mut body_path = std::env::temp_dir();
    body_path.push(format!("hq-sync-feedback-{}.md", std::process::id()));
    std::fs::write(&body_path, &body).map_err(|e| format!("write feedback body: {e}"))?;

    log("feedback", &format!("submitting bug report: {title}"));

    let result = Command::new(&hq)
        .args(["feedback", "bug", "--title", &title, "--body-file"])
        .arg(&body_path)
        // `hq` is a `#!/usr/bin/env node` script; a Dock/launchd-spawned app
        // gets a minimal PATH where `env` can't find node (exit 127). Hand it
        // the same enriched PATH the sync runner uses. See util::paths.
        .env("PATH", paths::child_path())
        .current_dir(&folder)
        .env("HQ_NO_UPDATE_CHECK", "1")
        .env("HQ_ROOT", &folder)
        .output()
        .await;

    // Best-effort cleanup — a leftover temp file is harmless.
    let _ = std::fs::remove_file(&body_path);

    let output = result.map_err(|e| format!("spawn `hq feedback`: {e}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let msg = format!(
            "`hq feedback` exited {}: {}",
            output.status.code().unwrap_or(-1),
            stderr.trim()
        );
        log("feedback", &format!("bug report failed: {msg}"));
        return Err(msg);
    }
    log("feedback", "bug report submitted");
    Ok(())
}
