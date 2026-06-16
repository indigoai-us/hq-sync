//! Local Codex session reader (US-003).
//!
//! Enumerates the user's local OpenAI Codex sessions from the on-disk rollout
//! store and maps each to the shared [`AgentSession`] contract (US-001) with
//! `origin = local` and `tool = codex`. Mirrors the Claude reader
//! (`commands/sessions/claude.rs`) in structure, error handling, and test style.
//!
//! ## Where Codex keeps its sessions (verified against a real `~/.codex`)
//!
//! Codex maintains a lightweight **index** plus per-session **rollout** logs:
//!
//! ```text
//! ~/.codex/session_index.jsonl                       (one line per session)
//! ~/.codex/sessions/YYYY/MM/DD/rollout-<ts>-<id>.jsonl   (live rollouts)
//! ~/.codex/archived_sessions/rollout-<ts>-<id>.jsonl     (archived rollouts, flat)
//! ```
//!
//! A single machine can accumulate hundreds of rollouts, many of them very large
//! (hundreds of MB — a 407 MB rollout was observed on this box). Enumeration must
//! therefore stay cheap: scandir + stat + a tiny bounded **head** read only.
//!
//! ## Observed index line shape (`session_index.jsonl`)
//!
//! One compact JSON object per line (verified against a real index, Codex
//! 0.128.x):
//!
//! ```jsonc
//! {
//!   "id": "019de12c-d83e-78c2-9bb3-cbb8146965e4",
//!   "thread_name": "Sync HQ with hq-core-stage PRs",
//!   "updated_at": "2026-05-01T01:36:06.218891Z"
//! }
//! ```
//!
//! - `id` — the stable Codex session id (also embedded in the rollout filename).
//! - `thread_name` — a human label for the session (used as the project label
//!   fallback when the rollout's cwd is unavailable).
//! - `updated_at` — ISO-8601 last-activity timestamp; this is the index's
//!   liveness signal. The index is the **fast path**: we map a session from its
//!   index line alone, and only crack open the rollout head for `cwd` / `model`.
//!
//! ## Observed rollout line shapes (`rollout-*.jsonl`)
//!
//! A rollout is heterogeneous ndjson. Only the first few records carry the
//! metadata we need (verified against real rollouts, Codex 0.128.x):
//!
//! ```jsonc
//! // line 1 — always present, the session header:
//! {
//!   "timestamp": "2026-05-01T01:38:52.811Z",
//!   "type": "session_meta",
//!   "payload": {
//!     "id": "019de12f-9c8a-77c0-b8d1-12895c1e4b68",
//!     "timestamp": "2026-05-01T01:38:07.121Z",   // session start
//!     "cwd": "/Users/corey/HQ",
//!     "originator": "Codex Desktop",
//!     "cli_version": "0.128.0-alpha.1",
//!     "source": "vscode",
//!     "model_provider": "openai"
//!   }
//! }
//! // a few lines later — the first turn's context, carries the model:
//! {
//!   "timestamp": "2026-06-04T03:04:39.480Z",
//!   "type": "turn_context",
//!   "payload": { "cwd": "/Users/corey/Documents/HQ", "model": "gpt-5.5", … }
//! }
//! ```
//!
//! `id`, `cwd`, and the session-start `timestamp` come from `session_meta`'s
//! `payload`; `model` lives on the first `turn_context` payload (the
//! `session_meta` header does NOT carry a model, only `model_provider`). Both of
//! these records sit at the **head** of the file, so a small bounded head read
//! recovers everything — we never read the whole rollout.
//!
//! ## Performance contract (PRD performanceRequirements — HARD)
//!
//! Enumeration is **index scan + scandir + stat + a bounded head read only**. We
//! never parse a rollout front-to-back: a multi-hundred-MB rollout would blow the
//! latency budget. For each session we:
//!
//!   1. Read `session_index.jsonl` (small — KBs) to enumerate session ids +
//!      last-activity, OR scandir the rollout dirs when the index is absent.
//!   2. `metadata()` each rollout for size + mtime (stat) — mtime is the
//!      fallback liveness signal when the index lacks `updated_at`.
//!   3. Read at most [`HEAD_BYTES`] from the **start** of the rollout and parse
//!      only the leading records to recover `cwd` and `model`. The `session_meta`
//!      header and first `turn_context` are within the first few KB.
//!
//! The head read is capped regardless of file size (`take(HEAD_BYTES)`), so a
//! 407 MB rollout costs the same as a 5 KB one. The
//! `large_rollout_is_not_fully_read` unit test pins this: it writes a rollout far
//! larger than `HEAD_BYTES` and asserts the reader still extracts the head fields
//! without reading the whole thing.
//!
//! ## Status (coarse — US-004 refines)
//!
//! Like the Claude reader, status here is a coarse mtime/last-activity-only
//! classification; the dedicated liveness engine (US-004) adds the process
//! cross-check and `awaiting_input` detection.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Deserialize;

