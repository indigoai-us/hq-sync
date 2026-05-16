<script lang="ts">
  /**
   * Meetings modal — URL-input invite + upcoming-meetings list with per-row
   * Invite/Uninvite. Mounts inside App.svelte; opens via the MeetingIcon in
   * Popover header (SYNC-2). Feature-gated upstream — by the time we render
   * here, the user is already on the @getindigo.ai allowlist.
   *
   * Data flow:
   *   open=true → fetch meetings_list_upcoming + meetings_list_scheduled_bots
   *               in parallel
   *             → build a Map<calendarEventId, ScheduledBot> for O(1) lookup
   *   click Invite → meetings_invite_bot(url, eventId, companyId?)
   *               → optimistic flip to Uninvite + refresh bots
   *   click Uninvite → meetings_cancel_bot(botId)
   *               → optimistic flip to Invite + refresh bots
   *   paste URL + Invite → meetings_invite_bot(url, undefined, undefined)
   *               → no companyId → meeting lands in personal vault
   */

  import { invoke } from '@tauri-apps/api/core';

  interface MeetingEvent {
    id: string;
    summary?: string;
    start: { dateTime?: string; date?: string; timeZone?: string };
    end: { dateTime?: string; date?: string; timeZone?: string };
    status: string;
    hangoutLink?: string;
    sourceCalendarId?: string;
    sourceCompanyUid?: string;
  }

  interface ScheduledBot {
    botId: string;
    meetingUrl: string;
    platform: string;
    status: string;
    calendarEventId?: string | null;
    meetingTitle?: string | null;
    scheduledStartTime?: string | null;
    autoScheduled: boolean;
    errorMessage?: string | null;
  }

  interface Props {
    open: boolean;
    onclose: () => void;
  }
  let { open, onclose }: Props = $props();

  let events = $state<MeetingEvent[]>([]);
  let botsByEventId = $state<Map<string, ScheduledBot>>(new Map());
  let loading = $state(false);
  let listError = $state<string | null>(null);
  let toast = $state<{ kind: 'info' | 'error'; text: string } | null>(null);

  let urlInput = $state('');
  let urlInviting = $state(false);

  // Per-row in-flight state — keyed by event id (calendar rows) or 'url:<u>'
  // (manual URL rows) so multiple clicks don't double-fire.
  let rowPending = $state<Set<string>>(new Set());

  // Refresh whenever the modal transitions open → true.
  $effect(() => {
    if (open) {
      void refresh();
    } else {
      // Stop accumulating state between opens.
      events = [];
      botsByEventId = new Map();
      listError = null;
      toast = null;
      urlInput = '';
      urlInviting = false;
      rowPending = new Set();
    }
  });

  function onkeydown(e: KeyboardEvent) {
    if (e.key === 'Escape' && open) {
      e.preventDefault();
      onclose();
    }
  }

  function onBackdropClick(e: MouseEvent) {
    if (e.target === e.currentTarget) onclose();
  }

  async function refresh() {
    loading = true;
    listError = null;
    try {
      const [evts, bots] = await Promise.all([
        invoke<MeetingEvent[]>('meetings_list_upcoming'),
        invoke<ScheduledBot[]>('meetings_list_scheduled_bots', {
          calendarEventIds: null,
        }),
      ]);
      events = evts ?? [];
      botsByEventId = buildBotMap(bots ?? []);
    } catch (err) {
      listError = String(err);
    } finally {
      loading = false;
    }
  }

  function buildBotMap(bots: ScheduledBot[]): Map<string, ScheduledBot> {
    const m = new Map<string, ScheduledBot>();
    for (const b of bots) {
      // Only consider bots that are still scheduled/in-progress for the
      // invite/uninvite toggle. Cancelled/failed bots show as "Invite" again.
      if (b.calendarEventId && isActiveStatus(b.status)) {
        m.set(b.calendarEventId, b);
      }
    }
    return m;
  }

  function isActiveStatus(s: string): boolean {
    // hq-pro statuses: scheduled | in-progress | failed | completed | recording.
    // "failed" includes cancelled bots (errorMessage="Cancelled by user").
    return s === 'scheduled' || s === 'in-progress' || s === 'recording';
  }

  async function onInvite(evt: MeetingEvent) {
    const url = evt.hangoutLink;
    if (!url) {
      flashToast('error', 'No meeting URL on this event.');
      return;
    }
    const key = evt.id;
    if (rowPending.has(key)) return;
    rowPending = new Set(rowPending).add(key);
    try {
      await invoke<ScheduledBot>('meetings_invite_bot', {
        meetingUrl: url,
        calendarEventId: evt.id,
        companyId: evt.sourceCompanyUid ?? null,
      });
      flashToast('info', 'Bot invited.');
      await refresh();
    } catch (err) {
      flashToast('error', `Invite failed: ${err}`);
    } finally {
      const next = new Set(rowPending);
      next.delete(key);
      rowPending = next;
    }
  }

  async function onUninvite(evt: MeetingEvent) {
    const bot = botsByEventId.get(evt.id);
    if (!bot) return;
    const key = evt.id;
    if (rowPending.has(key)) return;
    rowPending = new Set(rowPending).add(key);
    try {
      await invoke('meetings_cancel_bot', { botId: bot.botId });
      flashToast('info', 'Bot uninvited.');
      await refresh();
    } catch (err) {
      flashToast('error', `Uninvite failed: ${err}`);
    } finally {
      const next = new Set(rowPending);
      next.delete(key);
      rowPending = next;
    }
  }

  async function onUrlInvite() {
    const url = urlInput.trim();
    if (!isPlausibleMeetingUrl(url)) return;
    urlInviting = true;
    try {
      await invoke<ScheduledBot>('meetings_invite_bot', {
        meetingUrl: url,
        calendarEventId: null,
        companyId: null, // off-calendar invites land in personal vault
      });
      urlInput = '';
      flashToast(
        'info',
        'Bot invited — meeting will save to Personal. You can move it after sync.',
      );
      await refresh();
    } catch (err) {
      flashToast('error', `Invite failed: ${err}`);
    } finally {
      urlInviting = false;
    }
  }

  function isPlausibleMeetingUrl(url: string): boolean {
    if (!url) return false;
    // Permissive — better to let hq-pro/Recall.ai validate than reject a
    // valid URL here. Match the major platforms.
    return (
      /^https:\/\/[^\s/]*\.zoom\.us\/j\/[^\s]+/i.test(url) ||
      /^https:\/\/meet\.google\.com\/[a-z-]+/i.test(url) ||
      /^https:\/\/teams\.microsoft\.com\/l\/meetup-join\/[^\s]+/i.test(url) ||
      /^https:\/\/[^\s/]*\.webex\.com\/[^\s]+/i.test(url)
    );
  }

  function flashToast(kind: 'info' | 'error', text: string) {
    toast = { kind, text };
    // Auto-dismiss after a few seconds so it doesn't linger across opens.
    setTimeout(() => {
      if (toast && toast.text === text) toast = null;
    }, 4000);
  }

  // ── Grouping by day for display ─────────────────────────────────────

  interface DayGroup {
    label: string;
    events: MeetingEvent[];
  }

  const groupedEvents = $derived<DayGroup[]>(groupByDay(events));

  function groupByDay(list: MeetingEvent[]): DayGroup[] {
    const out: DayGroup[] = [];
    const byLabel = new Map<string, MeetingEvent[]>();
    for (const e of list) {
      const t = eventStart(e);
      if (!t) continue;
      const label = dayLabel(t);
      let bucket = byLabel.get(label);
      if (!bucket) {
        bucket = [];
        byLabel.set(label, bucket);
      }
      bucket.push(e);
    }
    // Preserve insertion order (events arrive in start-time order).
    for (const [label, eventsInDay] of byLabel) {
      out.push({ label, events: eventsInDay });
    }
    return out;
  }

  function eventStart(e: MeetingEvent): Date | null {
    const raw = e.start.dateTime ?? e.start.date;
    if (!raw) return null;
    const d = new Date(raw);
    return Number.isNaN(d.getTime()) ? null : d;
  }

  function dayLabel(d: Date): string {
    const now = new Date();
    const today = new Date(now.getFullYear(), now.getMonth(), now.getDate());
    const tomorrow = new Date(today);
    tomorrow.setDate(tomorrow.getDate() + 1);
    const eventDay = new Date(d.getFullYear(), d.getMonth(), d.getDate());
    if (eventDay.getTime() === today.getTime()) return 'Today';
    if (eventDay.getTime() === tomorrow.getTime()) return 'Tomorrow';
    return d.toLocaleDateString(undefined, {
      weekday: 'short',
      month: 'short',
      day: 'numeric',
    });
  }

  function timeLabel(e: MeetingEvent): string {
    const d = eventStart(e);
    if (!d) return '';
    return d.toLocaleTimeString(undefined, {
      hour: 'numeric',
      minute: '2-digit',
    });
  }

  function companyLabel(e: MeetingEvent): string {
    // For v1, render the raw uid prefix as a hint. Real human-readable names
    // would require a memberships fetch — deferred to a follow-up so we
    // don't bloat the modal's first paint.
    if (!e.sourceCompanyUid) return 'Personal';
    const short = e.sourceCompanyUid.slice(0, 12);
    return short.length === 12 ? `${short}…` : short;
  }

  function platformLabel(e: MeetingEvent): string {
    const url = e.hangoutLink ?? '';
    if (url.includes('meet.google.com')) return 'Google Meet';
    if (url.includes('zoom.us')) return 'Zoom';
    if (url.includes('teams.microsoft.com')) return 'Teams';
    if (url.includes('webex.com')) return 'Webex';
    return '';
  }
