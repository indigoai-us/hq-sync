<script lang="ts">
  import {
    companyLabel,
    durationMinutes,
    eventMeetingUrl,
    meetingState,
    rowButtonKind,
    signalCounts,
    signalSummary,
    timeLabel,
    type DayGroup,
    type MeetingEvent,
    type ScheduledBot,
  } from '../lib/meetings-model';

  interface Props {
    /** Pre-grouped, chronologically ordered day buckets (see groupByDay). */
    groups: DayGroup[];
    /** Up-next pick — used to mark the "Next" row (hero owns the big card). */
    upNext: MeetingEvent | null;
    /** Total upcoming meetings across all days — shown in the panel header. */
    totalCount: number;
    /** uid -> company display name, for each row's routing subtitle. */
    companyNames?: Map<string, string>;
    /** sourceEventId of the active/live meeting, if any — marks the "Live" row. */
    liveEventId?: string | null;
    /** calendarEventId -> active scheduled bot, drives the per-row action state. */
    botsByEventId?: Map<string, ScheduledBot>;
    /** event ids with an in-flight bot action — disables + spins that row. */
    pendingEventIds?: Set<string>;
    /** Bot-action callbacks. The store owns the network call; this stays presentational. */
    onInvite?: (evt: MeetingEvent) => void;
    onUninvite?: (evt: MeetingEvent) => void;
    onJoinNow?: (evt: MeetingEvent) => void;
    /** Open a meeting URL in the system browser (Tauri shell open, passed in). */
    onOpenExternal?: (url: string) => void;
  }

  let {
    groups,
    upNext,
    totalCount,
    companyNames = new Map(),
    liveEventId = null,
    botsByEventId = new Map(),
    pendingEventIds = new Set(),
    onInvite = () => {},
    onUninvite = () => {},
    onJoinNow = () => {},
    onOpenExternal = () => {},
  }: Props = $props();

  const upNextId = $derived(upNext?.id ?? null);
</script>

