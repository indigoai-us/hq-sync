import { describe, expect, it } from 'vitest';
import { get } from 'svelte/store';
import {
  buildConnectedCalendarRows,
  pickLiveMeeting,
  totalSignalCounts,
  type MeetingEvent,
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
});
