//! Mission Control — the shared `AgentSession` contract (US-001).
//!
//! This module is the Rust half of a cross-language contract. The TypeScript
//! half lives in `src/desktop-alt/lib/sessions.ts` and declares the same shape;
//! both sides serialise to camelCase JSON so the local readers, the outpost
//! heartbeat, and the desktop UI all speak one shape.
//!
//! Contract-first by design (PRD US-001): the cross-repo pieces (the on-box
//! outpost emitter and the desktop subscriber) serialise/deserialise the *same*
//! [`AgentSession`], so the wire payloads map 1:1 across the boundary. Later
//! stories (US-002+) populate these records from on-disk Claude/Codex artifacts;
//! this module owns only the type definitions and the status taxonomy.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Runtime};

use crate::util::logfile::log;

use self::history::HistoryEvent;
use self::liveness::scan_running_agents;

// ─────────────────────────────────────────────────────────────────────────────
// Readers (per-tool submodules)
// ─────────────────────────────────────────────────────────────────────────────

/// Local Claude Code session reader (US-002) — enumerates
/// `~/.claude/projects/**/<uuid>.jsonl` and maps to [`AgentSession`].
pub mod claude;

/// Local Codex session reader (US-003) — enumerates
/// `~/.codex/session_index.jsonl` + `sessions/**/rollout-*.jsonl` (and
/// `archived_sessions`) and maps to [`AgentSession`].
pub mod codex;

/// Liveness engine (US-004) — refines the readers' coarse mtime status into the
/// [`SessionStatus`] taxonomy via a last-activity window cross-checked against
/// running `claude`/`codex` processes (no live process → [`SessionStatus::Ended`]).
pub mod liveness;

/// Session history derivation (US-004) — builds the chronological Mission Control
/// history feed from `workspace/metrics/audit-log.jsonl` and
/// `workspace/threads/*.json` (dispatches, completions, checkpoints, handoffs).
pub mod history;

/// Desktop outpost subscriber + box-level status + merge (US-011) — subscribes
/// to the outpost sessions topic (reusing the `dm_mqtt.rs` pattern), merges the
/// remote `AgentSession[]` (origin=outpost) into this snapshot, and surfaces the
/// box-level status card sourced from `GET /outpost/status`. S3-heartbeat fallback
/// + a stale-after timeout keep it honest when the box stops reporting.
pub mod outpost;

// ─────────────────────────────────────────────────────────────────────────────
// Status taxonomy
// ─────────────────────────────────────────────────────────────────────────────

/// Canonical session status taxonomy (US-001).
///
/// This is the ONE place the status values are defined on the Rust side; the
/// readers (US-002/US-003) and the liveness engine (US-004) map onto these
/// variants, and the UI renders them. Keep this in lock-step with the TS
/// `SessionStatus` union in `sessions.ts`.
///
/// Serialises to camelCase-context snake_case strings (`running`,
/// `awaiting_input`, `idle`, `ended`) so the JSON matches the TS literal union
/// exactly. Liveness is best-effort (observed from on-disk artifacts + process
/// checks) and the UI labels it as such.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionStatus {
    /// The agent is actively working (live process + fresh activity).
    Running,
    /// Alive but blocked on the human (e.g. a prompt/approval).
    AwaitingInput,
    /// Recently active but quiet now; no fresh activity.
    Idle,
    /// The session is over (no live process, or long-stale).
    Ended,
}

impl SessionStatus {
    /// Whether this status counts as "live" for summary/badge purposes —
    /// `Running` and `AwaitingInput` are live; `Idle` and `Ended` are not.
    /// Centralised so the backend and UI agree on what "active" means.
    pub fn is_live(self) -> bool {
        matches!(self, SessionStatus::Running | SessionStatus::AwaitingInput)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tool + origin
// ─────────────────────────────────────────────────────────────────────────────

/// The agent tool that owns the session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentTool {
    /// Claude Code.
    Claude,
    /// OpenAI Codex.
    Codex,
}

/// Where the session is observed: this machine (`local`) or the outpost VM.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentOrigin {
    /// Observed on the local machine via filesystem reads.
    Local,
    /// Reported by the user's outpost VM via the realtime heartbeat.
    Outpost,
}

