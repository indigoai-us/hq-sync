import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { describe, expect, it } from 'vitest';
import type { ActiveMeeting } from '../../src/lib/activeMeetings';
import {
  buildConnectedCalendarRows,
  eventEnd,
  eventStart,
  extractedSignalLabels,
  isToday,
  pickLiveMeeting,
  pickUpNext,
  signalCounts,
  sortByStart,
  totalSignalCounts,
  type MeetingEvent,
} from '../../src/desktop-alt/lib/meetings-model';

const meetingsPage = readFileSync(
  resolve(process.cwd(), 'src/desktop-alt/pages/MeetingsPage.svelte'),
  'utf8',
);
const liveNowCard = readFileSync(
  resolve(process.cwd(), 'src/desktop-alt/components/LiveNowCard.svelte'),
  'utf8',
);
const meetingsToday = readFileSync(
  resolve(process.cwd(), 'src/desktop-alt/components/MeetingsToday.svelte'),
  'utf8',
);
const activeMeetings = readFileSync(resolve(process.cwd(), 'src/lib/activeMeetings.ts'), 'utf8');

function normalize(source: string): string {
  return source.replace(/\s+/g, ' ');
}

function event(overrides: Partial<MeetingEvent> & Pick<MeetingEvent, 'id'>): MeetingEvent {
  return {
    summary: overrides.id,
    status: 'confirmed',
    start: { dateTime: '2026-05-27T17:00:00.000Z' },
    end: { dateTime: '2026-05-27T17:30:00.000Z' },
    ...overrides,
  };
}

