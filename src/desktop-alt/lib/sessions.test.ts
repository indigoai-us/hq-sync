import { describe, expect, it } from 'vitest';
import {
  SESSION_KINDS,
  SESSION_STATUSES,
  deriveEventTool,
  deriveSessionKind,
  eventNodeTone,
  filterHistory,
  groupKeyFor,
  groupSessions,
  historyCompanies,
  isActiveForLivePanel,
  isLiveStatus,
  isSessionStatus,
  partitionByOrigin,
  relativeActivity,
  resolveOutpostCard,
  sortHistoryNewestFirst,
  type AgentSession,
  type HistoryEvent,
  type HistoryEventKind,
  type OutpostStatus,
  type SessionKind,
  type SessionStatus,
} from './sessions';

const session = (overrides: Partial<AgentSession> = {}): AgentSession => ({
  id: '25f8d9da-435d-44e6-8bb7-849fd8ad67c8',
  tool: 'claude',
  origin: 'local',
  cwd: '/Users/corey/Documents/HQ/repos/public/hq-sync',
  project: 'mission-control',
  company: 'indigo',
  model: 'claude-opus-4-8',
  status: 'running',
  startedAt: '2026-06-15T18:00:00Z',
  lastActivityAt: '2026-06-15T18:43:20Z',
  source: 'claude-jsonl',
  ...overrides,
});

describe('status taxonomy', () => {
  it('declares exactly running | awaiting_input | idle | ended in order', () => {
    expect(SESSION_STATUSES).toEqual(['running', 'awaiting_input', 'idle', 'ended']);
  });

  it('accepts every taxonomy member via the type guard', () => {
    for (const status of SESSION_STATUSES) {
      expect(isSessionStatus(status)).toBe(true);
    }
  });

  it('rejects non-members and non-strings', () => {
    expect(isSessionStatus('paused')).toBe(false);
    expect(isSessionStatus('Running')).toBe(false); // case-sensitive
    expect(isSessionStatus(undefined)).toBe(false);
    expect(isSessionStatus(42)).toBe(false);
  });
});

describe('isLiveStatus', () => {
  it('treats running and awaiting_input as live', () => {
    expect(isLiveStatus('running')).toBe(true);
    expect(isLiveStatus('awaiting_input')).toBe(true);
  });

  it('treats idle and ended as not live', () => {
    expect(isLiveStatus('idle')).toBe(false);
    expect(isLiveStatus('ended')).toBe(false);
  });
});

describe('AgentSession wire shape', () => {
  it('round-trips through JSON unchanged (camelCase contract with Rust)', () => {
    const original = session();
    const roundTripped = JSON.parse(JSON.stringify(original)) as AgentSession;
    expect(roundTripped).toEqual(original);
  });

  it('serialises camelCase keys that match the Rust struct', () => {
    const json = JSON.parse(JSON.stringify(session()));
    expect(Object.keys(json).sort()).toEqual(
      [
        'id',
        'tool',
        'origin',
        'cwd',
        'project',
        'company',
        'model',
        'status',
        'startedAt',
        'lastActivityAt',
        'source',
      ].sort(),
    );
    // No snake_case leakage.
    expect(json).not.toHaveProperty('started_at');
    expect(json).not.toHaveProperty('last_activity_at');
  });

  it('carries each origin/tool combination', () => {
    const combos: Array<[AgentSession['tool'], AgentSession['origin']]> = [
      ['claude', 'local'],
      ['claude', 'outpost'],
      ['codex', 'local'],
      ['codex', 'outpost'],
    ];
    for (const [tool, origin] of combos) {
      const s = session({ tool, origin, source: `${tool}-${origin}` });
      expect(s.tool).toBe(tool);
      expect(s.origin).toBe(origin);
    }
  });

  it('a deserialised status narrows back into the taxonomy', () => {
    const wire = JSON.parse(JSON.stringify(session({ status: 'awaiting_input' })));
    expect(isSessionStatus(wire.status)).toBe(true);
    const status: SessionStatus = wire.status;
    expect(status).toBe('awaiting_input');
  });
});

