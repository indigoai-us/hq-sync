//! Append-only diagnostic log at `~/.hq/logs/hq-sync.log`.
//!
//! All `eprintln!` checkpoints in the sync pipeline are gated on
//! `#[cfg(debug_assertions)]` and only land in the terminal where `tauri dev`
//! was launched. That is fine for active development but leaves zero
//! breadcrumbs when the menubar app is launched normally and a sync hangs.
//! This module gives those checkpoints a persistent destination so we can
//! diagnose stuck syncs after the fact.
//!
//! Design notes:
//! - **Best-effort, never panic.** A logging failure must not break sync —
//!   the file handle is opened lazily and any I/O error is swallowed.
//! - **Single global handle behind a `Mutex`.** Sync emits roughly one line
//!   per ndjson event; lock contention is irrelevant at that rate.
//! - **No rotation.** The file grows unbounded; users can `rm` it. A future
//!   nightly truncate is fine to add when this becomes an actual problem.

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use chrono::SecondsFormat;

use super::paths::hq_config_dir;

#[cfg(test)]
static LOG_PATH_TEST_OVERRIDE: OnceLock<Mutex<Option<PathBuf>>> = OnceLock::new();

/// Returns `~/.hq/logs/hq-sync.log`. The directory is created on demand.
///
/// In test builds an override slot is consulted first so tests can redirect
/// the log to an isolated tempdir without mutating `HOME` (which `dirs::home_dir`
/// falls back to via passwd, so HOME-mutation isn't sufficient anyway).
pub fn log_path() -> Result<PathBuf, String> {
    #[cfg(test)]
    {
        if let Some(slot) = LOG_PATH_TEST_OVERRIDE.get() {
            if let Ok(guard) = slot.lock() {
                if let Some(p) = guard.clone() {
                    if let Some(parent) = p.parent() {
                        if !parent.exists() {
                            fs::create_dir_all(parent)
                                .map_err(|e| format!("create logs dir: {e}"))?;
                        }
                    }
                    return Ok(p);
                }
            }
        }
    }
    let dir = hq_config_dir()?.join("logs");
    if !dir.exists() {
        fs::create_dir_all(&dir).map_err(|e| format!("create logs dir: {e}"))?;
    }
    Ok(dir.join("hq-sync.log"))
}

static LOG_FILE: OnceLock<Mutex<Option<File>>> = OnceLock::new();

fn handle() -> &'static Mutex<Option<File>> {
    LOG_FILE.get_or_init(|| Mutex::new(None))
}

fn ensure_open(slot: &mut Option<File>) {
    if slot.is_some() {
        return;
    }
    let path = match log_path() {
        Ok(p) => p,
        Err(_) => return,
    };
    if let Ok(file) = OpenOptions::new().create(true).append(true).open(&path) {
        *slot = Some(file);
    }
}

/// Append a single timestamped line tagged with `tag` to the log file.
///
/// Best-effort: any failure (no home dir, disk full, file vanished) is
/// silently swallowed. The shape is:
///
/// ```text
/// 2026-04-25T13:45:09.123Z [sync] start_sync invoked
/// ```
pub fn log(tag: &str, msg: &str) {
    let line = format!(
        "{} [{}] {}\n",
        chrono::Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true),
        tag,
        msg,
    );
    let mutex = handle();
    let mut slot = match mutex.lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    };
    ensure_open(&mut slot);
    if let Some(file) = slot.as_mut() {
        let _ = file.write_all(line.as_bytes());
        // Flush each line so a hung sync still leaves a trail. The volume
        // is too low (one line per ndjson event) for fsync overhead to
        // matter.
        let _ = file.flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Process-wide mutex — every test in this module mutates the
    /// `LOG_PATH_TEST_OVERRIDE` slot and the global `LOG_FILE` cache, so they
    /// cannot run in parallel without trampling each other.
    static TEST_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

    fn lock() -> std::sync::MutexGuard<'static, ()> {
        TEST_LOCK
            .get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|p| p.into_inner())
    }

    /// Point `log_path()` at the given file and drop the cached file handle
    /// so the next `log()` reopens against the override. Returning the
    /// `TempDir` keeps the directory alive for the test's body.
    fn with_test_log() -> tempfile::TempDir {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("hq-sync.log");
        let slot = LOG_PATH_TEST_OVERRIDE.get_or_init(|| Mutex::new(None));
        *slot.lock().unwrap_or_else(|p| p.into_inner()) = Some(path);

        if let Some(slot) = LOG_FILE.get() {
            if let Ok(mut g) = slot.lock() {
                *g = None;
            }
        }
        tmp
    }

    fn clear_override() {
        if let Some(slot) = LOG_PATH_TEST_OVERRIDE.get() {
            if let Ok(mut g) = slot.lock() {
                *g = None;
            }
        }
        if let Some(slot) = LOG_FILE.get() {
            if let Ok(mut g) = slot.lock() {
                *g = None;
            }
        }
    }

    #[test]
    fn test_log_path_default_under_dot_hq_logs() {
        let _g = lock();
        clear_override();

        let path = log_path().unwrap();
        assert!(
            path.ends_with(".hq/logs/hq-sync.log"),
            "default path must live under ~/.hq/logs, got {path:?}"
        );
    }

    #[test]
    fn test_log_appends_timestamped_line() {
        let _g = lock();
        let tmp = with_test_log();
        let path = tmp.path().join("hq-sync.log");

        log("sync", "first message");
        log("sync", "second message");

        let contents = fs::read_to_string(&path).unwrap();
        assert!(contents.contains("[sync] first message"));
        assert!(contents.contains("[sync] second message"));
        assert!(contents.starts_with("20"));
        assert!(contents.contains("Z [sync]"));
        assert_eq!(contents.matches('\n').count(), 2);

        clear_override();
    }

    #[test]
    fn test_log_swallows_errors_when_path_unwritable() {
        let _g = lock();
        // Point the override at a path inside a non-existent parent that
        // lives under a *file* (not a directory) — `create_dir_all` cannot
        // succeed because `/dev/null` is a character device. Best-effort
        // logging must swallow the error, not panic.
        let bad = std::path::PathBuf::from("/dev/null/cannot-create/hq-sync.log");
        let slot = LOG_PATH_TEST_OVERRIDE.get_or_init(|| Mutex::new(None));
        *slot.lock().unwrap_or_else(|p| p.into_inner()) = Some(bad);
        if let Some(slot) = LOG_FILE.get() {
            if let Ok(mut g) = slot.lock() {
                *g = None;
            }
        }

        log("sync", "should not panic");

        clear_override();
    }
}
