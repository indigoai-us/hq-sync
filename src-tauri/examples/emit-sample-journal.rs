//! Cross-language parity smoke test: writes a sample journal file that the TS
//! `readJournal` function must be able to parse identically.
//!
//! Compiled only when the `journal-cli` feature is active (feature-gated so it
//! is never included in release app builds).

// Pull the journal module directly from the crate source tree.
#[path = "../util/journal.rs"]
mod journal;

use journal::{write_journal, Direction, JournalEntry, SyncJournal};
use std::collections::BTreeMap;

fn main() {
    let mut files = BTreeMap::new();
    files.insert(
        "README.md".to_string(),
        JournalEntry {
            hash: "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
                .to_string(),
            size: 0,
            synced_at: "2026-01-01T00:00:00Z".to_string(),
            direction: Direction::Up,
        },
    );
    let j = SyncJournal {
        version: "1".to_string(),
        last_sync: "2026-01-01T00:00:00Z".to_string(),
        files,
    };
    write_journal("newco", &j).expect("write journal");
    println!("wrote sample journal for slug=newco");
}