// ─────────────────────────────────────────────────────────────────────────────
// AgentSession
// ─────────────────────────────────────────────────────────────────────────────

/// The unified agent-session record (US-001 data model).
///
/// One shape for every session Mission Control knows about, regardless of tool
/// (Claude Code / Codex) or origin (local filesystem / outpost heartbeat). The
/// field order and names mirror the TS `AgentSession` interface exactly;
/// camelCase serialisation keeps the two sides on one wire shape.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentSession {
    /// Stable session id (e.g. the Claude transcript uuid or Codex rollout id).
    pub id: String,
    /// Which agent tool owns the session.
    pub tool: AgentTool,
    /// Where the session is observed (local machine vs. outpost VM).
    pub origin: AgentOrigin,
    /// Working directory the session is running in.
    pub cwd: String,
    /// Project the session is working on (derived from cwd / HQ metadata).
    pub project: String,
    /// Owning company slug, when resolvable; empty string when unknown.
    pub company: String,
    /// Model the session is using (e.g. `claude-opus-4-8`), when known.
    pub model: String,
    /// Best-effort lifecycle status — see [`SessionStatus`].
    pub status: SessionStatus,
    /// ISO-8601 timestamp the session started, when known.
    pub started_at: String,
    /// ISO-8601 timestamp of the most recent observed activity.
    pub last_activity_at: String,
    /// Where this record was sourced from — a short provenance tag (e.g.
    /// `claude-jsonl`, `codex-rollout`, `outpost-heartbeat`). Lets the UI label
    /// the observation channel and aids debugging.
    pub source: String,
}

// ─────────────────────────────────────────────────────────────────────────────
// Mission Control snapshot (US-005) — the command + polling payload
// ─────────────────────────────────────────────────────────────────────────────

/// The full Mission Control payload (US-005): the merged local fleet plus the
/// history feed, in one shape so the command return value and the poll event
/// carry exactly the same thing.
///
/// camelCase serialisation matches the rest of the sessions contract so the TS
/// side reads it without remapping (`{ sessions, history }`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MissionControlSnapshot {
    /// The merged fleet — local `AgentSession[]` (Claude + Codex, liveness
    /// applied) PLUS any fresh outpost sessions (origin=outpost) folded in from
    /// the realtime heartbeat (US-011), so local + remote agents are one list.
    pub sessions: Vec<AgentSession>,
    /// The derived history feed (tasks dispatched, stories completed,
    /// checkpoints, handoffs), newest-first.
    pub history: Vec<HistoryEvent>,
    /// The box-level outpost status card (US-011), or `None` when no outpost is
    /// known. Heads the outpost group in the UI; reflects up/down, runtime,
    /// relay, and last-seen (aged by the heartbeat stale-after timeout).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outpost: Option<outpost::OutpostStatus>,
}

/// Event name the polling loop emits on each re-scan (US-005).
///
/// Follows the established `<domain>:<event>` convention used across the app
/// (`sync:*`, `share:*`, `meeting:*`) — see `events.rs`. The frontend store
/// `listen`s for this to stay fresh without a manual refresh, mirroring how the
/// share/sync surfaces consume their typed events.
pub const EVENT_SESSIONS_UPDATED: &str = "sessions:updated";

/// How often the polling loop re-scans the local fleet, in seconds. Configurable
/// at runtime via the `HQ_SYNC_SESSIONS_POLL_SECS` env var (clamped to a sane
/// floor so a typo can't busy-spin the readers); defaults to this value.
/// Mirrors `share_notify::SHARE_POLL_INTERVAL_SECS` — a single named cadence the
/// outpost emitter (US-009) is documented to match.
const SESSIONS_POLL_INTERVAL_SECS: u64 = 5;