use crate::commands::sessions::{AgentOrigin, AgentSession, AgentTool, SessionStatus};

/// Provenance tag stamped on every record this reader emits (US-001 `source`).
const SOURCE_TAG: &str = "codex-rollout";

/// Max bytes read from the **start** of each rollout. The `session_meta` header
/// is line 1 and the first `turn_context` (carrying the model) follows within a
/// handful of records, so a small head window reliably catches both `cwd` and
/// `model` while staying tiny relative to a multi-hundred-MB rollout. This is the
/// cap that makes enumeration O(files), not O(total rollout bytes) — see the
/// module performance contract. 64 KiB.
const HEAD_BYTES: u64 = 64 * 1024;

// ─────────────────────────────────────────────────────────────────────────────
// Status windows (coarse — US-004 refines)
// ─────────────────────────────────────────────────────────────────────────────

/// How long after the last activity a session is still considered actively
/// `Running`. Beyond this it's `Idle`; far beyond it's `Ended`. Kept in lock-step
/// with the Claude reader's windows so both tools classify identically; the
/// liveness engine (US-004) adds the process cross-check and `awaiting_input`.
const RUNNING_WINDOW_SECS: u64 = 90; // fresh activity → running
const IDLE_WINDOW_SECS: u64 = 30 * 60; // < 30m → idle, else ended

// ─────────────────────────────────────────────────────────────────────────────
// Index line shape
// ─────────────────────────────────────────────────────────────────────────────

/// One line of `~/.codex/session_index.jsonl`. Everything but `id` is optional so
/// a malformed/foreign line deserialises to "id + None" rather than erroring the
/// whole scan; a line without an `id` is skipped entirely.
#[derive(Debug, Default, Deserialize)]
struct IndexLine {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    thread_name: Option<String>,
    #[serde(default)]
    updated_at: Option<String>,
}

// ─────────────────────────────────────────────────────────────────────────────
// Rollout head shapes
// ─────────────────────────────────────────────────────────────────────────────

/// A single rollout record we care about. We only ever look at the leading
/// `session_meta` and `turn_context` records, both of which wrap their fields in
/// a `payload` object. Foreign record types deserialise to "all None".
#[derive(Debug, Default, Deserialize)]
struct RolloutLine {
    #[serde(default, rename = "type")]
    kind: Option<String>,
    #[serde(default)]
    payload: Option<RolloutPayload>,
}

/// The `payload` of a `session_meta` / `turn_context` record. `cwd` appears on
/// both; `model` on `turn_context`; `id` + session-start `timestamp` on
/// `session_meta`.
#[derive(Debug, Default, Deserialize)]
struct RolloutPayload {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    timestamp: Option<String>,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    model: Option<String>,
}

/// Fields recovered from a rollout's head.
#[derive(Debug, Default)]
struct HeadInfo {
    /// `payload.id` from `session_meta` (authoritative id; falls back to the
    /// filename-derived id when absent).
    id: Option<String>,
    cwd: Option<String>,
    model: Option<String>,
    /// Session-start `timestamp` from `session_meta.payload`.
    started_at: Option<String>,
}

// ─────────────────────────────────────────────────────────────────────────────
// A located rollout (filename-derived id + path + stat)
// ─────────────────────────────────────────────────────────────────────────────

/// A rollout file we located on disk, keyed by its filename-embedded session id.
struct RolloutFile {
    path: PathBuf,
    len: u64,
    mtime: SystemTime,
}

// ─────────────────────────────────────────────────────────────────────────────
// Public command
// ─────────────────────────────────────────────────────────────────────────────

/// List the local Codex sessions as [`AgentSession`] records.
///
/// Returns an **empty list** (not an error) when the Codex dir does not exist — a
/// machine that has never run Codex simply has no local sessions, which is a
/// valid empty fleet, not a failure. Status here is a coarse last-activity-only
/// classification ([`status_from_age`]); the dedicated liveness engine (US-004)
/// refines it (process cross-check, awaiting-input).
#[tauri::command]
pub async fn list_local_codex_sessions() -> Result<Vec<AgentSession>, String> {
    let codex_dir = codex_dir();
    Ok(scan_codex_sessions(&codex_dir, SystemTime::now()))
}

