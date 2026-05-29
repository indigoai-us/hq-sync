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
  import {
    loadMeetingsCache,
    saveMeetingsCache,
  } from '../lib/meetingsCache';

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

  /**
   * Stale-while-revalidate hydrate: synchronously read the last cached
   * snapshot so $state defaults below paint rows on open instead of an
   * empty skeleton. `refresh()` still runs from the lifecycle $effect to
   * pull fresh data — the cache just gets us to first paint without
   * waiting for the round-trip. Returns null on cache miss / corrupt /
   * expired entry (>24h old), in which case every state var falls back
   * to its empty default and the cold-start skeleton renders as before.
   * Lives outside any $state so it runs exactly once at script-init.
   */
  const cachedSnapshot = loadMeetingsCache<
    MeetingEvent,
    ScheduledBot,
    GoogleAccount,
    GoogleCalendar
  >();

  let events = $state<MeetingEvent[]>(cachedSnapshot?.events ?? []);
  let botsByEventId = $state<Map<string, ScheduledBot>>(
    new Map(cachedSnapshot?.botsByEventId ?? []),
  );
  let companyNamesByUid = $state<Map<string, string>>(
    new Map(cachedSnapshot?.companyNamesByUid ?? []),
  );
  let loading = $state(false);
  let listError = $state<string | null>(null);
  let toast = $state<{ kind: 'info' | 'warn'; text: string } | null>(null);

  /**
   * Distill an upstream error (Tauri command Result::Err string, fetch
   * failure, or a thrown Error) into a single readable sentence.
   *
   * Tauri error strings look like `bot/invite HTTP 409: {"error":"A bot is
   * already scheduled","code":"bot-already-scheduled"}`. The raw JSON is
   * noise in a toast — parse it out, prefer the `error` field, and fall
   * back to a friendly per-status message so we never surface a blob of
   * JSON or a stack trace to the user. Matches the policy of avoiding
   * red error states for recoverable user-facing failures.
   */
  function friendlyError(err: unknown, fallback: string): string {
    const raw = String(err ?? '').trim();
    const jsonStart = raw.indexOf('{');
    if (jsonStart >= 0) {
      try {
        const parsed = JSON.parse(raw.slice(jsonStart)) as {
          error?: string;
          message?: string;
        };
        if (typeof parsed.error === 'string' && parsed.error.length > 0) {
          return parsed.error;
        }
        if (typeof parsed.message === 'string' && parsed.message.length > 0) {
          return parsed.message;
        }
      } catch {
        // Not JSON — fall through to HTTP-status heuristics below.
      }
    }
    if (/\b409\b/.test(raw)) return 'A bot is already scheduled for this meeting.';
    if (/\b401\b/.test(raw)) return 'You need to sign in again.';
    if (/\b403\b/.test(raw)) return "You don't have permission for that.";
    if (/\b5\d{2}\b/.test(raw)) return 'Server hiccup — try again in a moment.';
    return fallback;
  }

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
  let accounts = $state<GoogleAccount[]>(cachedSnapshot?.accounts ?? []);
  let calendarsByAccount = $state<Map<string, GoogleCalendar[]>>(
    new Map(cachedSnapshot?.calendarsByAccount ?? []),
  );
  /** Calendar IDs the user has actually enabled in hq-console Integrations
   *  per account. Used to scope the filter dropdown so it never lists a
   *  calendar that hq-pro isn't fanning out against — prevents the trap
   *  where checking a dropdown box does nothing because the calendar
   *  isn't enabled server-side. */
  let enabledCalIdsByAccount = $state<Map<string, Set<string>>>(
    new Map(
      (cachedSnapshot?.enabledCalIdsByAccount ?? []).map(
        ([acct, ids]) => [acct, new Set(ids)],
      ),
    ),
  );
  let accountEmailById = $state<Map<string, string>>(
    new Map(cachedSnapshot?.accountEmailById ?? []),
  );
  let calendarSummaryByKey = $state<Map<CalendarKey, string>>(
    new Map(cachedSnapshot?.calendarSummaryByKey ?? []),
  );
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

    // Refresh on window-focus regain — the user re-opening this detached
    // window (or alt-tabbing back to it) is the canonical "I want fresh
    // data" signal. Pairs with the cache hydrate up top: cached rows
    // paint synchronously on open, the focus event fires within a beat,
    // fresh data swaps in. Mount-then-focus pair on first open is deduped
    // by the `loading` guard at the top of refresh(), so we don't pay
    // for a double-fetch.
    let unlistenFocus: (() => void) | null = null;
    void getCurrentWindow()
      .onFocusChanged(({ payload: focused }) => {
        if (focused) void refresh();
      })
      .then((fn) => {
        unlistenFocus = fn;
      });

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
      unlistenFocus?.();
    };
  });

  async function refresh() {
    // Dedupe concurrent calls. The lifecycle $effect, the 30s poll, the
    // focus-refresh listener, and the manual refresh button can all race;
    // bailing on an in-flight load is cheaper than queueing them and
    // matches the existing refresh-button-disabled-while-loading UX.
    if (loading) return;
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

      // Persist after EVERYTHING (events + calendars) so the next open
      // hydrates a complete view — calendar maps need to be in the cache
      // too or the filter dropdown would be empty on next paint until the
      // second refresh ran. Best-effort: any write failure is swallowed
      // inside saveMeetingsCache and never breaks this code path.
      persistSnapshot();
    } catch (err) {
      listError = String(err);
    } finally {
      loading = false;
    }
  }

  /**
   * Serialize the current $state into a cache snapshot. Maps and Sets
   * become their `Array.from()` form so the payload roundtrips through
   * `JSON.stringify` — `JSON.stringify(new Map())` returns `"{}"`, which
   * would silently empty every map on the next load.
   */
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
    // `completed` stays in the map so a meeting whose transcript+notes
    // have landed renders a "Done" affordance on its row instead of
    // disappearing back to an "Invite" CTA — the user wants confirmation
    // that the bot attended AND finished its job. Past events drop out
    // of `events` itself on the next 30s poll, so the "Done" row clears
    // naturally without us having to remember to forget it.
    return (
      s === 'scheduled' ||
      s === 'joining' ||
      s === 'recording' ||
      s === 'processing' ||
      s === 'completed'
    );
  }

  async function onInvite(evt: MeetingEvent) {
    const url = eventMeetingUrl(evt);
    if (!url) {
      flashToast('warn', 'No meeting URL on this event.');
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
        flashToast('warn', friendlyError(err, "Couldn't invite the bot."));
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
      flashToast('warn', friendlyError(err, "Couldn't remove the bot."));
    } finally {
      const next = new Set(rowPending);
      next.delete(key);
      rowPending = next;
    }
  }

  /**
   * Force the bot to join NOW — third row icon, the "bot join now"
   * affordance. Same shape as `onInvite` but hits `meetings_join_bot_now`
   * which routes to `POST /v1/bot/join-now`.
   *
   * The server decides whether to PATCH `join_at` on an existing scheduled
   * bot, no-op an already-joining/recording one, or create a fresh bot
   * (meeting-restarted / failed-join cases) — see bot.service.ts::joinBotNow.
   * From the UI side this is always the same call.
   *
   * The 409 special-case from `onInvite` doesn't apply here — join-now
   * intentionally bypasses dedup, so a 409 would be a real conflict (e.g.
   * the inviteBot race-window catcher) and should surface as a warning.
   */
  async function onJoinNow(evt: MeetingEvent) {
    const url = eventMeetingUrl(evt);
    if (!url) {
      flashToast('warn', 'No meeting URL on this event.');
      return;
    }
    const key = evt.id;
    if (rowPending.has(key)) return;
    rowPending = new Set(rowPending).add(key);
    try {
      await invoke<ScheduledBot>('meetings_join_bot_now', {
        meetingUrl: url,
        calendarEventId: evt.id,
        companyId: evt.sourceCompanyUid ?? null,
      });
      flashToast('info', "Bot's on the way.");
      await refresh();
    } catch (err) {
      flashToast('warn', friendlyError(err, "Couldn't tell the bot to join."));
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
      flashToast('warn', friendlyError(err, "Couldn't invite the bot."));
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

  function flashToast(kind: 'info' | 'warn', text: string) {
    toast = { kind, text };
    setTimeout(() => {
      if (toast && toast.text === text) toast = null;
    }, 4000);
  }

  interface DayGroup {
    label: string;
    events: MeetingEvent[];
  }

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

  /** Default ON — most users open the window to invite a bot, and a
   *  link-less event is non-actionable. Toggle off via the link chip in
   *  the controls row to also surface meetings without a join URL. */
  let showOnlyWithUrl = $state(true);

  /** Fixed colour palette assigned to calendars in stable sorted order.
   *  Chosen for legibility on the `#18181b` background — saturated enough
   *  to scan-distinguish, muted enough not to vibrate. Repeats once the
   *  user has >12 enabled calendars, which is rare and acceptable since
   *  the dropdown still labels each one. */
  const CAL_PALETTE = [
    '#60a5fa', // blue
    '#f87171', // red
    '#34d399', // green
    '#fbbf24', // amber
    '#a78bfa', // purple
    '#f472b6', // pink
    '#2dd4bf', // teal
    '#fb923c', // orange
    '#93c5fd', // sky
    '#c084fc', // violet
    '#fcd34d', // yellow
    '#4ade80', // lime
  ] as const;

  /** Stable colour-per-calendar map. Sorted by key so a calendar keeps
   *  the same colour across refreshes (and across days within a
   *  session). When the enabled-calendar set changes, the assignment
   *  re-derives — which can shuffle colours if a calendar is
   *  added/removed in the middle of the list. Acceptable trade-off:
   *  the dropdown also re-renders with the new swatches in lockstep. */
  const calendarColors = $derived<Map<CalendarKey, string>>(
    buildCalendarColors(allCalKeys),
  );

  function buildCalendarColors(
    keys: Array<{ key: CalendarKey }>,
  ): Map<CalendarKey, string> {
    const sorted = [...keys].sort((a, b) => (a.key < b.key ? -1 : 1));
    const out = new Map<CalendarKey, string>();
    sorted.forEach((c, i) => {
      out.set(c.key, CAL_PALETTE[i % CAL_PALETTE.length]);
    });
    return out;
  }

  /** Colour for an event's source calendar. Returns a neutral gray when
   *  the event predates BE-4 (no sourceAccountId/sourceCalendarId) or
   *  references a calendar the user has since disabled. */
  function eventCalColor(e: MeetingEvent): string {
    if (!e.sourceAccountId || !e.sourceCalendarId) return '#3f3f46';
    return calendarColors.get(calKey(e.sourceAccountId, e.sourceCalendarId)) ?? '#3f3f46';
  }

  /** Composite tooltip for the title — surfaces what the now-removed
   *  badges used to show (calendar/account/company/platform), so users
   *  who need that context can still hover for it without us spending
   *  vertical real-estate on chips. */
  function eventRowTooltip(e: MeetingEvent): string {
    const lines: string[] = [];
    if (e.summary) lines.push(e.summary);
    const src = sourceLabel(e);
    if (src && src !== 'Calendar') lines.push(`Source: ${src}`);
    const co = companyLabel(e);
    if (co) lines.push(`Company: ${co}`);
    const plat = platformLabel(e);
    if (plat) lines.push(`Platform: ${plat}`);
    return lines.join('\n');
  }

  /**
   * Events filtered by the user's current selection AND the link-only
   * toggle. `null` selectedCalKeys means "show all calendars" — the
   * default. Switching to a Set (even empty) is the filter active state.
   */
  const filteredEvents = $derived<MeetingEvent[]>(
    filterEvents(events, selectedCalKeys, showOnlyWithUrl),
  );

  /** Filtered events grouped into day buckets for rendering. Declared after
   *  `filteredEvents` (and `showOnlyWithUrl`) so it never references a
   *  block-scoped binding before its declaration. */
  const groupedEvents = $derived<DayGroup[]>(groupByDay(filteredEvents));

  /** Count of events the link-only filter is currently hiding. Drives
   *  the "X hidden — show all" recovery affordance so the user doesn't
   *  hit an empty list and assume the calendar is broken. */
  const hiddenByUrlFilter = $derived(
    showOnlyWithUrl ? events.filter((e) => eventMeetingUrl(e) === null).length : 0,
  );

  function filterEvents(
    list: MeetingEvent[],
    selection: Set<CalendarKey> | null,
    onlyWithUrl: boolean,
  ): MeetingEvent[] {
    let out = list;
    if (selection !== null) {
      out = out.filter((e) => {
        if (!e.sourceAccountId || !e.sourceCalendarId) return false;
        return selection.has(calKey(e.sourceAccountId, e.sourceCalendarId));
      });
    }
    if (onlyWithUrl) {
      out = out.filter((e) => eventMeetingUrl(e) !== null);
    }
    return out;
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
    | 'processing'
    | 'done';

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
      case 'completed':
        return 'done';
      default:
        // Defensive fallback — failed bots aren't in the active map, so
        // hitting here means an unknown status. Render as Invite so the
        // user can recover by re-scheduling.
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
      case 'done':
        return 'Done';
    }
  }
</script>

<div class="meetings-page">
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
    <p class="toast" class:toast-warn={toast.kind === 'warn'}>
      {toast.text}
    </p>
  {/if}

  <!-- Controls row — always present below the URL input. Hosts the
       refresh button (right) and, when the user has more than one
       connected account / any enabled calendars, the multi-account
       filter dropdown trigger (left). Single-account users still see
       a working refresh; the filter trigger just sits hidden until
       there's something to filter by. -->
  <div class="controls-row" bind:this={filterRowEl}>
    <div class="controls-row-left">
      {#if accounts.length > 1 || allCalKeys.length > 0}
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
      {/if}
      <!-- Link-only filter chip. Default ON so the list focuses on
           actionable meetings; click to also surface link-less events
           (one-on-one calendar holds, all-day blocks, focus time). -->
      <button
        type="button"
        class="filter-trigger filter-link"
        class:filter-link-active={showOnlyWithUrl}
        aria-pressed={showOnlyWithUrl}
        title={showOnlyWithUrl
          ? 'Showing meetings with join links — click to show all'
          : 'Showing all meetings — click to filter to those with join links'}
        onclick={() => (showOnlyWithUrl = !showOnlyWithUrl)}
      >
        <svg width="11" height="11" viewBox="0 0 16 16" aria-hidden="true">
          <path
            d="M6.5 9.5l-2 2a2.5 2.5 0 1 1-3.5-3.5l3-3a2.5 2.5 0 0 1 3.5 0M9.5 6.5l2-2a2.5 2.5 0 1 1 3.5 3.5l-3 3a2.5 2.5 0 0 1-3.5 0"
            stroke="currentColor"
            stroke-width="1.4"
            fill="none"
            stroke-linecap="round"
            stroke-linejoin="round"
          />
        </svg>
        <span>{showOnlyWithUrl ? 'With link' : 'All'}</span>
      </button>
    </div>
    <button
      type="button"
      class="controls-refresh"
      onclick={refresh}
      disabled={loading}
      title="Refresh"
      aria-label="Refresh meetings"
    >
      <svg width="13" height="13" viewBox="0 0 16 16" fill="none" xmlns="http://www.w3.org/2000/svg" aria-hidden="true">
        <path d="M1.5 8a6.5 6.5 0 0 1 11.48-4.16" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" />
        <path d="M14.5 8A6.5 6.5 0 0 1 3.02 12.16" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" />
        <path d="M11 1.5v2.5h2.5" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" />
        <path d="M5 12h-2.5v2.5" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" />
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
                    <!-- Colour swatch matches the row's left bar so users
                         can map "this blue line in the list" back to
                         "this calendar" without reading the label. -->
                    <span
                      class="filter-swatch"
                      style="background:{calendarColors.get(key) ?? '#3f3f46'}"
                      aria-hidden="true"
                    ></span>
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

  <section class="meetings-body">
    {#if loading && events.length === 0}
      <p class="meetings-placeholder">Loading…</p>
    {:else if listError}
      <p class="meetings-error">{listError}</p>
    {:else if accounts.length === 0}
      <div class="meetings-empty">
        <p class="meetings-empty-title">No calendars connected yet</p>
        <p class="meetings-empty-copy">
          Connect a Google Calendar in HQ Console to start capturing meetings here.
        </p>
        <button
          type="button"
          class="meetings-empty-btn"
          onclick={() => {
            openExternal('https://hq.getindigo.ai/integrations').catch((err) => {
              flashToast('warn', friendlyError(err, "Couldn't open HQ Console."));
            });
          }}
        >
          Open HQ Console Integrations
        </button>
      </div>
    {:else if events.length === 0}
      <p class="meetings-placeholder">
        No upcoming meetings in your connected calendars.
      </p>
    {:else if filteredEvents.length === 0}
      <p class="meetings-placeholder">
        {#if showOnlyWithUrl && hiddenByUrlFilter === events.length}
          No upcoming meetings have a join link.
          <button
            type="button"
            class="meetings-inline-link"
            onclick={() => (showOnlyWithUrl = false)}
          >
            Show all
          </button>
        {:else}
          No meetings match the current filters.
          {#if selectedCalKeys !== null}
            <button
              type="button"
              class="meetings-inline-link"
              onclick={selectAllCalKeys}
            >
              Show all calendars
            </button>
          {/if}
          {#if showOnlyWithUrl && hiddenByUrlFilter > 0}
            <button
              type="button"
              class="meetings-inline-link"
              onclick={() => (showOnlyWithUrl = false)}
            >
              Show {hiddenByUrlFilter} without link
            </button>
          {/if}
        {/if}
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
              <!-- Calendar colour bar — encodes which (account, calendar)
                   the event came from. Replaces the multi-row badge
                   block; the same colour shows next to the matching
                   calendar in the filter dropdown so the mapping is
                   self-explanatory. -->
              <span class="event-cal-bar" style="background:{eventCalColor(evt)}" aria-hidden="true"></span>
              <div class="event-meta">
                <span class="event-time">{timeLabel(evt)}</span>
                <span class="event-title" title={eventRowTooltip(evt)}>
                  {evt.summary ?? '(no title)'}
                </span>
              </div>
              <!-- Action cluster: Join (open URL) + per-state bot button.
                   Both icon-only — the rich state lives in colour + tooltip
                   so the row stays dense. Tooltips carry the meaning so an
                   icon-only design doesn't sacrifice accessibility. -->
              <div class="row-actions">
                {#if url}
                  <button
                    type="button"
                    class="row-icon-btn row-icon-join"
                    title="Open meeting in browser"
                    aria-label="Open meeting in browser"
                    onclick={() => {
                      openExternal(url).catch((err) => {
                        flashToast('warn', friendlyError(err, "Couldn't open the meeting."));
                      });
                    }}
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
                    onclick={() => onInvite(evt)}
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
                    onclick={() => onUninvite(evt)}
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
                    onclick={() => onUninvite(evt)}
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
                    onclick={() => onUninvite(evt)}
                  >
                    <span class="row-icon-spinner row-icon-spinner-amber" aria-hidden="true"></span>
                  </button>
                {:else if kind === 'processing'}
                  <!-- Meeting ended, transcript pipeline running. Not
                       cancellable — bot isn't holding a Recall slot
                       anymore. Muted blue tint, ellipsis glyph. -->
                  <span class="row-icon-btn row-icon-processing" title="Processing transcript">
                    <svg width="12" height="12" viewBox="0 0 12 12" fill="currentColor" aria-hidden="true">
                      <circle cx="2.5" cy="6" r="1" />
                      <circle cx="6" cy="6" r="1" />
                      <circle cx="9.5" cy="6" r="1" />
                    </svg>
                  </span>
                {:else}
                  <!-- done: pipeline finished, transcript + notes stored.
                       Past events fall out on the next 30s poll. -->
                  <span class="row-icon-btn row-icon-done" title="Done — transcript saved">
                    <svg width="12" height="12" viewBox="0 0 12 12" fill="none" aria-hidden="true">
                      <path d="M2.5 6.5L5 9L9.5 3.5" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" />
                    </svg>
                  </span>
                {/if}
                <!-- Bot-join-now — the third row icon. Force the bot to
                     join NOW, regardless of pre-scheduled join_at. Covers
                     three scenarios behind one click: meeting started
                     early (reschedule scheduled bot to now), meeting
                     restarted after the bot left (fresh invite), and
                     bot-failed-to-join recovery (fresh invite). Server
                     side picks the right path — `joinBotNow` in
                     hq-pro/src/meetings/bot/bot.service.ts. -->
                {#if url}
                  <button
                    type="button"
                    class="row-icon-btn row-icon-bot-now"
                    disabled={pending}
                    title={pending ? 'Telling bot to join…' : 'Tell bot to join now'}
                    aria-label="Tell bot to join now"
                    onclick={() => onJoinNow(evt)}
                  >
                    {#if pending}
                      <span class="row-icon-spinner" aria-hidden="true"></span>
                    {:else}
                      <svg width="12" height="12" viewBox="0 0 12 12" fill="none" aria-hidden="true">
                        <!-- antenna -->
                        <line x1="6" y1="1" x2="6" y2="2.5" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" />
                        <!-- head -->
                        <rect x="2" y="3" width="8" height="6.5" rx="1.5" stroke="currentColor" stroke-width="1.4" />
                        <!-- eyes -->
                        <circle cx="4.6" cy="6.5" r="0.7" fill="currentColor" />
                        <circle cx="7.4" cy="6.5" r="0.7" fill="currentColor" />
                      </svg>
                    {/if}
                  </button>
                {/if}
              </div>
            </li>
          {/each}
        </ul>
      {/each}
    {/if}
  </section>
</div>

<style>
  /* Scoped via `data-window` (set in main.ts) so this opaque body bg
     can't bleed into the main popover, which needs transparency for
     vibrancy to show through. Same rule for #app — was unscoped
     before and would have applied to every window's mount point. */
  :global(html[data-window='meetings-window']),
  :global(html[data-window='meetings-window'] body) {
    margin: 0;
    padding: 0;
    height: 100vh;
    background: #18181b;
    color: #f4f4f5;
    font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
    font-size: 13px;
    overflow: hidden;
  }
  :global(html[data-window='meetings-window'] #app) {
    height: 100vh;
  }

  .meetings-page {
    display: flex;
    flex-direction: column;
    height: 100vh;
    background: #18181b;
    color: #f4f4f5;
  }

  .url-invite-row {
    display: flex;
    gap: 8px;
    padding: 14px 18px 6px;
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
  /* Warn — yellow, used for recoverable user-facing failures (per HQ
     policy: avoid red error states for things the user can retry). Same
     amber palette as the cross-account conflict warning in hq-console
     for a consistent failure-vocabulary across the suite. */
  .toast-warn {
    background: rgba(202, 138, 4, 0.10);
    border-color: rgba(202, 138, 4, 0.40);
    color: #fcd34d;
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
  /* Compacted row — was 10px vertical, now 6px. Gap from meta to
     action cluster tightened to match the smaller icon buttons. */
  .event-row {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 6px 4px;
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
    gap: 1px;
  }
  .event-time {
    font-size: 10px;
    color: #a1a1aa;
    line-height: 1.2;
  }
  .event-title {
    font-size: 12px;
    color: #f4f4f5;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    line-height: 1.3;
  }
  /* Per-calendar colour bar at the row's left edge. Replaces the
     calendar/account/company/platform text chips that used to occupy
     a second row inside .event-meta. Same colour is drawn next to the
     calendar in the filter dropdown (.filter-swatch) so the encoding
     is discoverable. */
  .event-cal-bar {
    flex: 0 0 auto;
    align-self: stretch;
    width: 3px;
    border-radius: 2px;
    /* Small inset so the bar reads as a coloured marker rather than
       filling the row's full vertical padding region. */
    margin: 2px 0;
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

  /* Empty-state CTA shown when the user has zero connected calendar
     accounts. Distinct from `.meetings-placeholder` because we want a
     button-affordance, not just gray copy. Distinct from the
     "no events match the filter" case (which uses an inline link) —
     this is a first-run handoff to HQ Console where the actual
     calendar OAuth lives. */
  .meetings-empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 8px;
    padding: 28px 18px 12px;
    text-align: center;
  }
  .meetings-empty-title {
    margin: 0;
    color: #f4f4f5;
    font-size: 13px;
    font-weight: 500;
  }
  .meetings-empty-copy {
    margin: 0;
    color: #a1a1aa;
    font-size: 12px;
    max-width: 280px;
    line-height: 1.4;
  }
  .meetings-empty-btn {
    margin-top: 6px;
    background: rgba(255, 255, 255, 0.12);
    color: #f4f4f5;
    border: 1px solid rgba(255, 255, 255, 0.20);
    border-radius: 6px;
    padding: 7px 14px;
    font-size: 12px;
    cursor: pointer;
  }
  .meetings-empty-btn:hover {
    background: rgba(255, 255, 255, 0.18);
  }
  .meetings-empty-btn:focus-visible {
    outline: 2px solid #93c5fd;
    outline-offset: 2px;
  }

  /* ── Controls row (filter + refresh) ───────────────────────────────── */
  .controls-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    position: relative;
    /* Right padding matches `.url-invite-row` (18px) so the refresh
       button's right edge lines up with the Invite button above it.
       Off-by-6px alignment reads as broken even though it's tiny. */
    padding: 6px 18px 0;
  }
  .controls-row-left {
    display: flex;
    align-items: center;
    gap: 6px;
    min-height: 24px;
  }
  .controls-refresh {
    width: 24px;
    height: 24px;
    border: 1px solid rgba(255, 255, 255, 0.10);
    background: rgba(255, 255, 255, 0.03);
    color: #d4d4d8;
    border-radius: 6px;
    cursor: pointer;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    padding: 0;
  }
  .controls-refresh:hover:not(:disabled) {
    background: rgba(255, 255, 255, 0.08);
    color: #f4f4f5;
  }
  .controls-refresh:disabled {
    opacity: 0.5;
    cursor: wait;
  }

  /* ── Calendar filter dropdown ──────────────────────────────────────── */
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
    left: 18px;
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
  /* Colour swatch in the filter dropdown — matches the per-row left bar
     so users can map "the blue line over there" to "this calendar"
     without reading the label. 10x10 keeps it visible next to the
     checkbox without crowding the calendar name. */
  .filter-swatch {
    flex: 0 0 auto;
    width: 10px;
    height: 10px;
    border-radius: 2px;
    margin-right: 2px;
  }
  /* live-dot: still used inside the in-call row icon button to pulse a
     red marker while the bot is recording. The "Live" text badge that
     also referenced this was retired with the rest of the chips. */
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

  /* ── Compact icon-button row actions ─────────────────────────────────
     Replaced the prior text-pill buttons (~78px wide each) with 24x24
     icon buttons. The status colour vocabulary is preserved (muted /
     red live / amber joining / blue processing / green done) but each
     state collapses to a single glyph. Tooltips carry the meaning so
     accessibility doesn't degrade. */
  .row-actions {
    flex: 0 0 auto;
    display: inline-flex;
    align-items: center;
    gap: 4px;
  }
  .row-icon-btn {
    flex: 0 0 auto;
    width: 24px;
    height: 24px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    border-radius: 5px;
    border: 1px solid rgba(255, 255, 255, 0.10);
    background: rgba(255, 255, 255, 0.03);
    color: #d4d4d8;
    cursor: pointer;
    padding: 0;
    transition: background 120ms ease, color 120ms ease, border-color 120ms ease;
  }
  .row-icon-btn:hover:not(:disabled) {
    background: rgba(255, 255, 255, 0.10);
    color: #f4f4f5;
    border-color: rgba(255, 255, 255, 0.20);
  }
  .row-icon-btn:focus-visible {
    outline: 2px solid rgba(180, 180, 255, 0.7);
    outline-offset: 1px;
  }
  .row-icon-btn:disabled {
    opacity: 0.6;
    cursor: wait;
  }
  /* Empty placeholder — renders when there's no URL. Inert; same square
     so the trailing column stays aligned with rows that do have a URL. */
  .row-icon-empty {
    color: #52525b;
    cursor: default;
    background: transparent;
    border-color: transparent;
    font-size: 12px;
  }
  /* Open-in-browser — discreet so the eye lands on the primary state
     button first. Identical box size, just lower base contrast. */
  .row-icon-join {
    color: #a1a1aa;
    background: transparent;
    border-color: rgba(255, 255, 255, 0.08);
  }
  /* Invite CTA — brighter border + fill so it reads as actionable. */
  .row-icon-invite {
    color: #f4f4f5;
    background: rgba(255, 255, 255, 0.12);
    border-color: rgba(255, 255, 255, 0.28);
  }
  .row-icon-invite:hover:not(:disabled) {
    background: rgba(255, 255, 255, 0.20);
  }
  /* Invited — muted check; hover hints at the uninvite affordance. */
  .row-icon-invited {
    color: #a1a1aa;
  }
  .row-icon-invited:hover:not(:disabled) {
    color: #fca5a5;
    background: rgba(220, 38, 38, 0.12);
    border-color: rgba(220, 38, 38, 0.40);
  }
  /* In-call — red tint to broadcast "live" at a glance. The live-dot
     animation does the pulsing. */
  .row-icon-incall {
    color: #fca5a5;
    background: rgba(220, 38, 38, 0.12);
    border-color: rgba(220, 38, 38, 0.40);
  }
  .row-icon-incall:hover:not(:disabled) {
    background: rgba(220, 38, 38, 0.22);
  }
  /* Joining — amber spinner; transient state. */
  .row-icon-joining {
    color: #fcd34d;
    background: rgba(202, 138, 4, 0.10);
    border-color: rgba(202, 138, 4, 0.40);
  }
  /* Processing — blue muted; non-interactive (no hover lift). */
  .row-icon-processing {
    color: #93c5fd;
    background: rgba(59, 130, 246, 0.08);
    border-color: rgba(59, 130, 246, 0.30);
    cursor: default;
  }
  /* Done — green muted; non-interactive. */
  .row-icon-done {
    color: #86efac;
    background: rgba(34, 197, 94, 0.08);
    border-color: rgba(34, 197, 94, 0.30);
    cursor: default;
  }
  /* Bot-join-now — amber-accented "act now" affordance. Distinct from
     the green of `invite` / red of `incall` so users learn to read it
     as a separate, always-available control rather than confusing it
     with a state indicator. */
  .row-icon-bot-now {
    color: #fcd34d;
    background: rgba(202, 138, 4, 0.08);
    border-color: rgba(202, 138, 4, 0.32);
  }
  .row-icon-bot-now:hover:not(:disabled) {
    background: rgba(202, 138, 4, 0.18);
    border-color: rgba(202, 138, 4, 0.55);
  }

  /* Inline spinner — used inside row-icon-btn while a request is pending.
     12px box matches the SVG icons it replaces so the button doesn't
     resize when state flips between idle and pending. */
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

  /* ── Link-only filter chip ───────────────────────────────────────────
     Sits next to the calendar filter trigger. Default ON — visual is
     "filled" when active so the user sees at a glance that the list is
     filtered. Click toggles, tooltip explains. */
  .filter-link {
    /* Inherits .filter-trigger layout; only colour state diverges. */
  }
  .filter-link-active {
    color: #bfdbfe;
    background: rgba(96, 165, 250, 0.12);
    border-color: rgba(96, 165, 250, 0.35);
  }
  .filter-link-active:hover {
    background: rgba(96, 165, 250, 0.20);
  }
</style>
