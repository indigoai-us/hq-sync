//! Tauri commands for spawning and cancelling `hq sync --json`.
//!
//! Uses [`crate::commands::process`] for subprocess lifecycle (spawn, stream,
//! SIGTERM→SIGKILL). Emits typed sync events to the Svelte renderer.

use std::collections::HashMap;
use std::thread;
use std::time::Duration;

use tauri::{AppHandle, Emitter};

use crate::commands::config::{HqConfig, MenubarPrefs};
use crate::commands::process::{
    cancel_process_impl, deregister_process, is_registered, run_process_impl, try_register_handle,
    ProcessEvent, SpawnArgs,
};
use crate::events::{
    SyncEvent, EVENT_SYNC_COMPLETE, EVENT_SYNC_CONFLICT, EVENT_SYNC_ERROR, EVENT_SYNC_PROGRESS,
};
use crate::util::paths;

/// Singleton handle — only one sync at a time.
const SYNC_HANDLE: &str = "hq-sync";

/// Hard timeout for a sync run (10 minutes).
const SYNC_TIMEOUT: Duration = Duration::from_secs(600);

/// SIGKILL delay after SIGTERM on cancel.
const SIGKILL_DELAY: Duration = Duration::from_secs(5);

// ─────────────────────────────────────────────────────────────────────────────
// Config resolution (inline — avoids calling async Tauri command)
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
// SpawnArgs builder (testable)
// ─────────────────────────────────────────────────────────────────────────────