describe('deriveSessionKind (US-007 best-effort heuristic)', () => {
  // Each case names a project/cwd/source that should resolve to one kind. The
  // heuristic keys off the lowercased project + cwd-tail + source haystack.
  const cases: Array<[string, Partial<AgentSession>, SessionKind]> = [
    [
      'per-channel slack watcher (project)',
      { project: 'slack-watcher-hq-dev', source: 'claude-jsonl' },
      'slack-watcher',
    ],
    [
      'run-bot watcher (source)',
      { project: 'hassaan', source: 'run-bot' },
      'slack-watcher',
    ],
    [
      'PR babysitter (project)',
      { project: 'babysit-pr-1421', source: 'claude-jsonl' },
      'pr',
    ],
    [
      'land-batch PR loop',
      { project: 'land-batch', source: 'codex-rollout' },
      'pr',
    ],
    [
      'CI watcher beats deploy when both could match',
      { project: 'ci-build-watch', cwd: '/repos/ci-watch', source: 'claude-jsonl' },
      'ci',
    ],
    [
      'deploy monitor (project)',
      { project: 'deploy-monitor-prod', source: 'claude-jsonl' },
      'deploy',
    ],
    [
      'signup heartbeat',
      { project: 'signup-heartbeat', source: 'claude-jsonl' },
      'signup-heartbeat',
    ],
    [
      'discover / ingest',
      { project: 'discover-acme-repo', source: 'claude-jsonl' },
      'discover',
    ],
    [
      'plain interactive session falls through to interactive',
      { project: 'mission-control', cwd: '/Users/x/HQ/repos/public/hq-sync', source: 'claude-jsonl' },
      'interactive',
    ],
  ];

  for (const [name, overrides, expected] of cases) {
    it(name, () => {
      expect(deriveSessionKind(session(overrides))).toBe(expected);
    });
  }

  it('returns unknown when there is nothing to key on (empty project + cwd + source)', () => {
    expect(deriveSessionKind(session({ project: '', cwd: '', source: '' }))).toBe('unknown');
  });

  it('only ever returns a member of SESSION_KINDS', () => {
    const kind = deriveSessionKind(session());
    expect(SESSION_KINDS).toContain(kind);
  });

  it('is deterministic — the same input yields the same kind', () => {
    const s = session({ project: 'slack-watcher-x' });
    expect(deriveSessionKind(s)).toBe(deriveSessionKind(s));
  });
});

describe('groupKeyFor (graceful fallback)', () => {
  it('keys on the inferred kind when one is inferred', () => {
    expect(groupKeyFor(session({ project: 'deploy-monitor' }))).toBe('kind:deploy');
  });

  it('falls back to the project when no kind is inferred but a project exists', () => {
    // No cwd/source signal and an unmatched project → interactive (which IS a
    // kind), so to exercise the unknown fallback we strip everything but project.
    const s = session({ project: 'orphan-proj', cwd: '', source: '' });
    // `orphan-proj` matches no monitor pattern but is non-empty → interactive.
    expect(groupKeyFor(s)).toBe('kind:interactive');
  });

  it('falls back to origin:tool when even the project is empty', () => {
    const s = session({ project: '', cwd: '', source: '', origin: 'outpost', tool: 'codex' });
    expect(groupKeyFor(s)).toBe('origin:outpost:codex');
  });
});

describe('groupSessions (US-007 dense grouping)', () => {
  it('clusters near-identical monitors into one group and never drops a session', () => {
    const fleet = [
      session({ id: 'a', project: 'slack-watcher-1', status: 'running', lastActivityAt: '2026-06-15T18:40:00Z' }),
      session({ id: 'b', project: 'slack-watcher-2', status: 'idle', lastActivityAt: '2026-06-15T18:42:00Z' }),
      session({ id: 'c', project: 'slack-watcher-3', status: 'running', lastActivityAt: '2026-06-15T18:41:00Z' }),
      session({ id: 'd', project: 'mission-control', status: 'running', lastActivityAt: '2026-06-15T18:43:00Z' }),
    ];
    const groups = groupSessions(fleet);
    const total = groups.reduce((n, g) => n + g.count, 0);
    expect(total).toBe(4); // nothing dropped
    const slack = groups.find((g) => g.key === 'kind:slack-watcher');
    expect(slack?.count).toBe(3);
    expect(slack?.statusCounts.running).toBe(2);
    expect(slack?.statusCounts.idle).toBe(1);
  });

  it('orders rows within a group freshest-first', () => {
    const groups = groupSessions([
      session({ id: 'old', project: 'slack-watcher-1', lastActivityAt: '2026-06-15T18:00:00Z' }),
      session({ id: 'new', project: 'slack-watcher-2', lastActivityAt: '2026-06-15T18:30:00Z' }),
    ]);
    const slack = groups.find((g) => g.key === 'kind:slack-watcher');
    expect(slack?.sessions.map((s) => s.id)).toEqual(['new', 'old']);
    expect(slack?.freshestActivityAt).toBe('2026-06-15T18:30:00Z');
  });

  it('orders groups by most-live first', () => {
    const groups = groupSessions([
      // 1 idle deploy monitor
      session({ id: 'd1', project: 'deploy-monitor', status: 'idle' }),
      // 2 running interactive sessions
      session({ id: 'i1', project: 'mission-control', status: 'running' }),
      session({ id: 'i2', project: 'indigo-site', status: 'running' }),
    ]);
    expect(groups[0].kind).toBe('interactive'); // most live leads
  });

  it('produces a stable status-count shape with all four statuses', () => {
    const groups = groupSessions([session({ project: 'mission-control' })]);
    expect(Object.keys(groups[0].statusCounts).sort()).toEqual(
      [...SESSION_STATUSES].sort(),
    );
  });
});

