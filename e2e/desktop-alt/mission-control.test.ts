// @vitest-environment happy-dom
//
// US-012 — Mission Control E2E (the truth signal, HQ principle 5).
//
// This is a REAL component-mount E2E: it boots the actual
// `MissionControlPage.svelte` (which in turn mounts LiveSessionsPanel +
// SessionHistoryPanel and starts the shared sessions store) into a live DOM and
// asserts on the rendered markup — no source-contract stub, no bypass of the
// component. It exercises the two behaviours the story promises:
//
//   (a) Given a session FIXTURE, when Mission Control opens, the page lists the
//       live LOCAL sessions (rendered rows + the summary-strip counts).
//   (b) When a poll tick fires (the backend `sessions:updated` event), a newly
//       observed session appears WITHOUT any manual refresh — the store replaces
//       state from the event payload and the live DOM repaints.
//
// To drive the component without a running Tauri backend we mock the two Tauri
// bridge modules the store depends on (`@tauri-apps/api/core` `invoke` and
// `@tauri-apps/api/event` `listen`): `invoke('list_agent_sessions')` returns the
// initial fixture, and the captured `sessions:updated` listener lets the test
// emit the second snapshot exactly as the Rust polling loop would. Everything
// downstream of the bridge — the store, the page, the panels, the grouping, the
// summary tiles — is the real product code.

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { flushSync, mount, unmount } from 'svelte';
import type {
  AgentSession,
  MissionControlSnapshot,
} from '../../src/desktop-alt/lib/sessions';
import { SESSIONS_UPDATED_EVENT } from '../../src/desktop-alt/lib/sessions';

// ── Tauri bridge mocks ──────────────────────────────────────────────────────
//
// The sessions store (src/desktop-alt/lib/sessions-store.svelte.ts) calls
// `invoke('list_agent_sessions')` once for the instant paint and subscribes to
// `listen(SESSIONS_UPDATED_EVENT, …)` for every subsequent poll tick. We capture
// both so the test fully controls what the page sees and can fire a poll tick on
// demand. These are module mocks (hoisted by vitest), wired to mutable refs the
// per-test setup populates.

let nextSnapshot: MissionControlSnapshot = { sessions: [], history: [] };
let updatedHandler: ((event: { payload: MissionControlSnapshot }) => void) | null = null;

vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(async (command: string) => {
    if (command === 'list_agent_sessions') return nextSnapshot;
    return undefined;
  }),
}));

vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(
    async (event: string, handler: (e: { payload: MissionControlSnapshot }) => void) => {
      if (event === SESSIONS_UPDATED_EVENT) updatedHandler = handler;
      // The real `listen` resolves to an unlisten fn; the store stores it.
      return () => {
        updatedHandler = null;
      };
    },
  ),
}));

// ── Fixtures ────────────────────────────────────────────────────────────────

const NOW = Date.parse('2026-06-15T18:00:00.000Z');

function session(overrides: Partial<AgentSession>): AgentSession {
  return {
    id: 'sess-default',
    tool: 'claude',
    origin: 'local',
    cwd: '/Users/dev/HQ/repos/public/hq-sync',
    project: 'hq-sync',
    company: 'indigo',
    model: 'claude-opus-4-8',
    status: 'running',
    startedAt: new Date(NOW - 60_000).toISOString(),
    lastActivityAt: new Date(NOW - 5_000).toISOString(),
    source: 'claude-jsonl',
    ...overrides,
  };
}

/** Two live LOCAL sessions — what the page should list when it first opens. */
const INITIAL_SNAPSHOT: MissionControlSnapshot = {
  sessions: [
    session({
      id: 'sess-mission-control',
      project: 'mission-control',
      company: 'indigo',
      status: 'running',
      lastActivityAt: new Date(NOW - 2_000).toISOString(),
    }),
    session({
      id: 'sess-indigo-docs',
      project: 'indigo-docs',
      company: 'indigo',
      tool: 'codex',
      model: 'gpt-5-codex',
      status: 'awaiting_input',
      source: 'codex-rollout',
      lastActivityAt: new Date(NOW - 9_000).toISOString(),
    }),
  ],
  history: [],
};

/** A later poll snapshot that ADDS a third local session (a discover run). */
const POLLED_SNAPSHOT: MissionControlSnapshot = {
  sessions: [
    ...INITIAL_SNAPSHOT.sessions,
    session({
      id: 'sess-discover-liverecover',
      project: 'discover-liverecover',
      company: 'liverecover',
      status: 'running',
      source: 'claude-jsonl',
      lastActivityAt: new Date(NOW - 1_000).toISOString(),
    }),
  ],
  history: [],
};

// ── Harness ─────────────────────────────────────────────────────────────────

let host: HTMLElement;
let component: Record<string, unknown> | null = null;

/**
 * Mount the REAL Mission Control page into the live DOM and let the store's
 * async boot settle (the `list_agent_sessions` invoke + the `listen`
 * subscription both resolve on the microtask queue). Returns the mount host so
 * tests can query the rendered markup.
 */
