//! Per-detection notification ledger: prevents double-notifying the user when
//! the Recall Desktop SDK re-fires the same meeting event, or across app restarts.
//!
//! Persisted at `~/.hq/meeting-notify-ledger.json`.
//!
//! ## Key
//! The stable dedup key is the meeting URL (preferred); if absent, the SDK's
//! `sourceEventId`. This matches the PRD spec: meetingUrl is the most stable
//! identifier across SDK re-fires and calendar-vs-active-app detection sources.
//!
//! ## Suppression windows
//! - `notified` or `recorded`: suppressed for 6 hours after `notifiedAt`
//! - `dismissed`: suppressed for the rest of the same UTC calendar day
//!
//! ## Bounds
//! Entries older than 14 days are pruned on app launch via [`prune`].
//!
//! ## Write model
//! Atomic: write to `meeting-notify-ledger.json.tmp`, then rename. Matches the
//! same pattern as `util/journal.rs`.

use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

use super::paths::hq_config_dir;

// ── Test-mode path override ────────────────────────────────────────────────────

#[cfg(test)]
static LEDGER_PATH_TEST_OVERRIDE: OnceLock<Mutex<Option<PathBuf>>> = OnceLock::new();

// ── Public types ───────────────────────────────────────────────────────────────

/// The action recorded in the ledger for a given detection key.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum LedgerAction {
    /// The user was shown a macOS notification.
    Notified,
    /// The user clicked Record — a bot invite was sent.
    Recorded,
    /// The user dismissed the notification without acting.
    Dismissed,
}

/// A single entry in the ledger.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LedgerEntry {
    /// ISO-8601 timestamp when the entry was written.
    pub notified_at: String,
    /// What action was taken.
    pub action: LedgerAction,
}

/// The ledger map: stable detection key → [`LedgerEntry`].
pub type NotifyLedger = HashMap<String, LedgerEntry>;

// ── Path resolution ────────────────────────────────────────────────────────────

/// Returns the path to `~/.hq/meeting-notify-ledger.json`.
///
/// In test builds a per-test override slot is consulted first so tests
/// get fully isolated paths without HOME mutation.
pub fn ledger_path() -> Result<PathBuf, String> {
    #[cfg(test)]
    {
        if let Some(slot) = LEDGER_PATH_TEST_OVERRIDE.get() {
            if let Ok(guard) = slot.lock() {
                if let Some(ref p) = *guard {
                    return Ok(p.clone());
                }
            }
        }
    }
    Ok(hq_config_dir()?.join("meeting-notify-ledger.json"))
}

// ── Stable key ────────────────────────────────────────────────────────────────

/// Derive the stable dedup key for a detection.
///
/// Prefers `meeting_url` (most stable across re-fires); falls back to
/// `source_event_id`. Returns `None` when both are absent or empty.
pub fn stable_key(meeting_url: Option<&str>, source_event_id: Option<&str>) -> Option<String> {
    if let Some(url) = meeting_url {
        if !url.is_empty() {
            return Some(url.to_string());
        }
    }
    source_event_id
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
}

// ── I/O ────────────────────────────────────────────────────────────────────────

/// Load the ledger from disk. Returns an empty map if the file is absent.
pub fn read_ledger() -> Result<NotifyLedger, String> {
    let p = ledger_path()?;
    if !p.exists() {
        return Ok(HashMap::new());
    }
    let s = fs::read_to_string(&p).map_err(|e| format!("{}: {e}", p.display()))?;
    serde_json::from_str(&s).map_err(|e| format!("{}: {e}", p.display()))
}

/// Write the ledger to disk atomically (temp file + rename).
pub fn write_ledger(ledger: &NotifyLedger) -> Result<(), String> {
    let p = ledger_path()?;
    if let Some(parent) = p.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let tmp = p.with_extension("json.tmp");
    let body = serde_json::to_string_pretty(ledger).map_err(|e| e.to_string())?;
    let mut f = fs::File::create(&tmp).map_err(|e| e.to_string())?;
    f.write_all(body.as_bytes()).map_err(|e| e.to_string())?;
    f.sync_all().ok();
    fs::rename(&tmp, &p).map_err(|e| e.to_string())
}

