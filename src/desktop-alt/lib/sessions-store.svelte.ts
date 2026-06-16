import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import {
  SESSIONS_UPDATED_EVENT,
  type AgentSession,
  type HistoryEvent,
  type MissionControlSnapshot,
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
  sessions = snapshot.sessions ?? [];
  history = snapshot.history ?? [];
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
  get loading() {
    return loading;
  },
  get error() {
    return error;
  },
  refresh,
};
