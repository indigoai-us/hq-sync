//! Tests for the in-flight recordings ledger.
//!
//! Included from `recordings_ledger.rs` via `#[path = "recordings_ledger_test.rs"]`
//! so it sees the parent module's private items (`LEDGER_PATH_TEST_OVERRIDE`,
//! `classify`, …). Mirrors the test harness in `util/meeting_ledger.rs`:
//! per-module mutex + `LEDGER_PATH_TEST_OVERRIDE` for HOME-free path isolation.

use super::*;
use std::sync::{Mutex, OnceLock};
use tempfile::TempDir;

// ── Test harness ────────────────────────────────────────────────────────────────

/// Per-module mutex — the on-disk tests share the global
/// `LEDGER_PATH_TEST_OVERRIDE` slot, so they must not run concurrently.
static TEST_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

fn lock() -> std::sync::MutexGuard<'static, ()> {
    TEST_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|p| p.into_inner())
}

/// Point `ledger_path()` at an isolated tempdir for the duration of the test.
/// Returns the `TempDir` guard (must stay alive for the test body).
fn with_test_ledger() -> TempDir {
    let tmp = tempfile::tempdir().expect("tempdir");
    let path = tmp.path().join("recordings-ledger.json");
    let slot = LEDGER_PATH_TEST_OVERRIDE.get_or_init(|| Mutex::new(None));
    *slot.lock().unwrap_or_else(|p| p.into_inner()) = Some(path);
    tmp
}

fn clear_override() {
    if let Some(slot) = LEDGER_PATH_TEST_OVERRIDE.get() {
        if let Ok(mut g) = slot.lock() {
            *g = None;
        }
    }
}

fn ts(s: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(s)
        .expect("bad test timestamp")
        .with_timezone(&Utc)
}

/// Convenience builder for a fetched status.
fn status(s: &str, source_landed: bool) -> RecordingStatus {
    RecordingStatus {
        status: s.to_string(),
        source_landed,
        not_found: false,
    }
}

// ── In-memory mutations ──────────────────────────────────────────────────────────

#[test]
fn upsert_inserts_entry_keyed_by_window_id() {
    let mut ledger = RecordingsLedger::new();
    upsert(
        &mut ledger,
        "win-1".to_string(),
        "rec_abc".to_string(),
        Some("co_indigo".to_string()),
        ts("2026-06-03T10:00:00Z"),
    );
    let e = ledger.get("win-1").expect("entry present");
    assert_eq!(e.recording_id, "rec_abc");
    assert_eq!(e.company_uid.as_deref(), Some("co_indigo"));
    assert_eq!(e.started_at, "2026-06-03T10:00:00+00:00");
}

#[test]
fn upsert_normalises_empty_company_to_none() {
    let mut ledger = RecordingsLedger::new();
    upsert(
        &mut ledger,
        "win-personal".to_string(),
        "rec_p".to_string(),
        Some("".to_string()),
        ts("2026-06-03T10:00:00Z"),
    );
    assert!(ledger.get("win-personal").unwrap().company_uid.is_none());
}

#[test]
fn upsert_replaces_existing_window() {
    let mut ledger = RecordingsLedger::new();
    upsert(&mut ledger, "win-1".to_string(), "rec_old".to_string(), None, ts("2026-06-03T10:00:00Z"));
    upsert(&mut ledger, "win-1".to_string(), "rec_new".to_string(), None, ts("2026-06-03T11:00:00Z"));
    assert_eq!(ledger.len(), 1);
    assert_eq!(ledger.get("win-1").unwrap().recording_id, "rec_new");
}

#[test]
fn clear_removes_entry_and_reports_hit() {
    let mut ledger = RecordingsLedger::new();
    upsert(&mut ledger, "win-1".to_string(), "rec_abc".to_string(), None, ts("2026-06-03T10:00:00Z"));
    assert!(clear(&mut ledger, "win-1"));
    assert!(ledger.is_empty());
}

#[test]
fn clear_absent_window_is_noop() {
    let mut ledger = RecordingsLedger::new();
    assert!(!clear(&mut ledger, "no-such-window"));
}

// ── On-disk roundtrip (AC: ledger written on start) ──────────────────────────────

#[test]
fn record_started_persists_entry_to_disk() {
    let _g = lock();
    let _tmp = with_test_ledger();

    record_started(
        "win-1".to_string(),
        "rec_abc".to_string(),
        Some("co_indigo".to_string()),
        ts("2026-06-03T10:00:00Z"),
    )
    .unwrap();

    // Re-read from disk: the entry must survive a write/read cycle exactly as
    // it would across an app restart.
    let loaded = read_ledger().unwrap();
    let e = loaded.get("win-1").expect("entry persisted");
    assert_eq!(e.recording_id, "rec_abc");
    assert_eq!(e.company_uid.as_deref(), Some("co_indigo"));

    clear_override();
}

