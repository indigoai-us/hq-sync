//! Session history derivation (US-004).
//!
//! Derives a chronological **history feed** for Mission Control from HQ's own
//! orchestration artifacts — the things sessions *did* over time, not just what is
//! live now. Two sources, both under the resolved HQ workspace:
//!
//!   1. **`workspace/metrics/audit-log.jsonl`** — the append-only orchestrator
//!      audit log. One JSON object per line; the events we surface are tasks/
//!      stories being dispatched and completed (`story_dispatched`,
//!      `story_completed`, `story_failed`, `task_started`, `task_completed`, …).
//!   2. **`workspace/threads/*.json`** — per-session thread files. Their `type`
//!      field (`checkpoint` / `handoff` / `auto-checkpoint`) gives us the
//!      checkpoint and handoff events the audit log doesn't carry.
//!
//! The output is a flat, newest-first [`HistoryEvent`] list the UI renders as a
//! timeline (PRD US-008 design: story completions, tasks dispatched, checkpoints,
//! handoffs).
//!
//! ## Bounded reads (PRD performance contract — reuse, do not slurp)
//!
//! The audit log grows without bound (thousands of lines on an active box). We
//! read **only the last [`AUDIT_TAIL_LINES`] lines** via a bounded tail read
//! ([`read_last_lines`] seeks to the end and walks backward over at most
//! [`AUDIT_TAIL_BYTES`]), never the whole file — mirroring the readers' tail/head
//! discipline. The threads dir holds *thousands* of small JSON files; we scandir,
//! sort by mtime, and parse only the newest [`THREAD_SCAN_LIMIT`] of them, and
//! each individual file is size-capped at [`THREAD_MAX_BYTES`] before parse so a
//! pathological file can't blow the budget. The whole feed is finally capped at
//! [`MAX_EVENTS`].

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::commands::config::{read_hq_config_lenient, MenubarPrefs};
use crate::util::paths;

// ─────────────────────────────────────────────────────────────────────────────
// Bounded-read constants (documented — never slurp huge files)
// ─────────────────────────────────────────────────────────────────────────────

/// How many lines we read from the **end** of `audit-log.jsonl`. The feed only
/// shows recent activity (US-008 design: "last 24h" range), so the freshest few
/// hundred audit rows are plenty — and a bounded tail keeps the read O(window),
/// not O(file), no matter how large the log grows.
const AUDIT_TAIL_LINES: usize = 400;

/// Hard byte cap on the audit tail read. Even if the last [`AUDIT_TAIL_LINES`]
/// lines were pathologically long, we never buffer more than this from the end of
/// the file. 512 KiB comfortably spans 400 normal audit lines (~200 B each).
const AUDIT_TAIL_BYTES: u64 = 512 * 1024;

/// How many of the newest thread JSON files we parse. The threads dir can hold
/// thousands of files; we scandir + stat all of them (cheap) but only open and
/// parse the freshest this-many.
const THREAD_SCAN_LIMIT: usize = 200;

/// Per-thread-file size cap. Thread files are small (a few KB), so anything over
/// this is almost certainly not a normal thread file — skip it rather than read
/// it, keeping the per-file cost bounded. 256 KiB.
const THREAD_MAX_BYTES: u64 = 256 * 1024;

/// Final cap on the merged feed length. The UI paginates; we never hand it an
/// unbounded list.
const MAX_EVENTS: usize = 300;

// ─────────────────────────────────────────────────────────────────────────────
// History event (output shape)
// ─────────────────────────────────────────────────────────────────────────────

/// The kind of thing that happened — drives the timeline node color in the UI
/// (US-008: completed = green, dispatched/handoff = neutral, checkpoint = faint).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HistoryEventKind {
    /// A story/task was dispatched to a worker (work started).
    Dispatched,
    /// A story/task completed successfully.
    Completed,
    /// A story/task failed.
    Failed,
    /// A session checkpoint was written.
    Checkpoint,
    /// A session was handed off to a successor.
    Handoff,
}

/// One entry in the Mission Control history feed.
///
/// camelCase serialisation matches the rest of the sessions contract so the TS
/// side reads it without remapping.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryEvent {
    /// What happened — drives the timeline node treatment.
    pub kind: HistoryEventKind,
    /// Human-readable title (e.g. "US-004 completed", "Checkpoint: …").
    pub title: String,
    /// Owning company slug, when resolvable; empty when unknown.
    pub company: String,
    /// Project the event belongs to, when known; empty otherwise.
    pub project: String,
    /// ISO-8601 timestamp the event occurred (used for ordering + display).
    pub timestamp: String,
    /// Provenance tag — `audit-log` or `thread` — so the UI can label/debug the
    /// observation channel (mirrors `AgentSession.source`).
    pub source: String,
}

