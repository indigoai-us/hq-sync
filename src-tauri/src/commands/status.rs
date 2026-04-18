//! Sync status polling — invokes `hq sync status --json` or falls back to journal file.
//!
//! This is a one-shot synchronous query (not a streaming subprocess), so it uses
//! `std::process::Command` directly rather than the process registry.

use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::commands::config::{HqConfig, MenubarPrefs};
use crate::util::paths;

/// CLI command timeout (5 seconds).
const STATUS_TIMEOUT: Duration = Duration::from_secs(5);

// ─────────────────────────────────────────────────────────────────────────────
// Types
// ─────────────────────────────────────────────────────────────────────────────

/// Response returned to the frontend.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SyncStatus {
    pub last_sync_at: Option<String>, // ISO 8601 or null if never synced
    pub pending_files: u32,
    pub conflicts: u32,
    pub daemon_running: bool,
    pub source: String, // "cli", "journal", or "none"
}

/// Journal file structure at {HQ_FOLDER}/.hq-sync-journal.json.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncJournal {
    pub last_sync_at: Option<String>,
    pub pending_files: Option<u32>,
    pub conflicts: Option<u32>,
    pub daemon_running: Option<bool>,
}

// ─────────────────────────────────────────────────────────────────────────────
// Config resolution (same pattern as sync.rs)
// ─────────────────────────────────────────────────────────────────────────────

/// Resolve the HQ folder path by reading config.json and menubar.json directly.
fn resolve_hq_folder_path() -> Result<String, String> {
    let config_path = paths::config_json_path()?;
    let menubar_path = paths::menubar_json_path()?;

    let menubar_prefs: Option<MenubarPrefs> = if menubar_path.exists() {
        std::fs::read_to_string(&menubar_path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
    } else {
        None
    };

    let config: Option<HqConfig> = if config_path.exists() {
        let contents = std::fs::read_to_string(&config_path)
            .map_err(|e| format!("Failed to read config.json: {}", e))?;
        Some(
            serde_json::from_str(&contents)
                .map_err(|e| format!("Failed to parse config.json: {}", e))?,
        )
    } else {
        None
    };

    let hq_folder = paths::resolve_hq_folder(
        config
            .as_ref()
            .and_then(|c| c.hq_folder_path.as_deref()),
        menubar_prefs
            .as_ref()
            .and_then(|p| p.hq_path.as_deref()),
    );

    Ok(hq_folder.to_string_lossy().to_string())
}

// ─────────────────────────────────────────────────────────────────────────────
// CLI status query
// ─────────────────────────────────────────────────────────────────────────────

/// Try to get sync status via `hq sync status --json`.
/// Returns Ok(SyncStatus) on success, Err(reason) on failure.
/// Times out after STATUS_TIMEOUT (5 seconds).
fn try_cli_status(hq_folder_path: &str) -> Result<SyncStatus, String> {
    let mut child = Command::new("hq")
        .args(["sync", "status", "--json", "--hq-path", hq_folder_path])
        .env("HQ_ROOT", hq_folder_path)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("Failed to spawn hq CLI: {}", e))?;

    // Wait with timeout — kill the process if it takes too long
    let start = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_exit_status)) => break,
            Ok(None) => {
                if start.elapsed() >= STATUS_TIMEOUT {
                    let _ = child.kill();
                    let _ = child.wait(); // reap zombie
                    return Err("hq sync status timed out".to_string());
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) => return Err(format!("Failed to wait for hq CLI: {}", e)),
        }
    }

    let output = child
        .wait_with_output()
        .map_err(|e| format!("Failed to read hq CLI output: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "hq sync status exited with code {}: {}",
            output
                .status
                .code()
                .map(|c| c.to_string())
                .unwrap_or_else(|| "unknown".to_string()),
            stderr.trim()
        ));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    parse_cli_output(&stdout)
}

/// Parse CLI JSON output into SyncStatus.
pub fn parse_cli_output(stdout: &str) -> Result<SyncStatus, String> {
    let mut status: SyncStatus = serde_json::from_str(stdout.trim())
        .map_err(|e| format!("Failed to parse CLI JSON output: {}", e))?;
    status.source = "cli".to_string();
    Ok(status)
}

// ─────────────────────────────────────────────────────────────────────────────
// Journal fallback
// ─────────────────────────────────────────────────────────────────────────────

/// Read and parse the journal file at {hq_folder}/.hq-sync-journal.json.
fn try_journal_status(hq_folder_path: &str) -> Result<SyncStatus, String> {
    let journal_path = PathBuf::from(hq_folder_path).join(".hq-sync-journal.json");
    let contents = std::fs::read_to_string(&journal_path)
        .map_err(|e| format!("Failed to read journal file: {}", e))?;
    parse_journal(&contents)
}