// ── Business logic ─────────────────────────────────────────────────────────────

/// Returns `true` if this detection key should be suppressed (no new
/// notification should fire).
///
/// Suppression windows:
/// - `notified` / `recorded`: suppressed if `now - notifiedAt < 6 hours`
/// - `dismissed`: suppressed if `notifiedAt` is the same UTC calendar day as `now`
pub fn should_suppress(ledger: &NotifyLedger, key: &str, now: DateTime<Utc>) -> bool {
    let entry = match ledger.get(key) {
        Some(e) => e,
        None => return false,
    };
    let notified_at = match DateTime::parse_from_rfc3339(&entry.notified_at) {
        Ok(dt) => dt.with_timezone(&Utc),
        Err(_) => return false, // corrupt timestamp — don't suppress
    };
    match entry.action {
        LedgerAction::Notified | LedgerAction::Recorded => {
            now.signed_duration_since(notified_at) < Duration::hours(6)
        }
        LedgerAction::Dismissed => notified_at.date_naive() == now.date_naive(),
    }
}

/// Record a detection in the ledger with the given action and timestamp.
pub fn mark(ledger: &mut NotifyLedger, key: String, action: LedgerAction, now: DateTime<Utc>) {
    ledger.insert(
        key,
        LedgerEntry {
            notified_at: now.to_rfc3339(),
            action,
        },
    );
}

/// Remove entries whose `notifiedAt` is older than 14 days from `now`.
///
/// Corrupt entries (unparseable timestamp) are also removed.
/// Called on app launch to bound ledger growth.
pub fn prune(ledger: &mut NotifyLedger, now: DateTime<Utc>) {
    let cutoff = now - Duration::days(14);
    ledger.retain(|_, entry| {
        DateTime::parse_from_rfc3339(&entry.notified_at)
            .map(|dt| dt.with_timezone(&Utc) > cutoff)
            .unwrap_or(false)
    });
}

// ── Atomic single-flight claim ───────────────────────────────────────────────

/// Process-wide lock serialising the read→decide→mark→write critical section.
///
/// The ledger lives in a single JSON file, so the read-modify-write in
/// [`claim_notify`] / [`record_action`] is only safe if exactly one caller is
/// inside it at a time. Without this, two concurrent `meeting:detected`
/// listeners (the popover window and the desktop-alt window — both subscribed
/// to the global Tauri event) could each `read_ledger → should_suppress` before
/// either writes, both observe "not suppressed", and both fire a banner for the
/// SAME meeting. The lock makes the second caller observe the first's just-written
/// `Notified` entry and suppress.
///
/// CRITICAL: the guard must never be held across a `.await`. The file I/O here
/// is fully synchronous, and the only caller (`meetings_notify_detected`) takes
/// the claim inside a sync scope so the guard drops before any async work.
static LEDGER_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

fn ledger_lock() -> &'static Mutex<()> {
    LEDGER_LOCK.get_or_init(|| Mutex::new(()))
}

/// Atomically decide whether the caller should fire a "meeting detected"
/// notification for `key`, and — if so — record that decision before returning.
///
/// Under [`LEDGER_LOCK`] this performs, as one indivisible step:
///   1. `read_ledger`
///   2. `should_suppress(key, now)` → if suppressed, return `false` (do NOT fire)
///   3. otherwise `mark(key, Notified, now)` + `write_ledger`, return `true`
///
/// Returning `true` means "this caller won the race — fire the banner". A second
/// concurrent caller for the same key will block on the lock, then read the
/// just-written `Notified` entry and return `false`. This is the single-flight
/// guarantee that fixes the double-notification bug.
///
/// A read or write error fails CLOSED (returns `false`, suppressing) so a
/// transient I/O fault can never produce a duplicate banner; the worst case is a
/// missed notification, which the next detection re-attempts.
pub fn claim_notify(key: &str, now: DateTime<Utc>) -> bool {
    let _guard = ledger_lock().lock().unwrap_or_else(|p| p.into_inner());
    let mut ledger = match read_ledger() {
        Ok(l) => l,
        Err(_) => return false,
    };
    if should_suppress(&ledger, key, now) {
        return false;
    }
    mark(&mut ledger, key.to_string(), LedgerAction::Notified, now);
    write_ledger(&ledger).is_ok()
}

