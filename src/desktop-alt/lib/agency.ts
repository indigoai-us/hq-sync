/**
 * Mission Control — hq-pack-agency teams + answerable questions (frontend half).
 *
 * Mirrors the Rust wire types in `src-tauri/src/commands/agency.rs`
 * (`#[serde(rename_all = "camelCase")]`), so payloads map 1:1. Pure data — no
 * runes — so it stays trivially testable.
 */

/** One `(worker, instance)` in a team. */
export interface AgencyWorker {
  worker: string;
  instance: string;
  /** From the team's status.json (`running` | `stopped` | …); `unknown` when absent. */
  status: string;
  /** True once the worker posted its `ready` handshake. */
  ready: boolean;
  /** ISO `started_at` from status.json — drives the "up 12m" uptime label; '' when absent. */
  startedAt: string;
  /** ISO `updated_at` from status.json — drives the "seen 30s ago" freshness label; '' when absent. */
  updatedAt: string;
}

/** One running agency team. */
export interface AgencyTeam {
  company: string;
  team: string;
  workers: AgencyWorker[];
}

/** A team-manager question routed to the liaison and not yet answered. */
export interface AgencyQuestion {
  company: string;
  team: string;
  /** Dedup id = POSIX cksum of the question (matches the liaison's [ans:<id>]). */
  id: string;
  question: string;
  ts: string;
  /** Bounded answer choices the manager attached to the ASK; empty for free-text. */
  options: string[];
}

/** Map a worker status to a status-dot tone (tokens.css `--v4-*`). */
export function statusTone(status: string, ready: boolean): 'ok' | 'warn' | 'idle' {
  if (status === 'running') return ready ? 'ok' : 'warn';
  if (status === 'crash-loop') return 'warn';
  return 'idle';
}

/**
 * Relative age of an ISO timestamp — "just now", "4m ago", "3h ago", "2d ago".
 * Empty string when `iso` is blank or unparseable. `nowMs` is injectable so the
 * helper is deterministic under test.
 */
export function relativeTime(iso: string, nowMs: number = Date.now()): string {
  if (!iso) return '';
  const t = Date.parse(iso);
  if (Number.isNaN(t)) return '';
  const sec = Math.max(0, Math.round((nowMs - t) / 1000));
  if (sec < 45) return 'just now';
  const min = Math.round(sec / 60);
  if (min < 60) return `${min}m ago`;
  const hr = Math.floor(min / 60);
  if (hr < 24) return `${hr}h ago`;
  return `${Math.floor(hr / 24)}d ago`;
}

/**
 * Compact elapsed duration with no "ago" suffix — "12m", "3h", "2d" — for an
 * uptime label. Empty string when `iso` is blank or unparseable.
 */
export function shortDuration(iso: string, nowMs: number = Date.now()): string {
  if (!iso) return '';
  const t = Date.parse(iso);
  if (Number.isNaN(t)) return '';
  const sec = Math.max(0, Math.round((nowMs - t) / 1000));
  if (sec < 60) return `${sec}s`;
  const min = Math.round(sec / 60);
  if (min < 60) return `${min}m`;
  const hr = Math.floor(min / 60);
  if (hr < 24) return `${hr}h`;
  return `${Math.floor(hr / 24)}d`;
}