<section class="agenda-panel" aria-labelledby="agenda-title">
  <div class="panel-header">
    <h2 id="agenda-title">Upcoming</h2>
    <span>{totalCount} meeting{totalCount === 1 ? '' : 's'}</span>
  </div>

  {#each groups as group (group.label)}
    <h3 class="day-heading">{group.label}</h3>
    <div class="card meeting-card">
      {#each group.events as event (event.id)}
        {@const state = meetingState(event, { liveEventId, upNextId })}
        {@const dur = durationMinutes(event)}
        {@const sig = signalSummary(signalCounts(event))}
        {@const bot = botsByEventId.get(event.id)}
        {@const pending = pendingEventIds.has(event.id)}
        {@const kind = rowButtonKind(bot)}
        {@const url = eventMeetingUrl(event)}
        <div class="meeting-row" class:past={state === 'past'}>
          <div class="mtime">
            {timeLabel(event)}{#if dur}<span class="mdur"> &middot; {dur}m</span>{/if}
          </div>
          <div class="mmeta">
            <div class="mname">
              {#if state === 'live'}<span class="dot-live" aria-hidden="true">&#9679;</span>{:else if state === 'next'}<span
                  class="arrow-next"
                  aria-hidden="true">&#8593;</span
                >{/if}{event.summary ?? '(no title)'}
            </div>
            <div class="mcompany">{companyLabel(event, companyNames)}</div>
          </div>
          <div class="msig">{sig}</div>
          <div class="mstate">
            {#if state === 'live'}
              <span class="pill live">Live</span>
            {:else if state === 'next'}
              <span class="pill">Next</span>
            {:else if state === 'past'}
              <span class="pill ok"><span class="check" aria-hidden="true">&#10003;</span> Synced</span>
            {:else}
              <span class="pill">Scheduled</span>
            {/if}
          </div>
          <!-- Action cluster: Open (browser) + per-state bot button + join-now.
               Icon-only; the rich state lives in colour + tooltip so the row
               stays dense. The store owns the network call — these are pure
               callbacks, keeping this component presentational. -->
          <div class="mactions">
            {#if url}
              <button
                type="button"
                class="row-icon-btn row-icon-join"
                title="Open meeting in browser"
                aria-label="Open meeting in browser"
                onclick={() => onOpenExternal(url)}
              >
                <svg width="12" height="12" viewBox="0 0 12 12" fill="none" aria-hidden="true">
                  <path d="M4 2h6v6M10 2L4.5 7.5M2 4v6h6" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round" />
                </svg>
              </button>
            {/if}
            {#if !url}
              <span class="row-icon-btn row-icon-empty" title="No meeting URL on this event">—</span>
            {:else if kind === 'invite'}
              <button
                type="button"
                class="row-icon-btn row-icon-invite"
                disabled={pending}
                title={pending ? 'Inviting…' : 'Invite bot to this meeting'}
                aria-label="Invite bot"
                onclick={() => onInvite(event)}
              >
                {#if pending}
                  <span class="row-icon-spinner" aria-hidden="true"></span>
                {:else}
                  <svg width="12" height="12" viewBox="0 0 12 12" fill="none" aria-hidden="true">
                    <path d="M6 2v8M2 6h8" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" />
                  </svg>
                {/if}
              </button>
            {:else if kind === 'invited'}
              <button
                type="button"
                class="row-icon-btn row-icon-invited"
                disabled={pending}
                title={pending ? 'Cancelling…' : 'Bot scheduled — click to uninvite'}
                aria-label="Uninvite bot"
                onclick={() => onUninvite(event)}
              >
                {#if pending}
                  <span class="row-icon-spinner" aria-hidden="true"></span>
                {:else}
                  <svg width="12" height="12" viewBox="0 0 12 12" fill="none" aria-hidden="true">
                    <path d="M2.5 6.5L5 9L9.5 3.5" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" />
                  </svg>
                {/if}
              </button>
            {:else if kind === 'in-call'}
              <button
                type="button"
                class="row-icon-btn row-icon-incall"
                disabled={pending}
                title={pending ? 'Removing bot…' : 'Bot is in the meeting — click to remove'}
                aria-label="Remove bot from meeting"
                onclick={() => onUninvite(event)}
              >
                {#if pending}
                  <span class="row-icon-spinner" aria-hidden="true"></span>
                {:else}
                  <span class="live-dot" aria-hidden="true"></span>
                {/if}
              </button>
            {:else if kind === 'joining'}
              <button
                type="button"
                class="row-icon-btn row-icon-joining"
                disabled={pending}
                title={pending ? 'Cancelling…' : 'Bot is joining — click to cancel'}
                aria-label="Cancel bot join"
                onclick={() => onUninvite(event)}
              >
                <span class="row-icon-spinner row-icon-spinner-amber" aria-hidden="true"></span>
              </button>
            {:else if kind === 'processing'}
              <span class="row-icon-btn row-icon-processing" title="Processing transcript">
                <svg width="12" height="12" viewBox="0 0 12 12" fill="currentColor" aria-hidden="true">
                  <circle cx="2.5" cy="6" r="1" />
                  <circle cx="6" cy="6" r="1" />
                  <circle cx="9.5" cy="6" r="1" />
                </svg>
              </span>
            {:else}
              <span class="row-icon-btn row-icon-done" title="Done — transcript saved">
                <svg width="12" height="12" viewBox="0 0 12 12" fill="none" aria-hidden="true">
                  <path d="M2.5 6.5L5 9L9.5 3.5" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" />
                </svg>
              </span>
            {/if}
            {#if url}
              <button
                type="button"
                class="row-icon-btn row-icon-bot-now"
                disabled={pending}
                title={pending ? 'Telling bot to join…' : 'Tell bot to join now'}
                aria-label="Tell bot to join now"
                onclick={() => onJoinNow(event)}
              >
                {#if pending}
                  <span class="row-icon-spinner" aria-hidden="true"></span>
                {:else}
                  <svg width="12" height="12" viewBox="0 0 12 12" fill="none" aria-hidden="true">
                    <line x1="6" y1="1" x2="6" y2="2.5" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" />
                    <rect x="2" y="3" width="8" height="6.5" rx="1.5" stroke="currentColor" stroke-width="1.4" />
                    <circle cx="4.6" cy="6.5" r="0.7" fill="currentColor" />
                    <circle cx="7.4" cy="6.5" r="0.7" fill="currentColor" />
                  </svg>
                {/if}
              </button>
            {/if}
          </div>
        </div>
      {/each}
    </div>
  {:else}
    <div class="card meeting-card">
      <div class="meeting-row empty-row">No meetings in your synced calendars yet.</div>
    </div>
  {/each}
</section>

<style>
  .agenda-panel {
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
    font-size: var(--text-base);
    font-weight: 600;
    line-height: 20px;
  }

  .panel-header span {
    color: var(--muted);
    font-size: var(--text-base);
  }

  /* Day separator above each per-day card. Ported from the classic
     MeetingsWindow day-heading, retoned to the desktop-alt token palette. */
  .day-heading {
    margin: 14px 0 6px;
    color: var(--muted);
    font-size: var(--text-micro);
    font-weight: 600;
    letter-spacing: 0.08em;
    line-height: 16px;
    text-transform: uppercase;
  }

  .day-heading:first-of-type {
    margin-top: 0;
  }

  /* Card wrapping a day's rows — mirrors prototype `.card`. */
  .meeting-card {
    border: 1px solid var(--border);
    border-radius: 8px;
    background: var(--bg);
    overflow: hidden;
  }

  /* `.meeting-row`: 5-col grid — time / name+company / signals / status pill /
     action cluster. The 5th column (`.mactions`) is the parity addition over
     the informational design row: Open + bot state-machine + join-now. */
  .meeting-row {
    display: grid;
    grid-template-columns: 100px minmax(0, 1fr) auto auto auto;
    gap: 14px;
    align-items: center;
    padding: 9px 16px;
    border-top: 1px solid var(--border);
    transition: background-color 140ms ease;
  }

  .meeting-row:first-child {
    border-top: none;
  }

  .meeting-row:not(.empty-row):hover {
    background: var(--row-hover);
  }

  .meeting-row.past {
    opacity: 0.62;
  }

  .mtime {
    color: var(--muted);
    font-family: var(--font-mono);
    font-size: var(--text-base);
    white-space: nowrap;
  }

  .mtime .mdur {
    color: var(--muted-3);
  }

  .mmeta {
    min-width: 0;
  }

  .mname {
    overflow: hidden;
    color: var(--fg);
    font-size: var(--text-base);
    line-height: 18px;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .dot-live {
    margin-right: 6px;
    color: var(--emerald);
  }

  .arrow-next {
    margin-right: 6px;
    color: var(--muted);
  }

  .mcompany {
    overflow: hidden;
    color: var(--muted);
    font-size: var(--text-base);
    line-height: 16px;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .msig {
    color: var(--muted-3);
    font-size: var(--text-base);
    white-space: nowrap;
  }

  .pill {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 2px 8px;
    border: 1px solid var(--border);
    border-radius: 999px;
    color: var(--muted);
    font-size: var(--text-base);
    line-height: 16px;
    white-space: nowrap;
  }

  .pill.ok {
    color: var(--emerald);
    border-color: rgba(52, 211, 153, 0.22);
  }

  .pill.live {
    color: var(--emerald);
    border-color: rgba(52, 211, 153, 0.22);
  }

  .pill.live::before {
    content: '';
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--emerald);
    box-shadow: 0 0 8px rgba(52, 211, 153, 0.7);
    animation: livePulse 2s ease-in-out infinite;
  }

  .pill .check {
    font-size: var(--text-base);
  }

  /* ── Action cluster (parity 5th column) ───────────────────────────────
     Icon-only buttons ported from classic MeetingsWindow.row-actions, with
     base neutrals retoned to the desktop-alt token palette. The status
     colour vocabulary (red live / amber joining / blue processing / green
     done) is preserved; tooltips carry meaning so icon-only stays a11y-safe. */
  .mactions {
    flex: 0 0 auto;
    display: inline-flex;
    align-items: center;
    justify-content: flex-end;
    gap: 4px;
  }

  .row-icon-btn {
    flex: 0 0 auto;
    width: 24px;
    height: 24px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    padding: 0;
    border: 1px solid var(--border);
    border-radius: 5px;
    background: var(--row-hover);
    color: var(--muted-2);
    cursor: pointer;
    transition: background 120ms ease, color 120ms ease, border-color 120ms ease;
  }
  .row-icon-btn:hover:not(:disabled) {
    background: rgba(255, 255, 255, 0.1);
    border-color: var(--border-strong);
    color: var(--fg);
  }
  .row-icon-btn:focus-visible {
    outline: 2px solid rgba(180, 180, 255, 0.7);
    outline-offset: 1px;
  }
  .row-icon-btn:disabled {
    opacity: 0.6;
    cursor: wait;
  }

  /* No URL — inert placeholder, keeps the trailing column aligned. */
  .row-icon-empty {
    color: var(--muted-3);
    background: transparent;
    border-color: transparent;
    cursor: default;
    font-size: var(--text-base);
  }
  /* Open-in-browser — discreet so the eye lands on the state button first. */
  .row-icon-join {
    color: var(--muted-2);
    background: transparent;
    border-color: var(--border);
  }
  /* Invite CTA — brighter so it reads as actionable. */
  .row-icon-invite {
    color: var(--fg);
    background: rgba(255, 255, 255, 0.12);
    border-color: rgba(255, 255, 255, 0.28);
  }
  .row-icon-invite:hover:not(:disabled) {
    background: rgba(255, 255, 255, 0.2);
  }
  /* Invited — muted check; hover hints at the uninvite affordance. */
  .row-icon-invited {
    color: var(--muted-2);
  }
  .row-icon-invited:hover:not(:disabled) {
    color: #fca5a5;
    background: rgba(220, 38, 38, 0.12);
    border-color: rgba(220, 38, 38, 0.4);
  }
  /* In-call — red tint broadcasts "live" at a glance. */
  .row-icon-incall {
    color: #fca5a5;
    background: rgba(220, 38, 38, 0.12);
    border-color: rgba(220, 38, 38, 0.4);
  }
  .row-icon-incall:hover:not(:disabled) {
    background: rgba(220, 38, 38, 0.22);
  }
  /* Joining — amber spinner; transient. */
  .row-icon-joining {
    color: #fcd34d;
    background: rgba(202, 138, 4, 0.1);
    border-color: rgba(202, 138, 4, 0.4);
  }
  /* Processing — muted blue; non-interactive. */
  .row-icon-processing {
    color: #93c5fd;
    background: rgba(59, 130, 246, 0.08);
    border-color: rgba(59, 130, 246, 0.3);
    cursor: default;
  }
  /* Done — muted green; non-interactive. */
  .row-icon-done {
    color: var(--emerald);
    background: rgba(52, 211, 153, 0.08);
    border-color: rgba(52, 211, 153, 0.3);
    cursor: default;
  }
  /* Join-now — amber-accented "act now", distinct from state colours. */
  .row-icon-bot-now {
    color: #fcd34d;
    background: rgba(202, 138, 4, 0.08);
    border-color: rgba(202, 138, 4, 0.32);
  }
  .row-icon-bot-now:hover:not(:disabled) {
    background: rgba(202, 138, 4, 0.18);
    border-color: rgba(202, 138, 4, 0.55);
  }

  .live-dot {
    display: inline-block;
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: #ef4444;
    box-shadow: 0 0 0 0 rgba(239, 68, 68, 0.7);
    animation: live-pulse 1.6s ease-out infinite;
  }
  @keyframes live-pulse {
    0% {
      box-shadow: 0 0 0 0 rgba(239, 68, 68, 0.55);
    }
    70% {
      box-shadow: 0 0 0 6px rgba(239, 68, 68, 0);
    }
    100% {
      box-shadow: 0 0 0 0 rgba(239, 68, 68, 0);
    }
  }

  /* Inline spinner while a request is pending. 12px box matches the SVG
     icons so the button doesn't resize when state flips. */
  .row-icon-spinner {
    width: 12px;
    height: 12px;
    border-radius: 50%;
    border: 1.5px solid currentColor;
    border-right-color: transparent;
    animation: row-icon-spin 0.7s linear infinite;
    opacity: 0.85;
  }
  .row-icon-spinner-amber {
    color: #fcd34d;
  }
  @keyframes row-icon-spin {
    to {
      transform: rotate(360deg);
    }
  }

  .empty-row {
    display: block;
    color: var(--muted);
    font-size: var(--text-base);
    line-height: 18px;
  }

  @keyframes livePulse {
    0%,
    100% {
      opacity: 1;
    }
    50% {
      opacity: 0.45;
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .meeting-row {
      transition: none;
    }

    .pill.live::before {
      animation: none;
    }
  }

  @media (max-width: 520px) {
    .meeting-row {
      grid-template-columns: minmax(0, 1fr) auto;
      gap: 4px 12px;
    }

    .mtime {
      grid-column: 1;
    }

    .mstate {
      grid-column: 2;
      grid-row: 1;
      justify-self: end;
    }

    .mmeta {
      grid-column: 1 / -1;
    }

    .msig {
      grid-column: 1 / -1;
      white-space: normal;
    }

    /* Actions drop to their own full-width row, right-aligned and tappable. */
    .mactions {
      grid-column: 1 / -1;
      justify-content: flex-end;
    }
  }
</style>
