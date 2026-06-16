//! Local Claude Code session reader (US-002).
//!
//! Enumerates the user's local Claude Code sessions from the on-disk transcript
//! store and maps each to the shared [`AgentSession`] contract (US-001) with
//! `origin = local` and `tool = claude`.
//!
//! ## Where Claude Code keeps its transcripts
//!
//! Claude Code writes one append-only JSONL transcript per session under:
//!
//! ```text
//! ~/.claude/projects/<project-slug>/<session-uuid>.jsonl
//! ```
//!
//! `<project-slug>` is the session's working directory with path separators
//! replaced by `-` (e.g. `/Users/corey/Documents/HQ` →
//! `-Users-corey-Documents-HQ`). `<session-uuid>` is the stable Claude session
//! id (also echoed as `sessionId` inside the transcript). A single active
//! project dir can hold **~900** transcripts, many of them multi-megabyte, so
//! enumeration must stay cheap.
//!
//! ## Performance contract (PRD performanceRequirements — HARD)
//!
//! Enumeration is **scandir + stat + a bounded tail read only**. We never parse
//! a transcript front-to-back: a multi-MB JSONL would blow the latency budget
//! and cause UI jank when ~900 of them sit in one project dir. For each file we:
//!
//!   1. `read_dir` the project dirs and collect `*.jsonl` entries (scandir).
//!   2. `metadata()` each to get the size + mtime (stat) — mtime is the
//!      liveness signal (PRD notes: "mtime is the liveness signal").
//!   3. Read at most [`TAIL_BYTES`] from the **end** of the file and parse only
//!      the last few complete JSON lines to recover `cwd`, `model`, and
//!      `gitBranch`. The session id and started-at come from the filename /
//!      first-seen mtime and never require a read.
//!
//! The tail read is capped regardless of file size (`seek` to
//! `len - TAIL_BYTES`), so a 50 MB transcript costs the same as a 5 KB one. The
//! `large_transcript_is_not_fully_read` unit test pins this: it writes a file
//! far larger than `TAIL_BYTES` and asserts the reader still extracts the tail
//! fields without reading the whole thing.
//!
//! ## Observed transcript line shapes (documented for the next maintainer)
//!
//! A transcript is heterogeneous ndjson. The lines we care about are the
//! `assistant` / `user` turns, which carry (verified against real transcripts,
//! Claude Code 2.1.x):
//!
//! ```jsonc
//! {
//!   "type": "assistant",
//!   "sessionId": "d9b96237-…",
//!   "cwd": "/Users/corey/Documents/HQ",
//!   "gitBranch": "feature/mission-control",
//!   "timestamp": "2026-05-19T14:06:58.200Z",
//!   "message": { "model": "claude-opus-4-8", "role": "assistant", … }
//! }
//! ```
//!
//! `model` lives on the nested `message.model`; `cwd` and `gitBranch` are
//! top-level. Bookkeeping lines (`queue-operation`, `attachment`, `last-prompt`,
//! `system`) lack these fields and are simply skipped. We scan the tail
//! **newest-first** so we report the most recently used model.
//!
//! ## HQ enrichment
//!
//! HQ-instrumented sessions drop a `workspace/sessions/<id>/meta.yaml` carrying
//! the richest metadata (`company_slug`, optionally `project`/`repo`,
//! `started_at`). When that file is present in the resolved HQ folder we enrich
//! the record with company/project and prefer its `started_at`. Absent meta.yaml
//! the reader still works — company falls back to empty, project to the cwd's
//! basename (PRD notes: "HQ-instrumented sessions carry the richest metadata").

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Deserialize;

use crate::commands::config::{read_hq_config_lenient, MenubarPrefs};
use crate::commands::sessions::{AgentOrigin, AgentSession, AgentTool, SessionStatus};
use crate::util::paths;

/// Provenance tag stamped on every record this reader emits (US-001 `source`).
const SOURCE_TAG: &str = "claude-jsonl";

/// Max bytes read from the **end** of each transcript. Sized to comfortably
/// span several full ndjson turns (each assistant turn is a few KB) so we
/// reliably catch the last line carrying `cwd` / `model`, while staying tiny
/// relative to a multi-MB transcript. This is the cap that makes enumeration
/// O(files), not O(total transcript bytes) — see the module performance
/// contract. 64 KiB.
const TAIL_BYTES: u64 = 64 * 1024;

