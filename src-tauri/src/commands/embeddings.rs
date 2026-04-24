//! Tauri commands for running `qmd embed` as a singleton background job.
//!
//! Mirrors `commands::sync` closely: singleton handle, streamed stdout/stderr
//! via [`run_process_impl`], SIGTERM → SIGKILL cancel, typed events emitted to
//! the Svelte renderer, and a journal file written on exit for status polling.
//!
//! ## Why a dedicated command (not `spawn_process`)
//! Embeddings can take 10+ minutes on CPU-only machines. We want the same
//! guarantees sync has — singleton enforcement, cancel-with-grace, crash-safe
//! journal — plus a *visible* last-run stderr tail when things go wrong.
//! Reusing `spawn_process` would mean reinventing all of that in the renderer.
//!
//! ## Cross-repo handoff
//! The hq-installer Verify step writes a pending marker
//! (`{hq_folder}/.hq-embeddings-pending.json` or `~/.hq/embeddings-pending.json`);
//! on a successful run this command removes both locations. On an error run the
//! marker is deliberately *kept* so the next app launch can retry.

use std::collections::VecDeque;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use chrono::SecondsFormat;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};

use crate::commands::config::{HqConfig, MenubarPrefs};
use crate::commands::process::{
    cancel_process_impl, deregister_process, run_process_impl, try_register_handle, ProcessEvent,
    SpawnArgs,
};
use crate::util::paths;

// ─────────────────────────────────────────────────────────────────────────────
// Constants
// ─────────────────────────────────────────────────────────────────────────────

/// Singleton handle — only one `qmd embed` run at a time.
pub const EMBEDDINGS_HANDLE: &str = "hq-embeddings";

/// SIGKILL delay after SIGTERM on cancel. Matches sync.rs.
pub const SIGKILL_DELAY: Duration = Duration::from_secs(5);

/// How much stderr to persist into the error journal (bytes). Keeps the
/// journal diagnostic without unbounded growth on a wedged `qmd embed` that
/// spams stderr for an hour.
pub const STDERR_TAIL_BYTES: usize = 2048;

/// Event names emitted to the renderer. Svelte listens for these in
/// `App.svelte` and on the popover/settings surfaces.
pub const EVENT_EMBEDDINGS_START: &str = "embeddings:start";
pub const EVENT_EMBEDDINGS_PROGRESS: &str = "embeddings:progress";
pub const EVENT_EMBEDDINGS_COMPLETE: &str = "embeddings:complete";
pub const EVENT_EMBEDDINGS_ERROR: &str = "embeddings:error";

