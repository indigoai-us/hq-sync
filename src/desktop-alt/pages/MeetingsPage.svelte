<script lang="ts">
  import { open as openExternal } from '@tauri-apps/plugin-shell';
  import { onMount } from 'svelte';
  import {
    activeMeetings,
    recordingMemberships,
    setRecordingCompany,
    startRecording,
    stopRecording,
  } from '../../lib/activeMeetings';
  import {
    meetingsStore,
    startMeetingsStore,
    type ToastDescriptor,
  } from '../lib/meetings-store.svelte';
  import LiveNowCard from '../components/LiveNowCard.svelte';
  import MeetingsAgenda from '../components/MeetingsAgenda.svelte';
  import {
    buildConnectedCalendarRows,
    activeRecordingsFromScheduledBots,
    companyLabel,
    durationMinutes,
    eventEnd,
    eventStart,
    extractedSignalLabels,
    groupByDay,
    pickLiveMeeting,
    pickUpNext,
    sortByStart,
    timeLabel,
    totalSignalCounts,
    type MeetingEvent,
  } from '../lib/meetings-model';

  // Store-backed data. The singleton (started at app launch in
  // DesktopApp.onMount) loads once + polls every 30s, so this page is a thin
  // consumer: it reads the already-warm store instead of running a blocking
  // network fetch on every nav remount — which is what made the page take
  // 5-10s to paint. Aliased through $derived so the presentation derives below
  // — and the US-006 source-contract strings — stay unchanged.
  const events = $derived(meetingsStore.events);
  const botsByEventId = $derived(meetingsStore.botsByEventId);
  const accounts = $derived(meetingsStore.accounts);
  const calendarsByAccount = $derived(meetingsStore.calendarsByAccount);
  const enabledCalIdsByAccount = $derived(meetingsStore.enabledCalIdsByAccount);
  const companyNamesByUid = $derived(meetingsStore.companyNamesByUid);
  const memberships = $derived(meetingsStore.memberships);
  const membershipsError = $derived(meetingsStore.membershipsError);
  const fetchError = $derived(meetingsStore.fetchError);
  const loading = $derived(meetingsStore.loading);
  // Per-row in-flight set for bot actions, owned by the store. Passed to the
  // agenda so each row can disable its buttons + spin while its invoke runs.
  const pendingEventIds = $derived(meetingsStore.pendingEventIds);

  // Recordings inferred from the calendar snapshot's scheduled bots. Derived
  // (not manually assigned) so it recomputes whenever the cache-first paint or
  // the live network refresh swaps `events`/`botsByEventId`.
  const cachedActiveRecordings = $derived(
    activeRecordingsFromScheduledBots(events, botsByEventId),
  );

  const liveMeeting = $derived(pickLiveMeeting([...cachedActiveRecordings, ...$activeMeetings]));
  // The calendar event id behind the live detection, so the agenda can mark
  // exactly that row "Live" (recall bots carry the originating event id).
  const liveEventId = $derived(liveMeeting?.sourceEventId ?? null);
  // Multi-day agenda: `meetings_list_upcoming` already returns events across
  // the server's sync window, so we show them all grouped by day rather than
  // narrowing to today (the old `isToday` filter hid every non-today meeting,
  // which read as an empty "no meetings" view).
  const upcomingEvents = $derived([...events].sort(sortByStart));
  const dayGroups = $derived(groupByDay(upcomingEvents));
  const upNext = $derived(pickUpNext(upcomingEvents));
  const signalTotals = $derived(totalSignalCounts(upcomingEvents));
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

  // Upcoming meetings carrying >=1 extracted signal — powers "from N meetings" caption.
  const signalMeetingCount = $derived(
    upcomingEvents.filter((event) => extractedSignalLabels(event).length > 0).length,
  );

  // Transient action feedback. The store owns the invoke + decides the copy
  // (returns a ToastDescriptor next to the call that produced it); this page
  // only renders it. `null` = nothing to surface (no-op dedupe / missing bot).
  let toast = $state<ToastDescriptor | null>(null);
  function flashToast(kind: 'info' | 'warn', text: string): void {
    toast = { kind, text };
    setTimeout(() => {
      if (toast && toast.text === text) toast = null;
    }, 4000);
  }

  // Thin wrappers: delegate the invoke to the store, surface its toast (if any).
  // The agenda calls these via callback props so it stays 'invoke'-free.
  async function onInvite(evt: MeetingEvent): Promise<void> {
    const t = await meetingsStore.inviteBot(evt);
    if (t) flashToast(t.kind, t.text);
  }
  async function onUninvite(evt: MeetingEvent): Promise<void> {
    const t = await meetingsStore.cancelBot(evt);
    if (t) flashToast(t.kind, t.text);
  }
  async function onJoinNow(evt: MeetingEvent): Promise<void> {
    const t = await meetingsStore.joinBotNow(evt);
    if (t) flashToast(t.kind, t.text);
  }

  function openCalendar(): void {
    void openExternal('https://calendar.google.com');
  }

  onMount(() => {
    // The store is a module-level singleton started once at app launch from
    // DesktopApp.onMount. Calling it here too keeps the page self-sufficient
    // for isolated mounts (tests / direct nav); it's idempotent via an internal
    // `started` guard, so this never double-fetches or double-polls. The
    // cache-first paint, live refresh, 30s poll, and focus/storage listeners
    // all live in the store now — this remount just reads the already-warm
    // singleton, which is what makes the nav instant instead of 5-10s.
    startMeetingsStore();
  });
