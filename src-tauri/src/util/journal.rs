// companies/indigo/repos/hq-sync/src-tauri/src/util/journal.rs
use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use serde::{Deserialize, Serialize};

/// Mirrors packages/hq-cloud/src/types.ts `JournalEntry`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct JournalEntry {
    pub hash: String,                // hex sha256 of file contents
    pub size: u64,
    #[serde(rename = "syncedAt")]
    pub synced_at: String,           // ISO-8601
    pub direction: Direction,        // "up" | "down"
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Direction { Up, Down }

/// Mirrors packages/hq-cloud/src/types.ts `SyncJournal`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SyncJournal {
    pub version: String,             // "1"
    #[serde(rename = "lastSync")]
    pub last_sync: String,           // ISO-8601 (empty string if never)
    pub files: BTreeMap<String, JournalEntry>,
}

impl Default for SyncJournal {
    fn default() -> Self {
        Self { version: "1".into(), last_sync: String::new(), files: BTreeMap::new() }
    }
}

/// Resolve HQ_STATE_DIR env first; else ~/.hq. Matches `getStateDir()` in
/// packages/hq-cloud/src/journal.ts.
pub fn state_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("HQ_STATE_DIR") {
        return PathBuf::from(dir);
    }
    dirs::home_dir().expect("home dir").join(".hq")
}

/// Verbatim port of `sanitizeSlug(slug)`:
///   - replace `[^a-zA-Z0-9_-]` with `_`
///   - throw if empty OR result is all `_`/`-`.
pub fn sanitize_slug(slug: &str) -> Result<String, String> {
    if slug.is_empty() {
        return Err("journal: slug is required (empty or undefined)".into());
    }
    let cleaned: String = slug
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '-' { c } else { '_' })
        .collect();
    if cleaned.is_empty() || cleaned.chars().all(|c| c == '_' || c == '-') {
        return Err(format!("journal: slug \"{slug}\" sanitizes to an empty identifier"));
    }
    Ok(cleaned)
}

pub fn journal_path(slug: &str) -> Result<PathBuf, String> {
    let name = format!("sync-journal.{}.json", sanitize_slug(slug)?);
    Ok(state_dir().join(name))
}

pub fn read_journal(slug: &str) -> Result<SyncJournal, String> {
    let p = journal_path(slug)?;
    if !p.exists() { return Ok(SyncJournal::default()); }
    let s = fs::read_to_string(&p).map_err(|e| format!("{}: {e}", p.display()))?;
    serde_json::from_str(&s).map_err(|e| format!("{}: {e}", p.display()))
}

/// Atomic write via temp file + rename.
pub fn write_journal(slug: &str, journal: &SyncJournal) -> Result<(), String> {
    let p = journal_path(slug)?;
    if let Some(parent) = p.parent() { fs::create_dir_all(parent).map_err(|e| e.to_string())?; }
    let tmp = p.with_extension("json.tmp");
    let body = serde_json::to_string_pretty(journal).map_err(|e| e.to_string())?;
    let mut f = fs::File::create(&tmp).map_err(|e| e.to_string())?;
    f.write_all(body.as_bytes()).map_err(|e| e.to_string())?;
    f.sync_all().ok();
    fs::rename(&tmp, &p).map_err(|e| e.to_string())
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::test_support::with_state_dir;

    // (a) sanitize_slug("newco") → Ok("newco")
    #[test]
    fn sanitize_slug_happy_path() {
        assert_eq!(sanitize_slug("newco").unwrap(), "newco");
    }

    // (b) sanitize_slug("") → Err containing "required"
    #[test]
    fn sanitize_slug_empty_err() {
        let err = sanitize_slug("").unwrap_err();
        assert!(err.contains("required"), "expected 'required' in: {err}");
    }

    // (c) sanitize_slug("__") → Err containing "sanitizes to an empty identifier"
    #[test]
    fn sanitize_slug_all_underscores_err() {
        let err = sanitize_slug("__").unwrap_err();
        assert!(err.contains("sanitizes to an empty identifier"), "got: {err}");
    }

    // (d) sanitize_slug("bad/slug?") → Ok("bad_slug_")
    #[test]
    fn sanitize_slug_replaces_special_chars() {
        assert_eq!(sanitize_slug("bad/slug?").unwrap(), "bad_slug_");
    }

    // (e) journal_path("newco") with HQ_STATE_DIR set ends with sync-journal.newco.json
    #[test]
    fn journal_path_uses_state_dir() {
        with_state_dir(|dir| {
            let p = journal_path("newco").unwrap();
            assert!(
                p.starts_with(dir),
                "expected path under state dir; got {}",
                p.display()
            );
            assert!(
                p.to_string_lossy().ends_with("sync-journal.newco.json"),
                "expected sync-journal.newco.json suffix; got {}",
                p.display()
            );
        });
    }

    // (f) roundtrip: write_journal → read_journal → same SyncJournal
    #[test]
    fn journal_roundtrip() {
        with_state_dir(|_dir| {
            let mut files = BTreeMap::new();
            files.insert(
                "README.md".to_string(),
                JournalEntry {
                    hash: "abc123".into(),
                    size: 42,
                    synced_at: "2026-01-01T00:00:00Z".into(),
                    direction: Direction::Up,
                },
            );
            let original = SyncJournal {
                version: "1".into(),
                last_sync: "2026-01-01T00:00:00Z".into(),
                files,
            };
            write_journal("testslug", &original).unwrap();
            let read_back = read_journal("testslug").unwrap();
            assert_eq!(original, read_back);
        });
    }
}
