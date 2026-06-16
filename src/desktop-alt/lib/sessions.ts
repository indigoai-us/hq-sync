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

// ───────────────────────────────────────────────────────────────────────────
// Mission Control snapshot (US-005 wire shape)
// ───────────────────────────────────────────────────────────────────────────

/**
 * The kind of a history event — mirrors the Rust `HistoryEventKind` enum
 * (snake_case on the wire). Drives the timeline node color in the History
 * panel (US-008); declared here so the snapshot type is complete on the TS side.
 */
export type HistoryEventKind =
  | 'dispatched'
  | 'completed'
  | 'failed'
  | 'checkpoint'
  | 'handoff';

/**
 * One entry in the Mission Control history feed (US-008). Mirrors the Rust
 * `HistoryEvent` struct field-for-field (camelCase wire shape).
 */
export interface HistoryEvent {
  kind: HistoryEventKind;
  title: string;
  company: string;
  project: string;
  /** ISO-8601 timestamp the event occurred. */
  timestamp: string;
  source: string;
}

/**
 * The box-level outpost status card (US-011). Mirrors the Rust
 * `outpost::OutpostStatus` struct field-for-field (camelCase wire shape). Heads
 * the outpost group in the Live panel; sourced from `GET /outpost/status` and
 * aged by the heartbeat stale-after timeout (so `stale`/`lastSeenAt` reflect
 * actual per-session reporting, not just the control-plane state).
 */
export interface OutpostStatus {
  /** Whether the box reads as up (green card) vs down (red card). */
  up: boolean;
  /** The agent runtime the box runs (`claude` / `codex`) — the RUNTIME stat. */
  runtime: string;
  /** Relay connected (green "connected") vs disconnected (red) — the RELAY stat. */
  relayConnected: boolean;
  /** Static IP, when known (the `ip · region` meta line). */
  ip: string;
  /** Region, when known. */
  region: string;
  /** ISO-8601 of the most recent heartbeat (the LAST SEEN stat); '' if never. */
  lastSeenAt: string;
  /**
   * `true` once the heartbeat has gone stale past the 90s timeout — the card
   * renders its down/last-seen treatment + the stale-sessions note even if
   * `/outpost/status` still reports the box exists.
   */
  stale: boolean;
}

/**
 * The full Mission Control payload (US-005/US-011): the merged fleet (local +
 * outpost), the history feed, and the box-level outpost status. This is BOTH the
 * `list_agent_sessions` command return value AND the `sessions:updated`
 * poll-event payload — the Rust `MissionControlSnapshot` serialises exactly this
 * shape (`{ sessions, history, outpost? }`), so the store reads it without
 * remapping. `outpost` is omitted by the backend when no outpost is known.
 */
export interface MissionControlSnapshot {
  sessions: AgentSession[];
  history: HistoryEvent[];
  outpost?: OutpostStatus | null;
}

/** The Tauri event name the polling loop emits on each re-scan (US-005). Kept
 *  in lock-step with the Rust `EVENT_SESSIONS_UPDATED` constant. */
export const SESSIONS_UPDATED_EVENT = 'sessions:updated';

// ───────────────────────────────────────────────────────────────────────────
// Best-effort "kind" derivation (US-007)
// ───────────────────────────────────────────────────────────────────────────
//
// The AgentSession contract (US-001) deliberately has NO `kind`/`type` field —
// the readers observe on-disk artifacts and can't know a session's intent. But
// a power user runs dozens of near-identical monitor bots (per-channel Slack
// watchers, PR babysitters, deploy/CI monitors, signup heartbeats), so the Live
// panel groups by an inferred *kind* to keep ≈40 sessions legible (design.md
// "built for fleet scale").
//
// This is a pure, best-effort heuristic over the session's project / cwd /
// source strings. It is intentionally isolated here (not in the panel) so it is
// trivially unit-testable and easy to refine later as we learn real-world
// naming patterns. When nothing matches, callers fall back to a project/origin
// grouping (see `groupKeyFor` / `groupSessions`) — a session is NEVER dropped.

