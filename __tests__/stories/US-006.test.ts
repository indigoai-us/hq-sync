import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { describe, expect, it } from 'vitest';
import type { ActiveMeeting } from '../../src/lib/activeMeetings';
import {
  buildConnectedCalendarRows,
  eventEnd,
  eventStart,
  extractedSignalLabels,
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
// The network fetch, cache hydration, detection listeners, and 30s poll moved
// out of MeetingsPage into a module-level singleton store so the data survives
// the {#key routeKey} remount (preload + poll, not a per-nav blocking fetch).
// Assertions that pin those calls now read the store source, not the page.
const meetingsStore = readFileSync(
  resolve(process.cwd(), 'src/desktop-alt/lib/meetings-store.svelte.ts'),
  'utf8',
);
const liveNowCard = readFileSync(
  resolve(process.cwd(), 'src/desktop-alt/components/LiveNowCard.svelte'),
  'utf8',
);
const meetingsAgenda = readFileSync(
  resolve(process.cwd(), 'src/desktop-alt/components/MeetingsAgenda.svelte'),
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

    expect(meetingsStore).toContain('ensureActiveMeetingListeners()');
    expect(meetingsPage).toContain(
      'const liveMeeting = $derived(pickLiveMeeting([...cachedActiveRecordings, ...$activeMeetings]))',
    );
    expect(meetingsPage).toContain('activeRecordingsFromScheduledBots(events, botsByEventId)');
    // The Live now card now also receives the active memberships and an
    // oncompany callback so the user can attribute the recording (parity with
    // the classic popover). Normalized to tolerate the multi-line element.
    const page = normalize(meetingsPage);
    expect(page).toContain(
      '<LiveNowCard meeting={liveMeeting} memberships={$recordingMemberships} onstart={startRecording} onstop={stopRecording} oncompany={setRecordingCompany} />',
    );

    const card = normalize(liveNowCard);
    expect(card).toContain("onclick={() => onstart(meeting.windowId)}");
    expect(card).toContain("onclick={() => onstop(meeting.windowId)}");
    expect(card).toContain('Start recording');
    expect(card).toContain('Stop recording');
    expect(activeMeetings).toContain("invoke<string>('start_recording'");
    expect(activeMeetings).toContain("await invoke('stop_recording'");
    // Desktop-alt must also consume the classic popover snapshot. That
    // snapshot is the proven source old MeetingsWindow uses for the active
    // memberships list; without it the new Live now picker can degrade to
    // Personal-only while the old UX still shows companies.
    expect(activeMeetings).toContain("'popover:meetings-snapshot'");
    expect(activeMeetings).toContain("emit('meetings-window:request-snapshot')");
    expect(activeMeetings).toContain('setRecordingCompanyContext(');
    // Per-meeting recording-company picker: a "Record as" <select> that
    // delegates the choice through the oncompany callback (card stays
    // invoke-free — attribution helpers live in the store layer).
    expect(card).toContain('Record as');
    expect(card).toContain('onchange={(e) => oncompany(');
    expect(activeMeetings).toContain('setRecordingCompany');
    expect(activeMeetings).toContain('loadRecordingCompanyContext');
    expect(meetingsStore).toContain('loadRecordingCompanyContext');
  });

  it('renders every upcoming meeting grouped by day with Up next, cached signal totals, and recent signals', () => {
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
        signals: { actions: ['Counted now — multi-day'] },
      }),
    ];

    // Multi-day agenda: every upcoming event is shown (sorted chronologically),
    // not just today's. The old isToday() filter hid all non-today meetings,
    // which read as an empty view. `tomorrow` must now be included everywhere.
    const upcomingEvents = [...events].sort(sortByStart);
    const recentlySynced = events
      .filter((row) => extractedSignalLabels(row).length > 0)
      .sort(
        (a, b) =>
          (eventEnd(b)?.getTime() ?? eventStart(b)?.getTime() ?? 0) -
          (eventEnd(a)?.getTime() ?? eventStart(a)?.getTime() ?? 0),
      )
      .slice(0, 3);

    expect(upcomingEvents.map((row) => row.id)).toEqual([
      'standup',
      'planning',
      'review',
      'retro',
      'tomorrow',
    ]);
    expect(pickUpNext(upcomingEvents, now)?.id).toBe('standup');
    // `tomorrow`'s action is now counted (it was excluded under the today-only filter).
    expect(totalSignalCounts(upcomingEvents)).toEqual({ actions: 3, decisions: 2, risks: 1 });
    expect(signalCounts(upcomingEvents[2])).toEqual({ actions: 1, decisions: 1, risks: 0 });
    expect(recentlySynced.map((row) => row.id)).toEqual(['tomorrow', 'retro', 'review']);

    const page = normalize(meetingsPage);
    const agenda = normalize(meetingsAgenda);
    expect(meetingsStore).toContain('loadMeetingsCache<MeetingEvent, ScheduledBot, GoogleAccount, GoogleCalendar>()');
    expect(page).toContain('const upcomingEvents = $derived([...events].sort(sortByStart))');
    expect(page).toContain('const dayGroups = $derived(groupByDay(upcomingEvents))');
    expect(page).toContain('const upNext = $derived(pickUpNext(upcomingEvents))');
    expect(page).toContain('const signalTotals = $derived(totalSignalCounts(upcomingEvents))');
    expect(page).toContain('extractedSignalLabels(event).length > 0');
    expect(page).toContain('.slice(0, 3)');
    expect(page).toContain(
      '<MeetingsAgenda groups={dayGroups} {upNext} totalCount={upcomingEvents.length} companyNames={companyNamesByUid} {liveEventId} {botsByEventId} {pendingEventIds} {onInvite} {onUninvite} {onJoinNow} onOpenExternal={openExternal} />',
    );
    expect(agenda).toContain('{#each groups as group (group.label)}');
    expect(agenda).toContain('{#each group.events as event (event.id)}');
    // The store owns the network fetch via a typed invoke; the agenda subcomponent
    // is purely presentational and never touches Tauri/invoke.
    expect(meetingsStore).toContain("invoke<MeetingEvent[]>('meetings_list_upcoming')");
    expect(agenda).not.toContain('invoke');
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
    expect(meetingsStore).toContain("invoke<CompanyMembership[]>('meetings_list_memberships')");
    expect(page).toContain('buildConnectedCalendarRows( accounts, calendarsByAccount, enabledCalIdsByAccount, events, memberships, )');
    expect(page).toContain('<strong>{row.email}</strong>');
    expect(page).toContain('{row.calendar} -> {row.routingTarget}');
    expect(page).toContain('<span class="status-pill">{row.status}</span>');
  });
});
