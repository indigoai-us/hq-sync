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
}

/** Map a worker status to a status-dot tone (tokens.css `--v4-*`). */
export function statusTone(status: string, ready: boolean): 'ok' | 'warn' | 'idle' {
  if (status === 'running') return ready ? 'ok' : 'warn';
  if (status === 'crash-loop') return 'warn';
  return 'idle';
}
