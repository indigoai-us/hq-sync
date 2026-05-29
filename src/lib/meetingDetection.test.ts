import { describe, expect, it } from 'vitest';
import {
  handleMeetingDetected,
  resolveWindowId,
  type MeetingDetectedDeps,
  type DetectedMeetingSeed,
  type NotifyDetectedPayload,
} from './meetingDetection';

/**
 * Build a deps bundle whose collaborators record every call, so each test
 * can assert exactly which surfaces fired. `checkActiveBot` defaults to "no
 * bot" (the common ad-hoc-meeting case); override per test.
 */
function makeDeps(overrides: Partial<MeetingDetectedDeps> = {}): {
  deps: MeetingDetectedDeps;
  calls: {
    upsert: DetectedMeetingSeed[];
    remove: string[];
    notify: NotifyDetectedPayload[];
    botChecks: Array<[string, string | null]>;
  };
} {
  const calls = {
    upsert: [] as DetectedMeetingSeed[],
    remove: [] as string[],
    notify: [] as NotifyDetectedPayload[],
    botChecks: [] as Array<[string, string | null]>,
  };
  const deps: MeetingDetectedDeps = {
    checkActiveBot: async (url, eventId) => {
      calls.botChecks.push([url, eventId]);
      return false;
    },
    upsertRow: (seed) => {
      calls.upsert.push(seed);
    },
    removeRow: (wid) => {
      calls.remove.push(wid);
    },
    notify: async (payload) => {
      calls.notify.push(payload);
    },
    resolveValidDefault: () => null,
    now: () => '2026-05-28T00:00:00.000Z',
    warn: () => {},
    ...overrides,
  };
  return { deps, calls };
}

describe('handleMeetingDetected', () => {
  it('suppresses BOTH the row and the notification when an active bot already covers the meeting', async () => {
    // Regression guard. The old handler upserted the popover row
    // unconditionally *before* the bot check and only `return`ed out of the
    // notification — so a scheduled calendar meeting (covered by its bot)
    // still rendered a "you could record this" row in the popover forever.
    const { deps, calls } = makeDeps({ checkActiveBot: async () => true });

    await handleMeetingDetected(
      {
        meetingUrl: 'https://zoom.us/j/123',
        platform: 'zoom',
        windowId: 'win-1',
        sourceEventId: 'evt-1',
      },
      deps,
    );

    expect(calls.upsert).toHaveLength(0); // no recordable row
    expect(calls.notify).toHaveLength(0); // no notification
    expect(calls.remove).toEqual(['win-1']); // any stale row cleared
  });

  it('surfaces row + notification when no bot covers the meeting', async () => {
    const { deps, calls } = makeDeps({ checkActiveBot: async () => false });

    await handleMeetingDetected(
      { meetingUrl: 'https://zoom.us/j/123', platform: 'zoom', windowId: 'win-2' },
      deps,
    );

    expect(calls.upsert).toHaveLength(1);
    expect(calls.upsert[0]).toMatchObject({
      windowId: 'win-2',
      platform: 'zoom',
      state: 'detected',
      companyUserSet: false,
    });
    expect(calls.notify).toHaveLength(1);
    expect(calls.notify[0].windowId).toBe('win-2');
    expect(calls.remove).toHaveLength(0);
  });

  it('skips the bot check for synthetic recall-window URLs and always surfaces', async () => {
    // URL-less SDK detections (unscheduled meetings) carry a synthetic
    // `recall-window:<id>` URL hq-pro can't match — never query it, always
    // surface so the user can still record ad-hoc.
    const { deps, calls } = makeDeps();

    await handleMeetingDetected({ meetingUrl: 'recall-window:abc', platform: 'zoom' }, deps);

    expect(calls.botChecks).toHaveLength(0);
    expect(calls.upsert).toHaveLength(1);
    expect(calls.upsert[0].windowId).toBe('abc'); // id extracted from synthetic URL
    expect(calls.notify).toHaveLength(1);
  });

  it('fails open (surfaces) when the bot check throws', async () => {
    const warnings: unknown[] = [];
    const { deps, calls } = makeDeps({
      checkActiveBot: async () => {
        throw new Error('network down');
      },
      warn: (_msg, err) => warnings.push(err),
    });

    await handleMeetingDetected(
      { meetingUrl: 'https://zoom.us/j/123', platform: 'zoom', windowId: 'win-3' },
      deps,
    );

    expect(calls.upsert).toHaveLength(1);
    expect(calls.notify).toHaveLength(1);
    expect(warnings).toHaveLength(1);
  });

  it('passes meetingUrl + sourceEventId through to the bot check', async () => {
    const { deps, calls } = makeDeps();

    await handleMeetingDetected(
      { meetingUrl: 'https://zoom.us/j/123', windowId: 'w', sourceEventId: 'evt-9' },
      deps,
    );

    expect(calls.botChecks[0]).toEqual(['https://zoom.us/j/123', 'evt-9']);
  });

  it('seeds the row with the resolved default company', async () => {
    const { deps, calls } = makeDeps({
      checkActiveBot: async () => false,
      resolveValidDefault: () => 'cmp_123',
    });

    await handleMeetingDetected(
      { meetingUrl: 'https://zoom.us/j/123', windowId: 'win-4' },
      deps,
    );

    expect(calls.upsert[0].companyUid).toBe('cmp_123');
    expect(calls.upsert[0].detectedAt).toBe('2026-05-28T00:00:00.000Z');
  });
});

describe('resolveWindowId', () => {
  it('prefers the explicit windowId field', () => {
    expect(resolveWindowId({ windowId: 'direct', meetingUrl: 'https://zoom.us/j/1' })).toEqual({
      windowId: 'direct',
      isSyntheticUrl: false,
    });
  });

  it('extracts the windowId from a synthetic recall-window URL', () => {
    expect(resolveWindowId({ meetingUrl: 'recall-window:xyz' })).toEqual({
      windowId: 'xyz',
      isSyntheticUrl: true,
    });
  });

  it('falls back to the meetingUrl as the dedup key for real URLs', () => {
    expect(resolveWindowId({ meetingUrl: 'https://zoom.us/j/1' })).toEqual({
      windowId: 'https://zoom.us/j/1',
      isSyntheticUrl: false,
    });
  });

  it('yields an empty windowId when nothing identifies the window', () => {
    expect(resolveWindowId({})).toEqual({ windowId: '', isSyntheticUrl: false });
  });
});