/// Unconditionally record `action` for `key` under [`LEDGER_LOCK`].
///
/// Used by the record path to authoritatively mark a detection as
/// [`LedgerAction::Recorded`] when the user actually starts recording it, so any
/// later detection of the same meeting suppresses (a `Recorded` entry is honoured
/// by [`should_suppress`] for 6 h, identically to `Notified`). Unlike
/// [`claim_notify`] this never consults `should_suppress` — it overwrites so the
/// stronger `Recorded` action wins even if an earlier `Notified` exists.
///
/// Best-effort: a read/write error is swallowed (the caller treats recording
/// suppression as a nice-to-have, never a blocker).
pub fn record_action(key: &str, action: LedgerAction, now: DateTime<Utc>) {
    let _guard = ledger_lock().lock().unwrap_or_else(|p| p.into_inner());
    let mut ledger = read_ledger().unwrap_or_default();
    mark(&mut ledger, key.to_string(), action, now);
    let _ = write_ledger(&ledger);
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    /// Per-module mutex — tests share the global LEDGER_PATH_TEST_OVERRIDE slot.
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
        let path = tmp.path().join("meeting-notify-ledger.json");
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

    // ── stable_key ──────────────────────────────────────────────────────────

    #[test]
    fn stable_key_prefers_url() {
        let key = stable_key(Some("https://zoom.us/j/1"), Some("evt_1")).unwrap();
        assert_eq!(key, "https://zoom.us/j/1");
    }

    #[test]
    fn stable_key_falls_back_to_event_id() {
        let key = stable_key(None, Some("evt_1")).unwrap();
        assert_eq!(key, "evt_1");
    }

    #[test]
    fn stable_key_empty_url_falls_back() {
        let key = stable_key(Some(""), Some("evt_2")).unwrap();
        assert_eq!(key, "evt_2");
    }

    #[test]
    fn stable_key_both_absent_returns_none() {
        assert!(stable_key(None, None).is_none());
    }

    // ── should_suppress: notified within 6 hours ───────────────────────────

    #[test]
    fn notified_10min_later_suppressed() {
        let t0 = ts("2026-05-20T10:00:00Z");
        let t1 = ts("2026-05-20T10:10:00Z"); // +10 min
        let mut ledger = NotifyLedger::new();
        mark(&mut ledger, "https://zoom.us/j/1".to_string(), LedgerAction::Notified, t0);
        assert!(should_suppress(&ledger, "https://zoom.us/j/1", t1));
    }

    #[test]
    fn notified_6h_later_not_suppressed() {
        let t0 = ts("2026-05-20T10:00:00Z");
        let t1 = ts("2026-05-20T16:00:00Z"); // +6 h exactly — NOT suppressed
        let mut ledger = NotifyLedger::new();
        mark(&mut ledger, "https://zoom.us/j/2".to_string(), LedgerAction::Notified, t0);
        assert!(!should_suppress(&ledger, "https://zoom.us/j/2", t1));
    }

    #[test]
    fn recorded_within_6h_suppressed() {
        let t0 = ts("2026-05-20T10:00:00Z");
        let t1 = ts("2026-05-20T15:59:59Z"); // +5h59m59s — still suppressed
        let mut ledger = NotifyLedger::new();
        mark(&mut ledger, "url".to_string(), LedgerAction::Recorded, t0);
        assert!(should_suppress(&ledger, "url", t1));
    }

    // ── should_suppress: dismissed same calendar day ────────────────────────

    #[test]
    fn dismissed_same_day_suppressed() {
        let t0 = ts("2026-05-20T10:00:00Z");
        let t1 = ts("2026-05-20T22:30:00Z"); // same day, later
        let mut ledger = NotifyLedger::new();
        mark(&mut ledger, "url_d".to_string(), LedgerAction::Dismissed, t0);
        assert!(should_suppress(&ledger, "url_d", t1));
    }

    #[test]
    fn dismissed_next_day_not_suppressed() {
        let t0 = ts("2026-05-20T10:00:00Z");
        let t1 = ts("2026-05-21T00:01:00Z"); // next UTC day
        let mut ledger = NotifyLedger::new();
        mark(&mut ledger, "url_e".to_string(), LedgerAction::Dismissed, t0);
        assert!(!should_suppress(&ledger, "url_e", t1));
    }

    // ── unknown key never suppresses ───────────────────────────────────────

    #[test]
    fn unknown_key_not_suppressed() {
        let ledger = NotifyLedger::new();
        assert!(!should_suppress(&ledger, "no-such-url", ts("2026-05-20T10:00:00Z")));
    }

    // ── prune ───────────────────────────────────────────────────────────────

    #[test]
    fn prune_removes_entries_older_than_14_days() {
        let now = ts("2026-05-20T12:00:00Z");
        let old_ts = ts("2026-05-05T12:00:00Z"); // 15 days ago — should be pruned
        let recent_ts = ts("2026-05-15T12:00:00Z"); // 5 days ago — kept
        let mut ledger = NotifyLedger::new();
        mark(&mut ledger, "old".to_string(), LedgerAction::Notified, old_ts);
        mark(&mut ledger, "recent".to_string(), LedgerAction::Notified, recent_ts);
        prune(&mut ledger, now);
        assert!(!ledger.contains_key("old"), "old entry should be pruned");
        assert!(ledger.contains_key("recent"), "recent entry should remain");
    }

    #[test]
    fn prune_keeps_entry_exactly_14_days_old() {
        let now = ts("2026-05-20T12:00:00Z");
        let boundary = ts("2026-05-06T12:00:00Z"); // exactly 14 days ago
        let mut ledger = NotifyLedger::new();
        mark(&mut ledger, "boundary".to_string(), LedgerAction::Notified, boundary);
        prune(&mut ledger, now);
        // boundary is exactly 14 days — `> cutoff` is false (equal), so pruned
        assert!(!ledger.contains_key("boundary"), "entry at exactly 14d cutoff should be pruned");
    }

    #[test]
    fn prune_removes_corrupt_entries() {
        let mut ledger = NotifyLedger::new();
        ledger.insert(
            "corrupt".to_string(),
            LedgerEntry {
                notified_at: "not-a-date".to_string(),
                action: LedgerAction::Notified,
            },
        );
        prune(&mut ledger, ts("2026-05-20T12:00:00Z"));
        assert!(!ledger.contains_key("corrupt"));
    }

    // ── read/write roundtrip ────────────────────────────────────────────────

    #[test]
    fn ledger_roundtrip() {
        let _g = lock();
        let _tmp = with_test_ledger();

        let now = ts("2026-05-20T10:00:00Z");
        let mut ledger = NotifyLedger::new();
        mark(&mut ledger, "https://zoom.us/j/abc".to_string(), LedgerAction::Notified, now);
        mark(&mut ledger, "https://meet.google.com/xyz".to_string(), LedgerAction::Dismissed, now);

        write_ledger(&ledger).unwrap();
        let loaded = read_ledger().unwrap();
        assert_eq!(ledger, loaded);

        clear_override();
    }

    #[test]
    fn read_missing_ledger_returns_empty() {
        let _g = lock();
        let _tmp = with_test_ledger();

        let ledger = read_ledger().unwrap();
        assert!(ledger.is_empty());

        clear_override();
    }

    // ── claim_notify: atomic single-flight (the B1 regression test) ──────────

    /// Two threads claim the SAME key concurrently. Exactly one must win
    /// (return `true`) and the ledger must end with exactly one entry. This is
    /// the direct lock-down for the double-notification race: before the
    /// process-wide `LEDGER_LOCK`, both callers could read-then-write and both
    /// observe "not suppressed", yielding two notifications.
    #[test]
    fn claim_notify_is_single_flight_under_concurrency() {
        let _g = lock();
        let _tmp = with_test_ledger();

        let key = "https://zoom.us/j/concurrent";
        let now = ts("2026-05-20T10:00:00Z");

        // Spawn both threads, release them as close together as the scheduler
        // allows, and collect their claim results. Run many rounds so a missing
        // lock would flake into a double-true on at least one iteration.
        for round in 0..50 {
            // Fresh ledger each round so the suppression window doesn't carry
            // over and trivially make the second claim lose.
            write_ledger(&NotifyLedger::new()).unwrap();

            let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
            let mut wins = 0;
            let handles: Vec<_> = (0..2)
                .map(|_| {
                    let b = barrier.clone();
                    std::thread::spawn(move || {
                        b.wait();
                        claim_notify(key, now)
                    })
                })
                .collect();
            for h in handles {
                if h.join().unwrap() {
                    wins += 1;
                }
            }

            assert_eq!(
                wins, 1,
                "round {round}: expected exactly one winning claim, got {wins}"
            );
            let ledger = read_ledger().unwrap();
            assert_eq!(
                ledger.len(),
                1,
                "round {round}: expected exactly one ledger entry, got {}",
                ledger.len()
            );
            assert_eq!(ledger.get(key).unwrap().action, LedgerAction::Notified);
        }

        clear_override();
    }

    /// A second claim for the same key within the 6 h window is suppressed even
    /// when issued sequentially — the first claim's `Notified` entry blocks it.
    #[test]
    fn claim_notify_second_sequential_call_suppressed() {
        let _g = lock();
        let _tmp = with_test_ledger();
        write_ledger(&NotifyLedger::new()).unwrap();

        let key = "https://meet.google.com/seq";
        let t0 = ts("2026-05-20T10:00:00Z");
        let t1 = ts("2026-05-20T10:30:00Z"); // +30 min, within 6 h

        assert!(claim_notify(key, t0), "first claim should win");
        assert!(!claim_notify(key, t1), "second claim within 6h should be suppressed");
        assert_eq!(read_ledger().unwrap().len(), 1);

        clear_override();
    }

    /// After the 6 h window elapses a fresh claim wins again (the detection is a
    /// genuinely new meeting occurrence by then).
    #[test]
    fn claim_notify_after_window_wins_again() {
        let _g = lock();
        let _tmp = with_test_ledger();
        write_ledger(&NotifyLedger::new()).unwrap();

        let key = "https://zoom.us/j/reuse";
        let t0 = ts("2026-05-20T10:00:00Z");
        let t1 = ts("2026-05-20T16:00:01Z"); // just past +6 h

        assert!(claim_notify(key, t0));
        assert!(claim_notify(key, t1), "claim past the 6h window should win again");

        clear_override();
    }

    // ── record_action: Recorded suppresses a later claim (optional fix) ──────

    /// Recording a detected meeting writes a `Recorded` entry for its stable
    /// key, so a subsequent `claim_notify` within 6 h is suppressed. This locks
    /// down the optional "mark Recorded on start_recording" path.
    #[test]
    fn record_action_recorded_suppresses_subsequent_claim() {
        let _g = lock();
        let _tmp = with_test_ledger();
        write_ledger(&NotifyLedger::new()).unwrap();

        let key = "https://zoom.us/j/recorded";
        let t0 = ts("2026-05-20T10:00:00Z");
        let t1 = ts("2026-05-20T11:00:00Z"); // +1 h, within 6 h

        record_action(key, LedgerAction::Recorded, t0);
        let ledger = read_ledger().unwrap();
        assert_eq!(ledger.get(key).unwrap().action, LedgerAction::Recorded);

        assert!(
            !claim_notify(key, t1),
            "a detection after the user recorded should be suppressed"
        );
        // The claim must NOT have downgraded the Recorded entry to Notified.
        assert_eq!(read_ledger().unwrap().get(key).unwrap().action, LedgerAction::Recorded);

        clear_override();
    }

    /// `record_action` overwrites an existing `Notified` entry with the stronger
    /// `Recorded` action (the user acting is more authoritative than a banner).
    #[test]
    fn record_action_overwrites_notified() {
        let _g = lock();
        let _tmp = with_test_ledger();
        write_ledger(&NotifyLedger::new()).unwrap();

        let key = "https://zoom.us/j/upgrade";
        let t0 = ts("2026-05-20T10:00:00Z");

        assert!(claim_notify(key, t0));
        assert_eq!(read_ledger().unwrap().get(key).unwrap().action, LedgerAction::Notified);

        record_action(key, LedgerAction::Recorded, t0);
        assert_eq!(read_ledger().unwrap().get(key).unwrap().action, LedgerAction::Recorded);

        clear_override();
    }
}
