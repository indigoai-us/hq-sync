import { describe, it, expect } from 'vitest';
import {
  statusTone,
  senderTone,
  relativeTime,
  shortDuration,
  type AgencyTeam,
  type AgencyQuestion,
  type AgencyMessage,
} from './agency';

describe('statusTone', () => {
  it('running + ready -> ok', () => expect(statusTone('running', true)).toBe('ok'));
  it('running + not-ready (booting) -> warn', () => expect(statusTone('running', false)).toBe('warn'));
  it('stopped -> idle', () => expect(statusTone('stopped', false)).toBe('idle'));
  it('crash-loop -> warn', () => expect(statusTone('crash-loop', false)).toBe('warn'));
  it('unknown -> idle', () => expect(statusTone('unknown', false)).toBe('idle'));
});

describe('senderTone', () => {
  it('manager -> ok', () => expect(senderTone('manager')).toBe('ok'));
  it('liaison -> warn', () => expect(senderTone('liaison')).toBe('warn'));
  it('operator -> unread', () => expect(senderTone('operator')).toBe('unread'));
  it('a worker -> idle', () => expect(senderTone('recruiter')).toBe('idle'));
});

describe('relativeTime', () => {
  const now = Date.parse('2026-06-18T12:00:00Z');
  it('blank / unparseable -> empty', () => {
    expect(relativeTime('', now)).toBe('');
    expect(relativeTime('not-a-date', now)).toBe('');
  });
  it('seconds -> just now', () => expect(relativeTime('2026-06-18T11:59:40Z', now)).toBe('just now'));
  it('minutes', () => expect(relativeTime('2026-06-18T11:56:00Z', now)).toBe('4m ago'));
  it('hours', () => expect(relativeTime('2026-06-18T09:00:00Z', now)).toBe('3h ago'));
  it('days', () => expect(relativeTime('2026-06-16T12:00:00Z', now)).toBe('2d ago'));
});

describe('shortDuration', () => {
  const now = Date.parse('2026-06-18T12:00:00Z');
  it('blank -> empty', () => expect(shortDuration('', now)).toBe(''));
  it('seconds', () => expect(shortDuration('2026-06-18T11:59:30Z', now)).toBe('30s'));
  it('minutes', () => expect(shortDuration('2026-06-18T11:48:00Z', now)).toBe('12m'));
  it('hours', () => expect(shortDuration('2026-06-18T09:00:00Z', now)).toBe('3h'));
  it('days', () => expect(shortDuration('2026-06-16T12:00:00Z', now)).toBe('2d'));
});

describe('wire shapes', () => {
  it('AgencyTeam / AgencyQuestion are plain data', () => {
    const team: AgencyTeam = {
      company: 'indigo',
      team: 'nick',
      workers: [
        { worker: 'team-manager', instance: 'main', status: 'running', ready: true, startedAt: '2026-06-18T11:48:00Z', updatedAt: '2026-06-18T11:59:30Z' },
      ],
    };
    const q: AgencyQuestion = {
      company: 'indigo',
      team: 'nick',
      id: '780494884',
      question: 'Ship it?',
      ts: '2026-06-18T00:00:00Z',
      options: ['Yes', 'No'],
    };
    expect(team.workers[0].worker).toBe('team-manager');
    expect(team.workers[0].startedAt).toBe('2026-06-18T11:48:00Z');
    expect(q.id).toBe('780494884');
    expect(q.options).toEqual(['Yes', 'No']);

    const msg: AgencyMessage = { from: 'manager', kind: 'ask', text: 'Deploy?', ts: '2026-06-18T00:00:00Z', inbox: 'team-liaison' };
    expect(msg.kind).toBe('ask');
    expect(senderTone(msg.from)).toBe('ok');
  });
});