describe('US-006: Alt Meetings page wires to existing detection + memberships', () => {
  it('shows the highest-priority active detection in Live now with working Record controls', () => {
    const meetings: ActiveMeeting[] = [
      {
        windowId: 'newer-detected',
        platform: 'zoom',
        meetingUrl: 'recall-window:newer-detected',
        detectedAt: '2026-05-27T19:00:00.000Z',
        state: 'detected',
        companyUid: null,
        summary: 'Newer detected call',
      },
      {
        windowId: 'recording-call',
        platform: 'meet',
        meetingUrl: 'recall-window:recording-call',
        detectedAt: '2026-05-27T18:00:00.000Z',
        state: 'recording',
        companyUid: null,
        summary: 'Recording call',
      },
    ];

    expect(pickLiveMeeting(meetings)?.windowId).toBe('recording-call');

    for (const eventName of [
      'meeting:detected',
      'recording:started',
      'recording:ended',
      'meeting:closed',
      'notification:meeting-action',
    ]) {
      expect(activeMeetings).toContain(`'${eventName}'`);
    }

    expect(meetingsPage).toContain('ensureActiveMeetingListeners()');
    expect(meetingsPage).toContain(
      'const liveMeeting = $derived(pickLiveMeeting([...cachedActiveRecordings, ...$activeMeetings]))',
    );
    expect(meetingsPage).toContain('activeRecordingsFromScheduledBots(events, botsByEventId)');
    expect(meetingsPage).toContain(
      '<LiveNowCard meeting={liveMeeting} onstart={startRecording} onstop={stopRecording} />',
    );

    const card = normalize(liveNowCard);
    expect(card).toContain("onclick={() => onstart(meeting.windowId)}");
    expect(card).toContain("onclick={() => onstop(meeting.windowId)}");
    expect(card).toContain('Start recording');
    expect(card).toContain('Stop recording');
    expect(activeMeetings).toContain("invoke<string>('start_recording'");
    expect(activeMeetings).toContain("await invoke('stop_recording'");
  });

  it('renders Today and Up next from meetingsCache in chronological order with cached signal totals and recent signals', () => {
    const now = new Date('2026-05-27T16:00:00.000Z');
    const events = [
      event({
        id: 'standup',
        summary: 'Standup',
        start: { dateTime: '2026-05-27T16:30:00.000Z' },
        end: { dateTime: '2026-05-27T16:45:00.000Z' },
        signals: { actions: ['Send notes'] },
      }),
      event({
        id: 'planning',
        summary: 'Planning',
        start: { dateTime: '2026-05-27T18:00:00.000Z' },
        end: { dateTime: '2026-05-27T18:45:00.000Z' },
        signals: [{ kind: 'decision', title: 'Use cached schedule' }],
      }),
      event({
        id: 'retro',
        summary: 'Retro',
        start: { dateTime: '2026-05-27T21:00:00.000Z' },
        end: { dateTime: '2026-05-27T21:30:00.000Z' },
        signals: { risks: ['Follow-up drift'] },
      }),
      event({
        id: 'review',
        summary: 'Review',
        start: { dateTime: '2026-05-27T20:00:00.000Z' },
        end: { dateTime: '2026-05-27T20:25:00.000Z' },
        signals: { decisions: ['Approved'], actionItems: ['File minutes'] },
      }),
      event({
        id: 'tomorrow',
        summary: 'Tomorrow',
        start: { dateTime: '2026-05-28T17:00:00.000Z' },
        end: { dateTime: '2026-05-28T17:30:00.000Z' },
        signals: { actions: ['Not counted today'] },
      }),
    ];

    const todayEvents = events.filter((row) => isToday(row, now)).sort(sortByStart);
    const recentlySynced = events
      .filter((row) => extractedSignalLabels(row).length > 0)
      .sort(
        (a, b) =>
          (eventEnd(b)?.getTime() ?? eventStart(b)?.getTime() ?? 0) -
          (eventEnd(a)?.getTime() ?? eventStart(a)?.getTime() ?? 0),
      )
      .slice(0, 3);

    expect(todayEvents.map((row) => row.id)).toEqual(['standup', 'planning', 'review', 'retro']);
    expect(pickUpNext(todayEvents, now)?.id).toBe('standup');
    expect(totalSignalCounts(todayEvents)).toEqual({ actions: 2, decisions: 2, risks: 1 });
    expect(signalCounts(todayEvents[2])).toEqual({ actions: 1, decisions: 1, risks: 0 });
    expect(recentlySynced.map((row) => row.id)).toEqual(['tomorrow', 'retro', 'review']);

    const page = normalize(meetingsPage);
    const today = normalize(meetingsToday);
    expect(page).toContain('loadMeetingsCache<MeetingEvent, ScheduledBot, GoogleAccount, GoogleCalendar>()');
    expect(page).toContain('const todayEvents = $derived(events.filter((event) => isToday(event)).sort(sortByStart))');
    expect(page).toContain('const upNext = $derived(pickUpNext(todayEvents))');
    expect(page).toContain('const signalTotals = $derived(totalSignalCounts(todayEvents))');
    expect(page).toContain('extractedSignalLabels(event).length > 0');
    expect(page).toContain('.slice(0, 3)');
    expect(today).toContain('{#each events as event (event.id)}');
    expect(`${page}\n${today}`).not.toContain("invoke<MeetingEvent[]>('meetings_list_upcoming'");
  });

  it('renders exactly the Personal connected calendar row when the user has no memberships', () => {
    const rows = buildConnectedCalendarRows(
      [{ accountId: 'personal-account', email: 'person@example.com' }],
      new Map([['personal-account', [{ id: 'primary', summary: 'Personal', primary: true }]]]),
      new Map([['personal-account', new Set(['primary'])]]),
      [
        event({
          id: 'personal-event',
          sourceAccountId: 'personal-account',
          sourceCalendarId: 'primary',
          sourceCompanyUid: null,
        }),
      ],
      [],
    );

    expect(rows).toEqual([
      {
        key: 'personal-account|primary',
        email: 'person@example.com',
        calendar: 'Personal',
        routingTarget: 'Personal',
        status: 'active',
      },
    ]);

    const page = normalize(meetingsPage);
    expect(page).toContain("invoke<CompanyMembership[]>('meetings_list_memberships')");
    expect(page).toContain('buildConnectedCalendarRows( accounts, calendarsByAccount, enabledCalIdsByAccount, events, memberships, )');
    expect(page).toContain('<strong>{row.email}</strong>');
    expect(page).toContain('{row.calendar} -> {row.routingTarget}');
    expect(page).toContain('<span class="status-pill">{row.status}</span>');
  });
});
