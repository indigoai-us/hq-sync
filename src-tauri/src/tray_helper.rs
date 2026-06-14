//! Spawns + talks to the native menu-bar helper process (`hq-tray-helper`).
//!
//! On macOS Tahoe the main app's Tauri/tao runtime parks any NSStatusItem
//! off-screen (verified on-device across every app version + a native item; a
//! clean AppKit process places its item correctly). So the visible "HQ" menu-bar
//! item lives in a tiny separate AppKit helper. The helper writes one-word
//! commands to `~/.hq/.tray-cmd`; we poll that file and act on them. Trivial,
//! robust IPC — no sockets/signals/entitlements.
#![cfg(target_os = "macos")]

use std::path::PathBuf;
use std::time::Duration;

use tauri::{AppHandle, Emitter, Manager};

use crate::util::logfile::log;

fn cmd_file() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(PathBuf::from(home).join(".hq").join(".tray-cmd"))
}

/// Resolve the bundled helper binary. In a packaged .app it sits in
/// `Contents/Resources/`; in a dev `tauri build` bundle it's placed alongside
/// the main executable. Check both.
fn helper_path() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let macos_dir = exe.parent()?; // …/Contents/MacOS
    let candidates = [
        macos_dir.join("../Resources/hq-tray-helper"),
        macos_dir.join("hq-tray-helper"),
    ];
    candidates.into_iter().find(|p| p.exists())
}

/// Spawn the helper (passing our PID so it self-exits if we die) and start the
/// command-file poller. Call once from `.setup()` on macOS.
pub fn spawn_and_poll(app: &AppHandle) {
    let pid = std::process::id();
    match helper_path() {
        Some(hp) => match std::process::Command::new(&hp).arg(pid.to_string()).spawn() {
            Ok(_) => log("tray", &format!("native menu-bar helper spawned: {}", hp.display())),
            Err(e) => log("tray", &format!("native helper spawn failed: {e}")),
        },
        None => log("tray", "native helper binary not found in bundle"),
    }

    let app = app.clone();
    std::thread::spawn(move || {
        let Some(cf) = cmd_file() else {
            return;
        };
        // Clear any stale command from a previous run.
        let _ = std::fs::remove_file(&cf);
        loop {
            std::thread::sleep(Duration::from_millis(250));
            let Ok(cmd) = std::fs::read_to_string(&cf) else {
                continue;
            };
            let _ = std::fs::remove_file(&cf);
            let cmd = cmd.trim();
            // Menu-bar click toggles the popover (show if hidden, hide if already
            // up) and hides the desktop window — one HQ window at a time. The
            // helper appends the icon's on-screen centre ("show <x>", Cocoa
            // points) so the popover anchors UNDER the icon; positioning +
            // blur-hide suppression live in `tray::show_popover_window`.
            if let Some(rest) = cmd.strip_prefix("show") {
                if let Ok(points) = rest.trim().parse::<f64>() {
                    crate::tray::set_tray_anchor_x(points);
                }
                // Window ops (esp. the `is_visible()` toggle query) MUST run on
                // the main thread — calling them from this poll thread deadlocks
                // AppKit and wedges the poller after the first click. Marshal it.
                let app_main = app.clone();
                let _ = app
                    .run_on_main_thread(move || crate::tray::toggle_popover_window(&app_main));
            } else {
                match cmd {
                    "sync" => {
                        let _ = app.emit("tray:sync-now", ());
                    }
                    "quit" => app.exit(0),
                    other => log("tray", &format!("native helper: unknown cmd '{other}'")),
                }
            }
        }
    });
}