/// Lower bound on the poll cadence (seconds). The readers are cheap (scandir +
/// stat + bounded tail), but a sub-second interval would still be wasteful; clamp
/// any override up to this floor.
const SESSIONS_POLL_FLOOR_SECS: u64 = 2;

/// Diagnostic-log tag for the sessions polling loop.
const LOG_TAG: &str = "sessions";

/// Resolve the effective poll interval: `HQ_SYNC_SESSIONS_POLL_SECS` when set to
/// a parseable positive integer (clamped to [`SESSIONS_POLL_FLOOR_SECS`]),
/// otherwise the [`SESSIONS_POLL_INTERVAL_SECS`] default. Pure over its input so
/// the clamp/parse rules are unit-testable without touching the real env.
fn resolve_poll_interval(env_value: Option<&str>) -> Duration {
    let secs = env_value
        .and_then(|s| s.trim().parse::<u64>().ok())
        .filter(|n| *n > 0)
        .map(|n| n.max(SESSIONS_POLL_FLOOR_SECS))
        .unwrap_or(SESSIONS_POLL_INTERVAL_SECS);
    Duration::from_secs(secs)
}

// ─────────────────────────────────────────────────────────────────────────────
// Merge + liveness (pure)
// ─────────────────────────────────────────────────────────────────────────────

/// Build the merged local fleet from the two readers' outputs, re-deriving each
/// session's [`SessionStatus`] against the live-process inventory.
///
/// The readers (US-002/US-003) stamp a *coarse* mtime-only status; this is the
/// one place the US-004 liveness engine is applied across the whole fleet. We run
/// the (relatively expensive) process scan **once** and reuse the inventory for
/// every session, so liveness for N sessions costs a single `pgrep`, not N.
///
/// Pure over its inputs (readers' output + the process inventory + injected
/// `now`) so the merge + liveness rule is unit-testable without filesystem or
/// process I/O. Output preserves the readers' ordering (Claude first, then
/// Codex), which keeps the snapshot stable across polls.
fn merge_sessions(
    claude: Vec<AgentSession>,
    codex: Vec<AgentSession>,
    agents: liveness::RunningAgents,
    now: SystemTime,
) -> Vec<AgentSession> {
    claude
        .into_iter()
        .chain(codex)
        .map(|mut session| {
            // The merged record only carries the ISO `lastActivityAt`; the raw
            // mtime fallback isn't on the wire shape, so use the epoch as the
            // fallback — the readers always stamp a parseable ISO, so the
            // fallback is effectively never hit in practice.
            session.status = liveness::derive_status(
                &session.last_activity_at,
                UNIX_EPOCH,
                session.tool,
                agents,
                now,
            );
            session
        })
        .collect()
}

// ─────────────────────────────────────────────────────────────────────────────
// Snapshot assembly (async, real I/O)
// ─────────────────────────────────────────────────────────────────────────────

/// Assemble a fresh [`MissionControlSnapshot`]: run both local readers, apply
/// liveness, and derive the history feed. Best-effort — a reader that errors
/// contributes an empty list rather than failing the whole snapshot, so one bad
/// store can't blank the fleet.
async fn collect_snapshot() -> MissionControlSnapshot {
    let now = SystemTime::now();
    let claude = claude::list_local_claude_sessions().await.unwrap_or_default();
    let codex = codex::list_local_codex_sessions().await.unwrap_or_default();
    let agents = scan_running_agents();
    let local = merge_sessions(claude, codex, agents, now);

    // Fold in the outpost fleet (US-011): the realtime subscriber + S3 fallback
    // keep a stale-aware cache; `outpost_view` returns FRESH outpost sessions
    // (empty past the stale timeout) plus the box-status card. Outpost sessions
    // carry the emitter's own liveness — we do NOT re-run the local process scan
    // against them (their processes live on the VM, not this box).
    let outpost_view = outpost::outpost_view(now);
    let sessions = append_outpost_sessions(local, outpost_view.sessions);

    let history = history::list_session_history().await.unwrap_or_default();

    MissionControlSnapshot {
        sessions,
        history,
        outpost: outpost_view.status,
    }
}

