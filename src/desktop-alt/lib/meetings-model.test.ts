import { describe, expect, it } from 'vitest';
import { get } from 'svelte/store';
import {
  activeRecordingsFromScheduledBots,
  buildConnectedCalendarRows,
  dayLabel,
  groupByDay,
  pickLiveMeeting,
  rowButtonKind,
  totalSignalCounts,
  type MeetingEvent,
  type ScheduledBot,
} from './meetings-model';
import {
  activeMeetings,
  upsertActiveMeeting,
  type ActiveMeeting,
} from '../../lib/activeMeetings';

describe('meetings-model', () => {
  it('prioritizes recording meetings over newer detections', () => {
    const rows: ActiveMeeting[] = [
      {
        windowId: 'newer',
        platform: 'zoom',
        meetingUrl: 'recall-window:newer',
        detectedAt: '2026-05-27T17:00:00.000Z',
        state: 'detected',
        companyUid: null,
      },
      {
        windowId: 'recording',
        platform: 'meet',
        meetingUrl: 'recall-window:recording',
        detectedAt: '2026-05-27T16:00:00.000Z',
        state: 'recording',
        companyUid: null,
      },
    ];

    expect(pickLiveMeeting(rows)?.windowId).toBe('recording');
  });

  it('preserves active recording state when the same meeting is detected again', () => {
    activeMeetings.set([]);
    upsertActiveMeeting({
      windowId: 'call-window',
      platform: 'meet',
      meetingUrl: 'recall-window:call-window',
      detectedAt: '2026-05-27T16:00:00.000Z',
      state: 'recording',
      recordingId: 'rec_123',
      companyUid: null,
    });
    upsertActiveMeeting({
      windowId: 'call-window',
      platform: 'meet',
      meetingUrl: 'recall-window:call-window',
      detectedAt: '2026-05-27T17:00:00.000Z',
      state: 'detected',
      companyUid: null,
      summary: 'Updated title',
    });

    expect(get(activeMeetings)[0]).toMatchObject({
      windowId: 'call-window',
      state: 'recording',
      recordingId: 'rec_123',
      summary: 'Updated title',
    });
  });

  it('counts action, decision, and risk signals across meetings', () => {
    const events: MeetingEvent[] = [
      {
        id: 'a',
        status: 'confirmed',
        start: { dateTime: '2026-05-27T17:00:00.000Z' },
        end: { dateTime: '2026-05-27T17:30:00.000Z' },
        signals: {
          actions: ['Follow up'],
          decisions: [{ title: 'Ship it' }],
          risks: ['Blocked'],
        },
      },
      {
        id: 'b',
        status: 'confirmed',
        start: { dateTime: '2026-05-27T18:00:00.000Z' },
        end: { dateTime: '2026-05-27T18:30:00.000Z' },
        signals: [{ type: 'action_item' }, { kind: 'decision' }],
      },
    ];

    expect(totalSignalCounts(events)).toEqual({
      actions: 2,
      decisions: 2,
      risks: 1,
    });
  });

  it('joins enabled cached calendars to membership routing status', () => {
    const rows = buildConnectedCalendarRows(
      [{ accountId: 'acct_1', email: 'person@example.com' }],
      new Map([['acct_1', [{ id: 'primary', summary: 'Person' }]]]),
      new Map([['acct_1', new Set(['primary'])]]),
      [
        {
          id: 'event_1',
          status: 'confirmed',
          start: { dateTime: '2026-05-27T17:00:00.000Z' },
          end: { dateTime: '2026-05-27T17:30:00.000Z' },
          sourceAccountId: 'acct_1',
          sourceCalendarId: 'primary',
          sourceCompanyUid: 'cmp_123',
        },
      ],
      [{ companyUid: 'cmp_123', companyName: 'Indigo', status: 'active' }],
    );

    expect(rows).toEqual([
      {
        key: 'acct_1|primary',
        email: 'person@example.com',
        calendar: 'Person',
        routingTarget: 'Indigo',
        status: 'active',
      },
    ]);
  });

  it('treats an explicitly empty enabled calendar set as no enabled calendars', () => {
    const rows = buildConnectedCalendarRows(
      [{ accountId: 'acct_1', email: 'person@example.com' }],
      new Map([['acct_1', [{ id: 'primary', summary: 'Person' }]]]),
      new Map([['acct_1', new Set()]]),
      [
        {
          id: 'event_1',
          status: 'confirmed',
          start: { dateTime: '2026-05-27T17:00:00.000Z' },
          end: { dateTime: '2026-05-27T17:30:00.000Z' },
          sourceAccountId: 'acct_1',
          sourceCalendarId: 'primary',
        },
      ],
      [{ companyUid: 'cmp_123', companyName: 'Indigo', status: 'active' }],
    );

    expect(rows).toEqual([]);
  });

  it('seeds active Live now rows from cached scheduled recordings', () => {
    const rows = activeRecordingsFromScheduledBots(
      [
        {
          id: 'event_1',
          summary: 'Product Review',
          status: 'confirmed',
          start: { dateTime: '2026-05-27T17:00:00.000Z' },
          end: { dateTime: '2026-05-27T17:30:00.000Z' },
          sourceCompanyUid: 'cmp_123',
        },
      ],
      new Map([
        [
          'event_1',
          {
            botId: 'bot_1',
            meetingUrl: 'https://meet.google.com/abc-defg-hij',
            platform: 'google_meet',
            status: 'recording',
            calendarEventId: 'event_1',
            meetingTitle: 'Product Review',
            scheduledStartTime: '2026-05-27T17:00:00.000Z',
            autoScheduled: true,
          },
        ],
        [
          'event_2',
          {
            botId: 'bot_2',
            meetingUrl: 'https://meet.google.com/abc-defg-hij',
            platform: 'google_meet',
            status: 'scheduled',
            calendarEventId: 'event_2',
            autoScheduled: true,
          },
        ],
      ]),
    );

    expect(rows).toEqual([
      expect.objectContaining({
        windowId: 'scheduled-bot:bot_1',
        state: 'recording',
        recordingId: 'bot_1',
        companyUid: 'cmp_123',
        summary: 'Product Review',
        sourceEventId: 'event_1',
      }),
    ]);
  });

  // `now` is a fixed local wall-clock reference. Event times are built from
  // local Date components and round-tripped through ISO so the local-day
  // comparison in dayLabel/groupByDay is stable regardless of the test TZ.
  const now = new Date(2026, 4, 27, 9, 0, 0); // Wed May 27 2026, 09:00 local

  function eventAt(id: string, local: Date): MeetingEvent {
    return {
      id,
      status: 'confirmed',
      start: { dateTime: local.toISOString() },
      end: { dateTime: new Date(local.getTime() + 30 * 60_000).toISOString() },
    };
  }

  it('labels days relative to now as Today / Tomorrow / dated', () => {
    expect(dayLabel(new Date(2026, 4, 27, 15, 0, 0), now)).toBe('Today');
    expect(dayLabel(new Date(2026, 4, 28, 8, 0, 0), now)).toBe('Tomorrow');

    const dated = dayLabel(new Date(2026, 4, 30, 8, 0, 0), now);
    expect(dated).not.toBe('Today');
    expect(dated).not.toBe('Tomorrow');
    expect(dated).toContain('30');
  });

  it('groups events into chronological per-day buckets', () => {
    const groups = groupByDay(
      [
        eventAt('today-late', new Date(2026, 4, 27, 16, 0, 0)),
        eventAt('tomorrow', new Date(2026, 4, 28, 10, 0, 0)),
        eventAt('today-early', new Date(2026, 4, 27, 9, 30, 0)),
      ],
      now,
    );

    expect(groups.map((g) => g.label)).toEqual(['Today', 'Tomorrow']);
    // Sorted within the day even though input order was late-then-early.
    expect(groups[0].events.map((e) => e.id)).toEqual(['today-early', 'today-late']);
    expect(groups[1].events.map((e) => e.id)).toEqual(['tomorrow']);
  });

  it('drops events with no parseable start from the day groups', () => {
    const groups = groupByDay(
      [
        eventAt('real', new Date(2026, 4, 27, 12, 0, 0)),
        { id: 'startless', status: 'confirmed', start: {}, end: {} },
      ],
      now,
    );

    expect(groups).toHaveLength(1);
    expect(groups[0].events.map((e) => e.id)).toEqual(['real']);
  });

  // US-010 — "Done — transcript saved" must be gated on the REAL source-landed
  // signal, not on bot.status === 'completed' alone. The two can diverge: the
  // bot lifecycle status flips on the Recall webhook / retry path while the
  // per-company source write is a separate S3 PUT that can hard-fail (the
  // 2026-06-02 KMS-grant drift dead-lettered transcripts for ~13 days while
  // bots still read "completed"). hq-pro exposes `sourceLanded`; the row gates
  // the terminal "done" affordance on it.
  describe('rowButtonKind — gates Done on the real source-landed signal', () => {
    function bot(overrides: Partial<ScheduledBot>): ScheduledBot {
      return {
        botId: 'bot-1',
        meetingUrl: 'https://meet.google.com/abc-defg-hij',
        platform: 'google_meet',
        status: 'completed',
        autoScheduled: true,
        ...overrides,
      };
    }

    it('no bot → invite', () => {
      expect(rowButtonKind(undefined)).toBe('invite');
    });

    it('REGRESSION: completed bot whose ingest FAILED (sourceLanded:false) does NOT render done', () => {
      // The exact #240 symptom from the user's POV — completed status, but the
      // transcript never landed as a per-company source. Must show processing,
      // never "Done — transcript saved".
      expect(rowButtonKind(bot({ status: 'completed', sourceLanded: false }))).toBe(
        'processing',
      );
    });

    it('REGRESSION: completed bot with sourceLanded absent (pre-US-010 server) does NOT render done', () => {
      // A backend that predates US-010 omits the field entirely. The client
      // must fail safe to "processing" rather than show a premature "saved".
      expect(rowButtonKind(bot({ status: 'completed' }))).toBe('processing');
    });

    it('completed bot whose source LANDED (sourceLanded:true) renders done', () => {
      expect(rowButtonKind(bot({ status: 'completed', sourceLanded: true }))).toBe(
        'done',
      );
    });

    it('source-landed only flips done at the completed status — earlier statuses are unaffected', () => {
      // sourceLanded true on a non-terminal status (shouldn't happen, but be
      // defensive) does NOT short-circuit the lifecycle to done.
      expect(rowButtonKind(bot({ status: 'scheduled', sourceLanded: true }))).toBe(
        'invited',
      );
      expect(rowButtonKind(bot({ status: 'joining', sourceLanded: true }))).toBe(
        'joining',
      );
      expect(rowButtonKind(bot({ status: 'recording', sourceLanded: true }))).toBe(
        'in-call',
      );
      expect(rowButtonKind(bot({ status: 'processing', sourceLanded: true }))).toBe(
        'processing',
      );
    });

    it('a failed/unknown status falls back to invite regardless of sourceLanded', () => {
      expect(rowButtonKind(bot({ status: 'failed', sourceLanded: false }))).toBe(
        'invite',
      );
    });
  });
});