#[test]
fn record_started_then_started_again_keeps_both_windows() {
    let _g = lock();
    let _tmp = with_test_ledger();

    record_started("win-1".to_string(), "rec_1".to_string(), None, ts("2026-06-03T10:00:00Z")).unwrap();
    record_started("win-2".to_string(), "rec_2".to_string(), None, ts("2026-06-03T10:05:00Z")).unwrap();

    let loaded = read_ledger().unwrap();
    assert_eq!(loaded.len(), 2);
    assert_eq!(loaded.get("win-1").unwrap().recording_id, "rec_1");
    assert_eq!(loaded.get("win-2").unwrap().recording_id, "rec_2");

    clear_override();
}

// ── On-disk clear (AC: cleared on recording:ended) ───────────────────────────────

#[test]
fn record_ended_clears_entry_on_disk() {
    let _g = lock();
    let _tmp = with_test_ledger();

    record_started("win-1".to_string(), "rec_abc".to_string(), None, ts("2026-06-03T10:00:00Z")).unwrap();
    record_ended("win-1").unwrap();

    let loaded = read_ledger().unwrap();
    assert!(
        loaded.get("win-1").is_none(),
        "a cleanly-ended recording leaves no ledger entry to reconcile"
    );

    clear_override();
}

#[test]
fn record_ended_only_clears_the_named_window() {
    let _g = lock();
    let _tmp = with_test_ledger();

    record_started("win-1".to_string(), "rec_1".to_string(), None, ts("2026-06-03T10:00:00Z")).unwrap();
    record_started("win-2".to_string(), "rec_2".to_string(), None, ts("2026-06-03T10:05:00Z")).unwrap();
    record_ended("win-1").unwrap();

    let loaded = read_ledger().unwrap();
    assert!(loaded.get("win-1").is_none());
    assert!(loaded.get("win-2").is_some(), "the other in-flight recording is untouched");

    clear_override();
}

#[test]
fn record_ended_absent_window_is_noop() {
    let _g = lock();
    let _tmp = with_test_ledger();

    // No entries written. Ending an untracked window must not error or create
    // a file with junk.
    record_ended("ghost-window").unwrap();
    assert!(read_ledger().unwrap().is_empty());

    clear_override();
}

#[test]
fn read_missing_ledger_returns_empty() {
    let _g = lock();
    let _tmp = with_test_ledger();
    assert!(read_ledger().unwrap().is_empty());
    clear_override();
}

#[test]
fn read_corrupt_ledger_returns_err() {
    let _g = lock();
    let tmp = with_test_ledger();
    // Write garbage to the ledger path and confirm read surfaces an Err (the
    // launch reconcile downgrades that to "empty" so it never blocks launch).
    let path = tmp.path().join("recordings-ledger.json");
    std::fs::write(&path, "{ this is not json").unwrap();
    assert!(read_ledger().is_err());
    clear_override();
}

// ── classify matrix ──────────────────────────────────────────────────────────────

fn entry_at(recording_id: &str, started_at: &str) -> RecordingEntry {
    RecordingEntry {
        recording_id: recording_id.to_string(),
        company_uid: None,
        started_at: started_at.to_string(),
    }
}

#[test]
fn classify_source_landed_is_saved() {
    let entry = entry_at("rec_1", "2026-06-03T10:00:00Z");
    let out = classify("win-1", &entry, &Ok(status("completed", true)));
    assert_eq!(
        out,
        ReconcileOutcome::Saved {
            window_id: "win-1".to_string(),
            recording_id: "rec_1".to_string(),
        }
    );
    assert!(out.clears_entry());
}

#[test]
fn classify_completed_not_landed_is_still_processing() {
    // `completed` status but the transcript hasn't landed as a source yet —
    // this is exactly the #240 symptom and must NOT read as Saved.
    let entry = entry_at("rec_1", "2026-06-03T10:00:00Z");
    let out = classify("win-1", &entry, &Ok(status("completed", false)));
    match out {
        ReconcileOutcome::StillProcessing { status, .. } => assert_eq!(status, "completed"),
        other => panic!("expected StillProcessing, got {other:?}"),
    }
}

#[test]
fn classify_processing_is_still_processing() {
    let entry = entry_at("rec_1", "2026-06-03T10:00:00Z");
    let out = classify("win-1", &entry, &Ok(status("processing", false)));
    assert!(matches!(out, ReconcileOutcome::StillProcessing { .. }));
    assert!(!out.clears_entry());
}

#[test]
fn classify_failed_is_ingest_failed() {
    let entry = entry_at("rec_1", "2026-06-03T10:00:00Z");
    let out = classify("win-1", &entry, &Ok(status("failed", false)));
    assert!(matches!(out, ReconcileOutcome::IngestFailed { .. }));
    assert!(out.clears_entry());
}

