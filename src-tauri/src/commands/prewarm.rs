//! Background warm-up of the npx cache for `@indigoai-us/hq-cloud`.
//!
//! ## Why this exists
//!
//! The sync path spawns
//! `npx -y --package=@indigoai-us/hq-cloud@<ver> hq-sync-runner …` (see
//! `commands::sync`). The *first* invocation after a fresh install — or
//! after bumping `sync::HQ_CLOUD_VERSION` — downloads the package into
//! npx's on-disk cache (`~/.npm/_npx/<hash>/`). That download takes
//! ~3–10s, which would otherwise pad the user's first click of
//! "Sync Now" and feel like the app is broken.
//!
//! By doing the same download in the background at app startup, the
//! cache is warm by the time the user actually triggers a sync. The
//! second and all subsequent syncs are then near-instant (~100ms npx
//! overhead). No-ops if the cache is already warm.
//!
//! ## Why fire-and-forget is safe
//!
//! Prewarm is a pure side-effect with no state to surface. If it
//! succeeds, the next sync is fast. If it fails (offline, npm registry
//! down), the next sync will either reuse whatever is cached or fail
//! with the same network error. Pre-warm failure and sync failure are
//! independent — there's nothing to roll back, retry, or report. We log
//! one stderr line per attempt for offline debugging and drop the
//! `JoinHandle`.
//!
//! ## Why `std::thread` and not tokio
//!
//! Tauri's `setup` callback runs synchronously on the main thread; we
//! need to return quickly so the tray icon appears. `std::thread::spawn`
//! is the simplest option — matches the existing pattern used for
//! feature-flagged daemon autostart in `main.rs`. No tokio runtime
//! dependency, no async-in-setup plumbing.
//!
//! ## What we spawn
//!
//! `npx -y --package=@indigoai-us/hq-cloud@<ver> hq-sync-runner --version`.
//! The `--version` invocation is the lightest runner mode that still
//! forces npx to materialise the package (bin entry must exist to
//! execute). Output is dropped; we only care about the side effect of
//! filling the cache.

use std::process::{Command, Stdio};
use std::thread;
use std::time::Instant;

use crate::commands::sync::{HQ_CLOUD_PACKAGE, HQ_CLOUD_VERSION, RUNNER_BIN};
use crate::util::paths;

/// Spawn a detached thread that warms the npx cache for
/// `@indigoai-us/hq-cloud@HQ_CLOUD_VERSION`. Returns immediately; the
/// caller never joins the thread.
///
/// Safe to call repeatedly — if the cache is already warm, npx is a
/// ~100ms no-op. No lock; concurrent invocations simply all hit the
/// same cache entry.
pub fn spawn_prewarm() {
    thread::spawn(|| {
        let started = Instant::now();
        let npx = paths::resolve_bin("npx");
        let package_spec = format!("--package={}@{}", HQ_CLOUD_PACKAGE, HQ_CLOUD_VERSION);
        let path = paths::child_path();

        let result = Command::new(&npx)
            .args([
                "-y",
                &package_spec,
                RUNNER_BIN,
                "--version",
            ])
            .env("PATH", &path)
            // Swallow output — we only care about the cache side effect.
            // Any useful diagnostic is in the exit status we log below.
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();

        let elapsed = started.elapsed();
        match result {
            Ok(status) if status.success() => {
                eprintln!(
                    "[prewarm] {}@{} warmed in {:.1}s",
                    HQ_CLOUD_PACKAGE,
                    HQ_CLOUD_VERSION,
                    elapsed.as_secs_f32(),
                );
            }
            Ok(status) => {
                eprintln!(
                    "[prewarm] npx exited with {} after {:.1}s — first sync may be slower",
                    status
                        .code()
                        .map(|c| c.to_string())
                        .unwrap_or_else(|| "signal".to_string()),
                    elapsed.as_secs_f32(),
                );
            }
            Err(err) => {
                eprintln!(
                    "[prewarm] failed to spawn npx after {:.1}s: {} — first sync may be slower",
                    elapsed.as_secs_f32(),
                    err,
                );
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Smoke test: `spawn_prewarm` must not block the caller. If the
    /// background thread tried to `join`, this test would time out.
    ///
    /// We don't assert the subprocess succeeded — on CI npx may not be
    /// on PATH, and that's exactly the failure mode `spawn_prewarm`
    /// logs-and-drops.
    #[test]
    fn test_spawn_prewarm_is_non_blocking() {
        let started = Instant::now();
        spawn_prewarm();
        let elapsed = started.elapsed();
        // 500ms is generous; the call should return in microseconds.
        // If this fails, someone accidentally made spawn_prewarm await
        // the child — which would block the Tauri setup callback.
        assert!(
            elapsed.as_millis() < 500,
            "spawn_prewarm blocked for {:?} — must return immediately",
            elapsed,
        );
    }
}
