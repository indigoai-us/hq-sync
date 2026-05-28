<script lang="ts">
  import { invoke } from '@tauri-apps/api/core';
  import { onMount } from 'svelte';
  import {
    activeMeetings,
    ensureActiveMeetingListeners,
    startRecording,
    stopRecording,
    type ActiveMeeting,
  } from '../../lib/activeMeetings';
  import { loadMeetingsCache, saveMeetingsCache } from '../../lib/meetingsCache';
  import LiveNowCard from '../components/LiveNowCard.svelte';
  import MeetingsToday from '../components/MeetingsToday.svelte';
  import {
    buildConnectedCalendarRows,
    activeRecordingsFromScheduledBots,
    eventEnd,
    eventStart,
    extractedSignalLabels,
    isToday,
    pickLiveMeeting,
    pickUpNext,
    sortByStart,
    totalSignalCounts,
    type CompanyMembership,
    type GoogleAccount,
    type GoogleCalendar,
    type MeetingEvent,
    type ScheduledBot,
  } from '../lib/meetings-model';

  /** Return shape of `meetings_list_calendars_for_account` — the per-account
   *  calendar list plus the user's enabled selection. Mirrors the inline type
   *  in the classic MeetingsWindow (not exported from the model). */
  interface AccountCalendars {
    calendars: GoogleCalendar[];
    selectedCalendarIds: string[];
  }

  let events = $state<MeetingEvent[]>([]);
  let accounts = $state<GoogleAccount[]>([]);
  let calendarsByAccount = $state<Map<string, GoogleCalendar[]>>(new Map());
  let enabledCalIdsByAccount = $state<Map<string, Set<string>>>(
    new Map(),
  );
  // Bot map drives the "recording" pills the alt page surfaces from the
  // calendar snapshot (distinct from the live `meeting:detected` channel,
  // which is owned by ensureActiveMeetingListeners and left untouched).
  let botsByEventId = $state<Map<string, ScheduledBot>>(new Map());
  // Persisted alongside the rest of the snapshot so the classic window and
  // the alt window stay in lockstep on the shared meetingsCache key.
  let companyNamesByUid = $state<Map<string, string>>(new Map());
  let accountEmailById = $state<Map<string, string>>(new Map());
  let calendarSummaryByKey = $state<Map<string, string>>(new Map());
  let memberships = $state<CompanyMembership[]>([]);
  let membershipsError = $state('');
  // Surfaced when the live calendar fetch fails outright (vs. cache miss):
  // we keep the stale paint on screen and show this rather than faking an
  // empty state. Auth failures are special-cased to a "sign in again" hint.
  let fetchError = $state('');
  let loading = $state(false);

  // Recordings inferred from the calendar snapshot's scheduled bots. Derived
  // (not manually assigned) so it recomputes whenever the cache-first paint or
  // the live network refresh swaps `events`/`botsByEventId`.
  const cachedActiveRecordings = $derived(
    activeRecordingsFromScheduledBots(events, botsByEventId),
  );

  const liveMeeting = $derived(pickLiveMeeting([...cachedActiveRecordings, ...$activeMeetings]));
  const todayEvents = $derived(events.filter((event) => isToday(event)).sort(sortByStart));
  const upNext = $derived(pickUpNext(todayEvents));
  const signalTotals = $derived(totalSignalCounts(todayEvents));
  const connectedRows = $derived(
    buildConnectedCalendarRows(
      accounts,
      calendarsByAccount,
      enabledCalIdsByAccount,
      events,
      memberships,
    ),
  );
  const recentlySynced = $derived(
    events
      .filter((event) => extractedSignalLabels(event).length > 0)
      .sort((a, b) => (eventEnd(b)?.getTime() ?? eventStart(b)?.getTime() ?? 0) - (eventEnd(a)?.getTime() ?? eventStart(a)?.getTime() ?? 0))
      .slice(0, 3),
  );

  onMount(() => {
    // Cache-first: paint the last good snapshot synchronously (instant, even
    // if stale), then revalidate from the network — same stale-while-revalidate
    // contract the classic MeetingsWindow uses. The alt window has its own
    // per-window localStorage, so without the network refresh below the cache
    // was always empty here (the original bug).
    hydrateFromCache();
    void ensureActiveMeetingListeners();
    void refresh();

    // Re-hydrate on focus/storage so a refresh in the classic window (or a
    // prior alt session) reflects here immediately; `refresh()` on focus then
    // pulls fresh data. The `loading` guard dedupes the mount+focus pair.
    const onFocus = () => {
      hydrateFromCache();
      void refresh();
    };
    const onStorage = () => hydrateFromCache();
    window.addEventListener('focus', onFocus);
    window.addEventListener('storage', onStorage);
    return () => {
      window.removeEventListener('focus', onFocus);
      window.removeEventListener('storage', onStorage);
    };
  });

  function hydrateFromCache() {
    const snapshot = loadMeetingsCache<MeetingEvent, ScheduledBot, GoogleAccount, GoogleCalendar>();
    if (!snapshot) return;
    events = snapshot.events ?? [];
    botsByEventId = new Map(snapshot.botsByEventId ?? []);
    companyNamesByUid = new Map(snapshot.companyNamesByUid ?? []);
    accounts = snapshot.accounts ?? [];
    accountEmailById = new Map(snapshot.accountEmailById ?? []);
    calendarsByAccount = new Map(snapshot.calendarsByAccount ?? []);
    calendarSummaryByKey = new Map(snapshot.calendarSummaryByKey ?? []);
    enabledCalIdsByAccount = new Map(
      (snapshot.enabledCalIdsByAccount ?? []).map(([accountId, ids]) => [
        accountId,
        new Set(ids),
      ]),
    );
  }

  /**
   * Live fetch — mirrors MeetingsWindow.svelte's mount `refresh()`. The alt
   * window must fetch calendar data itself: it can't piggyback on the classic
   * window's cache because localStorage is per-WebviewWindow. Fetches
   * events + bots + memberships + connected accounts in parallel, fans out to
   * per-account calendars, populates the model, and persists the snapshot.
   *
   * Errors are NOT swallowed to a blank state: on failure we keep whatever the
   * cache already painted, log the error, and surface a message (auth failures
   * prompt re-sign-in) instead of faking "0 meetings".
   */
  async function refresh() {
    if (loading) return;
    loading = true;
    fetchError = '';
    membershipsError = '';
    try {
      const [evts, bots, members, accts] = await Promise.all([
        invoke<MeetingEvent[]>('meetings_list_upcoming'),
        invoke<ScheduledBot[]>('meetings_list_scheduled_bots', {
          calendarEventIds: null,
        }),
        invoke<CompanyMembership[]>('meetings_list_memberships').catch((err) => {
          console.error('meetings_list_memberships failed:', err);
          membershipsError = 'Could not load calendar routing.';
          return [] as CompanyMembership[];
        }),
        invoke<GoogleAccount[]>('meetings_list_accounts').catch(
          () => [] as GoogleAccount[],
        ),
      ]);
      events = evts ?? [];
      botsByEventId = buildBotMap(bots ?? []);
      memberships = members ?? [];
      companyNamesByUid = buildCompanyNameMap(members ?? []);
      accounts = accts ?? [];
      accountEmailById = new Map(
        (accts ?? []).map((a) => [a.accountId, a.email ?? '']),
      );

      // Calendar fan-out is a second pass so the events render doesn't block
      // on calendar metadata; per-account failures are non-fatal.
      await loadCalendarsForAccounts(accts ?? []);

      // Persist AFTER everything (events + calendars) so the next paint — in
      // either window — hydrates a complete view.
      persistSnapshot();
    } catch (err) {
      // Keep the cached paint; surface the failure rather than blanking out.
      console.error('meetings refresh failed:', err);
      fetchError = friendlyFetchError(err);
    } finally {
      loading = false;
    }
  }

  async function loadCalendarsForAccounts(accts: GoogleAccount[]) {
    const nextByAccount = new Map<string, GoogleCalendar[]>();
    const nextEnabled = new Map<string, Set<string>>();
    const nextSummaries = new Map<string, string>();
    await Promise.all(
      accts.map(async (a) => {
        try {
          const resp = await invoke<AccountCalendars>(
            'meetings_list_calendars_for_account',
            { accountId: a.accountId },
          );
          nextByAccount.set(a.accountId, resp.calendars ?? []);
          nextEnabled.set(a.accountId, new Set(resp.selectedCalendarIds ?? []));
          for (const c of resp.calendars ?? []) {
            nextSummaries.set(`${a.accountId}|${c.id}`, c.summary);
          }
        } catch (err) {
          console.error(
            `meetings_list_calendars_for_account failed for ${a.accountId}:`,
            err,
          );
          nextByAccount.set(a.accountId, []);
          nextEnabled.set(a.accountId, new Set());
        }
      }),
    );
    calendarsByAccount = nextByAccount;
    enabledCalIdsByAccount = nextEnabled;
    calendarSummaryByKey = nextSummaries;
  }

  function persistSnapshot(): void {
    saveMeetingsCache<MeetingEvent, ScheduledBot, GoogleAccount, GoogleCalendar>({
      events,
      botsByEventId: Array.from(botsByEventId.entries()),
      companyNamesByUid: Array.from(companyNamesByUid.entries()),
      accounts,
      accountEmailById: Array.from(accountEmailById.entries()),
      calendarsByAccount: Array.from(calendarsByAccount.entries()),
      enabledCalIdsByAccount: Array.from(enabledCalIdsByAccount.entries()).map(
        ([acct, ids]) => [acct, Array.from(ids)],
      ),
      calendarSummaryByKey: Array.from(calendarSummaryByKey.entries()),
    });
  }

  function buildBotMap(bots: ScheduledBot[]): Map<string, ScheduledBot> {
    const m = new Map<string, ScheduledBot>();
    for (const b of bots) {
      if (b.calendarEventId && isActiveStatus(b.status)) {
        m.set(b.calendarEventId, b);
      }
    }
    return m;
  }

  function buildCompanyNameMap(rows: CompanyMembership[]): Map<string, string> {
    const m = new Map<string, string>();
    for (const row of rows) {
      if (row.companyName) m.set(row.companyUid, row.companyName);
    }
    return m;
  }

  function isActiveStatus(s: string): boolean {
    return (
      s === 'scheduled' ||
      s === 'joining' ||
      s === 'recording' ||
      s === 'processing' ||
      s === 'completed'
    );
  }

  function friendlyFetchError(err: unknown): string {
    const raw = String(err ?? '');
    if (/\b401\b/.test(raw) || /auth/i.test(raw)) {
      return 'Sign in again to load meetings.';
    }
    return 'Could not refresh meetings — showing the last cached view.';
  }