/**
 * The best-effort session "kind" clusters the Live panel groups by. `interactive`
 * is the catch-all for a human-driven session that matches no monitor pattern;
 * `unknown` is reserved for the rare case where even the project is empty (the
 * fallback grouping then keys on origin+tool instead).
 *
 * Order here is also the *display priority* used to sort groups (monitors that
 * tend to swarm sink below the more-relevant interactive/discover work — design
 * says "most-relevant group first").
 */
export const SESSION_KINDS = [
  'interactive',
  'discover',
  'slack-watcher',
  'pr',
  'ci',
  'deploy',
  'signup-heartbeat',
  'unknown',
] as const;

/** An inferred session kind — one of {@link SESSION_KINDS}. */
export type SessionKind = (typeof SESSION_KINDS)[number];

/** Human-readable group title for each kind (the group header label). */
export const SESSION_KIND_LABELS: Record<SessionKind, string> = {
  'slack-watcher': 'Slack mention watchers',
  pr: 'PR babysitters',
  deploy: 'Deploy monitors',
  ci: 'CI watchers',
  'signup-heartbeat': 'Signup heartbeat',
  discover: 'Discover / ingest',
  interactive: 'Interactive sessions',
  unknown: 'Other sessions',
};

/**
 * Each kind's heuristic, evaluated in array order — the FIRST match wins, so the
 * specific monitor patterns are checked before the `interactive` catch-all. Each
 * matcher tests a lowercased "haystack" built from the session's project, cwd,
 * and source. Kept as data (not a switch) so the rules are easy to read, reorder,
 * and extend.
 */
const KIND_MATCHERS: ReadonlyArray<{ kind: SessionKind; test: RegExp }> = [
  // Per-channel Slack mention watchers (run-bot watchers, slack-watcher skill).
  { kind: 'slack-watcher', test: /slack|run-bot|mention-watch|watcher/ },
  // PR babysitters — land / babysit / review-pr loops.
  { kind: 'pr', test: /\bpr\b|pull-request|babysit|land-batch|land-pr|review-pr/ },
  // CI watchers — distinct from deploy; checked before deploy so "ci" wins.
  { kind: 'ci', test: /\bci\b|workflow-run|gh-actions|github-actions|build-watch/ },
  // Deploy / release monitors.
  { kind: 'deploy', test: /deploy|release-monitor|ship-monitor|rollout/ },
  // Signup heartbeat / health pings.
  { kind: 'signup-heartbeat', test: /signup-heartbeat|heartbeat|signup-watch|health-?ping/ },
  // Repo discovery / ingest sessions.
  { kind: 'discover', test: /discover|ingest|index-repo|crawl/ },
];

/**
 * Build the lowercased match haystack for a session. Pulls from `project`, the
 * basename + full `cwd`, and `source` — the strings most likely to carry the
 * monitor/skill name. Exported for the unit tests, not the UI.
 */
export function kindHaystack(session: AgentSession): string {
  const cwdTail = session.cwd.split('/').filter(Boolean).slice(-2).join('/');
  return [session.project, cwdTail, session.cwd, session.source]
    .filter(Boolean)
    .join(' ')
    .toLowerCase();
}

/**
 * Infer a session's best-effort {@link SessionKind} (US-007).
 *
 * Returns the first {@link KIND_MATCHERS} pattern that hits the session's
 * haystack; falls back to `interactive` for a session that names a project but
 * matches no monitor pattern (a human-driven Claude/Codex session), and to
 * `unknown` only when there's nothing to key on at all (empty project + cwd).
 * Pure and deterministic — never throws.
 */
export function deriveSessionKind(session: AgentSession): SessionKind {
  const hay = kindHaystack(session);
  if (hay.trim() === '') return 'unknown';
  for (const { kind, test } of KIND_MATCHERS) {
    if (test.test(hay)) return kind;
  }
  return 'interactive';
}

// ───────────────────────────────────────────────────────────────────────────
// Grouping (US-007)
// ───────────────────────────────────────────────────────────────────────────

/**
 * Stable grouping key for a session. The primary axis is the inferred kind; for
 * the `unknown` fallback (no kind could be inferred) we degrade gracefully to a
 * `project`-then-`origin:tool` key so those sessions still cluster sensibly
 * instead of piling into one opaque bucket. Exported for the unit tests.
 */