/// Append the outpost fleet onto the local fleet for the merged snapshot
/// (US-011). Pure over its inputs so the merge ordering (local first, then
/// outpost) is unit-testable. Each outpost session is defensively re-stamped
/// `origin=outpost` so the UI's origin grouping never mis-buckets a remote
/// session as local, regardless of what the wire claimed.
fn append_outpost_sessions(
    mut local: Vec<AgentSession>,
    outpost: Vec<AgentSession>,
) -> Vec<AgentSession> {
    local.extend(outpost.into_iter().map(|mut s| {
        s.origin = AgentOrigin::Outpost;
        s
    }));
    local
}

// ─────────────────────────────────────────────────────────────────────────────
// Tauri command
// ─────────────────────────────────────────────────────────────────────────────

/// List the merged local agent sessions plus the history feed (US-005).
///
/// Returns the merged local `AgentSession[]` (Claude + Codex readers, with the
/// US-004 liveness engine applied) and the derived history feed in one
/// [`MissionControlSnapshot`]. Registered in `main.rs`'s `invoke_handler`; the
/// frontend store calls this on mount and the polling loop re-emits the same
/// shape on every tick.
#[tauri::command]
pub async fn list_agent_sessions() -> Result<MissionControlSnapshot, String> {
    Ok(collect_snapshot().await)
}

// ─────────────────────────────────────────────────────────────────────────────
// Polling loop (mirrors the sync-stats / share-notify poller pattern)
// ─────────────────────────────────────────────────────────────────────────────

/// Spawn the Mission Control polling loop. Called from `main.rs` setup.
///
/// Mirrors `share_notify::setup_share_notify_poller`: a launch poll after a short
/// delay (lets the app finish initialising), then a re-scan on an independent
/// interval timer. Each cycle assembles a fresh [`MissionControlSnapshot`] and
/// emits it to the frontend as a typed [`EVENT_SESSIONS_UPDATED`] event — the
/// same event-name convention and `app.emit` payload-typing approach the
/// sync/share surfaces use — so the UI stays fresh without a manual refresh.
///
/// The cadence is configurable via `HQ_SYNC_SESSIONS_POLL_SECS`
/// (see [`resolve_poll_interval`]); the outpost emitter (US-009) is documented to
/// match it.
pub fn setup_sessions_poller<R: Runtime>(app: AppHandle<R>) {
    let interval = resolve_poll_interval(std::env::var("HQ_SYNC_SESSIONS_POLL_SECS").ok().as_deref());
    tauri::async_runtime::spawn(async move {
        // Launch delay — give the app a moment to finish setup before the first
        // scan (mirrors the share/updater pollers' settle delay).
        tokio::time::sleep(Duration::from_secs(3)).await;
        emit_snapshot(&app).await;

        let mut ticker = tokio::time::interval(interval);
        // The first tick fires immediately; consume it so the launch emit above
        // isn't double-counted, then emit once per interval thereafter.
        ticker.tick().await;
        loop {
            ticker.tick().await;
            emit_snapshot(&app).await;
        }
    });
}