#[test]
fn classify_error_status_case_insensitive_is_ingest_failed() {
    let entry = entry_at("rec_1", "2026-06-03T10:00:00Z");
    let out = classify("win-1", &entry, &Ok(status("ERROR", false)));
    assert!(matches!(out, ReconcileOutcome::IngestFailed { .. }));
}

#[test]
fn classify_not_found_is_ingest_failed() {
    let entry = entry_at("rec_1", "2026-06-03T10:00:00Z");
    let s = RecordingStatus {
        status: "".to_string(),
        source_landed: false,
        not_found: true,
    };
    let out = classify("win-1", &entry, &Ok(s));
    match out {
        ReconcileOutcome::IngestFailed { reason, .. } => assert!(reason.contains("not found")),
        other => panic!("expected IngestFailed, got {other:?}"),
    }
}

#[test]
fn classify_fetch_error_is_unknown_and_retained() {
    let entry = entry_at("rec_1", "2026-06-03T10:00:00Z");
    let out = classify("win-1", &entry, &Err("network down".to_string()));
    match &out {
        ReconcileOutcome::Unknown { reason, .. } => assert_eq!(reason, "network down"),
        other => panic!("expected Unknown, got {other:?}"),
    }
    assert!(!out.clears_entry(), "transient fetch failure must retain the entry");
}

// ── reconcile (AC: reconcile open-on-launch) ─────────────────────────────────────

#[test]
fn reconcile_open_entry_still_processing_is_retained() {
    // The headline acceptance case: an entry left over from a crash/forced-quit
    // is reconciled on launch. Here the recording is still processing, so the
    // entry must survive (a later launch reconciles it again) and the UI gets
    // a StillProcessing thread instead of losing the recording.
    let mut ledger = RecordingsLedger::new();
    upsert(&mut ledger, "win-1".to_string(), "rec_abc".to_string(), Some("co_indigo".to_string()), ts("2026-06-03T10:00:00Z"));

    let now = ts("2026-06-03T10:30:00Z"); // 30 min later — well within MAX_AGE_DAYS
    let mut seen: Vec<(String, Option<String>)> = vec![];
    let outcomes = reconcile(&mut ledger, now, |recording_id, company_uid| {
        seen.push((recording_id.to_string(), company_uid.map(str::to_string)));
        Ok(status("processing", false))
    });

    // The fetcher was asked about the right recording, scoped to its company.
    assert_eq!(seen, vec![("rec_abc".to_string(), Some("co_indigo".to_string()))]);
    assert_eq!(outcomes.len(), 1);
    assert!(matches!(outcomes[0], ReconcileOutcome::StillProcessing { .. }));
    assert!(ledger.contains_key("win-1"), "still-processing entry is retained");
}

#[test]
fn reconcile_saved_entry_is_cleared() {
    let mut ledger = RecordingsLedger::new();
    upsert(&mut ledger, "win-1".to_string(), "rec_abc".to_string(), None, ts("2026-06-03T10:00:00Z"));

    let outcomes = reconcile(&mut ledger, ts("2026-06-03T10:30:00Z"), |_, _| Ok(status("completed", true)));

    assert!(matches!(outcomes[0], ReconcileOutcome::Saved { .. }));
    assert!(ledger.is_empty(), "a landed recording is cleared from the ledger");
}

#[test]
fn reconcile_failed_entry_is_cleared_and_surfaced() {
    let mut ledger = RecordingsLedger::new();
    upsert(&mut ledger, "win-1".to_string(), "rec_abc".to_string(), None, ts("2026-06-03T10:00:00Z"));

    let outcomes = reconcile(&mut ledger, ts("2026-06-03T10:30:00Z"), |_, _| Ok(status("failed", false)));

    assert!(matches!(outcomes[0], ReconcileOutcome::IngestFailed { .. }));
    assert!(ledger.is_empty(), "a failed recording is cleared once surfaced");
}

#[test]
fn reconcile_not_found_entry_is_ingest_failed_and_cleared() {
    let mut ledger = RecordingsLedger::new();
    upsert(&mut ledger, "win-1".to_string(), "rec_gone".to_string(), None, ts("2026-06-03T10:00:00Z"));

    let outcomes = reconcile(&mut ledger, ts("2026-06-03T10:30:00Z"), |_, _| {
        Ok(RecordingStatus { status: "".to_string(), source_landed: false, not_found: true })
    });

    assert!(matches!(outcomes[0], ReconcileOutcome::IngestFailed { .. }));
    assert!(ledger.is_empty());
}