export function groupKeyFor(session: AgentSession): string {
  const kind = deriveSessionKind(session);
  if (kind !== 'unknown') return `kind:${kind}`;
  const project = session.project.trim();
  if (project) return `project:${project}`;
  return `origin:${session.origin}:${session.tool}`;
}

/** Per-status counts for a group's status pips. */
export type StatusCounts = Record<SessionStatus, number>;

/** A resolved group of sessions for the Live panel (one collapsible cluster). */
export interface SessionGroup {
  /** Stable {@link groupKeyFor} key — used as the `{#each}` key + caret toggle id. */
  key: string;
  /** The inferred kind for this group (drives the header label + display order). */
  kind: SessionKind;
  /** Human-readable header title. */
  label: string;
  /** The group's sessions, freshest-first. */
  sessions: AgentSession[];
  /** Total session count (= sessions.length; surfaced as the header count chip). */
  count: number;
  /** Per-status counts, for the header status pips (zero-count statuses omitted by the UI). */
  statusCounts: StatusCounts;
  /** ISO timestamp of the freshest `lastActivityAt` in the group (header right-rail). */
  freshestActivityAt: string;
}

/** Display-order rank for a kind (lower = earlier). Unlisted → end. */
function kindRank(kind: SessionKind): number {
  const idx = SESSION_KINDS.indexOf(kind);
  return idx === -1 ? SESSION_KINDS.length : idx;
}

/** Epoch millis for an ISO/empty timestamp; non-parseable or empty → 0. */
function activityMillis(iso: string): number {
  if (!iso) return 0;
  const t = new Date(iso).getTime();
  return Number.isFinite(t) ? t : 0;
}

function emptyStatusCounts(): StatusCounts {
  return { running: 0, awaiting_input: 0, idle: 0, ended: 0 };
}

/**
 * Group a flat `AgentSession[]` into the Live panel's collapsible clusters
 * (US-007).
 *
 * - Keys each session via {@link groupKeyFor} (inferred kind, with the
 *   project/origin fallback for `unknown`), so a session is never dropped.
 * - Within a group, sessions are ordered freshest-`lastActivityAt`-first.
 * - Groups are ordered by: most live sessions (running + awaiting_input) first,
 *   then by kind display-rank, then by freshest activity — so the operator's
 *   most-relevant cluster leads (design.md "biggest / most-relevant first").
 *
 * Pure over its input; deterministic given equal timestamps (ties fall back to
 * the stable key). No filtering happens here — pass the already-active set in.
 */
export function groupSessions(sessions: AgentSession[]): SessionGroup[] {
  const byKey = new Map<string, SessionGroup>();

  for (const session of sessions) {
    const key = groupKeyFor(session);
    let group = byKey.get(key);
    if (!group) {
      const kind = deriveSessionKind(session);
      group = {
        key,
        kind,
        label: SESSION_KIND_LABELS[kind],
        sessions: [],
        count: 0,
        statusCounts: emptyStatusCounts(),
        freshestActivityAt: '',
      };
      byKey.set(key, group);
    }
    group.sessions.push(session);
    group.statusCounts[session.status] += 1;
    if (activityMillis(session.lastActivityAt) > activityMillis(group.freshestActivityAt)) {
      group.freshestActivityAt = session.lastActivityAt;
    }
  }

  const groups = [...byKey.values()];
  for (const group of groups) {
    group.count = group.sessions.length;
    group.sessions.sort(
      (a, b) => activityMillis(b.lastActivityAt) - activityMillis(a.lastActivityAt),
    );
  }

  groups.sort((a, b) => {
    const liveA = a.statusCounts.running + a.statusCounts.awaiting_input;
    const liveB = b.statusCounts.running + b.statusCounts.awaiting_input;
    if (liveB !== liveA) return liveB - liveA;
    const rankDelta = kindRank(a.kind) - kindRank(b.kind);
    if (rankDelta !== 0) return rankDelta;
    const freshDelta = activityMillis(b.freshestActivityAt) - activityMillis(a.freshestActivityAt);
    if (freshDelta !== 0) return freshDelta;
    return a.key.localeCompare(b.key);
  });

  return groups;
}