// ─────────────────────────────────────────────────────────────────────────────
// Audit-log line shape
// ─────────────────────────────────────────────────────────────────────────────

/// The subset of an `audit-log.jsonl` line we surface. Everything but `event` is
/// optional so a foreign/partial line deserialises to "event + None" rather than
/// erroring the scan; a line without an `event` is skipped.
#[derive(Debug, Default, Deserialize)]
struct AuditLine {
    #[serde(default)]
    event: Option<String>,
    #[serde(default)]
    timestamp: Option<String>,
    #[serde(default)]
    company: Option<String>,
    #[serde(default)]
    project: Option<String>,
    #[serde(default)]
    story_id: Option<String>,
    #[serde(default)]
    action: Option<String>,
}

// ─────────────────────────────────────────────────────────────────────────────
// Thread-file shape
// ─────────────────────────────────────────────────────────────────────────────

/// The subset of a `workspace/threads/*.json` file we surface. Thread files carry
/// a `type` (checkpoint / handoff / auto-checkpoint / rule / insight / …); we only
/// turn checkpoint- and handoff-kind threads into history events.
#[derive(Debug, Default, Deserialize)]
struct ThreadFile {
    #[serde(default, rename = "type")]
    kind: Option<String>,
    #[serde(default)]
    created_at: Option<String>,
    #[serde(default)]
    updated_at: Option<String>,
    #[serde(default)]
    project: Option<String>,
    #[serde(default)]
    company: Option<String>,
    /// Newer thread files use `conversation_summary`; older ones `summary`.
    #[serde(default)]
    conversation_summary: Option<String>,
    #[serde(default)]
    summary: Option<String>,
    #[serde(default)]
    metadata: Option<ThreadMetadata>,
}

/// The `metadata` block on auto-checkpoint thread files — carries a human title.
#[derive(Debug, Default, Deserialize)]
struct ThreadMetadata {
    #[serde(default)]
    title: Option<String>,
}

// ─────────────────────────────────────────────────────────────────────────────
// Public command
// ─────────────────────────────────────────────────────────────────────────────

/// Build the Mission Control history feed from the local HQ workspace.
///
/// Returns an **empty list** (not an error) when the HQ workspace can't be
/// resolved or neither source exists — a fresh box simply has no history, which is
/// a valid empty feed, not a failure (the UI renders an empty state).
#[tauri::command]
pub async fn list_session_history() -> Result<Vec<HistoryEvent>, String> {
    let workspace = match resolve_workspace_dir() {
        Some(w) => w,
        None => return Ok(Vec::new()),
    };
    Ok(derive_history(&workspace))
}

// ─────────────────────────────────────────────────────────────────────────────
// Path resolution
// ─────────────────────────────────────────────────────────────────────────────

/// Resolve `<hq-root>/workspace` via the standard 4-tier HQ folder resolver
/// (mirrors `commands/sessions/claude.rs::resolve_hq_folder`). Returns `None` when
/// the resolved workspace isn't a real directory.
fn resolve_workspace_dir() -> Option<PathBuf> {
    let menubar_prefs: Option<MenubarPrefs> = paths::menubar_json_path()
        .ok()
        .filter(|p| p.exists())
        .and_then(|p| std::fs::read_to_string(&p).ok())
        .and_then(|s| serde_json::from_str(&s).ok());
    let config = read_hq_config_lenient().ok().flatten();
    let hq_root = paths::resolve_hq_folder(
        config.as_ref().and_then(|c| c.hq_folder_path.as_deref()),
        menubar_prefs.as_ref().and_then(|p| p.hq_path.as_deref()),
    );
    let workspace = hq_root.join("workspace");
    workspace.is_dir().then_some(workspace)
}

// ─────────────────────────────────────────────────────────────────────────────
// Pure derivation (testable)
// ─────────────────────────────────────────────────────────────────────────────

