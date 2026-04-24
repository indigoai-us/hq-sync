//! Library interface for `hq-sync-menubar`.
//!
//! The primary entry point for users is the bin target at `main.rs`. This
//! lib exists so integration tests in `tests/*.rs` can link against the
//! crate's modules (Rust integration tests require a lib target).
//!
//! Keeping the module tree here mirrors the bin's `mod` declarations.
//! Cargo compiles the bin and the lib as two crates; each owns its own
//! copy of the module graph. That's fine: integration tests only ever see
//! the lib's copy, and the bin's at-runtime state (process registry,
//! tray OnceLock, etc.) is never shared with a test process.

pub mod commands;
pub mod events;
pub mod tray;
pub mod updater;
pub mod util;