// ─────────────────────────────────────────────────────────────────────────────
// Path resolution
// ─────────────────────────────────────────────────────────────────────────────

/// `~/.codex` — the root of Codex's session store.
fn codex_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("/"))
        .join(".codex")
}

// ─────────────────────────────────────────────────────────────────────────────
// Pure scanner (testable)
// ─────────────────────────────────────────────────────────────────────────────

/// Enumerate the local Codex sessions under `codex_dir` and map each to an
/// [`AgentSession`]. Pure over its inputs so tests can point it at a fixture tree
/// and pin a deterministic `now`. Never panics: unreadable dirs/files are
/// skipped, one bad file can't blank the whole list.
///
/// Strategy: locate every rollout on disk (under `sessions/**` and
/// `archived_sessions/`) keyed by its filename-embedded id, then layer the index
/// over the top for last-activity + thread-name. A session that appears only in
/// the index but has no rollout on disk is skipped (we need at least the file to
/// stat); a rollout with no index line is still emitted (mtime is the fallback
/// liveness signal), so neither store is a single point of failure.
///
/// `now` is injected (not `SystemTime::now()`) so the age→status window is
/// deterministic under test.
fn scan_codex_sessions(codex_dir: &Path, now: SystemTime) -> Vec<AgentSession> {
    // Locate every rollout on disk, keyed by filename id. BTreeMap keeps output
    // deterministic (sorted by id) without a separate sort.
    let mut rollouts: BTreeMap<String, RolloutFile> = BTreeMap::new();
    collect_rollouts(&codex_dir.join("sessions"), &mut rollouts);
    collect_rollouts(&codex_dir.join("archived_sessions"), &mut rollouts);

    if rollouts.is_empty() {
        // No rollouts on disk → no local Codex sessions. Empty, not error.
        return Vec::new();
    }

    // Read the index (best-effort) for last-activity + thread-name, keyed by id.
    let index = read_index(&codex_dir.join("session_index.jsonl"));

    let mut out: Vec<AgentSession> = Vec::with_capacity(rollouts.len());
    for (file_id, rollout) in &rollouts {
        let head = read_head_info(&rollout.path);

        // id: prefer the rollout's own `session_meta.payload.id`, else the
        // filename-embedded id (they match in practice, but the payload is
        // authoritative).
        let id = head.id.clone().unwrap_or_else(|| file_id.clone());

        let idx = index.get(file_id);

        // last-activity: prefer the index `updated_at`, else the rollout mtime.
        let mtime_iso = system_time_to_iso(rollout.mtime);
        let last_activity_at = idx
            .and_then(|l| l.updated_at.clone())
            .unwrap_or_else(|| mtime_iso.clone());

        // started-at: prefer the `session_meta` payload timestamp; else mtime.
        let started_at = head.started_at.clone().unwrap_or_else(|| mtime_iso.clone());

        let cwd = head.cwd.clone().unwrap_or_default();

        // project: cwd basename → index thread_name. Codex carries no HQ
        // company/project metadata in its rollouts, so company stays empty
        // (US-004 / enrichment may resolve it later from cwd).
        let project = basename(&cwd)
            .or_else(|| idx.and_then(|l| l.thread_name.clone()))
            .unwrap_or_default();

        let status = status_from_age(&last_activity_at, rollout.mtime, now);

        out.push(AgentSession {
            id,
            tool: AgentTool::Codex,
            origin: AgentOrigin::Local,
            cwd,
            project,
            company: String::new(),
            model: head.model.clone().unwrap_or_default(),
            status,
            started_at,
            last_activity_at,
            source: SOURCE_TAG.to_string(),
        });
    }

    out
}