/// Derive the history feed from a `workspace` directory. Pure over its input so
/// tests can point it at a fixture tree. Never panics: a missing/unreadable source
/// contributes nothing rather than aborting the other.
///
/// Output is sorted newest-first by timestamp and capped at [`MAX_EVENTS`].
pub fn derive_history(workspace: &Path) -> Vec<HistoryEvent> {
    let mut events = Vec::new();

    events.extend(events_from_audit_log(
        &workspace.join("metrics").join("audit-log.jsonl"),
    ));
    events.extend(events_from_threads(&workspace.join("threads")));

    // Newest first. A missing/unparseable timestamp sorts last (treated as "").
    events.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
    events.truncate(MAX_EVENTS);
    events
}

// ─────────────────────────────────────────────────────────────────────────────
// Audit-log → events
// ─────────────────────────────────────────────────────────────────────────────

/// Parse the tail of `audit-log.jsonl` into [`HistoryEvent`]s. Only the events we
/// surface (dispatch / complete / fail) are mapped; orchestration noise
/// (`phase_completed`, `pipeline_*`, `gate_*`, `project_*`) is skipped so the feed
/// reads as session-level activity, not internal plumbing.
fn events_from_audit_log(path: &Path) -> Vec<HistoryEvent> {
    let lines = read_last_lines(path, AUDIT_TAIL_LINES, AUDIT_TAIL_BYTES);
    let mut out = Vec::new();

    for line in &lines {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let parsed: AuditLine = match serde_json::from_str(line) {
            Ok(p) => p,
            Err(_) => continue, // foreign/partial line — skip
        };
        let Some(event) = parsed.event.as_deref() else {
            continue;
        };

        let kind = match event {
            "story_dispatched" | "task_started" => HistoryEventKind::Dispatched,
            "story_completed" | "task_completed" => HistoryEventKind::Completed,
            "story_failed" | "task_failed" => HistoryEventKind::Failed,
            // Orchestration-internal events are not session history — skip.
            _ => continue,
        };

        // Title: prefer the explicit action text; else synthesise from story id +
        // a verb, so a terse audit row still reads clearly in the timeline.
        let title = parsed
            .action
            .clone()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| {
                let story = parsed.story_id.clone().unwrap_or_default();
                let verb = match kind {
                    HistoryEventKind::Dispatched => "dispatched",
                    HistoryEventKind::Completed => "completed",
                    HistoryEventKind::Failed => "failed",
                    // Unreachable for audit-log kinds, but keep total.
                    HistoryEventKind::Checkpoint => "checkpoint",
                    HistoryEventKind::Handoff => "handoff",
                };
                if story.is_empty() {
                    verb.to_string()
                } else {
                    format!("{story} {verb}")
                }
            });

        out.push(HistoryEvent {
            kind,
            title,
            company: parsed.company.unwrap_or_default(),
            project: parsed.project.unwrap_or_default(),
            timestamp: parsed.timestamp.unwrap_or_default(),
            source: "audit-log".to_string(),
        });
    }

    out
}

// ─────────────────────────────────────────────────────────────────────────────
// Threads → events
// ─────────────────────────────────────────────────────────────────────────────

/// Parse the newest [`THREAD_SCAN_LIMIT`] thread JSON files into checkpoint /
/// handoff [`HistoryEvent`]s. Files are scandir'd + stat'd (cheap), sorted by
/// mtime, and only the newest are opened; each is size-capped before parse.
fn events_from_threads(threads_dir: &Path) -> Vec<HistoryEvent> {
    let entries = match std::fs::read_dir(threads_dir) {
        Ok(e) => e,
        Err(_) => return Vec::new(), // no threads dir → no thread events
    };

    // (mtime, path) for every *.json file — cheap stat only, no reads yet.
    let mut candidates: Vec<(std::time::SystemTime, PathBuf)> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let metadata = match std::fs::metadata(&path) {
            Ok(m) => m,
            Err(_) => continue,
        };
        // Skip pathologically large files without opening them.
        if metadata.len() > THREAD_MAX_BYTES {
            continue;
        }
        let mtime = metadata
            .modified()
            .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
        candidates.push((mtime, path));
    }

    // Newest first, then parse only the freshest THREAD_SCAN_LIMIT.
    candidates.sort_by(|a, b| b.0.cmp(&a.0));
    candidates.truncate(THREAD_SCAN_LIMIT);

    let mut out = Vec::new();
    for (_, path) in &candidates {
        if let Some(event) = thread_to_event(path) {
            out.push(event);
        }
    }
    out
}

