//! In-flight recordings ledger: the durable client-side record of which
//! Recall.ai recordings were started on this machine and have not yet been
//! cleanly finalised.
//!
//! Persisted at `~/.hq/recordings-ledger.json`.
//!
//! ## Why this exists
//! Before this ledger the only mapping `windowId -> recordingId` lived in the
//! in-memory Svelte store. If the app was force-quit (or crashed) mid-recording,
//! that mapping was lost — even though Recall still holds the recording
//! server-side and hq-pro is still processing its transcript. The recording
//! silently fell off the user's radar. This ledger persists the mapping the
//! moment a recording starts and reconciles any still-open entry on the next
//! launch, so a recording that was in flight across a restart is recovered
//! (its status surfaced) instead of dropped.
//!
//! ## Key
//! The map is keyed by `windowId` — the same stable per-meeting-window key the
//! SDK bridge uses for `start-recording` / `recording:ended`. The durable
//! Recall handle (`recordingId`) is stored in the value so the launch
//! reconcile can query hq-pro for that recording's status.
//!
//! ## Write model
//! - **Written on start:** [`upsert`] inserts `{ recordingId, companyUid,
//!   startedAt }` keyed by `windowId` the instant `start_recording` mints the
//!   upload token (we have all three fields at that point).
//! - **Cleared on clean end:** [`clear`] removes the `windowId` entry on
//!   `recording:ended` (and defensively on `meeting:closed`, where the SDK
//!   auto-ends). A cleanly-stopped recording leaves no ledger entry, so the
//!   next launch has nothing to reconcile for it.
//! - **Reconciled on launch:** [`reconcile`] walks every still-open entry,
//!   asks a status fetcher for the recording's server-side state, classifies
//!   it ([`ReconcileOutcome`]), and removes entries that have reached a
//!   terminal state (saved / ingest-failed / unknown-and-aged-out). Entries
//!   still genuinely in flight (`StillProcessing`) are retained so a later
//!   launch reconciles them again.
//!
//! ## Bounds
//! An entry whose recording never resolves (e.g. the server-side record is
//! gone but the transcript truly never landed) would otherwise live forever.
//! [`reconcile`] drops any entry older than [`MAX_AGE_DAYS`] regardless of the
//! fetched status, so the ledger can't grow without bound.
//!
//! ## Crash semantics
//! The entry is written synchronously (atomic temp-file + rename, same pattern
//! as `util/meeting_ledger.rs` / `util/journal.rs`) before the bridge is told
//! to start, and removed only on a clean terminal event. A forced-quit or
//! crash therefore leaves exactly the set of recordings that were in flight,
//! which the next launch reconciles. A corrupt / unparseable ledger is treated
//! as empty so a bad file never blocks launch or loses the recording silently
//! (the read error is logged by the caller).

use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

use super::paths::hq_config_dir;

// ── Test-mode path override ────────────────────────────────────────────────────
//
// Mirrors `util/meeting_ledger.rs::LEDGER_PATH_TEST_OVERRIDE`: tests point
// `ledger_path()` at an isolated tempdir without mutating HOME.

#[cfg(test)]
static LEDGER_PATH_TEST_OVERRIDE: OnceLock<Mutex<Option<PathBuf>>> = OnceLock::new();

// ── Bounds ─────────────────────────────────────────────────────────────────────

/// Entries older than this are dropped by [`reconcile`] regardless of fetched
/// status, so a recording whose server-side record is permanently gone can't
/// pin a ledger entry forever.
pub const MAX_AGE_DAYS: i64 = 7;

// ── Public types ───────────────────────────────────────────────────────────────

/// A single in-flight recording, keyed in the ledger by `windowId`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RecordingEntry {
    /// The durable Recall.ai Recording id returned by `start_recording`
    /// (`POST /v1/recall/upload-token` → `id`). This is the handle the launch
    /// reconcile uses to query hq-pro for status.
    pub recording_id: String,
    /// Company the recording is attributed to (routes the transcript on
    /// hq-pro). `None` for a Personal-vault recording.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub company_uid: Option<String>,
    /// ISO-8601 timestamp when the recording was started (ledger entry written).
    pub started_at: String,
}

