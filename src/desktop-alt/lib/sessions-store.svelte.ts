import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import {
  SESSIONS_UPDATED_EVENT,
  type AgentSession,
  type HistoryEvent,
  type MissionControlSnapshot,
  type OutpostStatus,
} from './sessions';

// ---------------------------------------------------------------------------
// Mission Control sessions store (US-007).
//
// Module-level singleton runes state — the same shape as meetings-store: the
// fleet lives here, loaded ONCE via `list_agent_sessions` (US-005) and kept
// fresh by the backend `sessions:updated` poll event (US-005), NOT by a JS
// timer. The Rust polling loop re-scans on its interval and emits the same
// `MissionControlSnapshot` on every tick; we just listen and replace state, so
// a new session appears without any manual refresh (US-007 e2e).
//
// Consumers (LiveSessionsPanel, the page summary tiles, the History panel) read
// the reactive getters inside their own $derived/template, which subscribes them
// to this $state so every poll repaints every open view automatically — mirrors
// the sync-stats event pattern (design.md "Implementation notes").
// ---------------------------------------------------------------------------

let sessions = $state<AgentSession[]>([]);
let history = $state<HistoryEvent[]>([]);
// The box-level outpost status card (US-011), or null when no outpost is known.
// Replaced on every poll; the Live panel heads its outpost group with it.
let outpost = $state<OutpostStatus | null>(null);
// The number of outpost sessions showing in the *previous* snapshot. When the
// box goes stale the backend drops them (sessions empty), so this lets the box
// card surface "N sessions dropped after the stale timeout" (design.md down
// state) — we remember how many we just lost.
let lastOutpostCount = $state(0);
// `true` until the very first snapshot lands — drives the loading skeleton.
let loading = $state(true);
// Set when the initial invoke fails; the panel can surface it instead of a
// misleading empty state. Poll-event failures are best-effort (logged only).
let error = $state('');

// Lifecycle guards — the store outlives any single page mount.
let started = false;
let unlisten: UnlistenFn | null = null;

/** Apply a fresh snapshot to the reactive state. */
function applySnapshot(snapshot: MissionControlSnapshot): void {
  const next = snapshot.sessions ?? [];
  // Track how many outpost sessions were showing BEFORE this snapshot, so when
  // the stale timeout drops them to zero the box card can report the count that
  // just vanished. Computed off the prior `sessions` (current state).
  const priorOutpostCount = sessions.filter((s) => s.origin === 'outpost').length;
  const nextOutpostCount = next.filter((s) => s.origin === 'outpost').length;
  // Remember the last NON-zero count so a freshly-stale snapshot (now zero)
  // still knows how many were dropped.
  lastOutpostCount = nextOutpostCount > 0 ? nextOutpostCount : priorOutpostCount;

  sessions = next;
  history = snapshot.history ?? [];
  outpost = snapshot.outpost ?? null;
  loading = false;
}

/**
 * Run one immediate scan via the command so the panel has data before the first
 * poll tick fires. Errors are surfaced (not swallowed to a blank state) but
 * never thrown — the poll listener may still deliver a snapshot afterwards.
 */
async function refresh(): Promise<void> {
  try {
    const snapshot = await invoke<MissionControlSnapshot>('list_agent_sessions');
    applySnapshot(snapshot);
    error = '';
  } catch (err) {
    console.error('list_agent_sessions failed:', err);
    error = 'Could not load sessions.';
    loading = false;
  }
}

/**
 * Start the singleton once for the app's lifetime. Subscribes to the backend
 * `sessions:updated` poll event FIRST (so no tick is missed), then does one
 * immediate `list_agent_sessions` for instant paint. Idempotent via `started`.
 * Called from MissionControlPage.onMount so the page works in isolation.
 */
export function startSessionsStore(): void {
  if (started) return;
  started = true;

  void listen<MissionControlSnapshot>(SESSIONS_UPDATED_EVENT, (event) => {
    applySnapshot(event.payload);
  }).then((fn) => {
    unlisten = fn;
  });

  void refresh();
}

/**
 * Tear down the listener. Not used in the running app (the store lives for the
 * whole session) but exported so tests can reset between runs.
 */
export function stopSessionsStore(): void {
  if (unlisten) {
    unlisten();
    unlisten = null;
  }
  started = false;
  sessions = [];
  history = [];
  outpost = null;
  lastOutpostCount = 0;
  loading = true;
  error = '';
}

/** Reactive read surface — getters keep consumers subscribed to the $state. */
export const sessionsStore = {
  get sessions() {
    return sessions;
  },
  get history() {
    return history;
  },
  /** The box-level outpost status card (US-011), or null when no outpost known. */
  get outpost() {
    return outpost;
  },
  /** How many outpost sessions were showing before they were last dropped — feeds
   *  the box card's stale-timeout "N sessions dropped" note. */
  get lastOutpostCount() {
    return lastOutpostCount;
  },
  get loading() {
    return loading;
  },
  get error() {
    return error;
  },
  refresh,
};
