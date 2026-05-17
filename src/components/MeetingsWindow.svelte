<script lang="ts">
  /**
   * Upcoming Meetings — standalone Tauri window (label: `meetings-window`).
   * Mirrors the new-files-detail pattern: own window, decorated, resizable.
   * Self-fetches via the meetings_* Tauri commands; no main-window handshake.
   *
   * Routed by main.ts based on `getCurrentWindow().label`.
   */

  import { invoke } from '@tauri-apps/api/core';
  import { getCurrentWindow } from '@tauri-apps/api/window';
  import { open as openExternal } from '@tauri-apps/plugin-shell';

  interface MeetingEvent {
    id: string;
    summary?: string;
    start: { dateTime?: string; date?: string; timeZone?: string };
    end: { dateTime?: string; date?: string; timeZone?: string };
    status: string;
    hangoutLink?: string;
    /** Server-extracted meeting URL (BE-5) — picks across hangoutLink,
     *  conferenceData entry points, and Zoom/Teams in description. Prefer
     *  this over hangoutLink when deciding "can I invite a bot to this?" */
    meetingUrl?: string | null;
    sourceCalendarId?: string;
    sourceCompanyUid?: string;
    /** Per-account ULID identifying which connected Google account this
     *  event was fetched from (hq-pro BE-4 fan-out). Drives the per-account
     *  source badge and the multi-account filter dropdown. */
    sourceAccountId?: string;
  }

  /** One connected Google account on the signed-in person. From
   *  `meetings_list_accounts` (hq-pro GET /v1/google/accounts). */
  interface GoogleAccount {
    accountId: string;
    email?: string | null;
    scope?: string | null;
    connectedAt?: string | null;
    capabilities?: string[];
  }

  /** Subset of the calendar metadata we need to render labels in the
   *  filter dropdown. From `meetings_list_calendars_for_account`. */
  interface GoogleCalendar {
    id: string;
    summary: string;
    primary?: boolean;
    accessRole?: string | null;
  }

  /** Wraps `GoogleCalendar[]` with the account's currently-enabled
   *  selection so the filter dropdown can scope to what hq-pro actually
   *  fans out against (instead of every calendar Google returns). */
  interface AccountCalendars {
    calendars: GoogleCalendar[];
    selectedCalendarIds: string[];
  }

  /** Composite key uniquely identifying a calendar across all connected
   *  accounts. Same calendar id can appear under multiple accountIds when
   *  shared as reader/writer — we still want them addressable separately
   *  in the filter (reader vs owner = different rows). */
  type CalendarKey = string; // `${accountId}|${calendarId}`
  function calKey(accountId: string, calendarId: string): CalendarKey {
    return `${accountId}|${calendarId}`;
  }

  /**
   * hq-pro `BotStatus`:
   *   scheduled  — bot created but not yet joined
   *   joining    — bot connecting to the meeting
   *   recording  — bot in call + actively recording (the "live" state)
   *   processing — meeting ended, transcript pipeline running
   *   completed  — done, transcript stored
   *   failed     — error or cancelled-by-user
   */
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

  interface CompanyMembership {
    companyUid: string;
    companyName?: string | null;
    role?: string | null;
    status: string;
  }

  let events = $state<MeetingEvent[]>([]);
  let botsByEventId = $state<Map<string, ScheduledBot>>(new Map());
  let companyNamesByUid = $state<Map<string, string>>(new Map());
  let loading = $state(false);
  let listError = $state<string | null>(null);
  let toast = $state<{ kind: 'info' | 'error'; text: string } | null>(null);

  let urlInput = $state('');
  let urlInviting = $state(false);
  /** Company to route the URL-input bot to. `null` = Personal (the
   *  default). When a user picks a company from the dropdown, the
   *  resolved companyUid lives here and gets passed to
   *  `meetings_invite_bot`. Resets on successful invite so the next
   *  paste starts fresh on Personal. */
  let urlInputCompanyId = $state<string | null>(null);

  let rowPending = $state<Set<string>>(new Set());

  // Multi-account filter state.
  //   `accounts`           — every connected Google account
  //   `calendarsByAccount` — flat per-account calendar list, used to (a) render
  //                          per-calendar labels on event rows and (b) populate
  //                          the filter dropdown checkboxes
  //   `accountEmailById`   — fast lookup for the source-account badge
  //   `selectedCalKeys`    — user's current filter set. `null` means "show
  //                          all" — distinct from the empty set ("show nothing")
  //                          so we can drop the filter without losing context
  //   `filterOpen`         — dropdown visibility
  let accounts = $state<GoogleAccount[]>([]);
  let calendarsByAccount = $state<Map<string, GoogleCalendar[]>>(new Map());
  /** Calendar IDs the user has actually enabled in hq-console Integrations
   *  per account. Used to scope the filter dropdown so it never lists a
   *  calendar that hq-pro isn't fanning out against — prevents the trap
   *  where checking a dropdown box does nothing because the calendar
   *  isn't enabled server-side. */
  let enabledCalIdsByAccount = $state<Map<string, Set<string>>>(new Map());
  let accountEmailById = $state<Map<string, string>>(new Map());
  let calendarSummaryByKey = $state<Map<CalendarKey, string>>(new Map());
  let selectedCalKeys = $state<Set<CalendarKey> | null>(null);
  let filterOpen = $state(false);
  /** Bound to the filter-row container so the outside-click effect can
   *  tell whether a click landed inside the dropdown's box (keep open)
   *  or outside it (close). */
  let filterRowEl = $state<HTMLDivElement | null>(null);

  /**
   * Close the filter on any click outside the filter-row (trigger +
   * dropdown). Standard dropdown behaviour — without it the user can
   * click off and the open menu sits there blocking event rows.
   *
   * Capture-phase listener so we react before downstream handlers; only
   * mounted while the menu is open so we don't pay for a document
   * listener at idle. Escape key also closes for keyboard parity.
   */
  $effect(() => {
    if (!filterOpen) return;
    const onPointerDown = (ev: MouseEvent) => {
      const target = ev.target as Node | null;
      if (filterRowEl && target && filterRowEl.contains(target)) return;
      filterOpen = false;
    };
    // Escape closes the filter only — without stopPropagation the
    // window-level Escape listener (which closes the whole Meetings
    // window) would also fire. Capture-phase keeps us ahead of it.
    const onKeyDown = (ev: KeyboardEvent) => {
      if (ev.key !== 'Escape') return;
      ev.preventDefault();
      ev.stopPropagation();
      ev.stopImmediatePropagation();
      filterOpen = false;
    };
    document.addEventListener('pointerdown', onPointerDown, true);
    window.addEventListener('keydown', onKeyDown, true);
    return () => {
      document.removeEventListener('pointerdown', onPointerDown, true);
      window.removeEventListener('keydown', onKeyDown, true);
    };
  });

  $effect(() => {
    void refresh();

    // Poll every 30s while the window is open. Bots transition states
    // server-side (scheduled → joining → recording → processing) and the
    // auto-schedule cron creates new bots on a 10-min rhythm, so without
    // polling the window can render stale "Invite" affordances for events
    // that already have a bot scheduled. 30s is a sweet spot: fast enough
    // for the user to see joining/recording flips within a meeting, slow
    // enough that the upstream API isn't hit on every redraw.
    const pollId = window.setInterval(() => {
      void refresh();
    }, 30_000);

    // Esc closes the window — feels native on macOS where ⌘W is the
    // standard but Esc is the common expectation for a detached panel.
    const onkeydown = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.preventDefault();
        void getCurrentWindow().close();
      }
    };
    window.addEventListener('keydown', onkeydown);
    return () => {
      window.clearInterval(pollId);
      window.removeEventListener('keydown', onkeydown);
    };
  });

  async function refresh() {
    loading = true;
    listError = null;
    try {
      // Fetch events + bots + memberships + connected accounts in parallel.
      // Accounts is best-effort — falls back to [] so a person with zero
      // connected Google accounts still gets the (empty) events render
      // instead of a hard error.
      const [evts, bots, memberships, accts] = await Promise.all([
        invoke<MeetingEvent[]>('meetings_list_upcoming'),
        invoke<ScheduledBot[]>('meetings_list_scheduled_bots', {
          calendarEventIds: null,
        }),
        // Memberships are tiny + rarely change — fetched on every open is
        // cheap and avoids stale company-name display after the user joins
        // a new company elsewhere.
        invoke<CompanyMembership[]>('meetings_list_memberships').catch(() => [] as CompanyMembership[]),
        invoke<GoogleAccount[]>('meetings_list_accounts').catch(
          () => [] as GoogleAccount[],
        ),
      ]);
      events = evts ?? [];
      botsByEventId = buildBotMap(bots ?? []);
      companyNamesByUid = buildCompanyNameMap(memberships ?? []);
      accounts = accts ?? [];
      accountEmailById = new Map(
        (accts ?? []).map((a) => [a.accountId, a.email ?? '']),
      );

      // Calendars per account — second-pass fan-out so the events render
      // doesn't block on calendar metadata. Failures per-account are
      // non-fatal — the filter dropdown just won't list calendars from
      // the failing account, but events from it still render with the
      // accountId as the badge fallback.
      await loadCalendarsForAccounts(accts ?? []);
    } catch (err) {
      listError = String(err);
    } finally {
      loading = false;
    }
  }

  async function loadCalendarsForAccounts(accts: GoogleAccount[]) {
    const nextByAccount = new Map<string, GoogleCalendar[]>();
    const nextEnabled = new Map<string, Set<string>>();
    const nextSummaries = new Map<CalendarKey, string>();
    await Promise.all(
      accts.map(async (a) => {
        try {
          const resp = await invoke<AccountCalendars>(
            'meetings_list_calendars_for_account',
            { accountId: a.accountId },
          );
          nextByAccount.set(a.accountId, resp.calendars ?? []);
          nextEnabled.set(
            a.accountId,
            new Set(resp.selectedCalendarIds ?? []),
          );
          for (const c of resp.calendars ?? []) {
            nextSummaries.set(calKey(a.accountId, c.id), c.summary);
          }
        } catch {
          nextByAccount.set(a.accountId, []);
          nextEnabled.set(a.accountId, new Set());
        }
      }),
    );
    calendarsByAccount = nextByAccount;
    enabledCalIdsByAccount = nextEnabled;
    calendarSummaryByKey = nextSummaries;
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

  function buildCompanyNameMap(memberships: CompanyMembership[]): Map<string, string> {
    const m = new Map<string, string>();
    for (const row of memberships) {
      if (row.companyName) m.set(row.companyUid, row.companyName);
    }
    return m;
  }

  /**
   * Bot statuses we still consider "live" for the row UI. Matches the
   * hq-pro BotStatus enum — used to keep the bot visible on its row until
   * the meeting fully concludes. `processing` stays in this set so the
   * row shows "Processing…" instead of flipping back to "Invite" the
   * moment the meeting ends but before the transcript lands.
   */
  function isActiveStatus(s: string): boolean {
    return (
      s === 'scheduled' ||
      s === 'joining' ||
      s === 'recording' ||
      s === 'processing'
    );
  }

  async function onInvite(evt: MeetingEvent) {
    const url = eventMeetingUrl(evt);
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
      // 409 "bot-already-scheduled" is benign and (usually) means the row
      // was stale — e.g. the auto-schedule cron picked up the event between
      // window-open and this click, or a separate hq-sync instance got
      // there first. Refresh + tell the user it's invited rather than
      // showing a scary failure toast.
      const msg = String(err);
      if (msg.includes('409') || msg.includes('bot-already-scheduled')) {
        flashToast('info', 'Already invited — refreshing.');
        await refresh();
      } else {
        flashToast('error', `Invite failed: ${err}`);
      }
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
    const submittedCompanyId = urlInputCompanyId;
    try {
      await invoke<ScheduledBot>('meetings_invite_bot', {
        meetingUrl: url,
        calendarEventId: null,
        companyId: submittedCompanyId,
      });
      // Clear the row on success — input AND the company pick — so the
      // next paste starts fresh on Personal. We snapshot the chosen id
      // BEFORE the invoke so a slow request that completes after the
      // user re-types doesn't drop their next selection.
      urlInput = '';
      urlInputCompanyId = null;
      const destLabel = submittedCompanyId
        ? (companyNamesByUid.get(submittedCompanyId) ?? 'company')
        : 'Personal';
      flashToast('info', `Bot invited — meeting will save to ${destLabel}.`);
      await refresh();
    } catch (err) {
      flashToast('error', `Invite failed: ${err}`);
    } finally {
      urlInviting = false;
    }
  }

  function isPlausibleMeetingUrl(url: string): boolean {
    if (!url) return false;
    return (
      /^https:\/\/[^\s/]*\.zoom\.us\/j\/[^\s]+/i.test(url) ||
      /^https:\/\/meet\.google\.com\/[a-z-]+/i.test(url) ||
      /^https:\/\/teams\.microsoft\.com\/l\/meetup-join\/[^\s]+/i.test(url) ||
      /^https:\/\/[^\s/]*\.webex\.com\/[^\s]+/i.test(url)
    );
  }

  function flashToast(kind: 'info' | 'error', text: string) {
    toast = { kind, text };
    setTimeout(() => {
      if (toast && toast.text === text) toast = null;
    }, 4000);
  }

  interface DayGroup {
    label: string;
    events: MeetingEvent[];
  }

  const groupedEvents = $derived<DayGroup[]>(groupByDay(filteredEvents));

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

  /**
   * Source-account + calendar label for the per-event badge.
   *
   * Most personal Gmail accounts name their primary calendar exactly the
   * same as the account email ("therealstefan@gmail.com"), so naively
   * concatenating gives a useless "therealstefan@gmail.com · therealstefan@…"
   * pill. Dedupe: if the calendar summary equals the email (or is the
   * account's primary calendar — Google's convention), show only the
   * email. Otherwise show "email · summary".
   *
   *   "Stefan Johnson · Indigo Standup" (Workspace primary + named calendar)
   *   "stefan@getindigo.ai"             (primary calendar or summary == email)
   *   "Calendar"                        (no source info, pre-BE-4 backend)
   */
  function sourceLabel(e: MeetingEvent): string {
    const acctId = e.sourceAccountId;
    const calId = e.sourceCalendarId;
    const email = acctId ? accountEmailById.get(acctId) : undefined;
    const summary =
      acctId && calId
        ? calendarSummaryByKey.get(calKey(acctId, calId))
        : undefined;
    const isPrimary =
      acctId && calId
        ? (calendarsByAccount.get(acctId) ?? []).find((c) => c.id === calId)
            ?.primary === true
        : false;

    const eqIgnoreCase = (a?: string, b?: string) =>
      !!a && !!b && a.trim().toLowerCase() === b.trim().toLowerCase();

    if (email && summary && !isPrimary && !eqIgnoreCase(email, summary)) {
      return `${email} · ${summary}`;
    }
    if (email) return email;
    if (summary) return summary;
    return 'Calendar';
  }

  /**
   * Flat list of every (accountId, calendarId) pair the user has actually
   * enabled in hq-console Integrations — the only calendars hq-pro fans
   * out against, so the only ones it makes sense to filter by.
   *
   * Scoping to enabled-only avoids a real UX trap: previously the dropdown
   * listed every calendar Google returned (including ones the user hadn't
   * opted into and even reader-only shared calendars filtered out by
   * hq-console), so checking a box did nothing — events still wouldn't
   * appear because hq-pro never fetched that calendar. The new scoping
   * means "nothing in the dropdown for an account" reads as "you haven't
   * enabled any calendars there yet" instead of failing silently.
   */
  const allCalKeys = $derived<
    Array<{ key: CalendarKey; accountId: string; email: string; summary: string }>
  >(buildAllCalKeys(accounts, calendarsByAccount, enabledCalIdsByAccount));

  function buildAllCalKeys(
    accts: GoogleAccount[],
    calMap: Map<string, GoogleCalendar[]>,
    enabledMap: Map<string, Set<string>>,
  ): Array<{ key: CalendarKey; accountId: string; email: string; summary: string }> {
    const out: Array<{
      key: CalendarKey;
      accountId: string;
      email: string;
      summary: string;
    }> = [];
    for (const a of accts) {
      const enabled = enabledMap.get(a.accountId) ?? new Set<string>();
      const cals = (calMap.get(a.accountId) ?? []).filter((c) =>
        enabled.has(c.id),
      );
      for (const c of cals) {
        out.push({
          key: calKey(a.accountId, c.id),
          accountId: a.accountId,
          email: a.email ?? a.accountId,
          summary: c.summary,
        });
      }
    }
    return out;
  }

  /**
   * Prune `selectedCalKeys` whenever the enabled set shrinks (e.g. the user
   * disabled a calendar in hq-console between refreshes). Without this, the
   * filter would display "2 of 1 selected" when the selection set contains
   * keys no longer in `allCalKeys`. Only runs when `selectedCalKeys` is an
   * explicit set — leaves the `null` ("show all") default alone.
   */
  $effect(() => {
    if (selectedCalKeys === null) return;
    const validKeys = new Set(allCalKeys.map((c) => c.key));
    const filtered = new Set<CalendarKey>();
    for (const k of selectedCalKeys) {
      if (validKeys.has(k)) filtered.add(k);
    }
    if (filtered.size !== selectedCalKeys.size) {
      selectedCalKeys = filtered;
    }
  });

  /**
   * Events filtered by the user's current selection. `null` selectedCalKeys
   * means "show all" — the default. Switching to a Set (even empty) is the
   * filter active state.
   */
  const filteredEvents = $derived<MeetingEvent[]>(
    filterEvents(events, selectedCalKeys),
  );

  function filterEvents(
    list: MeetingEvent[],
    selection: Set<CalendarKey> | null,
  ): MeetingEvent[] {
    if (selection === null) return list;
    return list.filter((e) => {
      if (!e.sourceAccountId || !e.sourceCalendarId) return false;
      return selection.has(calKey(e.sourceAccountId, e.sourceCalendarId));
    });
  }

  function toggleCalKey(key: CalendarKey) {
    // First toggle from "show all" snapshots the current full set so the
    // user can untoggle from a sensible starting point.
    if (selectedCalKeys === null) {
      const next = new Set<CalendarKey>(allCalKeys.map((c) => c.key));
      next.delete(key);
      selectedCalKeys = next;
      return;
    }
    const next = new Set(selectedCalKeys);
    if (next.has(key)) next.delete(key);
    else next.add(key);
    selectedCalKeys = next;
  }

  function selectAllCalKeys() {
    selectedCalKeys = null; // back to "show all"
  }

  function clearAllCalKeys() {
    selectedCalKeys = new Set();
  }

  function isCalKeySelected(key: CalendarKey): boolean {
    if (selectedCalKeys === null) return true;
    return selectedCalKeys.has(key);
  }

  /**
   * Human label for the filter trigger button. Default "All calendars" when
   * no filter applied; "N of M calendars" when an active subset is chosen.
   */
  const filterButtonLabel = $derived<string>(
    selectedCalKeys === null
      ? 'All calendars'
      : `${selectedCalKeys.size} of ${allCalKeys.length} calendars`,
  );

  function companyLabel(e: MeetingEvent): string {
    if (!e.sourceCompanyUid) return 'Personal';
    // Prefer the human-readable name from /membership/me. Fall back to
    // a UID prefix only when the membership map didn't include this
    // company (rare: should only happen if the user lost membership
    // between the calendar mapping save and now).
    const name = companyNamesByUid.get(e.sourceCompanyUid);
    if (name) return name;
    const short = e.sourceCompanyUid.slice(0, 12);
    return short.length === 12 ? `${short}…` : short;
  }

  function eventMeetingUrl(e: MeetingEvent): string | null {
    // Server-side BE-5 extracts from hangoutLink/conferenceData/description.
    // Fall back to raw hangoutLink for events served by a pre-BE-5 backend.
    return e.meetingUrl ?? e.hangoutLink ?? null;
  }

  function platformLabel(e: MeetingEvent): string {
    const url = eventMeetingUrl(e) ?? '';
    if (url.includes('meet.google.com')) return 'Google Meet';
    if (url.includes('zoom.us')) return 'Zoom';
    if (url.includes('teams.microsoft.com')) return 'Teams';
    if (url.includes('webex.com')) return 'Webex';
    return '';
  }

  /**
   * Three-state button per row, driven by `bot.status`:
   *
   *   no bot              → kind="invite"     (CTA — solid)
   *   scheduled           → kind="invited"    (muted, click to cancel)
   *   joining             → kind="joining"    (transient)
   *   recording           → kind="in-call"    (live red-dot indicator)
   *   processing          → kind="processing" (non-cancellable, transient)
   *
   * The user explicitly asked for "in call" visibility while a meeting is
   * live. We surface `recording` as a distinct state with a pulsing dot so
   * it reads at a glance.
   */
  type RowButtonKind =
    | 'invite'
    | 'invited'
    | 'joining'
    | 'in-call'
    | 'processing';

  function rowButtonKind(bot: ScheduledBot | undefined): RowButtonKind {
    if (!bot) return 'invite';
    switch (bot.status) {
      case 'scheduled':
        return 'invited';
      case 'joining':
        return 'joining';
      case 'recording':
        return 'in-call';
      case 'processing':
        return 'processing';
      default:
        // Defensive fallback — completed/failed shouldn't reach here because
        // buildBotMap filters them out, but if somehow they do, show Invite.
        return 'invite';
    }
  }

  function rowButtonLabel(kind: RowButtonKind, pending: boolean): string {
    if (pending) return '…';
    switch (kind) {
      case 'invite':
        return 'Invite';
      case 'invited':
        return 'Invited';
      case 'joining':
        return 'Joining…';
      case 'in-call':
        return 'In Call';
      case 'processing':
        return 'Processing';
    }
  }
</script>

<div class="meetings-page">
  <header class="meetings-header">
    <h1>Upcoming Meetings</h1>
    <button
      type="button"
      class="meetings-refresh"
      onclick={refresh}
      disabled={loading}
      title="Refresh"
      aria-label="Refresh meetings"
    >
      <svg width="14" height="14" viewBox="0 0 16 16" fill="none" xmlns="http://www.w3.org/2000/svg" aria-hidden="true">
        <path d="M1.5 8a6.5 6.5 0 0 1 11.48-4.16" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" />
        <path d="M14.5 8A6.5 6.5 0 0 1 3.02 12.16" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" />
        <path d="M11 1.5v2.5h2.5" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" />
        <path d="M5 12h-2.5v2.5" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" />
      </svg>
    </button>
  </header>

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
    {#if urlInput.trim().length > 0}
      <!-- Destination picker. Only renders once the user starts typing
           a URL — keeps the idle UI clean. `null` value means Personal
           (the default). Memberships come from /v1/users/me/memberships
           via meetings_list_memberships; users with no company
           memberships still see Personal as the only option. -->
      <select
        class="url-invite-company"
        aria-label="Save bot to"
        bind:value={urlInputCompanyId}
        disabled={urlInviting}
      >
        <option value={null}>Personal</option>
        {#each [...companyNamesByUid.entries()] as [uid, name] (uid)}
          <option value={uid}>{name}</option>
        {/each}
      </select>
    {/if}
    <button
      type="button"
      class="url-invite-btn"
      disabled={urlInviting || !isPlausibleMeetingUrl(urlInput.trim())}
      onclick={onUrlInvite}
    >
      {urlInviting ? 'Inviting…' : 'Invite'}
    </button>
  </div>

  {#if toast}
    <p class="toast" class:toast-error={toast.kind === 'error'}>
      {toast.text}
    </p>
  {/if}

  {#if accounts.length > 1 || allCalKeys.length > 0}
    <!-- Multi-account calendar filter. Only render when there's something
         to filter — single-account users get the cleaner pre-multi-account
         UI by default. -->
    <div class="filter-row" bind:this={filterRowEl}>
      <button
        type="button"
        class="filter-trigger"
        aria-haspopup="listbox"
        aria-expanded={filterOpen}
        onclick={() => (filterOpen = !filterOpen)}
      >
        <span>{filterButtonLabel}</span>
        <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden="true">
          <path d="M2 3.5L5 6.5L8 3.5" stroke="currentColor" stroke-width="1.3" fill="none" stroke-linecap="round" stroke-linejoin="round" />
        </svg>
      </button>
      {#if filterOpen}
        <div class="filter-menu" role="listbox" aria-label="Filter by calendar">
          <div class="filter-actions">
            <button type="button" class="filter-action" onclick={selectAllCalKeys}>
              All
            </button>
            <button type="button" class="filter-action" onclick={clearAllCalKeys}>
              None
            </button>
          </div>
          {#each accounts as a (a.accountId)}
            {@const enabledIds = enabledCalIdsByAccount.get(a.accountId) ?? new Set<string>()}
            {@const calsForAcct = (calendarsByAccount.get(a.accountId) ?? []).filter((c) => enabledIds.has(c.id))}
            <div class="filter-group">
              <p class="filter-group-label">{a.email ?? a.accountId}</p>
              {#if calsForAcct.length === 0}
                <p class="filter-group-empty">
                  No calendars enabled in Integrations
                </p>
              {:else}
                {#each calsForAcct as cal (cal.id)}
                  {@const key = calKey(a.accountId, cal.id)}
                  <label class="filter-option">
                    <input
                      type="checkbox"
                      checked={isCalKeySelected(key)}
                      onchange={() => toggleCalKey(key)}
                    />
                    <span class="filter-option-label">
                      {cal.summary}
                      {#if cal.primary}<span class="filter-primary">primary</span>{/if}
                    </span>
                  </label>
                {/each}
              {/if}
            </div>
          {/each}
        </div>
      {/if}
    </div>
  {/if}

  <section class="meetings-body">
    {#if loading && events.length === 0}
      <p class="meetings-placeholder">Loading…</p>
    {:else if listError}
      <p class="meetings-error">{listError}</p>
    {:else if events.length === 0}
      <p class="meetings-placeholder">
        No upcoming meetings in your connected calendars.
      </p>
    {:else if filteredEvents.length === 0}
      <p class="meetings-placeholder">
        No meetings match the current calendar filter.
        <button
          type="button"
          class="meetings-inline-link"
          onclick={selectAllCalKeys}
        >
          Show all calendars
        </button>
      </p>
    {:else}
      {#each groupedEvents as group (group.label)}
        <h3 class="day-heading">{group.label}</h3>
        <ul class="event-list">
          {#each group.events as evt (evt.id)}
            {@const bot = botsByEventId.get(evt.id)}
            {@const pending = rowPending.has(evt.id)}
            {@const kind = rowButtonKind(bot)}
            {@const url = eventMeetingUrl(evt)}
            <li class="event-row">
              <div class="event-meta">
                <span class="event-time">{timeLabel(evt)}</span>
                <span class="event-title" title={evt.summary ?? ''}>
                  {evt.summary ?? '(no title)'}
                </span>
                <span class="event-badges">
                  <span class="badge badge-company">{companyLabel(evt)}</span>
                  {#if accounts.length > 1}
                    <!-- Per-account source badge — only render when the
                         person has more than one connected account, so a
                         single-account user doesn't see redundant noise. -->
                    <span class="badge badge-source" title={sourceLabel(evt)}>
                      {sourceLabel(evt)}
                    </span>
                  {/if}
                  {#if platformLabel(evt)}
                    <span class="badge badge-platform">{platformLabel(evt)}</span>
                  {/if}
                  {#if kind === 'in-call'}
                    <span class="badge badge-live"
                      ><span class="live-dot"></span>Live</span
                    >
                  {/if}
                </span>
              </div>
              {#if url}
                <!-- Join link — opens the meeting URL in the OS default
                     browser (which then hands off to Zoom/Meet/Teams app
                     if installed). Renders whenever a URL exists, including
                     while the bot is in-call, so users can hop in
                     themselves at any time. Discreet styling so it doesn't
                     compete with the primary status button. -->
                <button
                  type="button"
                  class="row-btn-join"
                  title="Open meeting in browser"
                  aria-label="Join meeting"
                  onclick={() => {
                    openExternal(url).catch((err) => {
                      flashToast('error', `Couldn't open meeting: ${err}`);
                    });
                  }}
                >
                  Join
                  <svg
                    width="10"
                    height="10"
                    viewBox="0 0 12 12"
                    fill="none"
                    xmlns="http://www.w3.org/2000/svg"
                    aria-hidden="true"
                  >
                    <path
                      d="M4 2h6v6M10 2L4.5 7.5M2 4v6h6"
                      stroke="currentColor"
                      stroke-width="1.4"
                      stroke-linecap="round"
                      stroke-linejoin="round"
                    />
                  </svg>
                </button>
              {/if}
              {#if !url}
                <span class="row-disabled" title="No meeting URL on this event"
                  >—</span
                >
              {:else if kind === 'invite'}
                <button
                  type="button"
                  class="row-btn row-btn-invite"
                  disabled={pending}
                  onclick={() => onInvite(evt)}
                >
                  {rowButtonLabel(kind, pending)}
                </button>
              {:else if kind === 'invited'}
                <button
                  type="button"
                  class="row-btn row-btn-invited"
                  disabled={pending}
                  title="Click to uninvite the bot"
                  onclick={() => onUninvite(evt)}
                >
                  {rowButtonLabel(kind, pending)}
                </button>
              {:else if kind === 'in-call'}
                <button
                  type="button"
                  class="row-btn row-btn-incall"
                  disabled={pending}
                  title="Bot is in the meeting — click to remove it"
                  onclick={() => onUninvite(evt)}
                >
                  {rowButtonLabel(kind, pending)}
                </button>
              {:else if kind === 'joining'}
                <button
                  type="button"
                  class="row-btn row-btn-joining"
                  disabled={pending}
                  title="Bot is joining — click to cancel"
                  onclick={() => onUninvite(evt)}
                >
                  {rowButtonLabel(kind, pending)}
                </button>
              {:else}
                <!-- processing: meeting ended, transcript pipeline running.
                     Not cancellable — the bot isn't holding a Recall slot. -->
                <span class="row-disabled row-disabled-processing">
                  {rowButtonLabel(kind, pending)}
                </span>
              {/if}
            </li>
          {/each}
        </ul>
      {/each}
    {/if}
  </section>
</div>

<style>
  :global(html),
  :global(body) {
    margin: 0;
    padding: 0;
    height: 100vh;
    background: #18181b;
    color: #f4f4f5;
    font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
    font-size: 13px;
    overflow: hidden;
  }
  :global(#app) {
    height: 100vh;
  }

  .meetings-page {
    display: flex;
    flex-direction: column;
    height: 100vh;
    background: #18181b;
    color: #f4f4f5;
  }

  .meetings-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 14px 18px 10px;
    border-bottom: 1px solid rgba(255, 255, 255, 0.06);
  }
  .meetings-header h1 {
    margin: 0;
    font-size: 14px;
    font-weight: 600;
    letter-spacing: 0.02em;
  }
  .meetings-refresh {
    width: 28px;
    height: 28px;
    border: 1px solid rgba(255, 255, 255, 0.10);
    background: rgba(255, 255, 255, 0.04);
    color: #d4d4d8;
    border-radius: 6px;
    cursor: pointer;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    padding: 0;
  }
  .meetings-refresh:hover:not(:disabled) {
    background: rgba(255, 255, 255, 0.08);
    color: #f4f4f5;
  }
  .meetings-refresh:disabled {
    opacity: 0.5;
    cursor: wait;
  }

  .url-invite-row {
    display: flex;
    gap: 8px;
    padding: 12px 18px 6px;
  }
  .url-input {
    flex: 1 1 auto;
    background: rgba(255, 255, 255, 0.04);
    border: 1px solid rgba(255, 255, 255, 0.10);
    color: #f4f4f5;
    border-radius: 6px;
    padding: 7px 10px;
    font-size: 12px;
    outline: none;
  }
  .url-input:focus {
    border-color: rgba(255, 255, 255, 0.24);
  }
  .url-input:disabled {
    opacity: 0.6;
    cursor: wait;
  }
  /* Destination picker — visually paired with the input so it reads as a
     single composite control. Renders only while a URL is being typed. */
  .url-invite-company {
    flex: 0 0 auto;
    max-width: 140px;
    background: rgba(255, 255, 255, 0.04);
    color: #f4f4f5;
    border: 1px solid rgba(255, 255, 255, 0.10);
    border-radius: 6px;
    padding: 7px 8px;
    font-size: 12px;
    cursor: pointer;
  }
  .url-invite-company:focus {
    outline: none;
    border-color: rgba(255, 255, 255, 0.24);
  }
  .url-invite-company:disabled {
    opacity: 0.6;
    cursor: wait;
  }
  .url-invite-btn {
    background: rgba(255, 255, 255, 0.12);
    color: #f4f4f5;
    border: 1px solid rgba(255, 255, 255, 0.20);
    border-radius: 6px;
    padding: 7px 12px;
    font-size: 12px;
    cursor: pointer;
  }
  .url-invite-btn:hover:not(:disabled) {
    background: rgba(255, 255, 255, 0.18);
  }
  .url-invite-btn:disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }

  .toast {
    margin: 8px 18px 0;
    padding: 7px 10px;
    border-radius: 6px;
    background: rgba(255, 255, 255, 0.04);
    border: 1px solid rgba(255, 255, 255, 0.10);
    color: #f4f4f5;
    font-size: 11px;
  }
  .toast-error {
    background: rgba(220, 38, 38, 0.10);
    border-color: rgba(220, 38, 38, 0.40);
    color: #fca5a5;
  }

  .meetings-body {
    flex: 1 1 auto;
    overflow-y: auto;
    padding: 8px 18px 16px;
  }
  .meetings-placeholder,
  .meetings-error {
    margin: 0;
    color: #a1a1aa;
    font-size: 12px;
    text-align: center;
    padding: 20px 0;
  }
  .meetings-error {
    color: #fca5a5;
  }

  .day-heading {
    margin: 14px 0 6px;
    font-size: 10px;
    font-weight: 600;
    color: #a1a1aa;
    text-transform: uppercase;
    letter-spacing: 0.08em;
  }
  .day-heading:first-of-type {
    margin-top: 6px;
  }
  .event-list {
    margin: 0;
    padding: 0;
    list-style: none;
    display: flex;
    flex-direction: column;
  }
  .event-row {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 10px 4px;
    border-bottom: 1px solid rgba(255, 255, 255, 0.04);
  }
  .event-row:last-child {
    border-bottom: 0;
  }
  .event-meta {
    flex: 1 1 auto;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 3px;
  }
  .event-time {
    font-size: 10px;
    color: #a1a1aa;
  }
  .event-title {
    font-size: 13px;
    color: #f4f4f5;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .event-badges {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
    margin-top: 2px;
  }
  .badge {
    font-size: 9px;
    padding: 2px 6px;
    border-radius: 3px;
    border: 1px solid rgba(255, 255, 255, 0.10);
    color: #a1a1aa;
  }
  .badge-company {
    background: rgba(255, 255, 255, 0.04);
  }
  .badge-platform {
    background: transparent;
  }
  /* Per-account source badge — renders only when accounts.length > 1. The
     muted blue-ish tint distinguishes it from the company badge (which is
     about where data ends up) vs source (where data came from). */
  .badge-source {
    background: rgba(96, 165, 250, 0.10);
    border-color: rgba(96, 165, 250, 0.30);
    color: #bfdbfe;
    max-width: 200px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  /* Inline-link inside the meetings-placeholder copy — used by the
     "no events match the filter" recovery affordance. */
  .meetings-inline-link {
    appearance: none;
    background: none;
    border: 0;
    padding: 0;
    margin-left: 4px;
    color: #93c5fd;
    text-decoration: underline;
    cursor: pointer;
    font: inherit;
  }
  .meetings-inline-link:hover {
    color: #bfdbfe;
  }

  /* ── Calendar filter dropdown ──────────────────────────────────────── */
  .filter-row {
    position: relative;
    padding: 6px 12px 0;
  }
  .filter-trigger {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 4px 10px;
    border-radius: 6px;
    border: 1px solid rgba(255, 255, 255, 0.10);
    background: rgba(255, 255, 255, 0.03);
    color: #d4d4d8;
    font-size: 11px;
    cursor: pointer;
  }
  .filter-trigger:hover {
    background: rgba(255, 255, 255, 0.06);
  }
  .filter-menu {
    position: absolute;
    top: calc(100% + 4px);
    left: 12px;
    z-index: 20;
    min-width: 280px;
    max-height: 320px;
    overflow-y: auto;
    padding: 6px;
    border-radius: 8px;
    border: 1px solid rgba(255, 255, 255, 0.12);
    background: #161618;
    box-shadow: 0 8px 24px rgba(0, 0, 0, 0.5);
  }
  .filter-actions {
    display: flex;
    gap: 4px;
    padding: 4px 6px 8px;
    border-bottom: 1px solid rgba(255, 255, 255, 0.06);
    margin-bottom: 4px;
  }
  .filter-action {
    flex: 1;
    padding: 4px 8px;
    border-radius: 4px;
    border: 1px solid rgba(255, 255, 255, 0.08);
    background: transparent;
    color: #a1a1aa;
    font-size: 10px;
    text-transform: uppercase;
    letter-spacing: 0.08em;
    cursor: pointer;
  }
  .filter-action:hover {
    background: rgba(255, 255, 255, 0.04);
    color: #fafafa;
  }
  .filter-group {
    padding: 4px 0;
  }
  .filter-group + .filter-group {
    border-top: 1px solid rgba(255, 255, 255, 0.04);
    margin-top: 4px;
    padding-top: 8px;
  }
  .filter-group-label {
    font-size: 10px;
    text-transform: uppercase;
    letter-spacing: 0.08em;
    color: #71717a;
    margin: 0 0 4px;
    padding: 0 6px;
  }
  .filter-group-empty {
    font-size: 11px;
    color: #52525b;
    margin: 0;
    padding: 0 6px;
    font-style: italic;
  }
  .filter-option {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 4px 6px;
    border-radius: 4px;
    cursor: pointer;
  }
  .filter-option:hover {
    background: rgba(255, 255, 255, 0.04);
  }
  .filter-option input[type="checkbox"] {
    margin: 0;
    accent-color: #e4e4e7;
  }
  .filter-option-label {
    font-size: 12px;
    color: #d4d4d8;
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .filter-primary {
    display: inline-block;
    margin-left: 6px;
    padding: 1px 5px;
    border-radius: 3px;
    background: rgba(255, 255, 255, 0.06);
    color: #a1a1aa;
    font-size: 9px;
    text-transform: uppercase;
    letter-spacing: 0.06em;
  }
  /* "Live" badge: red dot + label, only shown while bot.status === 'recording'.
     The pulsing animation is what carries the at-a-glance "something is
     happening right now" cue — without it, a static red dot reads as a
     static indicator rather than a live one. */
  .badge-live {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    color: #fca5a5;
    border-color: rgba(220, 38, 38, 0.50);
    background: rgba(220, 38, 38, 0.12);
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

  /* Join link — opens the meeting URL in the OS default browser. Smaller
     and lower-contrast than the primary status button so the eye lands
     on Invite/Invited first. The external-link glyph signals "this
     leaves the app". */
  .row-btn-join {
    flex: 0 0 auto;
    display: inline-flex;
    align-items: center;
    gap: 4px;
    border: 1px solid rgba(255, 255, 255, 0.08);
    background: transparent;
    color: #a1a1aa;
    border-radius: 5px;
    padding: 5px 10px;
    font-size: 10px;
    cursor: pointer;
    transition: background 120ms ease, color 120ms ease, border-color 120ms ease;
  }
  .row-btn-join:hover {
    background: rgba(255, 255, 255, 0.06);
    color: #f4f4f5;
    border-color: rgba(255, 255, 255, 0.18);
  }
  .row-btn-join:focus-visible {
    outline: 2px solid rgba(180, 180, 255, 0.7);
    outline-offset: 1px;
  }

  /* Row button — base style. Per-state modifiers below override the look
     while preserving the layout (flex item, no shrink). */
  .row-btn {
    flex: 0 0 auto;
    border: 1px solid rgba(255, 255, 255, 0.20);
    border-radius: 6px;
    padding: 6px 14px;
    font-size: 11px;
    cursor: pointer;
    background: rgba(255, 255, 255, 0.06);
    color: #f4f4f5;
    min-width: 78px;
    text-align: center;
  }
  .row-btn:hover:not(:disabled) {
    background: rgba(255, 255, 255, 0.14);
  }
  .row-btn:disabled {
    opacity: 0.6;
    cursor: wait;
  }
  /* CTA — solid-ish, brighter border to read as the actionable state. */
  .row-btn-invite {
    background: rgba(255, 255, 255, 0.12);
    border-color: rgba(255, 255, 255, 0.28);
  }
  .row-btn-invite:hover:not(:disabled) {
    background: rgba(255, 255, 255, 0.20);
  }
  /* Confirmed state — muted; hover hints that clicking will uninvite. */
  .row-btn-invited {
    color: #a1a1aa;
    background: rgba(255, 255, 255, 0.03);
    border-color: rgba(255, 255, 255, 0.10);
  }
  .row-btn-invited:hover:not(:disabled) {
    color: #fca5a5;
    background: rgba(220, 38, 38, 0.10);
    border-color: rgba(220, 38, 38, 0.40);
  }
  /* In-call — distinct red tint so the row reads "live" even without the
     badge. Hovering reveals the uninvite affordance. */
  .row-btn-incall {
    color: #fca5a5;
    background: rgba(220, 38, 38, 0.12);
    border-color: rgba(220, 38, 38, 0.40);
  }
  .row-btn-incall:hover:not(:disabled) {
    background: rgba(220, 38, 38, 0.22);
  }
  /* Joining — transient. Subtle amber to differentiate from steady states. */
  .row-btn-joining {
    color: #fcd34d;
    background: rgba(202, 138, 4, 0.10);
    border-color: rgba(202, 138, 4, 0.40);
  }
  .row-btn-joining:hover:not(:disabled) {
    background: rgba(202, 138, 4, 0.18);
  }

  .row-disabled {
    flex: 0 0 auto;
    color: #71717a;
    font-size: 12px;
    padding: 0 8px;
    min-width: 78px;
    text-align: center;
  }
  /* Processing — meeting ended, transcript pipeline running. Not
     clickable because the bot isn't holding a Recall slot anymore;
     muted blue tint reads as "doing background work, hands off". */
  .row-disabled-processing {
    color: #93c5fd;
    background: rgba(59, 130, 246, 0.08);
    border: 1px solid rgba(59, 130, 246, 0.30);
    border-radius: 6px;
    padding: 6px 14px;
    font-size: 11px;
  }
</style>