// ─────────────────────────────────────────────────────────────────────────────
// Event payloads
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EmbeddingsStartEvent {
    pub reason: String,
    pub started_at: String, // ISO8601
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EmbeddingsProgressEvent {
    pub line: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EmbeddingsCompleteEvent {
    pub duration_sec: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EmbeddingsErrorEvent {
    pub message: String,
}

// ─────────────────────────────────────────────────────────────────────────────
// Journal file
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EmbeddingsJournal {
    pub last_run_at: String, // ISO8601
    pub duration_sec: u64,
    /// Either "ok" or "error".
    pub state: String,
    pub error_msg: Option<String>,
}

/// Status returned to the frontend by `get_embeddings_status`.
///
/// Distinct from [`EmbeddingsJournal`] because we want to expose a `journal`
/// source marker ("journal" | "none") the way [`crate::commands::status`]
/// does — the UI wants to know whether it's looking at a real last-run or a
/// synthetic default.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EmbeddingsStatus {
    pub last_run_at: Option<String>,
    pub duration_sec: u64,
    /// "ok" | "error" | "unknown" (when no journal yet).
    pub state: String,
    pub error_msg: Option<String>,
    /// "journal" | "none".
    pub source: String,
}

pub fn default_status() -> EmbeddingsStatus {
    EmbeddingsStatus {
        last_run_at: None,
        duration_sec: 0,
        state: "unknown".to_string(),
        error_msg: None,
        source: "none".to_string(),
    }
}

fn status_from_journal(j: EmbeddingsJournal) -> EmbeddingsStatus {
    EmbeddingsStatus {
        last_run_at: Some(j.last_run_at),
        duration_sec: j.duration_sec,
        state: j.state,
        error_msg: j.error_msg,
        source: "journal".to_string(),
    }
}

/// Serialize + write the journal to `{hq_folder}/.hq-embeddings-journal.json`.
pub fn write_embeddings_journal(
    hq_folder: &str,
    journal: &EmbeddingsJournal,
) -> Result<(), String> {
    let path = paths::embeddings_journal_path(Path::new(hq_folder));
    let contents = serde_json::to_string_pretty(journal)
        .map_err(|e| format!("Failed to serialize embeddings journal: {}", e))?;
    std::fs::write(&path, contents)
        .map_err(|e| format!("Failed to write embeddings journal: {}", e))?;
    Ok(())
}

/// Read the journal, returning `Err` when missing or malformed. Callers use
/// this as a best-effort probe and fall back to [`default_status`].
pub fn read_embeddings_journal(hq_folder: &str) -> Result<EmbeddingsJournal, String> {
    let path = paths::embeddings_journal_path(Path::new(hq_folder));
    let contents = std::fs::read_to_string(&path)
        .map_err(|e| format!("Failed to read embeddings journal: {}", e))?;
    serde_json::from_str::<EmbeddingsJournal>(contents.trim())
        .map_err(|e| format!("Failed to parse embeddings journal: {}", e))
}

// ─────────────────────────────────────────────────────────────────────────────
// Pending marker cleanup
// ─────────────────────────────────────────────────────────────────────────────

/// Remove both pending-marker locations. Called on a successful run only.
/// Best-effort: missing files are not treated as errors.
pub fn clear_pending_markers(hq_folder: &str) {
    for candidate in paths::embeddings_pending_paths(Path::new(hq_folder)) {
        if candidate.exists() {
            let _ = std::fs::remove_file(&candidate);
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Config resolution (same pattern as sync.rs / status.rs)
// ─────────────────────────────────────────────────────────────────────────────

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
        config.as_ref().and_then(|c| c.hq_folder_path.as_deref()),
        menubar_prefs.as_ref().and_then(|p| p.hq_path.as_deref()),
    );

    Ok(hq_folder.to_string_lossy().to_string())
}

// ─────────────────────────────────────────────────────────────────────────────
// Spawn args builder
// ─────────────────────────────────────────────────────────────────────────────

/// Build `qmd embed` spawn args. Cwd is the HQ folder so `qmd` picks up the
/// right collection from the surrounding config.
pub fn build_embeddings_spawn_args(hq_folder_path: &str) -> SpawnArgs {
    use std::collections::HashMap;

    let mut env = HashMap::new();
    // Same shebang/PATH gotcha as sync.rs — on Dock launches the minimal
    // launchd PATH can't find `node` / other interpreters `qmd` shells out to.
    env.insert("PATH".to_string(), paths::child_path());

    SpawnArgs {
        cmd: paths::resolve_bin("qmd"),
        args: vec!["embed".to_string()],
        cwd: Some(hq_folder_path.to_string()),
        env: Some(env),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Stderr ring buffer
// ─────────────────────────────────────────────────────────────────────────────

/// Append a stderr line (plus a trailing `\n`) to `buf`, trimming from the
/// front so only the last `STDERR_TAIL_BYTES` bytes are retained. `qmd embed`
/// can produce many MB of progress noise on stderr; we only care about the
/// tail for the error journal.
///
/// Separated for testability — no I/O, no thread state.
pub fn push_stderr_tail(buf: &mut VecDeque<u8>, line: &str) {
    for b in line.as_bytes() {
        buf.push_back(*b);
    }
    buf.push_back(b'\n');
    while buf.len() > STDERR_TAIL_BYTES {
        buf.pop_front();
    }
}

/// Extract the stderr buffer as a lossy UTF-8 string.
pub fn stderr_tail_to_string(buf: &VecDeque<u8>) -> String {
    // Two-slice view of VecDeque — avoids copying for the contiguous case but
    // handles the wrap-around case correctly.
    let (a, b) = buf.as_slices();
    let mut bytes = Vec::with_capacity(a.len() + b.len());
    bytes.extend_from_slice(a);
    bytes.extend_from_slice(b);
    String::from_utf8_lossy(&bytes).into_owned()
}

// ─────────────────────────────────────────────────────────────────────────────
// Tauri commands
// ─────────────────────────────────────────────────────────────────────────────

/// Spawn `qmd embed` as a singleton child process.
///
/// Emits typed events as ndjson-like progress lines arrive from stdout, and
/// writes a journal file to the HQ folder on exit. The pending marker is
/// cleared only on successful exit — failures leave it in place so the next
/// launch (or a manual Retry from Settings) can pick it up.
///
/// Returns the handle string on success (always `EMBEDDINGS_HANDLE`).
#[tauri::command]
pub fn start_embeddings(app: AppHandle, reason: String) -> Result<String, String> {
    #[cfg(debug_assertions)]
    eprintln!("[embeddings] start_embeddings invoked (reason={})", reason);

    // Atomic check-and-register — matches sync's anti-double-start semantics.
    if !try_register_handle(EMBEDDINGS_HANDLE) {
        #[cfg(debug_assertions)]
        eprintln!("[embeddings] BAIL: already running");
        return Err("already running".to_string());
    }

    let hq_folder_path = match resolve_hq_folder_path() {
        Ok(p) => p,
        Err(e) => {
            #[cfg(debug_assertions)]
            eprintln!("[embeddings] BAIL: resolve_hq_folder_path failed: {}", e);
            deregister_process(EMBEDDINGS_HANDLE);
            return Err(e);
        }
    };
    let spawn_args = build_embeddings_spawn_args(&hq_folder_path);

    // Emit `start` immediately from the caller thread so the UI can flip state
    // before the first stdout line. started_at is the same timestamp we'll
    // use as `lastRunAt` on success.
    let started_at = chrono::Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);
    let _ = app.emit(
        EVENT_EMBEDDINGS_START,
        EmbeddingsStartEvent {
            reason: reason.clone(),
            started_at: started_at.clone(),
        },
    );

    // Background thread owns the subprocess and streams events.
    let app_bg = app.clone();
    let hq_folder_for_handler = hq_folder_path.clone();
    // Bounded tail — we only persist the last 2KB into the error journal.
    let stderr_tail: Arc<Mutex<VecDeque<u8>>> =
        Arc::new(Mutex::new(VecDeque::with_capacity(STDERR_TAIL_BYTES + 256)));
    let stderr_tail_handler = Arc::clone(&stderr_tail);
    let start_instant = Instant::now();

    thread::spawn(move || {
        let result = run_process_impl(
            EMBEDDINGS_HANDLE,
            &spawn_args,
            |event| match event {
                ProcessEvent::Stdout(line) => {
                    let _ = app_bg.emit(
                        EVENT_EMBEDDINGS_PROGRESS,
                        EmbeddingsProgressEvent { line },
                    );
                }
                ProcessEvent::Stderr(line) => {
                    #[cfg(debug_assertions)]
                    eprintln!("[embeddings stderr] {}", line);
                    // Capture for the error journal; also surface to the live
                    // progress feed so users can see warnings from `qmd`.
                    {
                        let mut buf =
                            stderr_tail_handler.lock().unwrap_or_else(|e| e.into_inner());
                        push_stderr_tail(&mut buf, &line);
                    }
                    let _ = app_bg.emit(
                        EVENT_EMBEDDINGS_PROGRESS,
                        EmbeddingsProgressEvent { line },
                    );
                }
                ProcessEvent::Exit { code, success } => {
                    let duration_sec = start_instant.elapsed().as_secs();
                    let now_iso =
                        chrono::Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);

                    if success {
                        // Journal: state=ok, no error. Then clear both markers.
                        let journal = EmbeddingsJournal {
                            last_run_at: now_iso,
                            duration_sec,
                            state: "ok".to_string(),
                            error_msg: None,
                        };
                        if let Err(_e) =
                            write_embeddings_journal(&hq_folder_for_handler, &journal)
                        {
                            #[cfg(debug_assertions)]
                            eprintln!("[embeddings] journal write failed: {}", _e);
                        }
                        clear_pending_markers(&hq_folder_for_handler);
                        let _ = app_bg.emit(
                            EVENT_EMBEDDINGS_COMPLETE,
                            EmbeddingsCompleteEvent { duration_sec },
                        );
                    } else {
                        // Non-zero exit: persist stderr tail, keep the marker.
                        let tail = {
                            let buf = stderr_tail_handler
                                .lock()
                                .unwrap_or_else(|e| e.into_inner());
                            stderr_tail_to_string(&buf)
                        };
                        let err_msg = if tail.trim().is_empty() {
                            format!(
                                "qmd embed exited with code {}",
                                code.map(|c| c.to_string())
                                    .unwrap_or_else(|| "unknown".to_string())
                            )
                        } else {
                            tail.trim_end().to_string()
                        };
                        let journal = EmbeddingsJournal {
                            last_run_at: now_iso,
                            duration_sec,
                            state: "error".to_string(),
                            error_msg: Some(err_msg.clone()),
                        };
                        if let Err(_e) =
                            write_embeddings_journal(&hq_folder_for_handler, &journal)
                        {
                            #[cfg(debug_assertions)]
                            eprintln!("[embeddings] error-journal write failed: {}", _e);
                        }
                        let _ = app_bg.emit(
                            EVENT_EMBEDDINGS_ERROR,
                            EmbeddingsErrorEvent { message: err_msg },
                        );
                    }
                }
            },
        );

        // Spawn-level failure (e.g. `qmd` not on PATH) surfaces here — run_process_impl
        // already emitted an Exit event, but the spawn error carries the root cause
        // string we want in the journal + error event. Deregister defensively in case
        // run_process_impl bailed before registering.
        if let Err(e) = result {
            let duration_sec = start_instant.elapsed().as_secs();
            let now_iso = chrono::Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);
            let journal = EmbeddingsJournal {
                last_run_at: now_iso,
                duration_sec,
                state: "error".to_string(),
                error_msg: Some(e.clone()),
            };
            let _ = write_embeddings_journal(&hq_folder_for_handler, &journal);
            let _ = app_bg.emit(
                EVENT_EMBEDDINGS_ERROR,
                EmbeddingsErrorEvent { message: e },
            );
            deregister_process(EMBEDDINGS_HANDLE);
        }
    });

    Ok(EMBEDDINGS_HANDLE.to_string())
}

/// Cancel a running embeddings job. Mirrors `cancel_sync`: SIGTERM, 5s grace,
/// then SIGKILL.
#[tauri::command]
pub fn cancel_embeddings() -> bool {
    cancel_process_impl(EMBEDDINGS_HANDLE, SIGKILL_DELAY)
}

/// Read the embeddings journal and return a status payload for the UI.
/// When no journal exists (first launch before any run), returns a
/// `source: "none"` default.
#[tauri::command]
pub async fn get_embeddings_status() -> Result<EmbeddingsStatus, String> {
    let hq_folder_path = resolve_hq_folder_path()?;
    match read_embeddings_journal(&hq_folder_path) {
        Ok(journal) => Ok(status_from_journal(journal)),
        Err(_e) => {
            #[cfg(debug_assertions)]
            eprintln!(
                "[embeddings] journal not available, returning default: {}",
                _e
            );
            Ok(default_status())
        }
    }
}

// Test-only helper: compute the journal path without exposing paths:: to tests
// that only need this module.
#[cfg(test)]
fn test_journal_path(hq_folder: &str) -> std::path::PathBuf {
    paths::embeddings_journal_path(Path::new(hq_folder))
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    // ── Constants ────────────────────────────────────────────────────────────

    #[test]
    fn test_embeddings_handle_constant() {
        assert_eq!(EMBEDDINGS_HANDLE, "hq-embeddings");
    }

    #[test]
    fn test_event_name_constants() {
        assert_eq!(EVENT_EMBEDDINGS_START, "embeddings:start");
        assert_eq!(EVENT_EMBEDDINGS_PROGRESS, "embeddings:progress");
        assert_eq!(EVENT_EMBEDDINGS_COMPLETE, "embeddings:complete");
        assert_eq!(EVENT_EMBEDDINGS_ERROR, "embeddings:error");
    }

    #[test]
    fn test_sigkill_delay_matches_sync() {
        assert_eq!(SIGKILL_DELAY, Duration::from_secs(5));
    }

    // ── SpawnArgs ────────────────────────────────────────────────────────────

    #[test]
    fn test_build_embeddings_spawn_args_cmd() {
        let args = build_embeddings_spawn_args("/Users/test/HQ");
        // resolve_bin may absolutize on dev boxes; either way the basename is `qmd`.
        assert!(
            args.cmd == "qmd" || args.cmd.ends_with("/qmd"),
            "expected cmd to be `qmd` or `*/qmd`, got `{}`",
            args.cmd
        );
    }

    #[test]
    fn test_build_embeddings_spawn_args_only_embed_flag() {
        let args = build_embeddings_spawn_args("/Users/test/HQ");
        assert_eq!(args.args, vec!["embed".to_string()]);
    }

    #[test]
    fn test_build_embeddings_spawn_args_cwd_is_hq_folder() {
        let args = build_embeddings_spawn_args("/Users/test/HQ");
        assert_eq!(args.cwd, Some("/Users/test/HQ".to_string()));
    }

    #[test]
    fn test_build_embeddings_spawn_args_env_has_path_with_homebrew() {
        let args = build_embeddings_spawn_args("/Users/test/HQ");
        let env = args.env.expect("env must be set");
        let path = env.get("PATH").expect("PATH must be set");
        assert!(path.contains("/opt/homebrew/bin"));
    }

    // ── Event payload serialization (camelCase) ──────────────────────────────

    #[test]
    fn test_start_event_serializes_camel_case() {
        let ev = EmbeddingsStartEvent {
            reason: "post-install".to_string(),
            started_at: "2026-04-24T07:00:00.000Z".to_string(),
        };
        let json = serde_json::to_string(&ev).unwrap();
        assert!(json.contains("\"startedAt\""));
        assert!(!json.contains("\"started_at\""));
        assert!(json.contains("\"post-install\""));
    }

    #[test]
    fn test_complete_event_serializes_camel_case() {
        let ev = EmbeddingsCompleteEvent { duration_sec: 120 };
        let json = serde_json::to_string(&ev).unwrap();
        assert!(json.contains("\"durationSec\":120"));
    }

    #[test]
    fn test_error_event_serializes_camel_case() {
        let ev = EmbeddingsErrorEvent {
            message: "qmd not found".to_string(),
        };
        let json = serde_json::to_string(&ev).unwrap();
        assert!(json.contains("\"message\""));
        assert!(json.contains("\"qmd not found\""));
    }

    // ── Journal round-trip (AC: journal write/read roundtrip) ───────────────

    #[test]
    fn test_journal_roundtrip_ok_state() {
        let tmp = tempdir().unwrap();
        let hq_folder = tmp.path().to_str().unwrap();
        let journal = EmbeddingsJournal {
            last_run_at: "2026-04-24T07:00:00.000Z".to_string(),
            duration_sec: 45,
            state: "ok".to_string(),
            error_msg: None,
        };

        write_embeddings_journal(hq_folder, &journal).unwrap();
        let path = test_journal_path(hq_folder);
        assert!(path.exists(), "journal file should exist");
        assert!(path.ends_with(".hq-embeddings-journal.json"));

        let read_back = read_embeddings_journal(hq_folder).unwrap();
        assert_eq!(read_back, journal);
    }

    #[test]
    fn test_journal_serializes_camel_case_keys() {
        let tmp = tempdir().unwrap();
        let hq_folder = tmp.path().to_str().unwrap();
        let journal = EmbeddingsJournal {
            last_run_at: "2026-04-24T07:00:00.000Z".to_string(),
            duration_sec: 10,
            state: "ok".to_string(),
            error_msg: None,
        };
        write_embeddings_journal(hq_folder, &journal).unwrap();
        let contents = std::fs::read_to_string(test_journal_path(hq_folder)).unwrap();
        assert!(contents.contains("\"lastRunAt\""));
        assert!(contents.contains("\"durationSec\""));
        assert!(contents.contains("\"errorMsg\""));
        assert!(!contents.contains("\"last_run_at\""));
        assert!(!contents.contains("\"duration_sec\""));
        assert!(!contents.contains("\"error_msg\""));
    }

    // ── Error-path journal (AC: error-path journal write) ────────────────────

    #[test]
    fn test_journal_roundtrip_error_state_preserves_stderr_tail() {
        let tmp = tempdir().unwrap();
        let hq_folder = tmp.path().to_str().unwrap();
        let journal = EmbeddingsJournal {
            last_run_at: "2026-04-24T07:00:00.000Z".to_string(),
            duration_sec: 3,
            state: "error".to_string(),
            error_msg: Some("qmd: ModelResolutionError: checksum mismatch".to_string()),
        };

        write_embeddings_journal(hq_folder, &journal).unwrap();
        let read_back = read_embeddings_journal(hq_folder).unwrap();
        assert_eq!(read_back.state, "error");
        assert_eq!(
            read_back.error_msg,
            Some("qmd: ModelResolutionError: checksum mismatch".to_string())
        );
    }

    #[test]
    fn test_journal_overwrites_prior_error_on_success() {
        let tmp = tempdir().unwrap();
        let hq_folder = tmp.path().to_str().unwrap();

        // Simulate a failed run followed by a successful retry.
        let err = EmbeddingsJournal {
            last_run_at: "2026-04-24T06:30:00.000Z".to_string(),
            duration_sec: 1,
            state: "error".to_string(),
            error_msg: Some("transient failure".to_string()),
        };
        write_embeddings_journal(hq_folder, &err).unwrap();

        let ok = EmbeddingsJournal {
            last_run_at: "2026-04-24T07:00:00.000Z".to_string(),
            duration_sec: 90,
            state: "ok".to_string(),
            error_msg: None,
        };
        write_embeddings_journal(hq_folder, &ok).unwrap();

        let read_back = read_embeddings_journal(hq_folder).unwrap();
        assert_eq!(read_back.state, "ok");
        assert_eq!(read_back.error_msg, None);
        assert_eq!(read_back.duration_sec, 90);
    }

    #[test]
    fn test_read_embeddings_journal_errors_on_missing_file() {
        let tmp = tempdir().unwrap();
        let hq_folder = tmp.path().to_str().unwrap();
        let result = read_embeddings_journal(hq_folder);
        assert!(result.is_err());
    }

    #[test]
    fn test_read_embeddings_journal_errors_on_malformed_json() {
        let tmp = tempdir().unwrap();
        let hq_folder = tmp.path().to_str().unwrap();
        std::fs::write(
            tmp.path().join(".hq-embeddings-journal.json"),
            "not json at all",
        )
        .unwrap();
        let result = read_embeddings_journal(hq_folder);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Failed to parse"));
    }

    // ── Status derivation ────────────────────────────────────────────────────

    #[test]
    fn test_default_status_is_unknown_and_none_source() {
        let s = default_status();
        assert_eq!(s.source, "none");
        assert_eq!(s.state, "unknown");
        assert_eq!(s.last_run_at, None);
        assert_eq!(s.duration_sec, 0);
        assert_eq!(s.error_msg, None);
    }

    #[test]
    fn test_status_from_journal_sets_source_journal() {
        let j = EmbeddingsJournal {
            last_run_at: "2026-04-24T07:00:00.000Z".to_string(),
            duration_sec: 60,
            state: "ok".to_string(),
            error_msg: None,
        };
        let s = status_from_journal(j);
        assert_eq!(s.source, "journal");
        assert_eq!(s.state, "ok");
        assert_eq!(s.last_run_at.as_deref(), Some("2026-04-24T07:00:00.000Z"));
        assert_eq!(s.duration_sec, 60);
    }

    // ── Marker cleanup (AC: marker cleanup) ──────────────────────────────────

    #[test]
    fn test_clear_pending_markers_removes_primary_in_hq_folder() {
        let tmp = tempdir().unwrap();
        let hq_folder = tmp.path().to_str().unwrap();
        let primary = tmp.path().join(".hq-embeddings-pending.json");
        std::fs::write(&primary, r#"{"reason":"post-install"}"#).unwrap();
        assert!(primary.exists());

        clear_pending_markers(hq_folder);
        assert!(!primary.exists(), "primary marker should be removed");
    }

    #[test]
    fn test_clear_pending_markers_is_noop_when_missing() {
        let tmp = tempdir().unwrap();
        let hq_folder = tmp.path().to_str().unwrap();
        // No markers present — should not panic or error.
        clear_pending_markers(hq_folder);
        let primary = tmp.path().join(".hq-embeddings-pending.json");
        assert!(!primary.exists());
    }

    // ── Stderr ring buffer (for 2KB tail guarantee) ──────────────────────────

    #[test]
    fn test_push_stderr_tail_under_budget_preserves_all() {
        let mut buf = VecDeque::new();
        push_stderr_tail(&mut buf, "line 1");
        push_stderr_tail(&mut buf, "line 2");
        let s = stderr_tail_to_string(&buf);
        assert_eq!(s, "line 1\nline 2\n");
    }

    #[test]
    fn test_push_stderr_tail_truncates_front_over_budget() {
        let mut buf = VecDeque::new();
        // 4x ~800-byte lines — forces the buffer past 2KB.
        let big_line = "x".repeat(800);
        for _ in 0..4 {
            push_stderr_tail(&mut buf, &big_line);
        }
        assert!(
            buf.len() <= STDERR_TAIL_BYTES,
            "buffer should be bounded: len={}",
            buf.len()
        );
        let s = stderr_tail_to_string(&buf);
        // Tail must still be made of the recent content.
        assert!(s.contains("xxxx"));
    }

    #[test]
    fn test_push_stderr_tail_handles_huge_single_line() {
        let mut buf = VecDeque::new();
        let big = "q".repeat(STDERR_TAIL_BYTES * 3);
        push_stderr_tail(&mut buf, &big);
        assert!(buf.len() <= STDERR_TAIL_BYTES);
        let s = stderr_tail_to_string(&buf);
        // All surviving bytes should be `q` (and possibly the trailing '\n'
        // was trimmed out by the truncation — acceptable since we keep the tail).
        assert!(s.chars().all(|c| c == 'q' || c == '\n'));
    }

    // ── Singleton enforcement path ───────────────────────────────────────────

    #[test]
    fn test_try_register_handle_rejects_second_caller() {
        // Use a unique handle so this test doesn't collide with other tests or
        // a live registration leaking across threads.
        let handle = format!("hq-embeddings-test-{}", std::process::id());
        assert!(try_register_handle(&handle), "first call must succeed");
        assert!(
            !try_register_handle(&handle),
            "second call must fail (already registered)"
        );
        deregister_process(&handle);
        assert!(
            try_register_handle(&handle),
            "after deregister, re-registration must succeed"
        );
        deregister_process(&handle);
    }
}