</script>

<section class="meetings-page" aria-label="Meetings">
  <div class="meetings-hero">
    <div class="hero-main">
      <p class="hero-kicker">Calendar cache / menubar truth</p>
      <h1>Meetings</h1>
      <p class="hero-current">
        {todayEvents.length} on deck today / {signalTotals.actions + signalTotals.decisions + signalTotals.risks} signals extracted
      </p>
      {#if fetchError}
        <p class="hero-error" role="status">{fetchError}</p>
      {/if}
    </div>
    <div class="hero-metrics" aria-label="Meeting signal counts">
      <div class="metric">
        <span>Actions</span>
        <strong>{signalTotals.actions}</strong>
      </div>
      <div class="metric">
        <span>Decisions</span>
        <strong>{signalTotals.decisions}</strong>
      </div>
      <div class="metric">
        <span>Risks</span>
        <strong>{signalTotals.risks}</strong>
      </div>
    </div>
  </div>

  <div class="meetings-grid">
    <div class="main-column">
      <LiveNowCard meeting={liveMeeting} onstart={startRecording} onstop={stopRecording} />
      <MeetingsToday events={todayEvents} {upNext} />
    </div>

    <aside class="side-column">
      <section class="routing-panel" aria-labelledby="calendars-title">
        <div class="panel-header">
          <h2 id="calendars-title">Connected calendars</h2>
          <span>{connectedRows.length}</span>
        </div>
        {#if membershipsError}
          <p class="panel-error">{membershipsError}</p>
        {/if}
        <ol class="routing-list">
          {#each connectedRows as row (row.key)}
            <li>
              <div class="routing-copy">
                <strong>{row.email}</strong>
                <span>{row.calendar} -> {row.routingTarget}</span>
              </div>
              <span class="status-pill">{row.status}</span>
            </li>
          {:else}
            <li class="empty-row">No connected calendars in the cached snapshot.</li>
          {/each}
        </ol>
      </section>

      <section class="timeline-panel" aria-labelledby="synced-title">
        <div class="panel-header">
          <h2 id="synced-title">Recently synced</h2>
          <span>{recentlySynced.length}</span>
        </div>
        <ol class="timeline-list">
          {#each recentlySynced as event (event.id)}
            {@const labels = extractedSignalLabels(event)}
            <li>
              <span class="timeline-dot" aria-hidden="true"></span>
              <div>
                <strong>{event.summary ?? '(no title)'}</strong>
                <span>{labels.join(' / ')}</span>
              </div>
            </li>
          {:else}
            <li class="empty-row">Extracted meeting signals will appear after sync.</li>
          {/each}
        </ol>
      </section>
    </aside>
  </div>
</section>

<style>
  .meetings-page {
    display: grid;
    gap: 22px;
  }

  .meetings-hero {
    display: grid;
    grid-template-columns: minmax(0, 1fr) minmax(260px, 360px);
    gap: 16px 24px;
    padding-bottom: 22px;
    border-bottom: 1px solid var(--border);
  }

  .hero-main {
    min-width: 0;
  }

  .hero-kicker,
  .hero-current {
    margin: 0;
    color: var(--muted);
    font-size: 12px;
    line-height: 18px;
  }

  .meetings-hero h1 {
    margin: 2px 0 4px;
    color: var(--fg);
    font-size: 28px;
    font-weight: 680;
    letter-spacing: 0;
    line-height: 34px;
  }

  .hero-current {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .hero-error {
    margin: 6px 0 0;
    color: var(--red);
    font-size: 12px;
    line-height: 18px;
  }

  .hero-metrics {
    display: grid;
    grid-template-columns: repeat(3, minmax(0, 1fr));
    gap: 10px;
  }

  .metric {
    min-width: 0;
    padding: 12px;
    border: 1px solid var(--border);
    border-radius: 8px;
    background: var(--bg);
  }

  .metric span {
    display: block;
    color: var(--muted);
    font-size: 11px;
    font-weight: 650;
    line-height: 16px;
    text-transform: uppercase;
  }

  .metric strong {
    display: block;
    min-width: 0;
    overflow: hidden;
    color: var(--fg);
    font-size: 21px;
    font-weight: 680;
    line-height: 28px;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .meetings-grid {
    display: grid;
    grid-template-columns: minmax(0, 1fr) minmax(270px, 320px);
    align-items: start;
    gap: 22px;
  }

  .main-column,
  .side-column {
    display: grid;
    gap: 18px;
    min-width: 0;
  }

  .panel-header {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 12px;
    margin-bottom: 10px;
  }

  .panel-header h2 {
    margin: 0;
    color: var(--fg);
    font-size: 15px;
    font-weight: 680;
    line-height: 22px;
  }

  .panel-header span,
  .routing-copy span,
  .timeline-list span,
  .panel-error,
  .empty-row {
    color: var(--muted);
    font-size: 12px;
    line-height: 18px;
  }

  .panel-error {
    margin: 0 0 8px;
    color: var(--red);
  }

  .routing-list,
  .timeline-list {
    display: grid;
    gap: 0;
    margin: 0;
    padding: 6px 0;
    border: 1px solid var(--border);
    border-radius: 8px;
    background: var(--bg);
    list-style: none;
  }

  .routing-list li {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto;
    align-items: center;
    gap: 10px;
    padding: 9px 12px;
    transition:
      background 140ms cubic-bezier(.2, .7, .2, 1),
      transform 140ms cubic-bezier(.2, .7, .2, 1);
  }

  .routing-list li:not(.empty-row):hover,
  .timeline-list li:not(.empty-row):hover {
    background: var(--row-hover);
    transform: translateX(2px);
  }

  .routing-copy {
    min-width: 0;
  }

  .routing-copy strong,
  .routing-copy span,
  .timeline-list strong,
  .timeline-list span {
    display: block;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .routing-copy strong,
  .timeline-list strong {
    color: var(--fg);
    font-size: 13px;
    font-weight: 650;
    line-height: 18px;
  }

  .status-pill {
    max-width: 96px;
    overflow: hidden;
    padding: 3px 7px;
    border-radius: 999px;
    background: var(--row-active);
    color: var(--muted-2);
    font-size: 11px;
    font-weight: 650;
    line-height: 14px;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .timeline-list li {
    display: grid;
    grid-template-columns: 12px minmax(0, 1fr);
    gap: 8px;
    padding: 9px 12px;
    transition:
      background 140ms cubic-bezier(.2, .7, .2, 1),
      transform 140ms cubic-bezier(.2, .7, .2, 1);
  }

  .timeline-dot {
    width: 8px;
    height: 8px;
    margin-top: 5px;
    border-radius: 999px;
    background: var(--emerald);
    box-shadow: 0 0 0 3px rgba(52, 211, 153, 0.16);
  }

  .routing-list .empty-row,
  .timeline-list .empty-row {
    display: block;
  }

  @media (max-width: 980px) {
    .meetings-hero,
    .meetings-grid {
      grid-template-columns: minmax(0, 1fr);
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .routing-list li,
    .timeline-list li {
      transition: none;
    }

    .routing-list li:not(.empty-row):hover,
    .timeline-list li:not(.empty-row):hover {
      transform: none;
    }
  }

  @media (max-width: 520px) {
    .hero-metrics {
      grid-template-columns: minmax(0, 1fr);
    }

    .routing-list li {
      grid-template-columns: minmax(0, 1fr);
      align-items: start;
      gap: 6px;
    }

    .status-pill {
      justify-self: start;
      max-width: 100%;
    }
  }
</style>