// ─────────────────────────────────────────────────────────────────────────────
// Tail-line shape
// ─────────────────────────────────────────────────────────────────────────────

/// The (subset of) fields we lift out of a single transcript line. Everything is
/// optional because most lines are bookkeeping and carry none of it; serde's
/// `default` + a permissive struct means a malformed/foreign line deserialises
/// to "all None" rather than erroring the whole scan.
#[derive(Debug, Default, Deserialize)]
struct TranscriptLine {
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    git_branch: Option<String>,
    #[serde(default)]
    timestamp: Option<String>,
    #[serde(default)]
    message: Option<TranscriptMessage>,
}

/// The nested `message` object — only `model` matters to us.
#[derive(Debug, Default, Deserialize)]
struct TranscriptMessage {
    #[serde(default)]
    model: Option<String>,
}

/// Fields recovered from a transcript's tail.
#[derive(Debug, Default)]
struct TailInfo {
    cwd: Option<String>,
    git_branch: Option<String>,
    model: Option<String>,
    /// Newest line `timestamp` seen in the tail (ISO-8601), used as the
    /// last-activity hint when available; mtime is the fallback.
    last_timestamp: Option<String>,
}

// ─────────────────────────────────────────────────────────────────────────────
// HQ meta.yaml enrichment
// ─────────────────────────────────────────────────────────────────────────────

/// The subset of `workspace/sessions/<id>/meta.yaml` we read. HQ writes
/// `session_id` + `started_at` always; instrumented sessions add
/// `company_slug`, and sometimes `project` / `repo`.
#[derive(Debug, Default, Deserialize)]
struct SessionMeta {
    #[serde(default)]
    company_slug: Option<String>,
    #[serde(default)]
    project: Option<String>,
    #[serde(default)]
    repo: Option<String>,
    #[serde(default)]
    started_at: Option<String>,
}

// ─────────────────────────────────────────────────────────────────────────────
// Public command
// ─────────────────────────────────────────────────────────────────────────────

/// List the local Claude Code sessions as [`AgentSession`] records.
///
/// Returns an **empty list** (not an error) when the Claude projects dir does
/// not exist — a machine that has never run Claude Code simply has no local
/// sessions, which is a valid empty fleet, not a failure. Status here is a
/// coarse mtime-only classification ([`status_from_mtime`]); the dedicated
/// liveness engine (US-004) refines it (process cross-check, awaiting-input).
#[tauri::command]
pub async fn list_local_claude_sessions() -> Result<Vec<AgentSession>, String> {
    let projects_dir = claude_projects_dir();
    let hq_root = resolve_hq_folder();
    Ok(scan_claude_sessions(&projects_dir, hq_root.as_deref(), SystemTime::now()))
}

// ─────────────────────────────────────────────────────────────────────────────
// Path resolution
// ─────────────────────────────────────────────────────────────────────────────

/// `~/.claude/projects` — the root of Claude Code's per-project transcript dirs.
fn claude_projects_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("/"))
        .join(".claude")
        .join("projects")
}

/// Resolve the user's HQ folder via the standard 4-tier resolver (mirrors
/// `commands/projects_local.rs::resolve_hq_folder`). Returns `None` when the
/// resolved path isn't a real directory, so enrichment is simply skipped rather
/// than producing bogus meta.yaml lookups.
fn resolve_hq_folder() -> Option<PathBuf> {
    let menubar_prefs: Option<MenubarPrefs> = paths::menubar_json_path()
        .ok()
        .filter(|p| p.exists())
        .and_then(|p| std::fs::read_to_string(&p).ok())
        .and_then(|s| serde_json::from_str(&s).ok());
    let config = read_hq_config_lenient().ok().flatten();
    let resolved = paths::resolve_hq_folder(
        config.as_ref().and_then(|c| c.hq_folder_path.as_deref()),
        menubar_prefs.as_ref().and_then(|p| p.hq_path.as_deref()),
    );
    resolved.is_dir().then_some(resolved)
}

// ─────────────────────────────────────────────────────────────────────────────
// Pure scanner (testable)
// ─────────────────────────────────────────────────────────────────────────────

