import { describe, expect, it } from 'vitest';
import {
  SESSION_STATUSES,
  isLiveStatus,
  isSessionStatus,
  type AgentSession,
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