#[test]
fn reconcile_fetch_error_retains_entry_for_next_launch() {
    let mut ledger = RecordingsLedger::new();
    upsert(&mut ledger, "win-1".to_string(), "rec_abc".to_string(), None, ts("2026-06-03T10:00:00Z"));

    let outcomes = reconcile(&mut ledger, ts("2026-06-03T10:30:00Z"), |_, _| Err("offline".to_string()));

    assert!(matches!(outcomes[0], ReconcileOutcome::Unknown { .. }));
    assert!(
        ledger.contains_key("win-1"),
        "a transient fetch failure must not drop the recording — retry next launch"
    );
}

#[test]
fn reconcile_aged_out_entry_is_dropped_without_fetch() {
    // An entry older than MAX_AGE_DAYS is given up on (reported IngestFailed)
    // and never fetched — it can't pin the ledger forever.
    let mut ledger = RecordingsLedger::new();
    upsert(&mut ledger, "win-old".to_string(), "rec_old".to_string(), None, ts("2026-05-20T10:00:00Z"));

    let now = ts("2026-06-03T10:00:00Z"); // 14 days later — past the 7-day bound
    let mut fetched_any = false;
    let outcomes = reconcile(&mut ledger, now, |_, _| {
        fetched_any = true;
        Ok(status("processing", false))
    });

    assert!(!fetched_any, "aged-out entries must not be fetched");
    match &outcomes[0] {
        ReconcileOutcome::IngestFailed { reason, .. } => assert!(reason.contains("giving up")),
        other => panic!("expected aged-out IngestFailed, got {other:?}"),
    }
    assert!(ledger.is_empty(), "aged-out entry is dropped");
}

#[test]
fn reconcile_corrupt_started_at_ages_out() {
    let mut ledger = RecordingsLedger::new();
    ledger.insert(
        "win-bad".to_string(),
        RecordingEntry { recording_id: "rec_bad".to_string(), company_uid: None, started_at: "not-a-date".to_string() },
    );
    let outcomes = reconcile(&mut ledger, ts("2026-06-03T10:00:00Z"), |_, _| Ok(status("processing", false)));
    assert!(matches!(outcomes[0], ReconcileOutcome::IngestFailed { .. }));
    assert!(ledger.is_empty(), "a corrupt timestamp fails closed (dropped)");
}

#[test]
fn reconcile_mixed_ledger_partitions_correctly() {
    // One saved, one still-processing, one failed, one fetch-error — confirm
    // the ledger ends up holding exactly the two that should be retained.
    let mut ledger = RecordingsLedger::new();
    let t = ts("2026-06-03T10:00:00Z");
    upsert(&mut ledger, "win-saved".to_string(), "rec_saved".to_string(), None, t);
    upsert(&mut ledger, "win-proc".to_string(), "rec_proc".to_string(), None, t);
    upsert(&mut ledger, "win-failed".to_string(), "rec_failed".to_string(), None, t);
    upsert(&mut ledger, "win-err".to_string(), "rec_err".to_string(), None, t);

    let now = ts("2026-06-03T10:30:00Z");
    let outcomes = reconcile(&mut ledger, now, |recording_id, _| match recording_id {
        "rec_saved" => Ok(status("completed", true)),
        "rec_proc" => Ok(status("processing", false)),
        "rec_failed" => Ok(status("failed", false)),
        "rec_err" => Err("timeout".to_string()),
        other => panic!("unexpected recording {other}"),
    });

    assert_eq!(outcomes.len(), 4);
    // Retained: still-processing + fetch-error. Cleared: saved + failed.
    assert!(!ledger.contains_key("win-saved"));
    assert!(ledger.contains_key("win-proc"));
    assert!(!ledger.contains_key("win-failed"));
    assert!(ledger.contains_key("win-err"));
}

#[test]
fn reconcile_empty_ledger_is_noop() {
    let mut ledger = RecordingsLedger::new();
    let outcomes = reconcile(&mut ledger, ts("2026-06-03T10:00:00Z"), |_, _| {
        panic!("fetcher must not be called for an empty ledger")
    });
    assert!(outcomes.is_empty());
}

// ── outcome helpers ──────────────────────────────────────────────────────────────

#[test]
fn outcome_window_id_accessor() {
    let o = ReconcileOutcome::StillProcessing {
        window_id: "win-x".to_string(),
        recording_id: "rec_x".to_string(),
        status: "processing".to_string(),
    };
    assert_eq!(o.window_id(), "win-x");
}

#[test]
fn outcome_serialises_with_tag() {
    // The UI listens for these on a Tauri event; the discriminated `outcome`
    // tag must be present so the frontend can switch on it.
    let o = ReconcileOutcome::IngestFailed {
        window_id: "win-x".to_string(),
        recording_id: "rec_x".to_string(),
        reason: "boom".to_string(),
    };
    let json = serde_json::to_value(&o).unwrap();
    assert_eq!(json["outcome"], "ingestFailed");
    assert_eq!(json["windowId"], "win-x");
    assert_eq!(json["recordingId"], "rec_x");
}