/**
 * Whether a session is "active" for the Live panel. The panel shows the live
 * fleet, so we render `running` / `awaiting_input` / `idle` (recently-quiet) and
 * drop `ended` — except an `ended` session is kept when it ended very recently so
 * the operator can see something just finished (design.md "recently-ended only").
 * Centralised so the panel and any summary agree.
 *
 * `now` is injected for deterministic tests; defaults to wall-clock.
 */
export const RECENTLY_ENDED_WINDOW_MS = 2 * 60 * 1000;

export function isActiveForLivePanel(
  session: AgentSession,
  now: number = Date.now(),
): boolean {
  if (session.status !== 'ended') return true;
  const last = activityMillis(session.lastActivityAt);
  return last > 0 && now - last <= RECENTLY_ENDED_WINDOW_MS;
}

// ───────────────────────────────────────────────────────────────────────────
// Compact relative time (US-007 dense rows)
// ───────────────────────────────────────────────────────────────────────────

// ───────────────────────────────────────────────────────────────────────────
// Outpost grouping + box-card presentation (US-011)
// ───────────────────────────────────────────────────────────────────────────
//
// design.md "Outpost status card (US-011)": outpost sessions render under the
// SAME grouped model as local ones (origin badged), headed by a box-level status
// card sourced from `GET /outpost/status`. The card is green-tinted when up and a
// red card (relay disconnected + last-seen + the stale-sessions note) when down.
//
// The presentation decisions (up→green / down→red, relay text, the dropped-count
// note) are pure functions here, NOT inline in the panel, so they are
// unit-testable in the node test env (the panel itself is not component-mounted
// under vitest) — exactly the pattern `deriveSessionKind` / `eventNodeTone` use.

/** Partition a fleet into its local and outpost halves, preserving input order. */
export function partitionByOrigin(sessions: AgentSession[]): {
  local: AgentSession[];
  outpost: AgentSession[];
} {
  const local: AgentSession[] = [];
  const outpost: AgentSession[] = [];
  for (const s of sessions) {
    if (s.origin === 'outpost') outpost.push(s);
    else local.push(s);
  }
  return { local, outpost };
}

/** The visual tone for the outpost box card — `ok` (green, up) or `down` (red). */
export type OutpostCardTone = 'ok' | 'down';

/**
 * A fully-resolved, presentation-ready view of the outpost box card (US-011),
 * derived purely from the {@link OutpostStatus} + the count of outpost sessions
 * currently shown. The panel renders straight from this — no branching logic in
 * markup — so every card-state decision is pinned here and unit-tested.
 */
export interface OutpostCardView {
  /** `ok` → green-tinted card; `down` → red card. Drives the hairline tint. */
  tone: OutpostCardTone;
  /** State pill text — `UP` / `DOWN`. */
  stateLabel: 'UP' | 'DOWN';
  /** Runtime stat value (uppercased for the RUNTIME label), or `—` when unknown. */
  runtimeLabel: string;
  /** Relay stat value — `connected` (green) / `disconnected` (red). */
  relayLabel: 'connected' | 'disconnected';
  /** Whether the relay reads connected (drives the relay value color). */
  relayConnected: boolean;
  /** `ip · region` meta, or `—` when neither is known. */
  metaLabel: string;
  /**
   * The amber stale-timeout note shown ONLY in the down/stale state when outpost
   * sessions were dropped — `null` when there's nothing to note. design.md:
   * "N outpost sessions dropped after the 90s stale timeout — they reappear when
   * the box reports in."
   */
  staleNote: string | null;
}

/** The stale-timeout window the box card's note references (matches the Rust
 *  `HEARTBEAT_STALE_AFTER` = 90s). Display-only. */
export const OUTPOST_STALE_AFTER_SECONDS = 90;

/**
 * Resolve the outpost box card view (US-011) from the status + the number of
 * outpost sessions just dropped by the stale timeout (i.e. how many were showing
 * before the box went stale). Pure + deterministic; the single source of truth
 * for the card's up/down treatment.
 *
 * - `up && !stale` → green `ok` card, no note.
 * - otherwise → red `down` card; when `droppedCount > 0` it carries the
 *   stale-sessions note (design.md down state).
 */