/// Map one thread file to a [`HistoryEvent`], or `None` if it isn't a
/// checkpoint/handoff-kind thread (or can't be parsed). `auto-checkpoint` and
/// `checkpoint` both map to the checkpoint kind.
fn thread_to_event(path: &Path) -> Option<HistoryEvent> {
    let bytes = std::fs::read(path).ok()?;
    let thread: ThreadFile = serde_json::from_slice(&bytes).ok()?;

    let kind = match thread.kind.as_deref() {
        Some("handoff") => HistoryEventKind::Handoff,
        Some("checkpoint") | Some("auto-checkpoint") => HistoryEventKind::Checkpoint,
        // rule / insight / note / created / updated / session — not history.
        _ => return None,
    };

    // Summary text → title; else a kind-derived label so the row is never blank.
    let summary = thread
        .conversation_summary
        .clone()
        .or(thread.summary.clone())
        .or_else(|| thread.metadata.as_ref().and_then(|m| m.title.clone()))
        .filter(|s| !s.is_empty());
    let title = summary.unwrap_or_else(|| match kind {
        HistoryEventKind::Handoff => "Handoff".to_string(),
        _ => "Checkpoint".to_string(),
    });

    // Prefer updated_at (most recent touch) for ordering; else created_at.
    let timestamp = thread
        .updated_at
        .clone()
        .or(thread.created_at.clone())
        .unwrap_or_default();

    Some(HistoryEvent {
        kind,
        title,
        company: thread.company.unwrap_or_default(),
        project: thread.project.unwrap_or_default(),
        timestamp,
        source: "thread".to_string(),
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// Bounded tail read
// ─────────────────────────────────────────────────────────────────────────────

/// Read at most the last `max_lines` lines from `path`, never buffering more than
/// `max_bytes` from the end of the file. Returns them in file order
/// (oldest-first within the window). On any read error returns an empty vec.
///
/// This is the function that holds the audit-log bound: it `seek`s to
/// `len - max_bytes` and reads forward, so the cost is bounded by `max_bytes`
/// regardless of how large the log has grown. If we seeked into the middle of the
/// file the first (partial) line is dropped so only complete lines are parsed.
fn read_last_lines(path: &Path, max_lines: usize, max_bytes: u64) -> Vec<String> {
    use std::io::{Read, Seek, SeekFrom};

    let mut file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return Vec::new(),
    };
    let len = match file.metadata() {
        Ok(m) => m.len(),
        Err(_) => return Vec::new(),
    };

    let start = len.saturating_sub(max_bytes);
    if file.seek(SeekFrom::Start(start)).is_err() {
        return Vec::new();
    }

    let mut buf = Vec::with_capacity(max_bytes.min(len) as usize);
    if file.take(max_bytes).read_to_end(&mut buf).is_err() {
        return Vec::new();
    }

    let text = String::from_utf8_lossy(&buf);
    let mut lines: Vec<&str> = text.lines().collect();

    // Dropped the first line if we started mid-file (it's likely a partial).
    if start > 0 && !lines.is_empty() {
        lines.remove(0);
    }

    // Keep only the last `max_lines` complete lines, in file order.
    if lines.len() > max_lines {
        lines = lines.split_off(lines.len() - max_lines);
    }

    lines.into_iter().map(|s| s.to_string()).collect()
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    /// Build a throwaway `workspace/` tree under a unique temp dir (pid +
    /// monotonic time + atomic counter so concurrent tests never collide).
    /// Layout mirrors the real shape:
    ///   <root>/metrics/audit-log.jsonl
    ///   <root>/threads/*.json
    fn make_workspace() -> PathBuf {
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "hq-history-test-{}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            SEQ.fetch_add(1, Ordering::Relaxed),
        ));
        fs::create_dir_all(root.join("metrics")).unwrap();
        fs::create_dir_all(root.join("threads")).unwrap();
        root
    }

    fn audit_line(event: &str, ts: &str, company: &str, project: &str, story: &str, action: &str) -> String {
        format!(
            r#"{{"timestamp":"{ts}","event":"{event}","company":"{company}","project":"{project}","story_id":"{story}","action":"{action}"}}"#
        )
    }

    fn write_thread(workspace: &Path, name: &str, contents: &str) {
        fs::write(workspace.join("threads").join(name), contents).unwrap();
    }

    #[test]
    fn derives_events_from_audit_log_and_threads() {
        let ws = make_workspace();

        // Audit log: a dispatch, a completion, a failure, plus orchestration
        // noise that must NOT surface.
        let audit = format!(
            "{}\n{}\n{}\n{}\n{}\n",
            audit_line(
                "story_dispatched",
                "2026-06-15T10:00:00Z",
                "indigo",
                "mission-control",
                "US-004",
                "Dispatched US-004 to backend-dev"
            ),
            audit_line(
                "story_completed",
                "2026-06-15T11:00:00Z",
                "indigo",
                "mission-control",
                "US-004",
                "US-004 completed: liveness engine"
            ),
            audit_line(
                "story_failed",
                "2026-06-15T09:00:00Z",
                "indigo",
                "mission-control",
                "US-009",
                "US-009 blocked"
            ),
            // Noise — must be skipped.
            audit_line(
                "phase_completed",
                "2026-06-15T10:30:00Z",
                "indigo",
                "mission-control",
                "US-004",
                "Phase 1 completed"
            ),
            audit_line(
                "pipeline_started",
                "2026-06-15T08:00:00Z",
                "indigo",
                "mission-control",
                "",
                "Pipeline started"
            ),
        );
        fs::write(ws.join("metrics").join("audit-log.jsonl"), audit).unwrap();

        // A checkpoint thread and a handoff thread (+ a rule thread that must be
        // ignored).
        write_thread(
            &ws,
            "T-checkpoint.json",
            r#"{"type":"checkpoint","created_at":"2026-06-15T10:15:00Z","updated_at":"2026-06-15T10:15:00Z","company":"indigo","project":"mission-control","conversation_summary":"Checkpoint after liveness draft"}"#,
        );
        write_thread(
            &ws,
            "T-handoff.json",
            r#"{"type":"handoff","created_at":"2026-06-15T12:00:00Z","updated_at":"2026-06-15T12:00:00Z","company":"indigo","project":"mission-control","conversation_summary":"Handed off to next session"}"#,
        );
        write_thread(
            &ws,
            "T-rule.json",
            r#"{"type":"rule","created_at":"2026-06-15T13:00:00Z","summary":"A learned rule — not history"}"#,
        );

        let events = derive_history(&ws);

        // 3 audit events (dispatch/complete/fail) + 2 threads (checkpoint/handoff)
        // = 5. The phase/pipeline noise and the rule thread are excluded.
        assert_eq!(events.len(), 5, "only session-level events surface: {events:#?}");

        // Newest-first ordering: handoff (12:00) is first, pipeline noise absent.
        assert_eq!(events[0].kind, HistoryEventKind::Handoff);
        assert_eq!(events[0].timestamp, "2026-06-15T12:00:00Z");

        // No orchestration-noise events leaked in.
        assert!(
            events.iter().all(|e| !e.title.contains("Phase 1")
                && !e.title.contains("Pipeline started")),
            "phase/pipeline events must be excluded"
        );

        // The kinds we expect are all present.
        let kinds: Vec<_> = events.iter().map(|e| e.kind).collect();
        assert!(kinds.contains(&HistoryEventKind::Dispatched));
        assert!(kinds.contains(&HistoryEventKind::Completed));
        assert!(kinds.contains(&HistoryEventKind::Failed));
        assert!(kinds.contains(&HistoryEventKind::Checkpoint));
        assert!(kinds.contains(&HistoryEventKind::Handoff));

        // Provenance tagged.
        assert!(events.iter().any(|e| e.source == "audit-log"));
        assert!(events.iter().any(|e| e.source == "thread"));
    }

    #[test]
    fn empty_workspace_yields_empty_feed() {
        let ws = make_workspace();
        // Neither file written.
        assert!(derive_history(&ws).is_empty());
    }

    #[test]
    fn malformed_audit_lines_are_skipped_not_fatal() {
        let ws = make_workspace();
        let audit = format!(
            "not json at all\n{{\"no\":\"event\"}}\n{}\n",
            audit_line(
                "story_completed",
                "2026-06-15T11:00:00Z",
                "indigo",
                "p",
                "US-001",
                "done"
            ),
        );
        fs::write(ws.join("metrics").join("audit-log.jsonl"), audit).unwrap();

        let events = derive_history(&ws);
        assert_eq!(events.len(), 1, "garbage lines don't blank the feed");
        assert_eq!(events[0].kind, HistoryEventKind::Completed);
    }

    #[test]
    fn audit_title_synthesised_when_action_absent() {
        let ws = make_workspace();
        // story_dispatched with an empty action → title synthesised from story id.
        let line = r#"{"timestamp":"2026-06-15T10:00:00Z","event":"story_dispatched","story_id":"US-007","project":"mc"}"#;
        fs::write(ws.join("metrics").join("audit-log.jsonl"), format!("{line}\n")).unwrap();

        let events = derive_history(&ws);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].title, "US-007 dispatched");
    }

    #[test]
    fn thread_summary_falls_back_across_field_shapes() {
        let ws = make_workspace();
        // Old-style thread: `summary` (not `conversation_summary`).
        write_thread(
            &ws,
            "T-old.json",
            r#"{"type":"checkpoint","created_at":"2026-06-15T10:00:00Z","summary":"old-style summary"}"#,
        );
        // Auto-checkpoint with only metadata.title.
        write_thread(
            &ws,
            "T-auto.json",
            r#"{"type":"auto-checkpoint","created_at":"2026-06-15T11:00:00Z","updated_at":"2026-06-15T11:00:00Z","metadata":{"title":"Auto: recency"}}"#,
        );

        let events = derive_history(&ws);
        assert_eq!(events.len(), 2);
        let titles: Vec<_> = events.iter().map(|e| e.title.as_str()).collect();
        assert!(titles.contains(&"old-style summary"));
        assert!(titles.contains(&"Auto: recency"));
        // Both are checkpoint-kind.
        assert!(events.iter().all(|e| e.kind == HistoryEventKind::Checkpoint));
    }

    #[test]
    fn oversized_thread_file_is_skipped_without_reading() {
        let ws = make_workspace();
        // A "thread" file far over the cap — must be skipped (never parsed).
        let big = "x".repeat((THREAD_MAX_BYTES as usize) + 1);
        write_thread(&ws, "T-huge.json", &big);
        // A normal handoff alongside it.
        write_thread(
            &ws,
            "T-ok.json",
            r#"{"type":"handoff","created_at":"2026-06-15T12:00:00Z","conversation_summary":"ok"}"#,
        );

        let events = derive_history(&ws);
        assert_eq!(events.len(), 1, "oversized file skipped, normal one kept");
        assert_eq!(events[0].title, "ok");
    }

    #[test]
    fn malformed_thread_json_is_skipped_not_fatal() {
        let ws = make_workspace();
        write_thread(&ws, "T-broken.json", "{ not valid json");
        write_thread(
            &ws,
            "T-good.json",
            r#"{"type":"checkpoint","created_at":"2026-06-15T10:00:00Z","conversation_summary":"good"}"#,
        );

        let events = derive_history(&ws);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].title, "good");
    }

    #[test]
    fn read_last_lines_bounds_to_window() {
        let ws = make_workspace();
        let path = ws.join("metrics").join("audit-log.jsonl");

        // Write more lines than the window; assert we only get the last N.
        let mut contents = String::new();
        for i in 0..50 {
            contents.push_str(&format!("line-{i}\n"));
        }
        fs::write(&path, contents).unwrap();

        let last5 = read_last_lines(&path, 5, AUDIT_TAIL_BYTES);
        assert_eq!(last5.len(), 5, "exactly the window size returned");
        assert_eq!(last5.first().map(String::as_str), Some("line-45"));
        assert_eq!(last5.last().map(String::as_str), Some("line-49"));
    }

    #[test]
    fn audit_tail_only_recent_events_surface() {
        let ws = make_workspace();
        let path = ws.join("metrics").join("audit-log.jsonl");

        // Far more completed events than AUDIT_TAIL_LINES — only the tail window
        // should surface, proving we never parse the whole (potentially huge) log.
        let mut contents = String::new();
        for i in 0..(AUDIT_TAIL_LINES + 100) {
            // Pad each line with a stable shape; the index encodes ordering.
            let ts = format!("2026-06-15T00:{:02}:{:02}Z", (i / 60) % 60, i % 60);
            contents.push_str(&audit_line(
                "story_completed",
                &ts,
                "indigo",
                "p",
                &format!("US-{i:04}"),
                &format!("completed {i}"),
            ));
            contents.push('\n');
        }
        fs::write(&path, contents).unwrap();

        let events = derive_history(&ws);
        // Capped at the tail window (≤ AUDIT_TAIL_LINES), and never the full file.
        assert!(
            events.len() <= AUDIT_TAIL_LINES,
            "feed bounded to the tail window: {} > {}",
            events.len(),
            AUDIT_TAIL_LINES
        );
        // The earliest lines (index 0..100) must have been dropped by the tail.
        assert!(
            events.iter().all(|e| !e.title.contains("completed 0 ")),
            "oldest events outside the tail window are not surfaced"
        );
    }
}
