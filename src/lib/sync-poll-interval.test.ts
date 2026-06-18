import { describe, expect, it } from 'vitest';
import {
  DEFAULT_SYNC_POLL_SECONDS,
  humanizeSyncPollInterval,
  normalizeSyncPollSeconds,
} from './sync-poll-interval';

describe('sync poll interval helpers', () => {
  it('defaults missing or invalid values to 10 minutes', () => {
    expect(normalizeSyncPollSeconds(null)).toBe(DEFAULT_SYNC_POLL_SECONDS);
    expect(normalizeSyncPollSeconds(undefined)).toBe(DEFAULT_SYNC_POLL_SECONDS);
    expect(normalizeSyncPollSeconds(0)).toBe(DEFAULT_SYNC_POLL_SECONDS);
    expect(humanizeSyncPollInterval(null)).toBe('every 10 minutes');
  });

  it('humanizes minute intervals with singular and plural units', () => {
    expect(humanizeSyncPollInterval(60)).toBe('every 1 minute');
    expect(humanizeSyncPollInterval(120)).toBe('every 2 minutes');
    expect(humanizeSyncPollInterval(300)).toBe('every 5 minutes');
    expect(humanizeSyncPollInterval(600)).toBe('every 10 minutes');
  });
});