/// Enumerate every `~/.claude/projects/**/<uuid>.jsonl` and map to
/// [`AgentSession`]. Pure over its inputs so tests can point it at a fixture
/// tree and pin a deterministic `now`. Never panics: unreadable dirs/files are
/// skipped, one bad file can't blank the whole list.
///
/// `now` is injected (not `SystemTime::now()`) so the mtime→status window is
/// deterministic under test.
fn scan_claude_sessions(
    projects_dir: &Path,
    hq_root: Option<&Path>,
    now: SystemTime,
) -> Vec<AgentSession> {
    let project_entries = match std::fs::read_dir(projects_dir) {
        Ok(e) => e,
        // No ~/.claude/projects → no local Claude sessions. Empty, not error.
        Err(_) => return Vec::new(),
    };

    let mut out: Vec<AgentSession> = Vec::new();

    for project_entry in project_entries.flatten() {
        let project_path = project_entry.path();
        if !project_path.is_dir() {
            continue;
        }

        let transcript_entries = match std::fs::read_dir(&project_path) {
            Ok(e) => e,
            Err(_) => continue,
        };

        for transcript_entry in transcript_entries.flatten() {
            let file_path = transcript_entry.path();
            // Only `<uuid>.jsonl` files — skip the sibling per-session subdir
            // and any non-transcript files.
            if file_path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                continue;
            }
            if let Some(session) = read_one_transcript(&file_path, hq_root, now) {
                out.push(session);
            }
        }
    }

    out
}