export function resolveOutpostCard(
  status: OutpostStatus,
  droppedCount: number = 0,
): OutpostCardView {
  const isUp = status.up && !status.stale;
  const tone: OutpostCardTone = isUp ? 'ok' : 'down';
  const metaParts = [status.ip, status.region].filter((p) => p && p.trim());
  const staleNote =
    !isUp && droppedCount > 0
      ? `${droppedCount} outpost ${droppedCount === 1 ? 'session' : 'sessions'} dropped after the ${OUTPOST_STALE_AFTER_SECONDS}s stale timeout — ${droppedCount === 1 ? 'it reappears' : 'they reappear'} when the box reports in.`
      : null;

  return {
    tone,
    stateLabel: isUp ? 'UP' : 'DOWN',
    runtimeLabel: status.runtime ? status.runtime.toUpperCase() : '—',
    relayLabel: status.relayConnected ? 'connected' : 'disconnected',
    relayConnected: status.relayConnected,
    metaLabel: metaParts.length ? metaParts.join(' · ') : '—',
    staleNote,
  };
}

/**
 * Compact mono relative-activity label for a dense row / group header
 * (e.g. `now`, `5s`, `3m`, `2h`, `4d`). Distinct from `sync-model.timeAgo`,
 * which renders the longer "Xm ago" form for the sync surfaces — the dense rows
 * want the tightest possible token. `now` is injected for deterministic tests.
 */
export function relativeActivity(iso: string, now: number = Date.now()): string {
  const then = activityMillis(iso);
  if (then <= 0) return '—';
  const seconds = Math.max(0, Math.floor((now - then) / 1000));
  if (seconds < 5) return 'now';
  if (seconds < 60) return `${seconds}s`;
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes}m`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours}h`;
  const days = Math.floor(hours / 24);
  return `${days}d`;
}

// ───────────────────────────────────────────────────────────────────────────
// History timeline derivation (US-008)
// ───────────────────────────────────────────────────────────────────────────
//
// The History panel renders the `history` half of the snapshot (US-005) as a
// chronological timeline. The Rust `HistoryEvent` contract (US-001/US-004) is
// fixed and carries no `tool` field — the derivation reads orchestration
// artifacts (audit log + thread files) that don't always name the owning agent.
// But design.md "History timeline panel (US-008)" requires a tool filter
// (All · Claude · Codex), so — exactly like `deriveSessionKind` for the live
// panel — we infer the tool best-effort from the strings the event DOES carry
// (its `source` / `title` / `project`). This keeps the wire contract untouched
// and the inference pure + unit-testable here, not buried in the panel.

/**
 * The tool-filter selection for the History panel's segmented control. `all`
 * is the unfiltered default; `claude` / `codex` narrow to one agent (matching
 * {@link AgentTool}). Kept as its own type so the filter UI and the pure filter
 * helper share one vocabulary.
 */
export type HistoryToolFilter = 'all' | AgentTool;

/** The segmented tool-filter options, in display order (design.md "Filters row"). */
export const HISTORY_TOOL_FILTERS: ReadonlyArray<{ value: HistoryToolFilter; label: string }> = [
  { value: 'all', label: 'All' },
  { value: 'claude', label: 'Claude' },
  { value: 'codex', label: 'Codex' },
];

/**
 * Each tool's heuristic, evaluated in array order — the FIRST match wins. Tests a
 * lowercased haystack built from the event's `source`, `title`, and `project`.
 * Codex is checked before Claude so a `codex-rollout` source wins even if the
 * title also mentions "claude code". Kept as data (not a switch) so the rules are
 * easy to read and extend.
 */
const EVENT_TOOL_MATCHERS: ReadonlyArray<{ tool: AgentTool; test: RegExp }> = [
  { tool: 'codex', test: /\bcodex\b|codex-rollout|rollout/ },
  { tool: 'claude', test: /\bclaude\b|claude-jsonl|claude-code/ },
];