/// Build the SpawnArgs for `hq sync --json` with defensive double-binding of
/// the HQ folder path (both as env var and CLI flag).
pub fn build_sync_spawn_args(hq_folder_path: &str) -> SpawnArgs {
    let mut env = HashMap::new();
    env.insert("HQ_ROOT".to_string(), hq_folder_path.to_string());

    SpawnArgs {
        cmd: "hq".to_string(),
        args: vec![
            "sync".to_string(),
            "--json".to_string(),
            "--hq-path".to_string(),
            hq_folder_path.to_string(),
        ],
        cwd: None,
        env: Some(env),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// ndjson line handler (testable)
// ─────────────────────────────────────────────────────────────────────────────

/// Parse a single ndjson line and emit the corresponding Tauri event.
/// Unknown/malformed lines are silently skipped (logged in debug builds).
fn handle_sync_line(app: &AppHandle, line: &str) {
    let event: SyncEvent = match serde_json::from_str(line) {
        Ok(e) => e,
        Err(_e) => {
            #[cfg(debug_assertions)]
            eprintln!("[sync] skipping unparseable line: {}", _e);
            return;
        }
    };

    let result = match &event {
        SyncEvent::Progress(payload) => app.emit(EVENT_SYNC_PROGRESS, payload.clone()),
        SyncEvent::Conflict(payload) => app.emit(EVENT_SYNC_CONFLICT, payload.clone()),
        SyncEvent::Error(payload) => app.emit(EVENT_SYNC_ERROR, payload.clone()),
        SyncEvent::Complete(payload) => app.emit(EVENT_SYNC_COMPLETE, payload.clone()),
    };

    if let Err(_e) = result {
        #[cfg(debug_assertions)]
        eprintln!("[sync] failed to emit event: {}", _e);
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tauri commands
// ─────────────────────────────────────────────────────────────────────────────

/// Spawn `hq sync --json` as a child process.
///
/// - Only one sync can run at a time (singleton handle).
/// - Emits `sync:progress`, `sync:conflict`, `sync:error`, `sync:complete`
///   events to the Svelte renderer as ndjson lines arrive.
/// - Hard timeout of 10 minutes; the sync is cancelled if it exceeds this.
///
/// Returns the handle string on success (always `"hq-sync"`).
#[tauri::command]
pub fn start_sync(app: AppHandle) -> Result<String, String> {
    // Atomically check-and-register to prevent concurrent syncs (TOCTOU-safe)
    if !try_register_handle(SYNC_HANDLE) {
        return Err("Sync is already running".to_string());
    }

    // Resolve config — deregister on failure so future syncs aren't blocked
    let hq_folder_path = match resolve_hq_folder_path() {
        Ok(p) => p,
        Err(e) => {
            deregister_process(SYNC_HANDLE);
            return Err(e);
        }
    };
    let spawn_args = build_sync_spawn_args(&hq_folder_path);

    // Timeout watchdog — cancels sync after SYNC_TIMEOUT
    thread::spawn(move || {
        thread::sleep(SYNC_TIMEOUT);
        if is_registered(SYNC_HANDLE) {
            #[cfg(debug_assertions)]
            eprintln!("[sync] timeout reached, cancelling");
            cancel_process_impl(SYNC_HANDLE, SIGKILL_DELAY);
        }
    });

    // Background thread: run the subprocess and stream events
    let app_bg = app.clone();
    thread::spawn(move || {
        let result = run_process_impl(SYNC_HANDLE, &spawn_args, |event| match event {
            ProcessEvent::Stdout(line) => {
                handle_sync_line(&app_bg, &line);
            }
            ProcessEvent::Stderr(_line) => {
                #[cfg(debug_assertions)]
                eprintln!("[sync stderr] {}", _line);
            }
            ProcessEvent::Exit { code, success } => {
                if !success {
                    let _ = app_bg.emit(
                        EVENT_SYNC_ERROR,
                        crate::events::SyncErrorEvent {
                            code: "EXIT_NONZERO".to_string(),
                            message: format!(
                                "hq sync exited with code {}",
                                code.map(|c| c.to_string())
                                    .unwrap_or_else(|| "unknown".to_string())
                            ),
                        },
                    );
                }
            }
        });

        if let Err(e) = result {
            let _ = app_bg.emit(
                EVENT_SYNC_ERROR,
                crate::events::SyncErrorEvent {
                    code: "SPAWN_FAILED".to_string(),
                    message: e,
                },
            );
        }
    });

    Ok(SYNC_HANDLE.to_string())
}

/// Cancel a running sync.
///
/// Sends SIGTERM to the process group. If the process doesn't exit within 5
/// seconds, SIGKILL is sent.
///
/// Returns `true` if a sync was running and cancellation was initiated.
#[tauri::command]
pub fn cancel_sync() -> bool {
    cancel_process_impl(SYNC_HANDLE, SIGKILL_DELAY)
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_sync_spawn_args_cmd_and_args() {
        let args = build_sync_spawn_args("/Users/test/HQ");
        assert_eq!(args.cmd, "hq");
        assert_eq!(
            args.args,
            vec!["sync", "--json", "--hq-path", "/Users/test/HQ"]
        );
    }

    #[test]
    fn test_build_sync_spawn_args_env() {
        let args = build_sync_spawn_args("/Users/test/HQ");
        let env = args.env.unwrap();
        assert_eq!(env.get("HQ_ROOT"), Some(&"/Users/test/HQ".to_string()));
        assert_eq!(env.len(), 1);
    }

    #[test]
    fn test_build_sync_spawn_args_no_cwd() {
        let args = build_sync_spawn_args("/any/path");
        assert!(args.cwd.is_none());
    }

    #[test]
    fn test_parse_progress_ndjson() {
        let line = r#"{"type":"progress","phase":"uploading","filesComplete":3,"filesTotal":10}"#;
        let event: SyncEvent = serde_json::from_str(line).unwrap();
        match event {
            SyncEvent::Progress(p) => {
                assert_eq!(p.phase, "uploading");
                assert_eq!(p.files_complete, 3);
                assert_eq!(p.files_total, 10);
            }
            _ => panic!("Expected Progress event"),
        }
    }

    #[test]
    fn test_parse_conflict_ndjson() {
        let line = r#"{"type":"conflict","path":"file.txt","localHash":"aaa","remoteHash":"bbb","canAutoResolve":true}"#;
        let event: SyncEvent = serde_json::from_str(line).unwrap();
        match event {
            SyncEvent::Conflict(c) => {
                assert_eq!(c.path, "file.txt");
                assert!(c.can_auto_resolve);
            }
            _ => panic!("Expected Conflict event"),
        }
    }

    #[test]
    fn test_parse_error_ndjson() {
        let line = r#"{"type":"error","code":"NET_FAIL","message":"Connection reset"}"#;
        let event: SyncEvent = serde_json::from_str(line).unwrap();
        match event {
            SyncEvent::Error(e) => {
                assert_eq!(e.code, "NET_FAIL");
                assert_eq!(e.message, "Connection reset");
            }
            _ => panic!("Expected Error event"),
        }
    }

    #[test]
    fn test_parse_complete_ndjson() {
        let line = r#"{"type":"complete","filesChanged":7,"bytesTransferred":204800,"journalPath":"/tmp/j.log"}"#;
        let event: SyncEvent = serde_json::from_str(line).unwrap();
        match event {
            SyncEvent::Complete(c) => {
                assert_eq!(c.files_changed, 7);
                assert_eq!(c.bytes_transferred, 204800);
                assert_eq!(c.journal_path, "/tmp/j.log");
            }
            _ => panic!("Expected Complete event"),
        }
    }

    #[test]
    fn test_unknown_event_type_skipped() {
        let line = r#"{"type":"metrics","cpu":50}"#;
        let result: Result<SyncEvent, _> = serde_json::from_str(line);
        assert!(result.is_err(), "Unknown type should fail to parse");
    }

    #[test]
    fn test_malformed_json_skipped() {
        let line = "not json at all";
        let result: Result<SyncEvent, _> = serde_json::from_str(line);
        assert!(result.is_err());
    }

    #[test]
    fn test_empty_line_skipped() {
        let line = "";
        let result: Result<SyncEvent, _> = serde_json::from_str(line);
        assert!(result.is_err());
    }

    #[test]
    fn test_sync_handle_constant() {
        assert_eq!(SYNC_HANDLE, "hq-sync");
    }
}
