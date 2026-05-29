import { describe, it, expect } from 'vitest';
import { isAlreadyScheduledError } from './invite-errors';

describe('isAlreadyScheduledError', () => {
  it('matches the atomic dedup-lock 409 (bot-already-scheduling)', () => {
    expect(
      isAlreadyScheduledError(
        'bot/invite HTTP 409: {"error":"A bot is already being scheduled for this meeting","code":"bot-already-scheduling"}',
      ),
    ).toBe(true);
  });

  it('matches the sibling / Recall dedup 409 (bot-already-scheduled)', () => {
    expect(
      isAlreadyScheduledError(
        'bot/invite HTTP 409: {"error":"A bot is already scheduled","code":"bot-already-scheduled"}',
      ),
    ).toBe(true);
  });

  it('matches a bare 409 with no structured code', () => {
    expect(isAlreadyScheduledError('bot/invite HTTP 409: upstream conflict')).toBe(true);
  });

  it('matches an Error instance, not just a string', () => {
    expect(isAlreadyScheduledError(new Error('HTTP 409 bot-already-scheduling'))).toBe(true);
  });

  it('does NOT match unrelated failures', () => {
    expect(isAlreadyScheduledError('bot/invite HTTP 500: server error')).toBe(false);
    expect(isAlreadyScheduledError('bot/invite parse: missing field `autoScheduled`')).toBe(false);
    expect(isAlreadyScheduledError('bot/invite fetch: connection reset')).toBe(false);
    expect(isAlreadyScheduledError(null)).toBe(false);
    expect(isAlreadyScheduledError(undefined)).toBe(false);
  });
});
