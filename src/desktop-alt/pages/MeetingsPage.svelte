<script lang="ts">
  import { invoke } from '@tauri-apps/api/core';
  import { onMount } from 'svelte';
  import { activeMeetings, ensureActiveMeetingListeners, startRecording, stopRecording } from '../../lib/activeMeetings';
  import { loadMeetingsCache } from '../../lib/meetingsCache';
  import LiveNowCard from '../components/LiveNowCard.svelte';
  import MeetingsToday from '../components/MeetingsToday.svelte';
  import {
    buildConnectedCalendarRows,
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

  let events = $state<MeetingEvent[]>([]);
  let accounts = $state<GoogleAccount[]>([]);
  let calendarsByAccount = $state<Map<string, GoogleCalendar[]>>(new Map());
  let enabledCalIdsByAccount = $state<Map<string, Set<string>>>(
    new Map(),
  );
  let memberships = $state<CompanyMembership[]>([]);
  let membershipsError = $state('');

  const liveMeeting = $derived(pickLiveMeeting($activeMeetings));
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
    hydrateFromCache();
    void ensureActiveMeetingListeners();
    void loadMemberships();

    const refreshCachedSchedule = () => hydrateFromCache();
    window.addEventListener('focus', refreshCachedSchedule);
    window.addEventListener('storage', refreshCachedSchedule);
    return () => {
      window.removeEventListener('focus', refreshCachedSchedule);
      window.removeEventListener('storage', refreshCachedSchedule);
    };
  });

  function hydrateFromCache() {
    const snapshot = loadMeetingsCache<MeetingEvent, ScheduledBot, GoogleAccount, GoogleCalendar>();
    events = snapshot?.events ?? [];
    accounts = snapshot?.accounts ?? [];
    calendarsByAccount = new Map(snapshot?.calendarsByAccount ?? []);
    enabledCalIdsByAccount = new Map(
      (snapshot?.enabledCalIdsByAccount ?? []).map(([accountId, ids]) => [
        accountId,
        new Set(ids),
      ]),
    );
  }

  async function loadMemberships() {
    membershipsError = '';
    try {
      memberships = await invoke<CompanyMembership[]>('meetings_list_memberships');
    } catch (err) {
      console.error('meetings_list_memberships failed:', err);
      membershipsError = 'Could not load calendar routing.';
    }
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
    border-bottom: 1px solid #e4e4e7;
  }

  .hero-main {
    min-width: 0;
  }

  .hero-kicker,
  .hero-current {
    margin: 0;
    color: #71717a;
    font-size: 12px;
    line-height: 18px;
  }

  .meetings-hero h1 {
    margin: 2px 0 4px;
    color: #18181b;
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

  .hero-metrics {
    display: grid;
    grid-template-columns: repeat(3, minmax(0, 1fr));
    gap: 10px;
  }

  .metric {
    min-width: 0;
    padding: 12px;
    border: 1px solid #e4e4e7;
    border-radius: 8px;
    background: #ffffff;
  }

  .metric span {
    display: block;
    color: #71717a;
    font-size: 11px;
    font-weight: 650;
    line-height: 16px;
    text-transform: uppercase;
  }

  .metric strong {
    color: #18181b;
    font-size: 21px;
    font-weight: 680;
    line-height: 28px;
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
    color: #18181b;
    font-size: 15px;
    font-weight: 680;
    line-height: 22px;
  }

  .panel-header span,
  .routing-copy span,
  .timeline-list span,
  .panel-error,
  .empty-row {
    color: #71717a;
    font-size: 12px;
    line-height: 18px;
  }

  .panel-error {
    margin: 0 0 8px;
    color: #9f1239;
  }

  .routing-list,
  .timeline-list {
    display: grid;
    gap: 0;
    margin: 0;
    padding: 6px 0;
    border: 1px solid #e4e4e7;
    border-radius: 8px;
    background: #ffffff;
    list-style: none;
  }

  .routing-list li {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto;
    align-items: center;
    gap: 10px;
    padding: 9px 12px;
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
    color: #18181b;
    font-size: 13px;
    font-weight: 650;
    line-height: 18px;
  }

  .status-pill {
    max-width: 96px;
    overflow: hidden;
    padding: 3px 7px;
    border-radius: 999px;
    background: #f4f4f5;
    color: #52525b;
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
  }

  .timeline-dot {
    width: 8px;
    height: 8px;
    margin-top: 5px;
    border-radius: 999px;
    background: #0f766e;
    box-shadow: 0 0 0 3px rgb(15 118 110 / 0.12);
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
</style>