</script>

<svelte:window {onkeydown} />

{#if open}
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div
    class="meetings-backdrop"
    onclick={onBackdropClick}
    role="dialog"
    aria-modal="true"
    aria-label="Upcoming meetings"
  >
    <div class="meetings-shell">
      <header class="meetings-header">
        <h2>Upcoming Meetings</h2>
        <button
          type="button"
          class="meetings-close"
          aria-label="Close meetings"
          onclick={onclose}
        >
          ×
        </button>
      </header>

      <!-- URL-input row — invite to ad-hoc meeting (SYNC-4). -->
      <div class="url-invite-row">
        <input
          type="url"
          inputmode="url"
          autocomplete="off"
          spellcheck="false"
          placeholder="Paste a Zoom or Google Meet URL"
          bind:value={urlInput}
          disabled={urlInviting}
          class="url-input"
          onkeydown={(e) => {
            if (e.key === 'Enter' && isPlausibleMeetingUrl(urlInput.trim())) {
              e.preventDefault();
              void onUrlInvite();
            }
          }}
        />
        <button
          type="button"
          class="url-invite-btn"
          disabled={urlInviting || !isPlausibleMeetingUrl(urlInput.trim())}
          onclick={onUrlInvite}
        >
          {urlInviting ? 'Inviting…' : 'Invite'}
        </button>
      </div>
      {#if urlInput.trim() && !isPlausibleMeetingUrl(urlInput.trim())}
        <p class="url-hint">Enter a Zoom, Google Meet, Teams, or Webex URL.</p>
      {/if}

      {#if toast}
        <p class="toast" class:toast-error={toast.kind === 'error'}>
          {toast.text}
        </p>
      {/if}

      <section class="meetings-body">
        {#if loading}
          <p class="meetings-placeholder">Loading…</p>
        {:else if listError}
          <p class="meetings-error">{listError}</p>
        {:else if events.length === 0}
          <p class="meetings-placeholder">
            No upcoming meetings in your connected calendars.
          </p>
        {:else}
          {#each groupedEvents as group (group.label)}
            <h3 class="day-heading">{group.label}</h3>
            <ul class="event-list">
              {#each group.events as evt (evt.id)}
                {@const bot = botsByEventId.get(evt.id)}
                {@const hasBot = bot !== undefined}
                {@const pending = rowPending.has(evt.id)}
                <li class="event-row">
                  <div class="event-meta">
                    <span class="event-time">{timeLabel(evt)}</span>
                    <span class="event-title" title={evt.summary ?? ''}>
                      {evt.summary ?? '(no title)'}
                    </span>
                    <span class="event-badges">
                      <span class="badge badge-company">{companyLabel(evt)}</span>
                      {#if platformLabel(evt)}
                        <span class="badge badge-platform">{platformLabel(evt)}</span>
                      {/if}
                    </span>
                  </div>
                  {#if !evt.hangoutLink}
                    <span class="row-disabled" title="No meeting URL on this event">—</span>
                  {:else if hasBot}
                    <button
                      type="button"
                      class="row-btn row-btn-uninvite"
                      disabled={pending}
                      onclick={() => onUninvite(evt)}
                    >
                      {pending ? '…' : 'Uninvite'}
                    </button>
                  {:else}
                    <button
                      type="button"
                      class="row-btn row-btn-invite"
                      disabled={pending}
                      onclick={() => onInvite(evt)}
                    >
                      {pending ? '…' : 'Invite'}
                    </button>
                  {/if}
                </li>
              {/each}
            </ul>
          {/each}
        {/if}
      </section>
    </div>
  </div>
{/if}

<style>
  .meetings-backdrop {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.55);
    display: flex;
    align-items: flex-start;
    justify-content: center;
    z-index: 50;
    padding: 24px 12px 12px;
    overflow-y: auto;
  }
  .meetings-shell {
    width: 100%;
    max-width: 380px;
    max-height: calc(100vh - 36px);
    display: flex;
    flex-direction: column;
    background: var(--popover-surface, #18181b);
    border: 1px solid var(--popover-border, rgba(255, 255, 255, 0.08));
    border-radius: 12px;
    box-shadow: 0 12px 40px rgba(0, 0, 0, 0.45);
    color: var(--popover-primary-text, #f4f4f5);
    overflow: hidden;
  }
  .meetings-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 12px 14px 10px;
    border-bottom: 1px solid var(--popover-border, rgba(255, 255, 255, 0.06));
  }
  .meetings-header h2 {
    margin: 0;
    font-size: 13px;
    font-weight: 600;
    letter-spacing: 0.02em;
  }
  .meetings-close {
    width: 24px;
    height: 24px;
    border: 0;
    background: transparent;
    color: var(--popover-secondary-text, #a1a1aa);
    font-size: 18px;
    line-height: 1;
    cursor: pointer;
    border-radius: 4px;
    padding: 0;
  }
  .meetings-close:hover {
    background: var(--popover-surface-hover, rgba(255, 255, 255, 0.06));
    color: var(--popover-primary-text, #f4f4f5);
  }

  .url-invite-row {
    display: flex;
    gap: 6px;
    padding: 10px 14px 6px;
  }
  .url-input {
    flex: 1 1 auto;
    background: var(--popover-surface-soft, rgba(255, 255, 255, 0.04));
    border: 1px solid var(--popover-border, rgba(255, 255, 255, 0.08));
    color: var(--popover-primary-text, #f4f4f5);
    border-radius: 6px;
    padding: 6px 8px;
    font-size: 12px;
    outline: none;
  }
  .url-input:focus {
    border-color: var(--popover-border-strong, rgba(255, 255, 255, 0.20));
  }
  .url-input:disabled {
    opacity: 0.6;
    cursor: wait;
  }
  .url-invite-btn {
    background: var(--popover-action, rgba(255, 255, 255, 0.10));
    color: var(--popover-primary-text, #f4f4f5);
    border: 1px solid var(--popover-border, rgba(255, 255, 255, 0.16));
    border-radius: 6px;
    padding: 6px 10px;
    font-size: 12px;
    cursor: pointer;
  }
  .url-invite-btn:hover:not(:disabled) {
    background: var(--popover-action-hover, rgba(255, 255, 255, 0.16));
  }
  .url-invite-btn:disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }
  .url-hint {
    margin: 0 14px 0;
    padding: 0 0 6px;
    font-size: 10px;
    color: var(--popover-secondary-text, #a1a1aa);
  }

  .toast {
    margin: 4px 14px 0;
    padding: 6px 8px;
    border-radius: 6px;
    background: var(--popover-surface-soft, rgba(255, 255, 255, 0.04));
    border: 1px solid var(--popover-border, rgba(255, 255, 255, 0.08));
    color: var(--popover-primary-text, #f4f4f5);
    font-size: 11px;
  }
  .toast-error {
    background: rgba(220, 38, 38, 0.10);
    border-color: rgba(220, 38, 38, 0.35);
    color: #fca5a5;
  }

  .meetings-body {
    flex: 1 1 auto;
    overflow-y: auto;
    padding: 10px 14px 14px;
  }
  .meetings-placeholder,
  .meetings-error {
    margin: 0;
    color: var(--popover-secondary-text, #a1a1aa);
    font-size: 12px;
    text-align: center;
    padding: 14px 0;
  }
  .meetings-error {
    color: #fca5a5;
  }

  .day-heading {
    margin: 12px 0 6px;
    font-size: 10px;
    font-weight: 600;
    color: var(--popover-secondary-text, #a1a1aa);
    text-transform: uppercase;
    letter-spacing: 0.08em;
  }
  .day-heading:first-of-type {
    margin-top: 4px;
  }
  .event-list {
    margin: 0;
    padding: 0;
    list-style: none;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .event-row {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 4px;
    border-bottom: 1px solid var(--popover-border, rgba(255, 255, 255, 0.04));
  }
  .event-row:last-child {
    border-bottom: 0;
  }
  .event-meta {
    flex: 1 1 auto;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .event-time {
    font-size: 10px;
    color: var(--popover-secondary-text, #a1a1aa);
  }
  .event-title {
    font-size: 12px;
    color: var(--popover-primary-text, #f4f4f5);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .event-badges {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
    margin-top: 1px;
  }
  .badge {
    font-size: 9px;
    padding: 1px 5px;
    border-radius: 3px;
    border: 1px solid var(--popover-border, rgba(255, 255, 255, 0.10));
    color: var(--popover-secondary-text, #a1a1aa);
  }
  .badge-company {
    background: var(--popover-surface-soft, rgba(255, 255, 255, 0.04));
  }
  .badge-platform {
    background: transparent;
  }
  .row-btn {
    flex: 0 0 auto;
    border: 1px solid var(--popover-border, rgba(255, 255, 255, 0.16));
    border-radius: 6px;
    padding: 4px 10px;
    font-size: 11px;
    cursor: pointer;
    background: var(--popover-action, rgba(255, 255, 255, 0.06));
    color: var(--popover-primary-text, #f4f4f5);
  }
  .row-btn:hover:not(:disabled) {
    background: var(--popover-action-hover, rgba(255, 255, 255, 0.12));
  }
  .row-btn:disabled {
    opacity: 0.5;
    cursor: wait;
  }
  .row-btn-uninvite {
    color: #fca5a5;
    border-color: rgba(220, 38, 38, 0.35);
  }
  .row-disabled {
    flex: 0 0 auto;
    color: var(--popover-secondary-text, #71717a);
    font-size: 12px;
    padding: 0 8px;
  }
</style>