/// The ledger map: `windowId` → [`RecordingEntry`].
pub type RecordingsLedger = HashMap<String, RecordingEntry>;

/// Server-side status for a recording, as resolved by the reconcile fetcher.
/// This is the normalised shape the classifier consumes — the caller maps the
/// raw hq-pro `GET /v1/bot/{id}/status` body onto it.
#[derive(Debug, Clone, PartialEq)]
pub struct RecordingStatus {
    /// Raw hq-pro bot status (`scheduled`, `recording`, `processing`,
    /// `completed`, `failed`, `error`, …). Lower-cased by the classifier.
    pub status: String,
    /// True once the transcript has actually landed in the vault as a source
    /// (the US-010 `source_landed` signal). A `completed` status with this
    /// false is still in flight (ingest not confirmed), not done.
    pub source_landed: bool,
    /// True when hq-pro had no record of the recording at all (HTTP 404). A
    /// recording that was started but is entirely absent server-side never
    /// finalised — treat it as a lost/failed ingest, not "still processing".
    pub not_found: bool,
}

/// How the launch reconcile classified a still-open ledger entry. Surfaced to
/// the UI (via a `recording:reconciled` event) so a recording that was in
/// flight across a restart is recovered rather than silently lost.
/// The `outcome` discriminant tag and every inner field are camelCased so the
/// renderer (which listens for `recording:reconciled`) gets stable JS-style
/// keys — `outcome`, `windowId`, `recordingId`, `status`, `reason`. NOTE: with
/// an internally-tagged enum, `rename_all` on the enum only renames the variant
/// *tags*; each struct variant needs its own `rename_all` for its fields.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "outcome")]
pub enum ReconcileOutcome {
    /// The recording's transcript has landed — nothing for the user to do.
    /// The ledger entry is cleared.
    #[serde(rename_all = "camelCase")]
    Saved {
        window_id: String,
        recording_id: String,
    },
    /// The recording is still being processed server-side (recording /
    /// processing / completed-but-not-yet-landed). The ledger entry is
    /// retained so a later launch reconciles it again. The UI surfaces a
    /// "still processing" thread.
    #[serde(rename_all = "camelCase")]
    StillProcessing {
        window_id: String,
        recording_id: String,
        status: String,
    },
    /// The recording reached a terminal failure server-side, or hq-pro has no
    /// record of it (404 — it never finalised). The ledger entry is cleared
    /// and the UI surfaces an "ingest failed" thread so the recording isn't
    /// silently lost.
    #[serde(rename_all = "camelCase")]
    IngestFailed {
        window_id: String,
        recording_id: String,
        reason: String,
    },
    /// The status fetch itself failed (network / auth) and the entry is not yet
    /// past [`MAX_AGE_DAYS`]. The entry is retained and will be retried on a
    /// later launch. No UI thread (transient).
    #[serde(rename_all = "camelCase")]
    Unknown {
        window_id: String,
        recording_id: String,
        reason: String,
    },
}

impl ReconcileOutcome {
    /// The `windowId` this outcome pertains to (used to key ledger removal).
    pub fn window_id(&self) -> &str {
        match self {
            ReconcileOutcome::Saved { window_id, .. }
            | ReconcileOutcome::StillProcessing { window_id, .. }
            | ReconcileOutcome::IngestFailed { window_id, .. }
            | ReconcileOutcome::Unknown { window_id, .. } => window_id,
        }
    }

    /// Whether the corresponding ledger entry should be removed after this
    /// reconcile (i.e. the entry has reached a terminal state for the client).
    /// `StillProcessing` and (not-yet-aged) `Unknown` are retained.
    pub fn clears_entry(&self) -> bool {
        matches!(
            self,
            ReconcileOutcome::Saved { .. } | ReconcileOutcome::IngestFailed { .. }
        )
    }
}

// ── Path resolution ────────────────────────────────────────────────────────────