</script>

<div class="meetings" aria-label="Meetings">
  {#snippet iconCalendar()}
    <svg viewBox="0 0 24 24" width="13" height="13" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
      <rect x="3" y="4" width="18" height="18" rx="2" />
      <path d="M16 2v4M8 2v4M3 10h18" />
    </svg>
  {/snippet}
  {#snippet iconSync()}
    <svg viewBox="0 0 24 24" width="13" height="13" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
      <path d="M21 12a9 9 0 0 0-15-6.7L3 8" />
      <path d="M3 12a9 9 0 0 0 15 6.7L21 16" />
      <path d="M3 3v5h5" />
      <path d="M21 21v-5h-5" />
    </svg>
  {/snippet}

  <header class="page-header">
    <div class="ph-titles">
      <h1>Meetings</h1>
      <div class="subtitle">
        {upcomingEvents.length} upcoming across {dayGroups.length} day{dayGroups.length === 1 ? '' : 's'} · all companies
      </div>
      {#if fetchError}
        <div class="page-error" role="status">{fetchError}</div>
      {/if}
    </div>
    <div class="actions">
      <button type="button" class="btn subtle" onclick={openCalendar}>
        <span class="icon">{@render iconCalendar()}</span>
        Open calendar
      </button>
      <button type="button" class="btn" onclick={() => void meetingsStore.refresh()} disabled={loading}>
        <span class="icon">{@render iconSync()}</span>
        {loading ? 'Refreshing' : 'Refresh'}
      </button>
    </div>
  </header>

  {#if toast}
    <div class="toast" class:toast-warn={toast.kind === 'warn'} role="status">{toast.text}</div>
  {/if}

  <div class="content">
    <div class="three-col">
      <LiveNowCard
        meeting={liveMeeting}
        memberships={$recordingMemberships}
        onstart={startRecording}
        onstop={stopRecording}
        oncompany={setRecordingCompany}
      />

      <div class="card">
        <div class="card-header">
          <h3>Up next</h3>
          <span>{upNext ? timeLabel(upNext) : ''}</span>
        </div>
        <div class="card-body">
          {#if upNext}
            {@const dur = durationMinutes(upNext)}
            <div class="un-name">{upNext.summary ?? '(no title)'}</div>
            <div class="un-meta">
              {companyLabel(upNext, companyNamesByUid)}{#if dur} · {dur}m{/if}
            </div>
          {:else}
            <div class="card-empty">Nothing scheduled next.</div>
          {/if}
        </div>
      </div>

      <div class="card">
        <div class="card-header">
          <h3>Signal pool</h3>
          <span>extracted</span>
        </div>
        <div class="card-body">
          <div class="sp-stats">
            <div class="sp-stat">
              <span class="sp-num">{signalTotals.actions}</span>
              <span class="sp-lbl">Actions</span>
            </div>
            <div class="sp-stat">
              <span class="sp-num">{signalTotals.decisions}</span>
              <span class="sp-lbl">Decisions</span>
            </div>
            <div class="sp-stat">
              <span class="sp-num">{signalTotals.risks}</span>
              <span class="sp-lbl">Risks</span>
            </div>
          </div>
          <div class="sp-sub">from {signalMeetingCount} meeting{signalMeetingCount === 1 ? '' : 's'}</div>
        </div>
      </div>
    </div>

    <MeetingsAgenda
      groups={dayGroups}
      {upNext}
      totalCount={upcomingEvents.length}
      companyNames={companyNamesByUid}
      {liveEventId}
      {botsByEventId}
      {pendingEventIds}
      {onInvite}
      {onUninvite}
      {onJoinNow}
      onOpenExternal={openExternal}
    />

    <div class="section two-col">
      <div class="card">
        <div class="card-header">
          <h3>Connected calendars</h3>
          <span>{connectedRows.length}</span>
        </div>
        {#if membershipsError}
          <p class="card-error">{membershipsError}</p>
        {/if}
        <div class="sync-list">
          {#each connectedRows as row (row.key)}
            <div class="sync-source">
              <span class="icon-wrap">{@render iconCalendar()}</span>
              <div class="ss-copy">
                <strong>{row.email}</strong>
                <span class="sub">{row.calendar} -> {row.routingTarget}</span>
              </div>
              <span class="status-pill">{row.status}</span>
            </div>
          {:else}
            {#if accounts.length === 0}
              <div class="card-empty no-accounts">
                <div class="na-title">No calendars connected yet</div>
                <p class="na-copy">Connect a Google Calendar in HQ Console to start capturing meetings here.</p>
                <button type="button" class="btn" onclick={() => void openExternal('https://hq.getindigo.ai/integrations')}>Open HQ Console Integrations</button>
              </div>
            {:else}
              <div class="card-empty">No connected calendars in the cached snapshot.</div>
            {/if}
          {/each}
        </div>
      </div>

      <div class="card">
        <div class="card-header">
          <h3>Recently synced</h3>
          <span>{recentlySynced.length}</span>
        </div>
        <div class="card-body">
          {#if recentlySynced.length > 0}
            <div class="timeline">
              {#each recentlySynced as event (event.id)}
                {@const labels = extractedSignalLabels(event)}
                <div class="tl-row blue">
                  <div class="tl-copy">
                    <div class="what">{event.summary ?? '(no title)'}</div>
                    <div class="who">{labels.join(' / ')}</div>
                  </div>
                </div>
              {/each}
            </div>
          {:else}
            <div class="card-empty">Extracted meeting signals will appear after sync.</div>
          {/if}
        </div>
      </div>
    </div>
  </div>
</div>

<style>
  .meetings { min-width: 0; }

  .page-header {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 16px;
  }
  .ph-titles { min-width: 0; }
  .subtitle { margin-top: 4px; color: var(--muted); font-size: var(--text-base); line-height: 18px; }
  .page-error { margin-top: 6px; color: var(--red); font-size: var(--text-base); line-height: 18px; }
  .actions { display: flex; flex-shrink: 0; align-items: center; gap: 8px; }

  /* Transient action feedback. Amber (not red) for recoverable bot-action
     warnings — mirrors classic MeetingsWindow's deliberate `.toast-warn` tone. */
  .toast {
    margin: 12px 0 0; padding: 8px 12px; border: 1px solid rgba(52, 211, 153, 0.22);
    border-radius: 6px; background: rgba(52, 211, 153, 0.08);
    color: var(--emerald); font-size: var(--text-base); line-height: 18px;
  }
  .toast-warn {
    border-color: rgba(202, 138, 4, 0.35); background: rgba(202, 138, 4, 0.1); color: #fcd34d;
  }

  .btn {
    display: inline-flex; align-items: center; gap: 6px;
    padding: 5px 10px; border: 1px solid var(--border); border-radius: 5px;
    background: transparent; color: var(--fg); font: inherit; font-size: var(--text-base);
    white-space: nowrap; cursor: default;
    transition: background 140ms cubic-bezier(.2,.7,.2,1), border-color 140ms cubic-bezier(.2,.7,.2,1);
  }
  .btn:hover:not(:disabled) { border-color: var(--border-strong); background: var(--row-hover); }
  .btn:disabled { opacity: 0.5; }
  .btn.subtle { border-color: transparent; color: var(--muted); }
  .btn.subtle:hover:not(:disabled) { background: var(--row-hover); color: var(--fg); }
  .btn .icon { display: flex; align-items: center; justify-content: center; width: 14px; height: 14px; }

  .content { display: flex; flex-direction: column; gap: 28px; }

  .three-col { display: grid; grid-template-columns: 1.6fr 1fr 1fr; gap: 14px; align-items: start; }

  .card { min-width: 0; border: 1px solid var(--border); border-radius: 8px; background: var(--bg); }
  .card-header {
    display: flex; align-items: baseline; justify-content: space-between;
    gap: 12px; padding: 12px 16px; border-bottom: 1px solid var(--border);
  }
  .card-header h3 { margin: 0; color: var(--muted); font-size: var(--text-base); font-weight: 600; line-height: 18px; }
  .card-header > span { flex: 0 0 auto; color: var(--muted-3); font-size: var(--text-base); line-height: 18px; }
  .card-body { padding: 14px 16px; }
  .card-empty { padding: 14px 16px; color: var(--muted); font-size: var(--text-base); line-height: 18px; }
  .card-error { margin: 0; padding: 10px 16px 0; color: var(--red); font-size: var(--text-base); line-height: 18px; }

  .no-accounts { display: flex; flex-direction: column; align-items: flex-start; gap: 8px; }
  .na-title { color: var(--fg); font-size: var(--text-base); font-weight: 600; line-height: 18px; }
  .na-copy { margin: 0; color: var(--muted); font-size: var(--text-base); line-height: 18px; }

  .un-name { overflow: hidden; color: var(--fg); font-size: var(--text-base); font-weight: 600; line-height: 20px; text-overflow: ellipsis; white-space: nowrap; }
  .un-meta { margin-top: 4px; color: var(--muted); font-size: var(--text-base); line-height: 18px; }

  .sp-stats { display: flex; gap: 24px; }
  .sp-stat { display: flex; flex-direction: column; gap: 2px; }
  .sp-num { color: var(--fg); font-family: var(--font-mono); font-size: var(--text-base); font-weight: 600; line-height: 26px; }
  .sp-lbl { color: var(--muted); font-size: var(--text-micro); font-weight: 600; line-height: 14px; text-transform: uppercase; }
  .sp-sub { margin-top: 12px; color: var(--muted-3); font-size: var(--text-base); line-height: 18px; }

  .section { min-width: 0; }
  .two-col { display: grid; grid-template-columns: 1.4fr 1fr; gap: 14px; align-items: start; }

  .sync-list { display: flex; flex-direction: column; }
  .sync-source {
    display: grid; grid-template-columns: 22px minmax(0, 1fr) auto; align-items: center;
    gap: 14px; padding: 11px 16px; border-top: 1px solid var(--border);
    transition: background 140ms cubic-bezier(.2,.7,.2,1);
  }
  .sync-source:first-child { border-top: none; }
  .sync-source:hover { background: var(--row-hover); }
  .icon-wrap {
    display: flex; align-items: center; justify-content: center; width: 22px; height: 22px;
    border: 1px solid var(--border); border-radius: 6px; color: var(--muted);
  }
  .ss-copy { min-width: 0; }
  .ss-copy strong { display: block; overflow: hidden; color: var(--fg); font-size: var(--text-base); font-weight: 600; line-height: 18px; text-overflow: ellipsis; white-space: nowrap; }
  .ss-copy .sub { display: block; overflow: hidden; color: var(--muted-3); font-size: var(--text-base); line-height: 16px; text-overflow: ellipsis; white-space: nowrap; }
  .status-pill {
    max-width: 110px; overflow: hidden; padding: 2px 8px; border: 1px solid var(--border);
    border-radius: 999px; color: var(--muted); font-size: var(--text-base); font-weight: 600; line-height: 16px;
    text-overflow: ellipsis; white-space: nowrap;
  }

  .timeline { position: relative; padding-left: 18px; }
  .timeline::before { content: ''; position: absolute; left: 4px; top: 4px; bottom: 4px; width: 1px; background: var(--border); }
  .tl-row { position: relative; padding: 6px 0; }
  .tl-row::before { content: ''; position: absolute; left: -18px; top: 11px; width: 7px; height: 7px; border-radius: 50%; background: var(--muted-3); border: 2px solid var(--bg); }
  .tl-row.blue::before { background: var(--blue); }
  .tl-copy { min-width: 0; }
  .what { overflow: hidden; color: var(--fg); font-size: var(--text-base); line-height: 18px; text-overflow: ellipsis; white-space: nowrap; }
  .who { margin-top: 2px; overflow: hidden; color: var(--muted); font-size: var(--text-base); line-height: 16px; text-overflow: ellipsis; white-space: nowrap; }

  @media (max-width: 980px) { .three-col, .two-col { grid-template-columns: minmax(0, 1fr); } }
  @media (prefers-reduced-motion: reduce) { .btn, .sync-source { transition: none; } }
</style>
