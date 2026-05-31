import { invoke } from '@tauri-apps/api/core';
import {
  ensureActiveMeetingListeners,
  loadRecordingCompanyContext,
  seedActiveMeetingsFromBackend,
} from '../../lib/activeMeetings';
import { loadMeetingsCache, saveMeetingsCache } from '../../lib/meetingsCache';
import { eventMeetingUrl, friendlyError } from './meetings-model';
import type {
  CompanyMembership,
  GoogleAccount,
  GoogleCalendar,
  MeetingEvent,
  ScheduledBot,
} from './meetings-model';

/** Outcome of a bot row-action, returned to the page so it can render the
 *  toast. Keeping the copy here — next to the invoke that produces it — lets
 *  the store own the network + lifecycle while the page owns only presentation.
 *  `null` from a method means "nothing to surface" (a no-op dedupe or a missing
 *  bot), so the page simply skips the toast. */
export interface ToastDescriptor {
  kind: 'info' | 'warn';
  text: string;
}

/** Return shape of `meetings_list_calendars_for_account` — the per-account
 *  calendar list plus the user's enabled selection. Mirrors the inline type
 *  in the classic MeetingsWindow (not exported from the model). */
interface AccountCalendars {
  calendars: GoogleCalendar[];
  selectedCalendarIds: string[];
}

// Poll cadence for the background refresh. Long enough to be cheap, short
// enough that an agenda opened minutes later is already current.
const POLL_INTERVAL_MS = 30_000;

// ---------------------------------------------------------------------------
// Module-level singleton state.
//
// This is the heart of the preload fix: the Meetings data lives here, at module
// scope, loaded ONCE at app start (startMeetingsStore, called from
// DesktopApp.onMount) and kept warm by a 30s poll. DesktopApp wraps its routes
// in {#key routeKey}, so navigating to Meetings remounts MeetingsPage every
// time — previously that remount ran a blocking network refresh on mount, which
// is what made the page take 5-10s to paint. Now the page is a thin consumer
// that reads this already-populated singleton, so the remount is instant and
// the network work happens off the nav path.
// ---------------------------------------------------------------------------

let events = $state<MeetingEvent[]>([]);
let accounts = $state<GoogleAccount[]>([]);
let calendarsByAccount = $state<Map<string, GoogleCalendar[]>>(new Map());
let enabledCalIdsByAccount = $state<Map<string, Set<string>>>(new Map());
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
// Per-row optimistic lock for bot actions (invite / cancel / join-now), keyed
// by calendar event id. The agenda reads it to disable a row's buttons + show a
// spinner while its invoke is in flight. Re-assigned a cloned Set on every
// mutation so Svelte 5 sees a fresh reference (it tracks Sets by identity).
let rowPending = $state<Set<string>>(new Set());

// Idempotency + lifecycle guards. The store outlives any single page, so we
// only ever start the listeners/poll once for the app's lifetime.
let started = false;
let pollTimer: ReturnType<typeof setInterval> | null = null;

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

// ---------------------------------------------------------------------------
// Bot row-actions (invite / cancel / join-now).
//
// These own the Tauri invoke + the per-row pending lock + the post-action
// refresh, and return a ToastDescriptor for the page to render. The classic
// MeetingsWindow keeps this logic inline; here it lives in the store so the
// presentational MeetingsAgenda subcomponent never imports `invoke` (US-006
// pins the agenda as invoke-free). Payloads mirror the classic window exactly.
// ---------------------------------------------------------------------------

function lockRow(key: string): boolean {
  if (rowPending.has(key)) return false;
  rowPending = new Set(rowPending).add(key);
  return true;
}

function unlockRow(key: string): void {
  const next = new Set(rowPending);
  next.delete(key);
  rowPending = next;
}

/** Schedule a recording bot for the event's meeting. Mirrors classic onInvite,
 *  including the benign-409 path (auto-schedule cron / another instance got
 *  there first) which refreshes and reports "already invited" rather than a
 *  scary failure. */
async function inviteBot(evt: MeetingEvent): Promise<ToastDescriptor | null> {
  const url = eventMeetingUrl(evt);
  if (!url) return { kind: 'warn', text: 'No meeting URL on this event.' };
  const key = evt.id;
  if (!lockRow(key)) return null;
  try {
    await invoke<ScheduledBot>('meetings_invite_bot', {
      meetingUrl: url,
      calendarEventId: evt.id,
      companyId: evt.sourceCompanyUid ?? null,
    });
    await refresh();
    return { kind: 'info', text: 'Bot invited.' };
  } catch (err) {
    const msg = String(err);
    if (msg.includes('409') || msg.includes('bot-already-scheduled')) {
      await refresh();
      return { kind: 'info', text: 'Already invited — refreshing.' };
    }
    return { kind: 'warn', text: friendlyError(err, "Couldn't invite the bot.") };
  } finally {
    unlockRow(key);
  }
}

