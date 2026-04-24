//! Per-company sync journal enumeration.
//!
//! The runner writes one journal shard per company at
//! `{state_dir}/sync-journal.{slug}.json`. Shape (v1):
//!
//! ```json
//! {
//!   "version": "1",
//!   "lastSync": "2026-04-23T15:26:50.921Z",
//!   "files": { ... }
//! }
//! ```
//!
//! The menubar app only cares about the top-level `lastSync` field — the
//! per-file manifest is for conflict detection on the runner side. This
//! command scans the state dir and returns a `{slug: lastSync}` map that the
//! frontend uses to render the "Last synced · …" timestamp on each
//! `CompanyRow`.
//!
//! Unlike `status.rs` (which reads a single global journal and falls back to
//! defaults), this is a strict enumeration — a shard that's missing, malformed,
//! or lacks `lastSync` is silently skipped so one corrupt file can't hide the
//! healthy companies.

use std::collections::HashMap;

use serde::Deserialize;

use crate::util::paths;

/// Shape of a per-company journal shard — only the fields we need here.
///
/// `files` and other shard metadata (`version`, per-file hashes, etc.) are
/// intentionally ignored. That keeps this command resilient to shard-format
/// evolution: as long as `lastSync` remains a string we keep working.
#[derive(Debug, Deserialize)]
struct JournalShard {
    #[serde(rename = "lastSync")]
    last_sync: Option<String>,
}

/// Scan `state_dir` for `sync-journal.{slug}.json` shards and return a
/// `{slug: lastSync}` map. Shards that don't parse, don't carry a `lastSync`,
/// or live in a directory that doesn't exist are silently skipped.
fn scan_journals(state_dir: &std::path::Path) -> HashMap<String, String> {
    let mut out = HashMap::new();

    let entries = match std::fs::read_dir(state_dir) {
        Ok(e) => e,
        Err(_) => return out, // Pre-first-sync: nothing to read.
    };

    for entry in entries.flatten() {
        let file_name = entry.file_name();
        let name = match file_name.to_str() {
            Some(n) => n,
            None => continue, // Non-UTF8 filename — skip.
        };

        // Expected shape: `sync-journal.{slug}.json`
        let slug = match name
            .strip_prefix("sync-journal.")
            .and_then(|rest| rest.strip_suffix(".json"))
        {
            Some(s) if !s.is_empty() => s,
            _ => continue,
        };

        let contents = match std::fs::read_to_string(entry.path()) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let shard: JournalShard = match serde_json::from_str(&contents) {
            Ok(s) => s,
            Err(_) => continue, // Malformed JSON — skip silently.
        };

        if let Some(last_sync) = shard.last_sync {
            if !last_sync.is_empty() {
                out.insert(slug.to_string(), last_sync);
            }
        }
    }

    out
}

/// Enumerate per-company journal shards and return a `{slug: lastSyncISO}`
/// map.
///
/// Called once at app mount (see `App.svelte` → `loadJournals`) and then
/// augmented in-memory by the `sync:complete` event listener.
#[tauri::command]
pub fn list_sync_journals() -> Result<HashMap<String, String>, String> {
    let state_dir = paths::hq_state_dir()?;
    Ok(scan_journals(&state_dir))
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_scan_journals_empty_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let map = scan_journals(tmp.path());
        assert!(map.is_empty());
    }

    #[test]
    fn test_scan_journals_nonexistent_dir() {
        let map = scan_journals(std::path::Path::new(
            "/nonexistent/path/that/absolutely/does/not/exist/xyz",
        ));
        assert!(map.is_empty());
    }

    #[test]
    fn test_scan_journals_reads_valid_shards() {
        let tmp = tempfile::tempdir().unwrap();

        // Two valid shards
        std::fs::write(
            tmp.path().join("sync-journal.indigo.json"),
            r#"{"version":"1","lastSync":"2026-04-20T10:00:00Z","files":{}}"#,
        )
        .unwrap();
        std::fs::write(
            tmp.path().join("sync-journal.acme.json"),
            r#"{"version":"1","lastSync":"2026-04-21T11:00:00Z","files":{}}"#,
        )
        .unwrap();

        let map = scan_journals(tmp.path());
        assert_eq!(map.len(), 2);
        assert_eq!(
            map.get("indigo"),
            Some(&"2026-04-20T10:00:00Z".to_string())
        );
        assert_eq!(map.get("acme"), Some(&"2026-04-21T11:00:00Z".to_string()));
    }

    #[test]
    fn test_scan_journals_skips_malformed_and_missing_last_sync() {
        let tmp = tempfile::tempdir().unwrap();

        // Two valid shards
        std::fs::write(
            tmp.path().join("sync-journal.indigo.json"),
            r#"{"version":"1","lastSync":"2026-04-20T10:00:00Z","files":{}}"#,
        )
        .unwrap();
        std::fs::write(
            tmp.path().join("sync-journal.acme.json"),
            r#"{"version":"1","lastSync":"2026-04-21T11:00:00Z","files":{}}"#,
        )
        .unwrap();
        // Malformed JSON
        std::fs::write(
            tmp.path().join("sync-journal.broken.json"),
            "not json at all",
        )
        .unwrap();
        // Missing lastSync
        std::fs::write(
            tmp.path().join("sync-journal.nosync.json"),
            r#"{"version":"1","files":{}}"#,
        )
        .unwrap();

        let map = scan_journals(tmp.path());
        assert_eq!(map.len(), 2, "only the two valid shards should appear");
        assert!(map.contains_key("indigo"));
        assert!(map.contains_key("acme"));
        assert!(!map.contains_key("broken"));
        assert!(!map.contains_key("nosync"));
    }

    #[test]
    fn test_scan_journals_ignores_unrelated_files() {
        let tmp = tempfile::tempdir().unwrap();

        // Unrelated files that happen to sit next to the shards
        std::fs::write(tmp.path().join("menubar.json"), r#"{"hqPath":"/tmp"}"#).unwrap();
        std::fs::write(tmp.path().join("config.json"), r#"{"person":"y"}"#).unwrap();
        std::fs::write(
            tmp.path().join("sync-journal..json"), // empty slug between the dots
            r#"{"lastSync":"2026-04-20T10:00:00Z"}"#,
        )
        .unwrap();
        // Valid shard
        std::fs::write(
            tmp.path().join("sync-journal.indigo.json"),
            r#"{"version":"1","lastSync":"2026-04-20T10:00:00Z"}"#,
        )
        .unwrap();

        let map = scan_journals(tmp.path());
        assert_eq!(map.len(), 1);
        assert!(map.contains_key("indigo"));
    }

    #[test]
    fn test_scan_journals_handles_empty_last_sync_string() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("sync-journal.indigo.json"),
            r#"{"version":"1","lastSync":""}"#,
        )
        .unwrap();

        let map = scan_journals(tmp.path());
        assert!(
            !map.contains_key("indigo"),
            "empty lastSync should be treated as missing"
        );
    }

    #[test]
    fn test_scan_journals_preserves_slug_with_dots() {
        // Slugs can contain dots (e.g. "vyg-dev" has no dot, but future
        // company slugs might). The stripping logic uses the outer
        // `sync-journal.` / `.json` wrappers — anything in between is
        // part of the slug even if it contains a dot.
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("sync-journal.co.with.dots.json"),
            r#"{"version":"1","lastSync":"2026-04-20T10:00:00Z"}"#,
        )
        .unwrap();

        let map = scan_journals(tmp.path());
        assert_eq!(
            map.get("co.with.dots"),
            Some(&"2026-04-20T10:00:00Z".to_string())
        );
    }
}