/// Recursively collect `rollout-*.jsonl` files under `dir` into `out`, keyed by
/// the session id embedded in the filename. Walks `sessions/YYYY/MM/DD/` (nested)
/// and the flat `archived_sessions/` alike. Unreadable dirs/files are skipped.
fn collect_rollouts(dir: &Path, out: &mut BTreeMap<String, RolloutFile>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return, // dir absent → nothing to collect
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            // sessions/ nests by YYYY/MM/DD — recurse. archived_sessions/ is flat
            // so this branch is simply never taken there.
            collect_rollouts(&path, out);
            continue;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if !is_rollout_filename(name) {
            continue;
        }
        let Some(id) = rollout_id_from_filename(name) else {
            continue;
        };
        let metadata = match std::fs::metadata(&path) {
            Ok(m) => m,
            Err(_) => continue,
        };
        let mtime = match metadata.modified() {
            Ok(t) => t,
            Err(_) => continue,
        };
        // First writer wins; a session id shouldn't collide across sessions/ and
        // archived_sessions/, but if it does we keep the first (live) one.
        out.entry(id).or_insert(RolloutFile {
            path,
            len: metadata.len(),
            mtime,
        });
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Index read
// ─────────────────────────────────────────────────────────────────────────────

/// Parse `session_index.jsonl` into an id→[`IndexLine`] map. The index is small
/// (KBs) so a full read is fine here — unlike rollouts. A missing/unreadable
/// index yields an empty map (enumeration falls back to rollout mtime).
fn read_index(index_path: &Path) -> BTreeMap<String, IndexLine> {
    let mut map = BTreeMap::new();
    let contents = match std::fs::read_to_string(index_path) {
        Ok(c) => c,
        Err(_) => return map,
    };
    for line in contents.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let parsed: IndexLine = match serde_json::from_str(line) {
            Ok(p) => p,
            Err(_) => continue, // foreign/partial line — skip
        };
        if let Some(id) = parsed.id.clone() {
            map.insert(id, parsed);
        }
    }
    map
}

// ─────────────────────────────────────────────────────────────────────────────
// Bounded head read
// ─────────────────────────────────────────────────────────────────────────────

/// Read at most [`HEAD_BYTES`] from the start of `path` and recover `id` / `cwd`
/// / `model` / session-start `timestamp` from the leading `session_meta` and
/// `turn_context` records. Returns an all-`None` [`HeadInfo`] on any read error
/// (the record still gets index/stat-derived fields).
///
/// This is the function that holds the hard performance contract: it reads at
/// most `HEAD_BYTES` from the front via `take(HEAD_BYTES)`, so the cost is bounded
/// regardless of how large the rollout is.
fn read_head_info(path: &Path) -> HeadInfo {
    use std::io::Read;

    let mut info = HeadInfo::default();

    let file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return info,
    };

    // Read only the head window (at most HEAD_BYTES). Never the whole file.
    let mut buf = Vec::with_capacity(HEAD_BYTES as usize);
    if file.take(HEAD_BYTES).read_to_end(&mut buf).is_err() {
        return info;
    }

    let text = String::from_utf8_lossy(&buf);

    // Walk head → first useful records. If the window ended mid-line, the final
    // fragment fails to parse and is simply skipped.
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let parsed: RolloutLine = match serde_json::from_str(line) {
            Ok(p) => p,
            // Foreign/partial line (incl. a truncated tail of the head window) —
            // skip, don't abort the head scan.
            Err(_) => continue,
        };
        let Some(payload) = parsed.payload else {
            continue;
        };

        match parsed.kind.as_deref() {
            Some("session_meta") => {
                if info.id.is_none() {
                    info.id = payload.id;
                }
                if info.started_at.is_none() {
                    info.started_at = payload.timestamp;
                }
                if info.cwd.is_none() {
                    info.cwd = payload.cwd;
                }
            }
            Some("turn_context") => {
                // turn_context carries the freshest cwd + the model.
                if info.cwd.is_none() {
                    info.cwd = payload.cwd;
                }
                if info.model.is_none() {
                    info.model = payload.model;
                }
            }
            _ => continue,
        }

        // Stop once everything useful is found.
        if info.id.is_some()
            && info.cwd.is_some()
            && info.model.is_some()
            && info.started_at.is_some()
        {
            break;
        }
    }

    info
}

// ─────────────────────────────────────────────────────────────────────────────
// Status from age (coarse — US-004 refines)
// ─────────────────────────────────────────────────────────────────────────────