/** Cancel the event's scheduled bot. No-op (returns null) when there's no bot
 *  on the row. No 409 special-case — a cancel conflict is a real failure. */
async function cancelBot(evt: MeetingEvent): Promise<ToastDescriptor | null> {
  const bot = botsByEventId.get(evt.id);
  if (!bot) return null;
  const key = evt.id;
  if (!lockRow(key)) return null;
  try {
    await invoke('meetings_cancel_bot', { botId: bot.botId });
    await refresh();
    return { kind: 'info', text: 'Bot uninvited.' };
  } catch (err) {
    return { kind: 'warn', text: friendlyError(err, "Couldn't remove the bot.") };
  } finally {
    unlockRow(key);
  }
}

/** Force the bot to join NOW. Same payload shape as inviteBot but hits
 *  meetings_join_bot_now; intentionally bypasses dedup, so a 409 here is a real
 *  conflict and surfaces as a warning (no benign-409 path). */
async function joinBotNow(evt: MeetingEvent): Promise<ToastDescriptor | null> {
  const url = eventMeetingUrl(evt);
  if (!url) return { kind: 'warn', text: 'No meeting URL on this event.' };
  const key = evt.id;
  if (!lockRow(key)) return null;
  try {
    await invoke<ScheduledBot>('meetings_join_bot_now', {
      meetingUrl: url,
      calendarEventId: evt.id,
      companyId: evt.sourceCompanyUid ?? null,
    });
    await refresh();
    return { kind: 'info', text: "Bot's on the way." };
  } catch (err) {
    return { kind: 'warn', text: friendlyError(err, "Couldn't tell the bot to join.") };
  } finally {
    unlockRow(key);
  }
}

/**
 * Start the singleton once for the app's lifetime. Called from
 * DesktopApp.onMount at launch so the data is warm before the user ever
 * navigates to Meetings; MeetingsPage.onMount also calls it so the page still
 * works in isolation (tests / direct mount). Idempotent via the `started` guard.
 *
 * Flow: cache-first synchronous paint -> attach the live detection listeners ->
 * one immediate network refresh -> 30s poll to stay current -> re-hydrate +
 * refresh on window focus, and re-hydrate on cross-window storage writes (a
 * refresh in the classic window reflects here without a network round-trip).
 */
export function startMeetingsStore(): void {
  if (started) return;
  started = true;

  hydrateFromCache();
  void ensureActiveMeetingListeners();
  // The desktop-alt window is created on-demand (after launch), so its JS
  // context misses `meeting:detected` events that fired before it existed.
  // Seed any already-active detections from the backend registry so a meeting
  // detected while the window was closed shows up (with a Record control) the
  // moment we open. The live listener above covers everything from here on.
  void seedActiveMeetingsFromBackend();
  // Resolve the recording-company context (active memberships + validated
  // default) so a detected meeting is attributed correctly out of the gate;
  // back-fills any rows that detected before this resolved. Fails soft.
  void loadRecordingCompanyContext();
  void refresh();

  pollTimer = setInterval(() => {
    void refresh();
  }, POLL_INTERVAL_MS);

  const onFocus = () => {
    hydrateFromCache();
    void loadRecordingCompanyContext();
    void refresh();
  };
  const onStorage = () => hydrateFromCache();
  window.addEventListener('focus', onFocus);
  window.addEventListener('storage', onStorage);
}

/**
 * Tear down the poll + listeners. Not used in the app (the store is meant to
 * live for the whole session) but exported so tests can reset between runs.
 */
export function stopMeetingsStore(): void {
  if (pollTimer !== null) {
    clearInterval(pollTimer);
    pollTimer = null;
  }
  started = false;
}

// Reactive read surface. Consumers read these getters inside their own
// $derived / template, which subscribes them to the underlying $state so a
// poll-driven refresh repaints every open view automatically.
export const meetingsStore = {
  get events() {
    return events;
  },
  get accounts() {
    return accounts;
  },
  get calendarsByAccount() {
    return calendarsByAccount;
  },
  get enabledCalIdsByAccount() {
    return enabledCalIdsByAccount;
  },
  get botsByEventId() {
    return botsByEventId;
  },
  get companyNamesByUid() {
    return companyNamesByUid;
  },
  get accountEmailById() {
    return accountEmailById;
  },
  get calendarSummaryByKey() {
    return calendarSummaryByKey;
  },
  get memberships() {
    return memberships;
  },
  get membershipsError() {
    return membershipsError;
  },
  get fetchError() {
    return fetchError;
  },
  get loading() {
    return loading;
  },
  get pendingEventIds() {
    return rowPending;
  },
  refresh,
  inviteBot,
  cancelBot,
  joinBotNow,
};