describe('isActiveForLivePanel', () => {
  const now = Date.parse('2026-06-15T18:43:00Z');

  it('keeps running / awaiting_input / idle sessions', () => {
    for (const status of ['running', 'awaiting_input', 'idle'] as SessionStatus[]) {
      expect(isActiveForLivePanel(session({ status }), now)).toBe(true);
    }
  });

  it('drops long-ended sessions', () => {
    const s = session({ status: 'ended', lastActivityAt: '2026-06-15T18:00:00Z' });
    expect(isActiveForLivePanel(s, now)).toBe(false);
  });

  it('keeps a recently-ended session inside the window', () => {
    const s = session({ status: 'ended', lastActivityAt: '2026-06-15T18:42:30Z' });
    expect(isActiveForLivePanel(s, now)).toBe(true);
  });
});

describe('relativeActivity (compact mono token)', () => {
  const now = Date.parse('2026-06-15T18:00:00Z');
  it('renders compact units', () => {
    expect(relativeActivity('2026-06-15T18:00:00Z', now)).toBe('now');
    expect(relativeActivity('2026-06-15T17:59:30Z', now)).toBe('30s');
    expect(relativeActivity('2026-06-15T17:55:00Z', now)).toBe('5m');
    expect(relativeActivity('2026-06-15T16:00:00Z', now)).toBe('2h');
    expect(relativeActivity('2026-06-13T18:00:00Z', now)).toBe('2d');
  });
  it('renders an em-dash for an empty/unparseable timestamp', () => {
    expect(relativeActivity('', now)).toBe('—');
    expect(relativeActivity('not-a-date', now)).toBe('—');
  });
});

// ───────────────────────────────────────────────────────────────────────────
// History timeline (US-008)
// ───────────────────────────────────────────────────────────────────────────

const historyEvent = (overrides: Partial<HistoryEvent> = {}): HistoryEvent => ({
  kind: 'completed',
  title: 'US-007 completed',
  company: 'indigo',
  project: 'mission-control',
  timestamp: '2026-06-15T18:40:00Z',
  source: 'audit-log',
  ...overrides,
});

describe('deriveEventTool (US-008 best-effort tool inference)', () => {
  it('infers claude from a claude-flavoured source / title', () => {
    expect(deriveEventTool(historyEvent({ source: 'claude-jsonl' }))).toBe('claude');
    expect(deriveEventTool(historyEvent({ source: 'thread', title: 'Claude Code checkpoint' }))).toBe(
      'claude',
    );
  });

  it('infers codex from a codex-flavoured source / title', () => {
    expect(deriveEventTool(historyEvent({ source: 'codex-rollout' }))).toBe('codex');
    expect(deriveEventTool(historyEvent({ source: 'thread', title: 'Codex handoff' }))).toBe('codex');
  });

  it('returns null when the tool is indeterminate (plain audit-log row)', () => {
    expect(deriveEventTool(historyEvent({ source: 'audit-log', title: 'US-007 completed' }))).toBe(
      null,
    );
  });

  it('is deterministic for the same input', () => {
    const e = historyEvent({ source: 'codex-rollout' });
    expect(deriveEventTool(e)).toBe(deriveEventTool(e));
  });
});