/// Parse journal JSON content into SyncStatus.
pub fn parse_journal(contents: &str) -> Result<SyncStatus, String> {
    let journal: SyncJournal = serde_json::from_str(contents.trim())
        .map_err(|e| format!("Failed to parse journal JSON: {}", e))?;
    Ok(journal_to_status(journal))
}

/// Convert a SyncJournal into a SyncStatus with source="journal".
fn journal_to_status(journal: SyncJournal) -> SyncStatus {
    SyncStatus {
        last_sync_at: journal.last_sync_at,
        pending_files: journal.pending_files.unwrap_or(0),
        conflicts: journal.conflicts.unwrap_or(0),
        daemon_running: journal.daemon_running.unwrap_or(false),
        source: "journal".to_string(),
    }
}

/// Default status when neither CLI nor journal is available.
pub fn default_status() -> SyncStatus {
    SyncStatus {
        last_sync_at: None,
        pending_files: 0,
        conflicts: 0,
        daemon_running: false,
        source: "none".to_string(),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tauri command
// ─────────────────────────────────────────────────────────────────────────────

/// Get the current sync status.
///
/// Tries `hq sync status --json` first. On failure (command not found, non-zero
/// exit, parse error), falls back to reading `{hq_folder}/.hq-sync-journal.json`.
/// If both fail, returns a default SyncStatus with everything zeroed/null.
#[tauri::command]
pub async fn get_sync_status() -> Result<SyncStatus, String> {
    let hq_folder_path = resolve_hq_folder_path()?;

    // Try CLI first
    match try_cli_status(&hq_folder_path) {
        Ok(status) => return Ok(status),
        Err(_e) => {
            #[cfg(debug_assertions)]
            eprintln!("[status] CLI failed, trying journal: {}", _e);
        }
    }

    // Fallback to journal file
    match try_journal_status(&hq_folder_path) {
        Ok(status) => return Ok(status),
        Err(_e) => {
            #[cfg(debug_assertions)]
            eprintln!("[status] Journal failed, returning default: {}", _e);
        }
    }

    // Both failed — return default
    Ok(default_status())
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── SyncStatus serialization ──────────────────────────────────────────

    #[test]
    fn test_sync_status_serializes_camel_case() {
        let status = SyncStatus {
            last_sync_at: Some("2026-04-18T12:00:00Z".to_string()),
            pending_files: 3,
            conflicts: 1,
            daemon_running: true,
            source: "cli".to_string(),
        };
        let json = serde_json::to_string(&status).unwrap();
        assert!(json.contains("\"lastSyncAt\""));
        assert!(json.contains("\"pendingFiles\""));
        assert!(json.contains("\"daemonRunning\""));
        assert!(!json.contains("\"last_sync_at\""));
        assert!(!json.contains("\"pending_files\""));
        assert!(!json.contains("\"daemon_running\""));
    }

    #[test]
    fn test_sync_status_roundtrip() {
        let status = SyncStatus {
            last_sync_at: Some("2026-04-18T12:00:00Z".to_string()),
            pending_files: 5,
            conflicts: 2,
            daemon_running: true,
            source: "cli".to_string(),
        };
        let json = serde_json::to_string(&status).unwrap();
        let parsed: SyncStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(status, parsed);
    }

    #[test]
    fn test_sync_status_null_last_sync() {
        let status = SyncStatus {
            last_sync_at: None,
            pending_files: 0,
            conflicts: 0,
            daemon_running: false,
            source: "none".to_string(),
        };
        let json = serde_json::to_string(&status).unwrap();
        assert!(json.contains("\"lastSyncAt\":null"));
    }

    // ── SyncJournal deserialization ───────────────────────────────────────

    #[test]
    fn test_journal_deserialize_full() {
        let json = r#"{
            "lastSyncAt": "2026-04-18T10:30:00Z",
            "pendingFiles": 7,
            "conflicts": 2,
            "daemonRunning": true
        }"#;
        let journal: SyncJournal = serde_json::from_str(json).unwrap();
        assert_eq!(journal.last_sync_at, Some("2026-04-18T10:30:00Z".to_string()));
        assert_eq!(journal.pending_files, Some(7));
        assert_eq!(journal.conflicts, Some(2));
        assert_eq!(journal.daemon_running, Some(true));
    }

    #[test]
    fn test_journal_deserialize_minimal() {
        let json = r#"{}"#;
        let journal: SyncJournal = serde_json::from_str(json).unwrap();
        assert_eq!(journal.last_sync_at, None);
        assert_eq!(journal.pending_files, None);
        assert_eq!(journal.conflicts, None);
        assert_eq!(journal.daemon_running, None);
    }

    #[test]
    fn test_journal_deserialize_partial() {
        let json = r#"{"lastSyncAt": "2026-04-18T10:30:00Z", "daemonRunning": false}"#;
        let journal: SyncJournal = serde_json::from_str(json).unwrap();
        assert_eq!(journal.last_sync_at, Some("2026-04-18T10:30:00Z".to_string()));
        assert_eq!(journal.pending_files, None);
        assert_eq!(journal.conflicts, None);
        assert_eq!(journal.daemon_running, Some(false));
    }

    // ── Journal → SyncStatus conversion ──────────────────────────────────

    #[test]
    fn test_journal_to_status_full() {
        let journal = SyncJournal {
            last_sync_at: Some("2026-04-18T10:30:00Z".to_string()),
            pending_files: Some(3),
            conflicts: Some(1),
            daemon_running: Some(true),
        };
        let status = journal_to_status(journal);
        assert_eq!(status.last_sync_at, Some("2026-04-18T10:30:00Z".to_string()));
        assert_eq!(status.pending_files, 3);
        assert_eq!(status.conflicts, 1);
        assert!(status.daemon_running);
        assert_eq!(status.source, "journal");
    }

    #[test]
    fn test_journal_to_status_defaults() {
        let journal = SyncJournal {
            last_sync_at: None,
            pending_files: None,
            conflicts: None,
            daemon_running: None,
        };
        let status = journal_to_status(journal);
        assert_eq!(status.last_sync_at, None);
        assert_eq!(status.pending_files, 0);
        assert_eq!(status.conflicts, 0);
        assert!(!status.daemon_running);
        assert_eq!(status.source, "journal");
    }

    // ── parse_journal ────────────────────────────────────────────────────

    #[test]
    fn test_parse_journal_valid() {
        let contents = r#"{
            "lastSyncAt": "2026-04-18T10:30:00Z",
            "pendingFiles": 5,
            "conflicts": 0,
            "daemonRunning": true
        }"#;
        let status = parse_journal(contents).unwrap();
        assert_eq!(status.pending_files, 5);
        assert_eq!(status.conflicts, 0);
        assert!(status.daemon_running);
        assert_eq!(status.source, "journal");
    }

    #[test]
    fn test_parse_journal_invalid_json() {
        let result = parse_journal("not json");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Failed to parse journal JSON"));
    }

    #[test]
    fn test_parse_journal_with_whitespace() {
        let contents = "  \n  {\"pendingFiles\": 2}  \n  ";
        let status = parse_journal(contents).unwrap();
        assert_eq!(status.pending_files, 2);
        assert_eq!(status.source, "journal");
    }

    // ── parse_cli_output ─────────────────────────────────────────────────

    #[test]
    fn test_parse_cli_output_valid() {
        let stdout = r#"{
            "lastSyncAt": "2026-04-18T12:00:00Z",
            "pendingFiles": 10,
            "conflicts": 3,
            "daemonRunning": true,
            "source": "ignored"
        }"#;
        let status = parse_cli_output(stdout).unwrap();
        assert_eq!(status.last_sync_at, Some("2026-04-18T12:00:00Z".to_string()));
        assert_eq!(status.pending_files, 10);
        assert_eq!(status.conflicts, 3);
        assert!(status.daemon_running);
        // source is overwritten to "cli"
        assert_eq!(status.source, "cli");
    }

    #[test]
    fn test_parse_cli_output_invalid() {
        let result = parse_cli_output("garbage output");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Failed to parse CLI JSON output"));
    }

    #[test]
    fn test_parse_cli_output_with_trailing_newline() {
        let stdout = "{\"lastSyncAt\":null,\"pendingFiles\":0,\"conflicts\":0,\"daemonRunning\":false,\"source\":\"cli\"}\n";
        let status = parse_cli_output(stdout).unwrap();
        assert_eq!(status.pending_files, 0);
        assert_eq!(status.source, "cli");
    }

    // ── default_status ───────────────────────────────────────────────────

    #[test]
    fn test_default_status() {
        let status = default_status();
        assert_eq!(status.last_sync_at, None);
        assert_eq!(status.pending_files, 0);
        assert_eq!(status.conflicts, 0);
        assert!(!status.daemon_running);
        assert_eq!(status.source, "none");
    }

    // ── STATUS_TIMEOUT constant ──────────────────────────────────────────

    #[test]
    fn test_status_timeout_value() {
        assert_eq!(STATUS_TIMEOUT, Duration::from_secs(5));
    }
}