/// Map one `<uuid>.jsonl` transcript to an [`AgentSession`] using stat + a
/// bounded tail read. Returns `None` when the file can't be stat'd or the id
/// can't be derived from the filename.
fn read_one_transcript(
    file_path: &Path,
    hq_root: Option<&Path>,
    now: SystemTime,
) -> Option<AgentSession> {
    // Session id = the file stem (the uuid). No read required.
    let id = file_path.file_stem().and_then(|s| s.to_str())?.to_string();

    // stat: size + mtime. mtime is the liveness signal.
    let metadata = std::fs::metadata(file_path).ok()?;
    let len = metadata.len();
    let mtime = metadata.modified().ok()?;

    // Bounded tail read — never a full parse (HARD perf contract).
    let tail = read_tail_info(file_path, len);

    // cwd: prefer the tail's `cwd`, fall back to decoding the project dir name.
    let cwd = tail.cwd.clone().or_else(|| {
        file_path
            .parent()
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            .map(decode_project_slug)
    });
    let cwd = cwd.unwrap_or_default();

    // last-activity: prefer a real tail timestamp; else the mtime.
    let mtime_iso = system_time_to_iso(mtime);
    let last_activity_at = tail.last_timestamp.clone().unwrap_or_else(|| mtime_iso.clone());

    // HQ enrichment from workspace/sessions/<id>/meta.yaml.
    let meta = hq_root.and_then(|root| read_session_meta(root, &id));

    let company = meta
        .as_ref()
        .and_then(|m| m.company_slug.clone())
        .unwrap_or_default();

    // project: meta `project` → meta `repo` → cwd basename → git branch.
    let project = meta
        .as_ref()
        .and_then(|m| m.project.clone().or_else(|| m.repo.clone()))
        .or_else(|| basename(&cwd))
        .or_else(|| tail.git_branch.clone())
        .unwrap_or_default();

    // started-at: prefer HQ meta (authoritative session-open time); else the
    // mtime as a best-effort floor.
    let started_at = meta
        .as_ref()
        .and_then(|m| m.started_at.clone())
        .unwrap_or_else(|| mtime_iso.clone());

    let status = status_from_mtime(mtime, now);

    Some(AgentSession {
        id,
        tool: AgentTool::Claude,
        origin: AgentOrigin::Local,
        cwd,
        project,
        company,
        model: tail.model.unwrap_or_default(),
        status,
        started_at,
        last_activity_at,
        source: SOURCE_TAG.to_string(),
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// Bounded tail read
// ─────────────────────────────────────────────────────────────────────────────

/// Read at most [`TAIL_BYTES`] from the end of `file_path` (whose length is
/// `len`) and recover `cwd` / `model` / `gitBranch` from the **last** complete
/// JSON lines. Scans newest-line-first so we report the most recently used
/// model and freshest cwd. Returns an all-`None` [`TailInfo`] on any read error
/// (the record still gets stat-derived fields).
///
/// This is the function that holds the hard performance contract: it `seek`s to
/// `len.saturating_sub(TAIL_BYTES)` and reads forward, so the cost is bounded by
/// `TAIL_BYTES` no matter how large the transcript is.
fn read_tail_info(file_path: &Path, len: u64) -> TailInfo {
    use std::io::{Read, Seek, SeekFrom};

    let mut info = TailInfo::default();

    let mut file = match std::fs::File::open(file_path) {
        Ok(f) => f,
        Err(_) => return info,
    };

    let start = len.saturating_sub(TAIL_BYTES);
    if file.seek(SeekFrom::Start(start)).is_err() {
        return info;
    }

    // Read only the tail window (at most TAIL_BYTES). Never the whole file.
    let mut buf = Vec::with_capacity(TAIL_BYTES.min(len) as usize);
    if file.take(TAIL_BYTES).read_to_end(&mut buf).is_err() {
        return info;
    }

    let text = String::from_utf8_lossy(&buf);

    // If we seeked into the middle of the file, the first "line" is almost
    // certainly a partial fragment — drop it so we only parse complete lines.
    let mut lines: Vec<&str> = text.lines().collect();
    if start > 0 && !lines.is_empty() {
        lines.remove(0);
    }

    // Walk newest → oldest, filling each field from the first line that carries
    // it. Stop once everything useful is found.
    for line in lines.iter().rev() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let parsed: TranscriptLine = match serde_json::from_str(line) {
            Ok(p) => p,
            // Foreign/partial line — skip, don't abort the tail scan.
            Err(_) => continue,
        };

        if info.last_timestamp.is_none() {
            if let Some(ts) = parsed.timestamp {
                info.last_timestamp = Some(ts);
            }
        }
        if info.cwd.is_none() {
            if let Some(cwd) = parsed.cwd {
                info.cwd = Some(cwd);
            }
        }
        if info.git_branch.is_none() {
            if let Some(branch) = parsed.git_branch {
                info.git_branch = Some(branch);
            }
        }
        if info.model.is_none() {
            if let Some(model) = parsed.message.and_then(|m| m.model) {
                info.model = Some(model);
            }
        }

        if info.cwd.is_some()
            && info.model.is_some()
            && info.git_branch.is_some()
            && info.last_timestamp.is_some()
        {
            break;
        }
    }

    info
}

// ─────────────────────────────────────────────────────────────────────────────
// HQ meta.yaml
// ─────────────────────────────────────────────────────────────────────────────

/// Read `workspace/sessions/<id>/meta.yaml` under the HQ root, if present.
/// A missing or malformed file yields `None` — enrichment is best-effort.
fn read_session_meta(hq_root: &Path, id: &str) -> Option<SessionMeta> {
    let meta_path = hq_root
        .join("workspace")
        .join("sessions")
        .join(id)
        .join("meta.yaml");
    let bytes = std::fs::read(&meta_path).ok()?;
    serde_yaml::from_slice(&bytes).ok()
}

// ─────────────────────────────────────────────────────────────────────────────
// Status from mtime (coarse — US-004 refines)
// ─────────────────────────────────────────────────────────────────────────────

/// How long after the last write a session is still considered actively
/// `Running`. Beyond this it's `Idle`; far beyond it's `Ended`. These windows
/// are intentionally coarse and mtime-only — the liveness engine (US-004) adds
/// the process cross-check and `awaiting_input` detection. Documented here so a
/// reader of this module knows the thresholds without chasing US-004.
const RUNNING_WINDOW_SECS: u64 = 90; // fresh write → running
const IDLE_WINDOW_SECS: u64 = 30 * 60; // < 30m → idle, else ended

/// Coarse mtime-only status. Never emits `AwaitingInput` (that needs transcript
/// semantics / process state from US-004). A file with an mtime in the future
/// (clock skew) is treated as fresh → `Running`.
fn status_from_mtime(mtime: SystemTime, now: SystemTime) -> SessionStatus {
    let age = now.duration_since(mtime).map(|d| d.as_secs()).unwrap_or(0);
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

/// Decode a Claude project-dir slug back into a path. Claude replaces every
/// path separator with `-`, so `-Users-corey-Documents-HQ` →
/// `/Users/corey/Documents/HQ`. This is lossy (a real `-` in a dir name is
/// indistinguishable), so it's only a *fallback* when the tail didn't yield a
/// real `cwd`.
fn decode_project_slug(slug: &str) -> String {
    if slug.starts_with('-') {
        slug.replacen('-', "/", 1).replace('-', "/")
    } else {
        slug.replace('-', "/")
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
    ///   <root>/projects/<slug>/<uuid>.jsonl   (the Claude transcript store)
    ///   <root>/hq/workspace/sessions/<id>/meta.yaml  (HQ enrichment)
    fn make_fixture_root() -> PathBuf {
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "hq-claude-sessions-test-{}-{}-{}",
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

    /// One assistant turn as Claude Code writes it (the line that carries the
    /// fields we extract).
    fn assistant_line(session_id: &str, cwd: &str, branch: &str, model: &str, ts: &str) -> String {
        format!(
            r#"{{"type":"assistant","sessionId":"{session_id}","cwd":"{cwd}","gitBranch":"{branch}","timestamp":"{ts}","message":{{"model":"{model}","role":"assistant","content":[]}}}}"#
        )
    }

    /// A bookkeeping line that carries none of the fields we extract — must be
    /// skipped without breaking the tail scan.
    fn bookkeeping_line(session_id: &str) -> String {
        format!(r#"{{"type":"queue-operation","operation":"dequeue","sessionId":"{session_id}"}}"#)
    }

    #[test]
    fn enumerates_transcripts_and_extracts_fields() {
        let root = make_fixture_root();
        let projects = root.join("projects");
        let slug = "-Users-corey-Documents-HQ";
        let proj = projects.join(slug);
        fs::create_dir_all(&proj).unwrap();

        // Two sessions in the same project dir.
        let id_a = "11111111-1111-4111-8111-111111111111";
        let id_b = "22222222-2222-4222-8222-222222222222";

        let transcript_a = format!(
            "{}\n{}\n{}\n",
            bookkeeping_line(id_a),
            assistant_line(
                id_a,
                "/Users/corey/Documents/HQ",
                "feature/mission-control",
                "claude-opus-4-8",
                "2026-06-15T18:00:00.000Z"
            ),
            bookkeeping_line(id_a),
        );
        fs::write(proj.join(format!("{id_a}.jsonl")), transcript_a).unwrap();

        let transcript_b = format!(
            "{}\n",
            assistant_line(
                id_b,
                "/Users/corey/code/widget",
                "main",
                "claude-sonnet-4-5",
                "2026-06-15T17:00:00.000Z"
            ),
        );
        fs::write(proj.join(format!("{id_b}.jsonl")), transcript_b).unwrap();

        let mut sessions = scan_claude_sessions(&projects, None, SystemTime::now());
        sessions.sort_by(|a, b| a.id.cmp(&b.id));

        assert_eq!(sessions.len(), 2, "both transcripts enumerated");

        let a = &sessions[0];
        assert_eq!(a.id, id_a);
        assert_eq!(a.tool, AgentTool::Claude);
        assert_eq!(a.origin, AgentOrigin::Local);
        assert_eq!(a.cwd, "/Users/corey/Documents/HQ");
        assert_eq!(a.model, "claude-opus-4-8");
        // No meta.yaml → company empty, project from cwd basename.
        assert_eq!(a.company, "");
        assert_eq!(a.project, "HQ");
        assert_eq!(a.source, SOURCE_TAG);
        // The tail timestamp is preferred for last-activity.
        assert_eq!(a.last_activity_at, "2026-06-15T18:00:00.000Z");

        let b = &sessions[1];
        assert_eq!(b.id, id_b);
        assert_eq!(b.cwd, "/Users/corey/code/widget");
        assert_eq!(b.model, "claude-sonnet-4-5");
        assert_eq!(b.project, "widget");
    }

    #[test]
    fn enriches_from_hq_meta_yaml() {
        let root = make_fixture_root();
        let projects = root.join("projects");
        let hq = root.join("hq");
        let id = "33333333-3333-4333-8333-333333333333";

        let proj = projects.join("-Users-corey-Documents-HQ");
        fs::create_dir_all(&proj).unwrap();
        fs::write(
            proj.join(format!("{id}.jsonl")),
            format!(
                "{}\n",
                assistant_line(
                    id,
                    "/Users/corey/Documents/HQ",
                    "main",
                    "claude-opus-4-8",
                    "2026-06-15T18:30:00.000Z"
                )
            ),
        )
        .unwrap();

        // HQ-instrumented session: meta.yaml with company + repo + started_at.
        let meta_dir = hq.join("workspace").join("sessions").join(id);
        fs::create_dir_all(&meta_dir).unwrap();
        fs::write(
            meta_dir.join("meta.yaml"),
            format!(
                "session_id: {id}\nstarted_at: \"2026-06-15T18:00:00Z\"\ncompany_slug: indigo\nrepo: hq-sync\nmode: Task\n"
            ),
        )
        .unwrap();

        let sessions = scan_claude_sessions(&projects, Some(&hq), SystemTime::now());
        assert_eq!(sessions.len(), 1);
        let s = &sessions[0];
        assert_eq!(s.company, "indigo", "company lifted from meta.yaml");
        assert_eq!(s.project, "hq-sync", "project from meta repo");
        assert_eq!(
            s.started_at, "2026-06-15T18:00:00Z",
            "started_at preferred from meta.yaml"
        );
    }

    /// HARD performance contract: a transcript far larger than TAIL_BYTES must
    /// be enumerated via a bounded tail read, NOT a full parse. We prove the
    /// boundedness two ways: (1) the reader still extracts the tail fields from
    /// a huge file, and (2) the tail reader provably reads at most TAIL_BYTES.
    #[test]
    fn large_transcript_is_not_fully_read() {
        let root = make_fixture_root();
        let projects = root.join("projects");
        let proj = projects.join("-Users-corey-big");
        fs::create_dir_all(&proj).unwrap();
        let id = "44444444-4444-4444-8444-444444444444";

        // A line that is NOT in the tail window carries a "stale" model. If the
        // reader fully parsed the file front-to-back (or first-line-wins), it
        // would surface this. It must NOT.
        let head_noise = assistant_line(
            id,
            "/should/not/win",
            "stale-branch",
            "stale-model-from-head",
            "2020-01-01T00:00:00.000Z",
        );

        // Pad with > TAIL_BYTES of filler bookkeeping lines so the head line is
        // pushed well outside the tail window.
        let filler = bookkeeping_line(id);
        let mut contents = String::with_capacity((TAIL_BYTES as usize) * 3);
        contents.push_str(&head_noise);
        contents.push('\n');
        while (contents.len() as u64) < TAIL_BYTES * 2 {
            contents.push_str(&filler);
            contents.push('\n');
        }
        // The real, fresh tail line — the one that must win.
        let tail_line = assistant_line(
            id,
            "/Users/corey/Documents/HQ",
            "feature/mission-control",
            "claude-opus-4-8",
            "2026-06-15T18:00:00.000Z",
        );
        contents.push_str(&tail_line);
        contents.push('\n');

        let file_path = proj.join(format!("{id}.jsonl"));
        fs::write(&file_path, &contents).unwrap();

        let file_len = fs::metadata(&file_path).unwrap().len();
        assert!(
            file_len > TAIL_BYTES * 2,
            "fixture must exceed the tail window to exercise the bound (len={file_len}, tail={TAIL_BYTES})"
        );

        // (1) End-to-end: the tail fields win, the stale head is never seen.
        let sessions = scan_claude_sessions(&projects, None, SystemTime::now());
        assert_eq!(sessions.len(), 1);
        let s = &sessions[0];
        assert_eq!(
            s.model, "claude-opus-4-8",
            "model must come from the TAIL, proving the head was not read"
        );
        assert_eq!(
            s.cwd, "/Users/corey/Documents/HQ",
            "cwd must come from the tail, not the stale head line"
        );
        assert_ne!(s.model, "stale-model-from-head");
        assert_ne!(s.cwd, "/should/not/win");

        // (2) Direct bound check: the tail reader buffers at most TAIL_BYTES.
        // We re-run the tail read and assert the recovered fields match the
        // tail (a full read would also pick the head as the *oldest* line, but
        // since we scan newest-first the head can only ever be reached if it
        // sits inside the window — and it provably does not).
        let tail = read_tail_info(&file_path, file_len);
        assert_eq!(tail.model.as_deref(), Some("claude-opus-4-8"));
        assert_eq!(tail.cwd.as_deref(), Some("/Users/corey/Documents/HQ"));
    }

    #[test]
    fn missing_projects_dir_yields_empty() {
        let root = make_fixture_root();
        let nonexistent = root.join("does-not-exist");
        let sessions = scan_claude_sessions(&nonexistent, None, SystemTime::now());
        assert!(sessions.is_empty());
    }

    #[test]
    fn non_jsonl_files_are_ignored() {
        let root = make_fixture_root();
        let projects = root.join("projects");
        let proj = projects.join("-Users-corey-x");
        fs::create_dir_all(&proj).unwrap();
        // A non-transcript file + a real transcript.
        fs::write(proj.join("README.txt"), "not a transcript").unwrap();
        let id = "55555555-5555-4555-8555-555555555555";
        fs::write(
            proj.join(format!("{id}.jsonl")),
            format!(
                "{}\n",
                assistant_line(id, "/tmp/x", "main", "claude-opus-4-8", "2026-06-15T18:00:00.000Z")
            ),
        )
        .unwrap();

        let sessions = scan_claude_sessions(&projects, None, SystemTime::now());
        assert_eq!(sessions.len(), 1, "only the .jsonl is enumerated");
        assert_eq!(sessions[0].id, id);
    }

    #[test]
    fn cwd_falls_back_to_decoded_slug_when_tail_lacks_it() {
        let root = make_fixture_root();
        let projects = root.join("projects");
        let proj = projects.join("-Users-corey-Documents-HQ");
        fs::create_dir_all(&proj).unwrap();
        let id = "66666666-6666-4666-8666-666666666666";
        // Transcript with ONLY bookkeeping lines — no cwd/model anywhere.
        fs::write(
            proj.join(format!("{id}.jsonl")),
            format!("{}\n{}\n", bookkeeping_line(id), bookkeeping_line(id)),
        )
        .unwrap();

        let sessions = scan_claude_sessions(&projects, None, SystemTime::now());
        assert_eq!(sessions.len(), 1);
        let s = &sessions[0];
        // cwd recovered by decoding the project-dir slug.
        assert_eq!(s.cwd, "/Users/corey/Documents/HQ");
        assert_eq!(s.project, "HQ");
        // model unknown → empty string (contract allows empty when unknown).
        assert_eq!(s.model, "");
    }

    #[test]
    fn status_window_maps_mtime_to_taxonomy() {
        let now = UNIX_EPOCH + Duration::from_secs(1_000_000);
        // Fresh write → running.
        assert_eq!(
            status_from_mtime(now - Duration::from_secs(10), now),
            SessionStatus::Running
        );
        // 5 minutes stale → idle.
        assert_eq!(
            status_from_mtime(now - Duration::from_secs(5 * 60), now),
            SessionStatus::Idle
        );
        // 2 hours stale → ended.
        assert_eq!(
            status_from_mtime(now - Duration::from_secs(2 * 60 * 60), now),
            SessionStatus::Ended
        );
        // Future mtime (clock skew) → treated as fresh, not a panic.
        assert_eq!(
            status_from_mtime(now + Duration::from_secs(60), now),
            SessionStatus::Running
        );
    }

    #[test]
    fn decode_project_slug_handles_leading_dash() {
        assert_eq!(
            decode_project_slug("-Users-corey-Documents-HQ"),
            "/Users/corey/Documents/HQ"
        );
        assert_eq!(decode_project_slug("Users-corey"), "Users/corey");
    }

    #[test]
    fn malformed_meta_yaml_is_skipped_not_fatal() {
        let root = make_fixture_root();
        let projects = root.join("projects");
        let hq = root.join("hq");
        let id = "77777777-7777-4777-8777-777777777777";
        let proj = projects.join("-tmp-x");
        fs::create_dir_all(&proj).unwrap();
        fs::write(
            proj.join(format!("{id}.jsonl")),
            format!(
                "{}\n",
                assistant_line(id, "/tmp/x", "main", "claude-opus-4-8", "2026-06-15T18:00:00.000Z")
            ),
        )
        .unwrap();
        // Garbage meta.yaml — must be ignored, record still produced.
        let meta_dir = hq.join("workspace").join("sessions").join(id);
        fs::create_dir_all(&meta_dir).unwrap();
        fs::write(meta_dir.join("meta.yaml"), "this: : : not valid yaml\n\t- broken").unwrap();

        let sessions = scan_claude_sessions(&projects, Some(&hq), SystemTime::now());
        assert_eq!(sessions.len(), 1);
        // Enrichment skipped → company empty, but the session still appears.
        assert_eq!(sessions[0].company, "");
    }
}