describe('sortHistoryNewestFirst', () => {
  it('orders events newest-first by timestamp without mutating the input', () => {
    const feed = [
      historyEvent({ title: 'old', timestamp: '2026-06-15T17:00:00Z' }),
      historyEvent({ title: 'new', timestamp: '2026-06-15T18:00:00Z' }),
      historyEvent({ title: 'mid', timestamp: '2026-06-15T17:30:00Z' }),
    ];
    const sorted = sortHistoryNewestFirst(feed);
    expect(sorted.map((e) => e.title)).toEqual(['new', 'mid', 'old']);
    // Input untouched (pure).
    expect(feed.map((e) => e.title)).toEqual(['old', 'new', 'mid']);
  });

  it('sorts empty/unparseable timestamps last', () => {
    const sorted = sortHistoryNewestFirst([
      historyEvent({ title: 'blank', timestamp: '' }),
      historyEvent({ title: 'real', timestamp: '2026-06-15T18:00:00Z' }),
    ]);
    expect(sorted.map((e) => e.title)).toEqual(['real', 'blank']);
  });
});

describe('filterHistory (US-008 tool + company filter)', () => {
  // A small mixed feed: 2 codex, 1 claude, 1 indeterminate — across two companies.
  const feed: HistoryEvent[] = [
    historyEvent({ title: 'codex-a', source: 'codex-rollout', company: 'indigo', timestamp: '2026-06-15T18:10:00Z' }),
    historyEvent({ title: 'claude-a', source: 'claude-jsonl', company: 'indigo', timestamp: '2026-06-15T18:20:00Z' }),
    historyEvent({ title: 'codex-b', source: 'thread', kind: 'handoff', company: 'liverecover', timestamp: '2026-06-15T18:30:00Z' }),
    historyEvent({ title: 'plain', source: 'audit-log', company: 'liverecover', timestamp: '2026-06-15T18:40:00Z' }),
  ];

  it('all + no company keeps every event, newest-first (e2e: events render newest-first)', () => {
    const out = filterHistory(feed, { tool: 'all', company: '' });
    expect(out.map((e) => e.title)).toEqual(['plain', 'codex-b', 'claude-a', 'codex-a']);
  });

  it('tool=codex keeps only codex events (e2e: tool filter Codex → only Codex events)', () => {
    const out = filterHistory(feed, { tool: 'codex', company: '' });
    expect(out.map((e) => e.title)).toEqual(['codex-b', 'codex-a']);
    expect(out.every((e) => deriveEventTool(e) === 'codex')).toBe(true);
  });

  it('tool=claude keeps only claude events and drops indeterminate ones', () => {
    const out = filterHistory(feed, { tool: 'claude', company: '' });
    expect(out.map((e) => e.title)).toEqual(['claude-a']);
  });

  it('narrows by company slug', () => {
    const out = filterHistory(feed, { tool: 'all', company: 'liverecover' });
    expect(out.map((e) => e.title)).toEqual(['plain', 'codex-b']);
  });

  it('combines tool + company filters', () => {
    const out = filterHistory(feed, { tool: 'codex', company: 'liverecover' });
    expect(out.map((e) => e.title)).toEqual(['codex-b']);
  });

  it('renders an empty result cleanly when nothing matches (empty-state path)', () => {
    expect(filterHistory(feed, { tool: 'claude', company: 'liverecover' })).toEqual([]);
    expect(filterHistory([], { tool: 'all', company: '' })).toEqual([]);
  });
});

describe('historyCompanies', () => {
  it('returns distinct, non-empty company slugs alphabetically', () => {
    const companies = historyCompanies([
      historyEvent({ company: 'liverecover' }),
      historyEvent({ company: 'indigo' }),
      historyEvent({ company: 'indigo' }),
      historyEvent({ company: '' }),
    ]);
    expect(companies).toEqual(['indigo', 'liverecover']);
  });

  it('returns an empty list for an empty feed', () => {
    expect(historyCompanies([])).toEqual([]);
  });
});

describe('eventNodeTone (US-008 node color by kind)', () => {
  const cases: Array<[HistoryEventKind, ReturnType<typeof eventNodeTone>]> = [
    ['completed', 'ok'],
    ['dispatched', 'neutral'],
    ['handoff', 'neutral'],
    ['checkpoint', 'faint'],
    ['failed', 'error'],
  ];
  for (const [kind, tone] of cases) {
    it(`maps ${kind} → ${tone}`, () => {
      expect(eventNodeTone(kind)).toBe(tone);
    });
  }
});

// ── US-011: outpost subscriber + box status + merge ──────────────────────────