/// Assemble one snapshot and emit it to the frontend as [`EVENT_SESSIONS_UPDATED`].
/// Best-effort: a failed emit (e.g. no webview yet) is logged, never fatal.
async fn emit_snapshot<R: Runtime>(app: &AppHandle<R>) {
    let snapshot = collect_snapshot().await;
    if let Err(e) = app.emit(EVENT_SESSIONS_UPDATED, &snapshot) {
        log(LOG_TAG, &format!("SESSIONS_EMIT_FAILED {e}"));
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> AgentSession {
        AgentSession {
            id: "25f8d9da-435d-44e6-8bb7-849fd8ad67c8".to_string(),
            tool: AgentTool::Claude,
            origin: AgentOrigin::Local,
            cwd: "/Users/corey/Documents/HQ/repos/public/hq-sync".to_string(),
            project: "mission-control".to_string(),
            company: "indigo".to_string(),
            model: "claude-opus-4-8".to_string(),
            status: SessionStatus::Running,
            started_at: "2026-06-15T18:00:00Z".to_string(),
            last_activity_at: "2026-06-15T18:43:20Z".to_string(),
            source: "claude-jsonl".to_string(),
        }
    }

    #[test]
    fn status_serialises_to_taxonomy_strings() {
        // Must match the TS literal union exactly.
        assert_eq!(
            serde_json::to_string(&SessionStatus::Running).unwrap(),
            "\"running\""
        );
        assert_eq!(
            serde_json::to_string(&SessionStatus::AwaitingInput).unwrap(),
            "\"awaiting_input\""
        );
        assert_eq!(
            serde_json::to_string(&SessionStatus::Idle).unwrap(),
            "\"idle\""
        );
        assert_eq!(
            serde_json::to_string(&SessionStatus::Ended).unwrap(),
            "\"ended\""
        );
    }

    #[test]
    fn status_deserialises_from_taxonomy_strings() {
        assert_eq!(
            serde_json::from_str::<SessionStatus>("\"awaiting_input\"").unwrap(),
            SessionStatus::AwaitingInput
        );
        // An unknown status is rejected, not silently coerced.
        assert!(serde_json::from_str::<SessionStatus>("\"paused\"").is_err());
    }

    #[test]
    fn status_live_classification() {
        assert!(SessionStatus::Running.is_live());
        assert!(SessionStatus::AwaitingInput.is_live());
        assert!(!SessionStatus::Idle.is_live());
        assert!(!SessionStatus::Ended.is_live());
    }

    #[test]
    fn tool_and_origin_serialise_to_lowercase() {
        assert_eq!(serde_json::to_string(&AgentTool::Claude).unwrap(), "\"claude\"");
        assert_eq!(serde_json::to_string(&AgentTool::Codex).unwrap(), "\"codex\"");
        assert_eq!(serde_json::to_string(&AgentOrigin::Local).unwrap(), "\"local\"");
        assert_eq!(
            serde_json::to_string(&AgentOrigin::Outpost).unwrap(),
            "\"outpost\""
        );
    }

    #[test]
    fn agent_session_round_trips_through_json() {
        let original = sample();
        let json = serde_json::to_string(&original).unwrap();
        let back: AgentSession = serde_json::from_str(&json).unwrap();
        assert_eq!(original, back);
    }

    #[test]
    fn agent_session_serialises_camelcase_keys() {
        let value = serde_json::to_value(sample()).unwrap();
        let obj = value.as_object().unwrap();
        // camelCase keys present (matches the TS contract)…
        assert!(obj.contains_key("startedAt"));
        assert!(obj.contains_key("lastActivityAt"));
        // …and no snake_case leakage.
        assert!(!obj.contains_key("started_at"));
        assert!(!obj.contains_key("last_activity_at"));
        // Nested enums serialise as taxonomy strings, not structs.
        assert_eq!(obj.get("tool").unwrap(), "claude");
        assert_eq!(obj.get("origin").unwrap(), "local");
        assert_eq!(obj.get("status").unwrap(), "running");
    }

    // ── US-005: poll-interval resolution ────────────────────────────────────

    #[test]
    fn poll_interval_defaults_when_env_absent() {
        assert_eq!(
            resolve_poll_interval(None),
            Duration::from_secs(SESSIONS_POLL_INTERVAL_SECS)
        );
    }

    #[test]
    fn poll_interval_honours_a_valid_override() {
        assert_eq!(resolve_poll_interval(Some("15")), Duration::from_secs(15));
        // Whitespace is tolerated.
        assert_eq!(resolve_poll_interval(Some("  20 ")), Duration::from_secs(20));
    }

    #[test]
    fn poll_interval_clamps_to_the_floor_and_rejects_garbage() {
        // Below the floor → clamped up.
        assert_eq!(
            resolve_poll_interval(Some("1")),
            Duration::from_secs(SESSIONS_POLL_FLOOR_SECS)
        );
        // Zero / non-numeric / empty → default.
        for bad in ["0", "abc", "", "-5"] {
            assert_eq!(
                resolve_poll_interval(Some(bad)),
                Duration::from_secs(SESSIONS_POLL_INTERVAL_SECS),
                "{bad:?} should fall back to the default"
            );
        }
    }

    // ── US-005: merge + liveness ────────────────────────────────────────────

    fn session(id: &str, tool: AgentTool, last_activity_at: &str) -> AgentSession {
        AgentSession {
            id: id.to_string(),
            tool,
            origin: AgentOrigin::Local,
            cwd: "/tmp".to_string(),
            project: "p".to_string(),
            company: "indigo".to_string(),
            model: "m".to_string(),
            // Deliberately a *stale* coarse status so we can prove the merge
            // re-derives it rather than trusting the reader's value.
            status: SessionStatus::Ended,
            started_at: "2026-06-15T18:00:00Z".to_string(),
            last_activity_at: last_activity_at.to_string(),
            source: "test".to_string(),
        }
    }

    /// RFC-3339 string for `now - age_secs` (seconds precision, `Z` suffix).
    fn iso_ago(now: SystemTime, age_secs: u64) -> String {
        let secs = now
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0)
            - age_secs as i64;
        chrono::DateTime::<chrono::Utc>::from_timestamp(secs, 0)
            .unwrap()
            .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
    }

    #[test]
    fn merge_preserves_order_and_reapplies_liveness() {
        let now = SystemTime::now();
        let claude = vec![session("c1", AgentTool::Claude, &iso_ago(now, 10))];
        let codex = vec![session("x1", AgentTool::Codex, &iso_ago(now, 10))];
        let agents = liveness::RunningAgents { claude: true, codex: true };

        let merged = merge_sessions(claude, codex, agents, now);

        // Claude first, then Codex — readers' order preserved.
        assert_eq!(merged.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), ["c1", "x1"]);
        // Fresh activity + live process → re-derived to running (NOT the stale Ended).
        assert!(merged.iter().all(|s| s.status == SessionStatus::Running));
    }

    #[test]
    fn merge_marks_sessions_ended_when_no_live_process() {
        let now = SystemTime::now();
        // Fresh activity, but the owning tool has NO live process.
        let claude = vec![session("c1", AgentTool::Claude, &iso_ago(now, 5))];
        let agents = liveness::RunningAgents { claude: false, codex: false };

        let merged = merge_sessions(claude, Vec::new(), agents, now);

        assert_eq!(merged.len(), 1);
        // HARD rule: no live process → ended, regardless of freshness.
        assert_eq!(merged[0].status, SessionStatus::Ended);
    }

    // ── US-011: outpost merge into the same snapshot ────────────────────────

    fn outpost_session(id: &str) -> AgentSession {
        let mut s = session(id, AgentTool::Claude, "2026-06-15T18:43:20Z");
        // Wire says LOCAL — `append_outpost_sessions` must force it to outpost.
        s.origin = AgentOrigin::Local;
        s.source = "outpost-heartbeat".to_string();
        s
    }

    #[test]
    fn append_outpost_sessions_merges_into_one_fleet_with_outpost_origin() {
        let now = SystemTime::now();
        let local = vec![session("c1", AgentTool::Claude, &iso_ago(now, 5))];
        let outpost = vec![outpost_session("o1")];

        let merged = append_outpost_sessions(local, outpost);

        // Local first, then outpost — stable order.
        assert_eq!(merged.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), ["c1", "o1"]);
        // The local session stays local…
        assert_eq!(merged[0].origin, AgentOrigin::Local);
        // …and the outpost session is forced to origin=outpost (so the UI groups
        // it under the outpost group), even though the wire said local.
        assert_eq!(merged[1].origin, AgentOrigin::Outpost);
    }

    #[test]
    fn append_outpost_sessions_with_no_outpost_is_identity() {
        let now = SystemTime::now();
        let local = vec![session("c1", AgentTool::Claude, &iso_ago(now, 5))];
        let merged = append_outpost_sessions(local.clone(), Vec::new());
        assert_eq!(merged, local);
    }

    #[test]
    fn snapshot_omits_outpost_key_when_no_outpost() {
        // `outpost: None` must serialise to NO `outpost` key (skip_serializing_if)
        // so the existing frontend shape is unchanged when there's no box.
        let snapshot = MissionControlSnapshot {
            sessions: Vec::new(),
            history: Vec::new(),
            outpost: None,
        };
        let value = serde_json::to_value(&snapshot).unwrap();
        let obj = value.as_object().unwrap();
        assert!(obj.contains_key("sessions"));
        assert!(obj.contains_key("history"));
        assert!(!obj.contains_key("outpost"), "absent outpost is omitted from the wire");
    }

    #[test]
    fn snapshot_carries_outpost_card_when_present() {
        let snapshot = MissionControlSnapshot {
            sessions: Vec::new(),
            history: Vec::new(),
            outpost: Some(outpost::OutpostStatus {
                up: true,
                runtime: "claude".to_string(),
                relay_connected: true,
                ip: "203.0.113.7".to_string(),
                region: "us-east-1".to_string(),
                last_seen_at: "2026-06-15T18:43:20Z".to_string(),
                stale: false,
            }),
        };
        let value = serde_json::to_value(&snapshot).unwrap();
        let outpost = value.get("outpost").expect("outpost card present").as_object().unwrap();
        // camelCase keys on the wire, matching the TS card type.
        assert_eq!(outpost.get("up").unwrap(), true);
        assert_eq!(outpost.get("relayConnected").unwrap(), true);
        assert_eq!(outpost.get("lastSeenAt").unwrap(), "2026-06-15T18:43:20Z");
    }

    // ── US-005: command + event integration ─────────────────────────────────

    #[tokio::test]
    async fn list_agent_sessions_returns_a_snapshot_shape() {
        // The command never errors (best-effort readers) and returns the merged
        // snapshot shape. On a CI box with no Claude/Codex dirs this is empty,
        // which is a valid empty fleet — the shape is what we assert here.
        let snapshot = list_agent_sessions().await.unwrap();
        // Round-trips through camelCase JSON with the documented top-level keys.
        let value = serde_json::to_value(&snapshot).unwrap();
        let obj = value.as_object().unwrap();
        assert!(obj.contains_key("sessions"));
        assert!(obj.contains_key("history"));
    }

    #[tokio::test]
    async fn emit_snapshot_fires_the_typed_event() {
        use std::sync::{Arc, Mutex};
        use tauri::Listener;

        let app = tauri::test::mock_app();
        let handle = app.handle().clone();

        // Register a listener for the typed poll event BEFORE emitting, capturing
        // the payload so we assert both that the event fired and that it carries
        // the snapshot shape.
        let seen: Arc<Mutex<Option<MissionControlSnapshot>>> = Arc::new(Mutex::new(None));
        let seen_w = seen.clone();
        handle.listen(EVENT_SESSIONS_UPDATED, move |event| {
            let parsed: MissionControlSnapshot = serde_json::from_str(event.payload()).unwrap();
            *seen_w.lock().unwrap() = Some(parsed);
        });

        // Drive one poll cycle directly (the loop body), then let the listener run.
        emit_snapshot(&handle).await;
        tokio::time::sleep(Duration::from_millis(50)).await;

        let captured = seen.lock().unwrap();
        assert!(
            captured.is_some(),
            "expected an {EVENT_SESSIONS_UPDATED} event to be emitted"
        );
    }
}
