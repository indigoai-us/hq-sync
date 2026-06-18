import { describe, it, expect } from 'vitest';
import { statusTone, type AgencyTeam, type AgencyQuestion } from './agency';

describe('statusTone', () => {
  it('running + ready -> ok', () => expect(statusTone('running', true)).toBe('ok'));
  it('running + not-ready (booting) -> warn', () => expect(statusTone('running', false)).toBe('warn'));
  it('stopped -> idle', () => expect(statusTone('stopped', false)).toBe('idle'));
  it('crash-loop -> warn', () => expect(statusTone('crash-loop', false)).toBe('warn'));
  it('unknown -> idle', () => expect(statusTone('unknown', false)).toBe('idle'));
});

describe('wire shapes', () => {
  it('AgencyTeam / AgencyQuestion are plain data', () => {
    const team: AgencyTeam = { company: 'indigo', team: 'nick', workers: [{ worker: 'team-manager', instance: 'main', status: 'running', ready: true }] };
    const q: AgencyQuestion = { company: 'indigo', team: 'nick', id: '780494884', question: 'Ship it?', ts: '2026-06-18T00:00:00Z' };
    expect(team.workers[0].worker).toBe('team-manager');
    expect(q.id).toBe('780494884');
  });
});