async function mountMissionControl(): Promise<HTMLElement> {
  // Import the page AFTER the mocks are registered and the store reset so the
  // singleton store re-subscribes to our captured listener every test.
  const { default: MissionControlPage } = await import(
    '../../src/desktop-alt/pages/MissionControlPage.svelte'
  );
  const { stopSessionsStore } = await import(
    '../../src/desktop-alt/lib/sessions-store.svelte'
  );
  // The store is a lifetime singleton; reset it so this mount starts clean and
  // re-runs its subscribe + first-scan against this test's fixture.
  stopSessionsStore();

  component = mount(MissionControlPage, { target: host });
  flushSync();
  // Let the store's async refresh() (the awaited invoke) + listen() resolve, then
  // flush the resulting reactive updates into the DOM.
  await Promise.resolve();
  await Promise.resolve();
  flushSync();
  return host;
}

beforeEach(() => {
  vi.useFakeTimers();
  vi.setSystemTime(NOW);
  updatedHandler = null;
  nextSnapshot = { sessions: [], history: [] };
  host = document.createElement('div');
  document.body.appendChild(host);
});

afterEach(async () => {
  if (component) {
    await unmount(component);
    component = null;
  }
  host?.remove();
  vi.useRealTimers();
  vi.restoreAllMocks();
});

describe('US-012 — Mission Control E2E (real page render + poll refresh)', () => {
  it('lists the live local sessions from the fixture when the page opens', async () => {
    nextSnapshot = INITIAL_SNAPSHOT;
    const dom = await mountMissionControl();

    // The page chrome actually rendered (not a stub) — its title + best-effort
    // subtitle are in the DOM.
    expect(dom.querySelector('#mc-page-title')?.textContent).toBe('Mission Control');
    expect(dom.querySelector('.mc-subtitle')?.textContent).toContain('Best-effort liveness');

    // The Live panel rendered the two fixture sessions as dense rows — assert on
    // the real rendered row names, not on the store.
    const liveMount = dom.querySelector('.mc-live-mount');
    expect(liveMount).not.toBeNull();
    const rowNames = Array.from(dom.querySelectorAll('.ls-name')).map((el) =>
      el.textContent?.trim(),
    );
    expect(rowNames).toContain('mission-control');
    expect(rowNames).toContain('indigo-docs');

    // The summary strip counts are derived live from the store and painted into
    // the tiles: 1 running + 1 awaiting_input from the fixture.
    const tileValue = (label: string): string | undefined => {
      const tile = Array.from(dom.querySelectorAll('.mc-tile')).find((el) =>
        el.querySelector('.mc-tile-label')?.textContent?.includes(label),
      );
      return tile?.querySelector('.mc-tile-value')?.textContent?.trim();
    };
    expect(tileValue('RUNNING')).toBe('1');
    expect(tileValue('AWAITING INPUT')).toBe('1');

    // Subtitle session tally reflects the two local sessions (0 outpost).
    expect(dom.querySelector('.mc-subtitle')?.textContent).toContain('2 sessions');
    expect(dom.querySelector('.mc-subtitle')?.textContent).toContain('2 local');
  });

  it('adds a session on a poll tick — it appears with no manual refresh', async () => {
    nextSnapshot = INITIAL_SNAPSHOT;
    const dom = await mountMissionControl();

    // Sanity: the just-added discover session is NOT present before the tick.
    const namesBefore = Array.from(dom.querySelectorAll('.ls-name')).map((el) =>
      el.textContent?.trim(),
    );
    expect(namesBefore).not.toContain('discover-liverecover');
    expect(namesBefore).toHaveLength(2);

    // The backend polling loop emits `sessions:updated` with a fresh snapshot —
    // simulate exactly that. The store replaces state from the event payload; no
    // re-mount, no re-fetch, no manual refresh button.
    expect(updatedHandler).toBeTypeOf('function');
    updatedHandler!({ payload: POLLED_SNAPSHOT });
    flushSync();

    const namesAfter = Array.from(dom.querySelectorAll('.ls-name')).map((el) =>
      el.textContent?.trim(),
    );
    // The new session is now rendered, the originals are retained, and the
    // summary repainted to the new running count (2 running after the tick).
    expect(namesAfter).toContain('discover-liverecover');
    expect(namesAfter).toContain('mission-control');
    expect(namesAfter).toContain('indigo-docs');
    expect(namesAfter).toHaveLength(3);

    const runningTile = Array.from(dom.querySelectorAll('.mc-tile')).find((el) =>
      el.querySelector('.mc-tile-label')?.textContent?.includes('RUNNING'),
    );
    expect(runningTile?.querySelector('.mc-tile-value')?.textContent?.trim()).toBe('2');
    expect(dom.querySelector('.mc-subtitle')?.textContent).toContain('3 sessions');
  });
});