/// Returns the path to `~/.hq/recordings-ledger.json`.
///
/// In test builds a per-test override slot is consulted first so tests get
/// fully isolated paths without HOME mutation (same pattern as
/// `meeting_ledger::ledger_path`).
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
    Ok(hq_config_dir()?.join("recordings-ledger.json"))
}

// ── I/O ────────────────────────────────────────────────────────────────────────

/// Load the ledger from disk. Returns an empty map if the file is absent.
///
/// A present-but-corrupt file surfaces as `Err` so the caller can log it; the
/// launch reconcile treats that error as "empty" so a bad file never blocks
/// launch.
pub fn read_ledger() -> Result<RecordingsLedger, String> {
    let p = ledger_path()?;
    if !p.exists() {
        return Ok(HashMap::new());
    }
    let s = fs::read_to_string(&p).map_err(|e| format!("{}: {e}", p.display()))?;
    serde_json::from_str(&s).map_err(|e| format!("{}: {e}", p.display()))
}

/// Write the ledger to disk atomically (temp file + rename). Matches the
/// atomic-write pattern in `meeting_ledger.rs` / `journal.rs`.
pub fn write_ledger(ledger: &RecordingsLedger) -> Result<(), String> {
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

// ── Mutations (pure: operate on an in-memory map) ───────────────────────────────

/// Insert (or replace) the entry for `window_id`. Called when a recording
/// starts.
pub fn upsert(
    ledger: &mut RecordingsLedger,
    window_id: String,
    recording_id: String,
    company_uid: Option<String>,
    started_at: DateTime<Utc>,
) {
    ledger.insert(
        window_id,
        RecordingEntry {
            recording_id,
            company_uid: company_uid.filter(|s| !s.is_empty()),
            started_at: started_at.to_rfc3339(),
        },
    );
}

/// Remove the entry for `window_id`. Called when a recording cleanly ends.
/// Returns `true` if an entry was actually removed (idempotent — a no-op when
/// absent).
pub fn clear(ledger: &mut RecordingsLedger, window_id: &str) -> bool {
    ledger.remove(window_id).is_some()
}

// ── On-disk convenience wrappers ────────────────────────────────────────────────
//
// The Tauri command surface calls these read-modify-write helpers directly.
// Each reads the current ledger, applies the mutation, and writes it back
// atomically. These are deliberately thin so the pure mutations above stay
// trivially testable in-memory.

/// Persist a recording-started entry. Read-modify-write on disk.
pub fn record_started(
    window_id: String,
    recording_id: String,
    company_uid: Option<String>,
    started_at: DateTime<Utc>,
) -> Result<(), String> {
    let mut ledger = read_ledger()?;
    upsert(&mut ledger, window_id, recording_id, company_uid, started_at);
    write_ledger(&ledger)
}

/// Clear a recording entry by `window_id`. Read-modify-write on disk. No-op
/// (and no write) when the entry is absent, so a `recording:ended` for a
/// window we never tracked doesn't churn the file.
pub fn record_ended(window_id: &str) -> Result<(), String> {
    let mut ledger = read_ledger()?;
    if clear(&mut ledger, window_id) {
        write_ledger(&ledger)?;
    }
    Ok(())
}

// ── Reconcile ───────────────────────────────────────────────────────────────────

/// Classify one still-open entry given the fetched server-side status.
///
/// Mapping:
/// - fetch error → [`ReconcileOutcome::Unknown`] (retry next launch unless aged out)
/// - `not_found` (404) → [`ReconcileOutcome::IngestFailed`] (never finalised)
/// - `source_landed` → [`ReconcileOutcome::Saved`]
/// - status `failed`/`error` → [`ReconcileOutcome::IngestFailed`]
/// - anything else (scheduled/recording/processing/completed-not-landed) →
///   [`ReconcileOutcome::StillProcessing`]
fn classify(
    window_id: &str,
    entry: &RecordingEntry,
    fetched: &Result<RecordingStatus, String>,
) -> ReconcileOutcome {
    let recording_id = entry.recording_id.clone();
    let status = match fetched {
        Err(e) => {
            return ReconcileOutcome::Unknown {
                window_id: window_id.to_string(),
                recording_id,
                reason: e.clone(),
            };
        }
        Ok(s) => s,
    };

    if status.not_found {
        return ReconcileOutcome::IngestFailed {
            window_id: window_id.to_string(),
            recording_id,
            reason: "recording not found on server (never finalised)".to_string(),
        };
    }

    if status.source_landed {
        return ReconcileOutcome::Saved {
            window_id: window_id.to_string(),
            recording_id,
        };
    }

    let normalised = status.status.trim().to_ascii_lowercase();
    match normalised.as_str() {
        "failed" | "error" | "fatal" => ReconcileOutcome::IngestFailed {
            window_id: window_id.to_string(),
            recording_id,
            reason: format!("recording ended in status '{}'", status.status),
        },
        _ => ReconcileOutcome::StillProcessing {
            window_id: window_id.to_string(),
            recording_id,
            status: status.status.clone(),
        },
    }
}

/// Reconcile the ledger against server-side state.
///
/// For every entry: if it's older than [`MAX_AGE_DAYS`] it is dropped (and
/// reported as [`ReconcileOutcome::IngestFailed`] with an aged-out reason so
/// the user still learns the recording was lost); otherwise the provided
/// `fetch_status` resolver is asked for its status and the entry is classified.
///
/// `fetch_status` is injected (rather than calling reqwest directly) so the
/// reconcile logic is unit-testable without a live server — the Tauri command
/// passes the real `GET /v1/bot/{id}/status` call, tests pass a stub.
///
/// Mutates `ledger` in place: terminal entries are removed, in-flight ones
/// retained. Returns the per-entry outcomes (UI-surfacing happens in the
/// caller). Order is unspecified (HashMap iteration).
pub fn reconcile<F>(
    ledger: &mut RecordingsLedger,
    now: DateTime<Utc>,
    mut fetch_status: F,
) -> Vec<ReconcileOutcome>
where
    F: FnMut(&str, Option<&str>) -> Result<RecordingStatus, String>,
{
    let cutoff = now - Duration::days(MAX_AGE_DAYS);
    let mut outcomes = Vec::with_capacity(ledger.len());

    // Snapshot keys first so we can mutate the map while iterating.
    let window_ids: Vec<String> = ledger.keys().cloned().collect();

    for window_id in window_ids {
        // Re-borrow each iteration (entry is cloned-out so the map can be
        // mutated below without an aliasing borrow).
        let entry = match ledger.get(&window_id) {
            Some(e) => e.clone(),
            None => continue,
        };

        // Aged-out: a recording started more than MAX_AGE_DAYS ago that we
        // still haven't resolved is treated as lost so it can't pin the ledger
        // forever. A corrupt/unparseable startedAt also ages out (fail closed).
        let aged_out = DateTime::parse_from_rfc3339(&entry.started_at)
            .map(|dt| dt.with_timezone(&Utc) <= cutoff)
            .unwrap_or(true);
        if aged_out {
            outcomes.push(ReconcileOutcome::IngestFailed {
                window_id: window_id.clone(),
                recording_id: entry.recording_id.clone(),
                reason: format!(
                    "recording unresolved for more than {MAX_AGE_DAYS} days — giving up"
                ),
            });
            ledger.remove(&window_id);
            continue;
        }

        let fetched = fetch_status(&entry.recording_id, entry.company_uid.as_deref());
        let outcome = classify(&window_id, &entry, &fetched);
        if outcome.clears_entry() {
            ledger.remove(&window_id);
        }
        outcomes.push(outcome);
    }

    outcomes
}

// ── Tests ──────────────────────────────────────────────────────────────────────
//
// The bulk of the tests live in `recordings_ledger_test.rs` (the PRD-declared
// third file), included below so they share this module's private items
// (LEDGER_PATH_TEST_OVERRIDE, classify, etc.).

#[cfg(test)]
#[path = "recordings_ledger_test.rs"]
mod tests;