const outpostStatus = (overrides: Partial<OutpostStatus> = {}): OutpostStatus => ({
  up: true,
  runtime: 'claude',
  relayConnected: true,
  ip: '203.0.113.7',
  region: 'us-east-1',
  lastSeenAt: '2026-06-15T18:43:20Z',
  stale: false,
  ...overrides,
});

describe('partitionByOrigin (US-011 outpost split)', () => {
  it('splits a fleet into local and outpost, preserving input order', () => {
    const fleet = [
      session({ id: 'l1', origin: 'local' }),
      session({ id: 'o1', origin: 'outpost' }),
      session({ id: 'l2', origin: 'local' }),
      session({ id: 'o2', origin: 'outpost' }),
    ];
    const { local, outpost } = partitionByOrigin(fleet);
    expect(local.map((s) => s.id)).toEqual(['l1', 'l2']);
    expect(outpost.map((s) => s.id)).toEqual(['o1', 'o2']);
  });

  it('handles a local-only fleet (no outpost) without dropping anything', () => {
    const fleet = [session({ id: 'l1' }), session({ id: 'l2' })];
    const { local, outpost } = partitionByOrigin(fleet);
    expect(local).toHaveLength(2);
    expect(outpost).toHaveLength(0);
  });

  it('groups an origin=outpost session into its own bucket (the e2e merge path)', () => {
    // The US-011 e2e: a simulated outpost heartbeat → origin=outpost session →
    // renders under the outpost group. partitionByOrigin is the split that puts
    // it there; groupSessions then clusters it like any other.
    const fleet = [session({ id: 'o1', origin: 'outpost', project: 'remote-thing' })];
    const { local, outpost } = partitionByOrigin(fleet);
    expect(local).toHaveLength(0);
    expect(outpost).toHaveLength(1);
    const groups = groupSessions(outpost);
    expect(groups.flatMap((g) => g.sessions.map((s) => s.id))).toContain('o1');
  });
});

describe('resolveOutpostCard (US-011 box-card up/down states)', () => {
  it('renders an UP (green) card when the box is up and fresh', () => {
    const card = resolveOutpostCard(outpostStatus({ up: true, stale: false }), 0);
    expect(card.tone).toBe('ok');
    expect(card.stateLabel).toBe('UP');
    expect(card.relayLabel).toBe('connected');
    expect(card.relayConnected).toBe(true);
    expect(card.runtimeLabel).toBe('CLAUDE');
    expect(card.metaLabel).toBe('203.0.113.7 · us-east-1');
    // No stale note in the up state.
    expect(card.staleNote).toBeNull();
  });

  it('renders a DOWN (red) card with the stale-timeout note when the box went stale', () => {
    const card = resolveOutpostCard(
      outpostStatus({ up: false, stale: true, relayConnected: false }),
      3,
    );
    expect(card.tone).toBe('down');
    expect(card.stateLabel).toBe('DOWN');
    expect(card.relayLabel).toBe('disconnected');
    expect(card.relayConnected).toBe(false);
    // The down state carries the dropped-sessions note (design.md).
    expect(card.staleNote).toMatch(/3 outpost sessions dropped after the 90s stale timeout/);
    expect(card.staleNote).toMatch(/reappear when the box reports in/);
  });

  it('treats up-but-stale as DOWN (stale flips the card even if up is set)', () => {
    // The backend ages a box: /outpost/status may still say up, but a missed
    // heartbeat sets stale → the card MUST read down.
    const card = resolveOutpostCard(outpostStatus({ up: true, stale: true }), 1);
    expect(card.tone).toBe('down');
    expect(card.stateLabel).toBe('DOWN');
  });

  it('omits the stale note when nothing was dropped (down but zero prior sessions)', () => {
    const card = resolveOutpostCard(outpostStatus({ up: false, stale: true }), 0);
    expect(card.tone).toBe('down');
    expect(card.staleNote).toBeNull();
  });

  it('singularises the note for a single dropped session', () => {
    const card = resolveOutpostCard(outpostStatus({ up: false, stale: true }), 1);
    expect(card.staleNote).toMatch(/1 outpost session dropped/);
    expect(card.staleNote).toMatch(/it reappears when the box reports in/);
  });

  it('falls back to — for an unknown runtime / empty meta', () => {
    const card = resolveOutpostCard(
      outpostStatus({ runtime: '', ip: '', region: '' }),
      0,
    );
    expect(card.runtimeLabel).toBe('—');
    expect(card.metaLabel).toBe('—');
  });
});