/** Build the lowercased match haystack for an event's tool inference. */
function eventToolHaystack(event: HistoryEvent): string {
  return [event.source, event.title, event.project]
    .filter(Boolean)
    .join(' ')
    .toLowerCase();
}

/**
 * Best-effort infer the agent tool that produced a history event (US-008).
 *
 * Returns `claude` / `codex` when the event's strings name one, else `null` when
 * the tool is indeterminate (the common case for audit-log rows, which don't name
 * the agent). A `null`-tool event is shown under the `all` filter but hidden by an
 * explicit `claude`/`codex` filter — we never guess a tool we can't see. Pure and
 * deterministic; never throws.
 */
export function deriveEventTool(event: HistoryEvent): AgentTool | null {
  const hay = eventToolHaystack(event);
  if (hay.trim() === '') return null;
  for (const { tool, test } of EVENT_TOOL_MATCHERS) {
    if (test.test(hay)) return tool;
  }
  return null;
}

/** The active filter selection the History panel passes to {@link filterHistory}. */
export interface HistoryFilter {
  /** Tool segment — `all` keeps every event; `claude`/`codex` narrow by inferred tool. */
  tool: HistoryToolFilter;
  /** Company slug to narrow to, or `''` for "All companies". */
  company: string;
}

/** Epoch millis for an event timestamp; empty/unparseable sorts last (→ -Infinity). */
function eventMillis(iso: string): number {
  if (!iso) return Number.NEGATIVE_INFINITY;
  const t = new Date(iso).getTime();
  return Number.isFinite(t) ? t : Number.NEGATIVE_INFINITY;
}

/**
 * Sort a copy of the events newest-first by timestamp (US-008 "Newest first").
 *
 * The backend already returns newest-first, but the panel re-sorts defensively so
 * the ordering is pinned by this module (and unit-tested) regardless of source
 * order. Stable for equal timestamps. Pure — does not mutate the input.
 */
export function sortHistoryNewestFirst(events: HistoryEvent[]): HistoryEvent[] {
  return [...events].sort((a, b) => eventMillis(b.timestamp) - eventMillis(a.timestamp));
}

/**
 * Filter the history feed by tool + company (US-008), newest-first.
 *
 * - `tool: 'all'` keeps every event; `claude`/`codex` keep only events whose
 *   {@link deriveEventTool} matches (indeterminate-tool events drop under an
 *   explicit tool filter).
 * - `company: ''` keeps every company; otherwise only exact-slug matches remain.
 *
 * Result is always sorted newest-first via {@link sortHistoryNewestFirst}. Pure;
 * never mutates the input.
 */
export function filterHistory(events: HistoryEvent[], filter: HistoryFilter): HistoryEvent[] {
  const matched = events.filter((event) => {
    if (filter.tool !== 'all' && deriveEventTool(event) !== filter.tool) return false;
    if (filter.company && event.company !== filter.company) return false;
    return true;
  });
  return sortHistoryNewestFirst(matched);
}

/**
 * Distinct, non-empty company slugs present in the feed, alphabetically sorted —
 * the option set for the panel's company dropdown. Pure.
 */
export function historyCompanies(events: HistoryEvent[]): string[] {
  const seen = new Set<string>();
  for (const event of events) {
    if (event.company) seen.add(event.company);
  }
  return [...seen].sort((a, b) => a.localeCompare(b));
}

/**
 * The timeline node tone for an event kind (design.md "Node color by event
 * kind"): completed = green (`ok`); dispatched / handoff = neutral; checkpoint =
 * faint; failed = error. Returns a stable CSS-class suffix the panel maps to a
 * `--v4-*` color, so the color decision lives here (testable) not in markup.
 */
export type HistoryNodeTone = 'ok' | 'neutral' | 'faint' | 'error';

export function eventNodeTone(kind: HistoryEventKind): HistoryNodeTone {
  switch (kind) {
    case 'completed':
      return 'ok';
    case 'failed':
      return 'error';
    case 'checkpoint':
      return 'faint';
    case 'dispatched':
    case 'handoff':
    default:
      return 'neutral';
  }
}

/** Max events the panel renders before the "+N more" roll-up (design.md "Paginates/limits"). */
export const HISTORY_PAGE_SIZE = 30;