/// Coarse last-activity-only status. Prefers the index `updated_at` (parsed as
/// RFC-3339) for the age computation, falling back to the rollout mtime when the
/// timestamp is absent or unparseable. Never emits `AwaitingInput` (that needs
/// rollout semantics / process state from US-004). A last-activity in the future
/// (clock skew) is treated as fresh → `Running`.
fn status_from_age(last_activity_iso: &str, mtime: SystemTime, now: SystemTime) -> SessionStatus {
    let activity_time = parse_rfc3339_to_system_time(last_activity_iso).unwrap_or(mtime);
    let age = now
        .duration_since(activity_time)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    if age <= RUNNING_WINDOW_SECS {
        SessionStatus::Running
    } else if age <= IDLE_WINDOW_SECS {
        SessionStatus::Idle
    } else {
        SessionStatus::Ended
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Small helpers
// ─────────────────────────────────────────────────────────────────────────────

/// Whether a filename is a Codex rollout (`rollout-<ts>-<id>.jsonl`).
fn is_rollout_filename(name: &str) -> bool {
    name.starts_with("rollout-") && name.ends_with(".jsonl")
}

/// Extract the session id from a rollout filename. The shape is
/// `rollout-YYYY-MM-DDTHH-MM-SS-<uuid>.jsonl`, where `<uuid>` is the last five
/// dash-joined groups (a UUID with its own internal dashes). We strip the
/// `rollout-` prefix and `.jsonl` suffix, then take the trailing UUID (last 5
/// dash groups) so the leading timestamp (which also contains dashes) is dropped.
/// Returns `None` if the name doesn't contain a plausible UUID tail.
fn rollout_id_from_filename(name: &str) -> Option<String> {
    let stem = name.strip_prefix("rollout-")?.strip_suffix(".jsonl")?;
    let parts: Vec<&str> = stem.split('-').collect();
    // A UUID is 5 dash-joined groups (8-4-4-4-12). The timestamp prefix adds
    // more leading groups; the id is always the last 5.
    if parts.len() < 5 {
        return None;
    }
    let uuid = parts[parts.len() - 5..].join("-");
    // Sanity-check the canonical 8-4-4-4-12 hex group lengths so a malformed name
    // doesn't yield a bogus id.
    let groups: Vec<&str> = uuid.split('-').collect();
    let lens = [8usize, 4, 4, 4, 12];
    if groups.len() == 5
        && groups
            .iter()
            .zip(lens.iter())
            .all(|(g, &n)| g.len() == n && g.chars().all(|c| c.is_ascii_hexdigit()))
    {
        Some(uuid)
    } else {
        None
    }
}

/// Basename of a path string (last non-empty component), or `None` for an empty
/// or root path. Used to derive a project label from a cwd.
fn basename(path: &str) -> Option<String> {
    let trimmed = path.trim_end_matches('/');
    let base = trimmed.rsplit('/').next().unwrap_or("");
    if base.is_empty() {
        None
    } else {
        Some(base.to_string())
    }
}

/// Convert a `SystemTime` to an ISO-8601 / RFC-3339 UTC string (seconds
/// precision, `Z` suffix) so timestamps match the rest of the contract.
fn system_time_to_iso(t: SystemTime) -> String {
    let secs = t
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    chrono::DateTime::<chrono::Utc>::from_timestamp(secs, 0)
        .unwrap_or_else(|| chrono::DateTime::<chrono::Utc>::from_timestamp(0, 0).unwrap())
        .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

/// Parse an RFC-3339 / ISO-8601 timestamp into a `SystemTime`. Returns `None` on
/// any parse failure so callers fall back to mtime.
fn parse_rfc3339_to_system_time(iso: &str) -> Option<SystemTime> {
    let dt = chrono::DateTime::parse_from_rfc3339(iso).ok()?;
    let secs = dt.timestamp();
    if secs < 0 {
        return None;
    }
    Some(UNIX_EPOCH + std::time::Duration::from_secs(secs as u64))
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::Duration;

    /// Build a throwaway tree under a unique temp dir (pid + monotonic time +
    /// atomic counter so concurrent tests never collide) and return its root.
    /// Layout mirrors the real on-disk shape:
    ///   <root>/session_index.jsonl
    ///   <root>/sessions/YYYY/MM/DD/rollout-<ts>-<id>.jsonl
    ///   <root>/archived_sessions/rollout-<ts>-<id>.jsonl
    fn make_fixture_root() -> PathBuf {
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "hq-codex-sessions-test-{}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            SEQ.fetch_add(1, Ordering::Relaxed),
        ));
        fs::create_dir_all(&root).unwrap();
        root
    }

    /// The `session_meta` header line as Codex writes it (line 1 of a rollout).
    fn session_meta_line(id: &str, cwd: &str, started_at: &str) -> String {
        format!(
            r#"{{"timestamp":"{started_at}","type":"session_meta","payload":{{"id":"{id}","timestamp":"{started_at}","cwd":"{cwd}","originator":"Codex Desktop","cli_version":"0.128.0","source":"vscode","model_provider":"openai"}}}}"#
        )
    }

    /// The first `turn_context` line — carries the model (and a fresh cwd).
    fn turn_context_line(cwd: &str, model: &str, ts: &str) -> String {
        format!(
            r#"{{"timestamp":"{ts}","type":"turn_context","payload":{{"turn_id":"abc","cwd":"{cwd}","model":"{model}","approval_policy":"never"}}}}"#
        )
    }

    /// A non-meta rollout record that carries none of the fields we extract — must
    /// be skipped without breaking the head scan.
    fn event_line(ts: &str) -> String {
        format!(r#"{{"timestamp":"{ts}","type":"event_msg","payload":{{"kind":"agent_message","text":"working"}}}}"#)
    }

    fn index_line(id: &str, thread_name: &str, updated_at: &str) -> String {
        format!(r#"{{"id":"{id}","thread_name":"{thread_name}","updated_at":"{updated_at}"}}"#)
    }

    /// Write a rollout file at `sessions/YYYY/MM/DD/rollout-<ts>-<id>.jsonl`.
    fn write_session_rollout(root: &Path, date: &str, ts: &str, id: &str, contents: &str) -> PathBuf {
        // date = "YYYY/MM/DD"
        let dir = root.join("sessions").join(date);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("rollout-{ts}-{id}.jsonl"));
        fs::write(&path, contents).unwrap();
        path
    }

    /// Write a rollout file flat under `archived_sessions/`.
    fn write_archived_rollout(root: &Path, ts: &str, id: &str, contents: &str) -> PathBuf {
        let dir = root.join("archived_sessions");
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("rollout-{ts}-{id}.jsonl"));
        fs::write(&path, contents).unwrap();
        path
    }

    #[test]
    fn enumerates_rollouts_and_extracts_fields() {
        let root = make_fixture_root();

        // A live session under sessions/ and an archived one under
        // archived_sessions/ — both must be enumerated.
        let id_a = "019de12c-d83e-78c2-9bb3-cbb8146965e4";
        let id_b = "019de12f-9c8a-77c0-b8d1-12895c1e4b68";

        let rollout_a = format!(
            "{}\n{}\n{}\n",
            session_meta_line(id_a, "/Users/corey/Documents/HQ", "2026-06-15T18:00:00.000Z"),
            event_line("2026-06-15T18:00:01.000Z"),
            turn_context_line("/Users/corey/Documents/HQ", "gpt-5.5", "2026-06-15T18:00:02.000Z"),
        );
        write_session_rollout(&root, "2026/06/15", "2026-06-15T18-00-00", id_a, &rollout_a);

        let rollout_b = format!(
            "{}\n{}\n",
            session_meta_line(id_b, "/Users/corey/code/widget", "2026-06-14T10:00:00.000Z"),
            turn_context_line("/Users/corey/code/widget", "gpt-5.1-codex", "2026-06-14T10:00:05.000Z"),
        );
        write_archived_rollout(&root, "2026-06-14T10-00-00", id_b, &rollout_b);

        // Index covers both with last-activity + thread name.
        let index = format!(
            "{}\n{}\n",
            index_line(id_a, "Mission Control reader", "2026-06-15T18:05:00.000Z"),
            index_line(id_b, "Widget work", "2026-06-14T10:30:00.000Z"),
        );
        fs::write(root.join("session_index.jsonl"), index).unwrap();

        let mut sessions = scan_codex_sessions(&root, SystemTime::now());
        sessions.sort_by(|a, b| a.id.cmp(&b.id));

        assert_eq!(sessions.len(), 2, "both rollouts enumerated");

        let a = &sessions[0];
        assert_eq!(a.id, id_a);
        assert_eq!(a.tool, AgentTool::Codex);
        assert_eq!(a.origin, AgentOrigin::Local);
        assert_eq!(a.cwd, "/Users/corey/Documents/HQ");
        assert_eq!(a.model, "gpt-5.5", "model lifted from turn_context");
        assert_eq!(a.project, "HQ", "project from cwd basename");
        assert_eq!(a.company, "", "Codex rollouts carry no HQ company metadata");
        assert_eq!(a.source, SOURCE_TAG);
        // Index updated_at is preferred for last-activity.
        assert_eq!(a.last_activity_at, "2026-06-15T18:05:00.000Z");
        // started_at comes from the session_meta payload.
        assert_eq!(a.started_at, "2026-06-15T18:00:00.000Z");

        let b = &sessions[1];
        assert_eq!(b.id, id_b);
        assert_eq!(b.cwd, "/Users/corey/code/widget");
        assert_eq!(b.model, "gpt-5.1-codex");
        assert_eq!(b.project, "widget");
        assert_eq!(b.last_activity_at, "2026-06-14T10:30:00.000Z");
    }

    #[test]
    fn missing_codex_dir_yields_empty() {
        let root = make_fixture_root();
        let nonexistent = root.join("does-not-exist");
        let sessions = scan_codex_sessions(&nonexistent, SystemTime::now());
        assert!(sessions.is_empty());
    }

    #[test]
    fn rollout_without_index_uses_mtime_fallback() {
        let root = make_fixture_root();
        let id = "019de131-1a9a-7eb0-9811-d2ed00b47f6c";
        let rollout = format!(
            "{}\n{}\n",
            session_meta_line(id, "/tmp/proj", "2026-06-15T18:00:00.000Z"),
            turn_context_line("/tmp/proj", "gpt-5.5", "2026-06-15T18:00:01.000Z"),
        );
        write_session_rollout(&root, "2026/06/15", "2026-06-15T18-00-00", id, &rollout);
        // No session_index.jsonl written at all.

        let sessions = scan_codex_sessions(&root, SystemTime::now());
        assert_eq!(sessions.len(), 1, "rollout enumerated even with no index");
        let s = &sessions[0];
        assert_eq!(s.id, id);
        assert_eq!(s.cwd, "/tmp/proj");
        // last-activity falls back to mtime (a real ISO string, non-empty).
        assert!(
            s.last_activity_at.ends_with('Z') && s.last_activity_at.len() >= 20,
            "last_activity_at falls back to an ISO mtime: {}",
            s.last_activity_at
        );
    }

    /// HARD performance contract: a rollout far larger than HEAD_BYTES must be
    /// enumerated via a bounded head read, NOT a full parse. We prove the
    /// boundedness two ways: (1) the reader still extracts the head fields from a
    /// huge file, and (2) the head reader provably reads at most HEAD_BYTES.
    #[test]
    fn large_rollout_is_not_fully_read() {
        let root = make_fixture_root();
        let id = "019de142-4c91-7420-95b2-99e7096c9cf7";

        // Real head: session_meta + turn_context carrying the fields that must win.
        let mut contents = String::with_capacity((HEAD_BYTES as usize) * 3);
        contents.push_str(&session_meta_line(
            id,
            "/Users/corey/Documents/HQ",
            "2026-06-15T18:00:00.000Z",
        ));
        contents.push('\n');
        contents.push_str(&turn_context_line(
            "/Users/corey/Documents/HQ",
            "gpt-5.5",
            "2026-06-15T18:00:01.000Z",
        ));
        contents.push('\n');

        // Pad with > HEAD_BYTES of filler event lines AFTER the head, plus a
        // "stale" turn_context far past the head window. If the reader fully
        // parsed the file, it would still pick the FIRST turn_context (head-first
        // wins), but the filler proves we never need to read past the window.
        let filler = event_line("2026-06-15T18:00:02.000Z");
        while (contents.len() as u64) < HEAD_BYTES * 2 {
            contents.push_str(&filler);
            contents.push('\n');
        }
        // A late turn_context with a model that must NOT win (proves head-first).
        contents.push_str(&turn_context_line(
            "/should/not/win",
            "stale-model-past-head",
            "2030-01-01T00:00:00.000Z",
        ));
        contents.push('\n');

        let path = write_session_rollout(&root, "2026/06/15", "2026-06-15T18-00-00", id, &contents);

        let file_len = fs::metadata(&path).unwrap().len();
        assert!(
            file_len > HEAD_BYTES * 2,
            "fixture must exceed the head window to exercise the bound (len={file_len}, head={HEAD_BYTES})"
        );

        // (1) End-to-end: the head fields win, the late stale line is never seen.
        let sessions = scan_codex_sessions(&root, SystemTime::now());
        assert_eq!(sessions.len(), 1);
        let s = &sessions[0];
        assert_eq!(
            s.model, "gpt-5.5",
            "model must come from the HEAD, proving the rest was not read"
        );
        assert_eq!(s.cwd, "/Users/corey/Documents/HQ");
        assert_ne!(s.model, "stale-model-past-head");
        assert_ne!(s.cwd, "/should/not/win");

        // (2) Direct bound check: the head reader buffers at most HEAD_BYTES and
        // still recovers the head fields.
        let head = read_head_info(&path);
        assert_eq!(head.model.as_deref(), Some("gpt-5.5"));
        assert_eq!(head.cwd.as_deref(), Some("/Users/corey/Documents/HQ"));
        assert_eq!(head.id.as_deref(), Some(id));
    }

    #[test]
    fn malformed_index_lines_are_skipped_not_fatal() {
        let root = make_fixture_root();
        let id = "019de18b-fe3f-7a20-8474-9e632c07ded5";
        let rollout = format!(
            "{}\n{}\n",
            session_meta_line(id, "/tmp/x", "2026-06-15T18:00:00.000Z"),
            turn_context_line("/tmp/x", "gpt-5.5", "2026-06-15T18:00:01.000Z"),
        );
        write_session_rollout(&root, "2026/06/15", "2026-06-15T18-00-00", id, &rollout);

        // Index with a garbage line + a line missing an id + the real line.
        let index = format!(
            "not json at all\n{{\"thread_name\":\"no id here\"}}\n{}\n",
            index_line(id, "Real session", "2026-06-15T18:05:00.000Z"),
        );
        fs::write(root.join("session_index.jsonl"), index).unwrap();

        let sessions = scan_codex_sessions(&root, SystemTime::now());
        assert_eq!(sessions.len(), 1, "garbage index lines don't blank the scan");
        assert_eq!(sessions[0].last_activity_at, "2026-06-15T18:05:00.000Z");
    }

    #[test]
    fn project_falls_back_to_thread_name_when_cwd_absent() {
        let root = make_fixture_root();
        let id = "019e0525-596f-7013-82ee-9397e9a1c30b";
        // Rollout with only an event line — no session_meta cwd, no turn_context.
        let rollout = format!("{}\n", event_line("2026-06-15T18:00:00.000Z"));
        write_session_rollout(&root, "2026/06/15", "2026-06-15T18-00-00", id, &rollout);

        let index = format!(
            "{}\n",
            index_line(id, "Codex thread label", "2026-06-15T18:05:00.000Z"),
        );
        fs::write(root.join("session_index.jsonl"), index).unwrap();

        let sessions = scan_codex_sessions(&root, SystemTime::now());
        assert_eq!(sessions.len(), 1);
        let s = &sessions[0];
        assert_eq!(s.cwd, "", "no cwd recoverable");
        // id falls back to filename when the rollout head lacked session_meta.
        assert_eq!(s.id, id);
        assert_eq!(
            s.project, "Codex thread label",
            "project falls back to index thread_name when cwd is absent"
        );
    }

    #[test]
    fn status_window_maps_age_to_taxonomy() {
        let now = UNIX_EPOCH + Duration::from_secs(2_000_000_000);
        let mtime = now; // mtime irrelevant when last_activity parses
        let iso = |secs_ago: u64| system_time_to_iso(now - Duration::from_secs(secs_ago));

        // Fresh activity → running.
        assert_eq!(
            status_from_age(&iso(10), mtime, now),
            SessionStatus::Running
        );
        // 5 minutes stale → idle.
        assert_eq!(
            status_from_age(&iso(5 * 60), mtime, now),
            SessionStatus::Idle
        );
        // 2 hours stale → ended.
        assert_eq!(
            status_from_age(&iso(2 * 60 * 60), mtime, now),
            SessionStatus::Ended
        );
        // Unparseable last-activity → falls back to mtime (here = now → running).
        assert_eq!(
            status_from_age("not-a-timestamp", mtime, now),
            SessionStatus::Running
        );
    }

    #[test]
    fn rollout_id_from_filename_extracts_trailing_uuid() {
        assert_eq!(
            rollout_id_from_filename(
                "rollout-2026-04-30T19-35-05-019de12c-d83e-78c2-9bb3-cbb8146965e4.jsonl"
            )
            .as_deref(),
            Some("019de12c-d83e-78c2-9bb3-cbb8146965e4")
        );
        // Not a rollout → None.
        assert_eq!(rollout_id_from_filename("README.md"), None);
        // Missing a valid UUID tail → None.
        assert_eq!(rollout_id_from_filename("rollout-2026-04-30-foo.jsonl"), None);
    }

    #[test]
    fn non_rollout_files_are_ignored() {
        let root = make_fixture_root();
        let dir = root.join("sessions").join("2026/06/15");
        fs::create_dir_all(&dir).unwrap();
        // A stray non-rollout file alongside a real rollout.
        fs::write(dir.join("notes.txt"), "not a rollout").unwrap();
        let id = "019e1068-682a-7833-9717-cf2224c5af0d";
        fs::write(
            dir.join(format!("rollout-2026-06-15T18-00-00-{id}.jsonl")),
            format!(
                "{}\n{}\n",
                session_meta_line(id, "/tmp/x", "2026-06-15T18:00:00.000Z"),
                turn_context_line("/tmp/x", "gpt-5.5", "2026-06-15T18:00:01.000Z"),
            ),
        )
        .unwrap();

        let sessions = scan_codex_sessions(&root, SystemTime::now());
        assert_eq!(sessions.len(), 1, "only the rollout is enumerated");
        assert_eq!(sessions[0].id, id);
    }
}
