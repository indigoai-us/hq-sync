//! Library crate surface for `hq-sync-menubar`.
//!
//! The primary artifact of this crate is the binary at `src/main.rs` — this
//! `lib.rs` exists purely so integration tests under `src-tauri/tests/` can
//! reach into the Tauri command modules (e.g. [`commands::companies`]) via
//! `use hq_sync_menubar::…`.
//!
//! It re-exports only what the tests need. Adding modules here does NOT
//! change the app binary's behavior — the binary continues to compile these
//! modules as part of its own dependency tree. Cargo is happy to compile a
//! package's source tree twice (once for the bin, once for the lib); the
//! cost is a small hit to clean-build time, not runtime correctness.

pub mod commands;
pub mod events;
pub mod tray;
pub mod updater;
pub mod util;
