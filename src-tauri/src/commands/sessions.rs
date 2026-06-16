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

use serde::{Deserialize, Serialize};

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
}
