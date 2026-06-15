/**
 * Mission Control — the shared `AgentSession` contract (US-001).
 *
 * This module is the TypeScript half of a cross-language contract. The Rust
 * half lives in `src-tauri/src/commands/sessions.rs` and serialises the same
 * shape with `#[serde(rename_all = "camelCase")]`, so the wire payloads map
 * 1:1 onto these types — the local readers, the outpost heartbeat, and the UI
 * all speak this single shape.
 *
 * No Svelte runes here — just data and pure helpers, so the contract stays
 * trivially unit-testable under vitest. The matching Rust round-trip test lives
 * beside the struct definition so both sides are pinned to the same taxonomy.
 */

/**
 * Canonical session status taxonomy (US-001).
 *
 * This is the ONE place the status values are spelled out on the TS side;
 * readers and UI both import from here rather than re-declaring the strings.
 * Keep this in lock-step with the Rust `SessionStatus` enum in `sessions.rs`.
 *
 * - `running`        — the agent is actively working (live process + fresh activity).
 * - `awaiting_input` — alive but blocked on the human (e.g. a prompt/approval).
 * - `idle`           — recently active but quiet now; no fresh activity.
 * - `ended`          — the session is over (no live process, or long-stale).
 *
 * Liveness is best-effort (observed from on-disk artifacts + process checks),
 * and the UI labels it as such.
 */
export const SESSION_STATUSES = ['running', 'awaiting_input', 'idle', 'ended'] as const;

/** A session's lifecycle status — one of {@link SESSION_STATUSES}. */
export type SessionStatus = (typeof SESSION_STATUSES)[number];

/** The agent tool that owns the session. */
export type AgentTool = 'claude' | 'codex';

/** Where the session is observed: this machine (`local`) or the user's outpost VM. */
export type AgentOrigin = 'local' | 'outpost';

/**
 * The unified agent-session record (US-001 data model).
 *
 * One shape for every session Mission Control knows about, regardless of tool
 * (Claude Code / Codex) or origin (local filesystem / outpost heartbeat). The
 * field order and names mirror the Rust `AgentSession` struct exactly.
 */
export interface AgentSession {
  /** Stable session id (e.g. the Claude transcript uuid or Codex rollout id). */
  id: string;
  /** Which agent tool owns the session. */
  tool: AgentTool;
  /** Where the session is observed (local machine vs. outpost VM). */
  origin: AgentOrigin;
  /** Working directory the session is running in. */
  cwd: string;
  /** Project the session is working on (derived from cwd / HQ metadata). */
  project: string;
  /** Owning company slug, when resolvable; empty string when unknown. */
  company: string;
  /** Model the session is using (e.g. `claude-opus-4-8`), when known. */
  model: string;
  /** Best-effort lifecycle status — see {@link SESSION_STATUSES}. */
  status: SessionStatus;
  /** ISO-8601 timestamp the session started, when known. */
  startedAt: string;
  /** ISO-8601 timestamp of the most recent observed activity. */
  lastActivityAt: string;
  /**
   * Where this record was sourced from — a short provenance tag (e.g.
   * `claude-jsonl`, `codex-rollout`, `outpost-heartbeat`). Lets the UI label
   * the observation channel and aids debugging.
   */
  source: string;
}

/** Type guard: is `value` a member of the status taxonomy? */
export function isSessionStatus(value: unknown): value is SessionStatus {
  return typeof value === 'string' && (SESSION_STATUSES as readonly string[]).includes(value);
}

/**
 * Whether a status counts as "live" for summary/badge purposes — `running` and
 * `awaiting_input` are live; `idle` and `ended` are not. Centralised so the
 * summary strip and live panel agree on what "active" means.
 */
export function isLiveStatus(status: SessionStatus): boolean {
  return status === 'running' || status === 'awaiting_input';
}
