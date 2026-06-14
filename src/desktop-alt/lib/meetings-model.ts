import type { ActiveMeeting } from '../../lib/activeMeetings';

export interface MeetingSignal {
  type?: string;
  kind?: string;
  category?: string;
  title?: string;
  text?: string;
  summary?: string;
}

export interface MeetingEvent {
  id: string;
  summary?: string;
  start: { dateTime?: string; date?: string; timeZone?: string };
  end: { dateTime?: string; date?: string; timeZone?: string };
  status: string;
  hangoutLink?: string;
  meetingUrl?: string | null;
  sourceCalendarId?: string;
  sourceCompanyUid?: string;
  sourceAccountId?: string;
  signals?: unknown;
}

export interface ScheduledBot {
  botId: string;
  meetingUrl: string;
  platform: string;
  status: string;
  calendarEventId?: string | null;
  meetingTitle?: string | null;
  scheduledStartTime?: string | null;
  autoScheduled: boolean;
  errorMessage?: string | null;
  /**
   * US-010 — the real source-landed signal from hq-pro: true only when the
   * transcript has actually been persisted to the vault as a queryable source
   * (`GET /v1/bot/list` HEADs `sources/meetings/{botId}.md`). The "Done —
   * transcript saved" row state is gated on this, NOT on `status ===
   * 'completed'` alone — a completed bot whose per-company ingest hard-failed
   * (the #240 KMS-drift symptom) arrives with `sourceLanded: false` and must
   * keep showing the processing state, not a false "saved". Optional on the
   * wire: a pre-US-010 server omits it (and the Rust client defaults it to
   * `false`), so an older backend never produces a premature "saved".
   */
  sourceLanded?: boolean;
}

export interface GoogleAccount {
  accountId: string;
  email?: string | null;
  scope?: string | null;
  connectedAt?: string | null;
  capabilities?: string[];
}

export interface GoogleCalendar {
  id: string;
  summary: string;
  primary?: boolean;
  accessRole?: string | null;
}

export interface CompanyMembership {
  companyUid: string;
  companyName?: string | null;
  role?: string | null;
  status: string;
}

export interface SignalCounts {
  actions: number;
  decisions: number;
  risks: number;
}

export interface ConnectedCalendarRow {
  key: string;
  email: string;
  calendar: string;
  routingTarget: string;
  status: string;
}

export function eventStart(event: MeetingEvent): Date | null {
  const raw = event.start.dateTime ?? event.start.date;
  if (!raw) return null;
  const date = new Date(raw);
  return Number.isNaN(date.getTime()) ? null : date;
}

export function eventEnd(event: MeetingEvent): Date | null {
  const raw = event.end.dateTime ?? event.end.date;
  if (!raw) return eventStart(event);
  const date = new Date(raw);
  return Number.isNaN(date.getTime()) ? eventStart(event) : date;
}

export function timeLabel(event: MeetingEvent): string {
  const start = eventStart(event);
  if (!start) return 'Time pending';
  return start.toLocaleTimeString(undefined, {
    hour: 'numeric',
    minute: '2-digit',
  });
}

export function rangeLabel(event: MeetingEvent): string {
  const start = eventStart(event);
  const end = eventEnd(event);
  if (!start) return 'Time pending';
  if (!end || end.getTime() === start.getTime()) return timeLabel(event);
  return `${timeLabel(event)}-${end.toLocaleTimeString(undefined, {
    hour: 'numeric',
    minute: '2-digit',
  })}`;
}

/**
 * Whole-minute duration between start and end. Returns null when either edge
 * is unparseable or the span is non-positive (so the row can omit the "· Nm"
 * suffix rather than render "· 0m"). Powers the design `.mtime` cell.
 */
export function durationMinutes(event: MeetingEvent): number | null {
  const start = eventStart(event);
  const end = eventEnd(event);
  if (!start || !end) return null;
  const mins = Math.round((end.getTime() - start.getTime()) / 60000);
  return mins > 0 ? mins : null;
}

export function isToday(event: MeetingEvent, now = new Date()): boolean {
  const start = eventStart(event);
  if (!start) return false;
  return (
    start.getFullYear() === now.getFullYear() &&
    start.getMonth() === now.getMonth() &&
    start.getDate() === now.getDate()
  );
}

export function sortByStart(a: MeetingEvent, b: MeetingEvent): number {
  return (eventStart(a)?.getTime() ?? 0) - (eventStart(b)?.getTime() ?? 0);
}

export function pickUpNext(events: MeetingEvent[], now = new Date()): MeetingEvent | null {
  return (
    events
      .filter((event) => (eventEnd(event)?.getTime() ?? 0) >= now.getTime())
      .sort(sortByStart)[0] ?? null
  );
}

/** Row lifecycle state, mirrored from the Claude Design `.meeting-row` mock. */
export type MeetingRowState = 'live' | 'next' | 'past' | 'scheduled';

/**
 * Resolve a row's display state. `live` wins when the event is the active
 * detection/recording (matched by id against the live meeting's sourceEventId),
 * then `next` for the up-next pick, then `past` once the event has ended, else
 * `scheduled`. Pure + `now`-injectable so the agenda stays presentational.
 */
export function meetingState(
  event: MeetingEvent,
  opts: { liveEventId?: string | null; upNextId?: string | null; now?: Date } = {},
): MeetingRowState {
  const { liveEventId = null, upNextId = null, now = new Date() } = opts;
  if (liveEventId && event.id === liveEventId) return 'live';
  if (upNextId && event.id === upNextId) return 'next';
  const end = eventEnd(event);
  if (end && end.getTime() < now.getTime()) return 'past';
  return 'scheduled';
}

/**
 * Subtitle label for a meeting row — the routed company name when known,
 * falling back to a short UID, else "Personal" for un-routed calendars.
 * The design row shows "{with} · {company}"; real events carry no attendee,
 * so we surface company only.
 */
export function companyLabel(event: MeetingEvent, companyNames: Map<string, string>): string {
  const uid = event.sourceCompanyUid;
  if (!uid) return 'Personal';
  // Never surface the raw cmp_… UID as a label — degrade to the generic name.
  return companyNames.get(uid) ?? 'Company';
}

export interface DayGroup {
  label: string;
  events: MeetingEvent[];
}

/**
 * Human day label relative to `now` — "Today", "Tomorrow", else a short
 * "Wed, Jun 3" style date. Ported from the classic MeetingsWindow so the alt
 * window's multi-day agenda reads identically. `now` is injectable for tests.
 */
export function dayLabel(date: Date, now = new Date()): string {
  const today = new Date(now.getFullYear(), now.getMonth(), now.getDate());
  const tomorrow = new Date(today);
  tomorrow.setDate(tomorrow.getDate() + 1);
  const eventDay = new Date(date.getFullYear(), date.getMonth(), date.getDate());
  if (eventDay.getTime() === today.getTime()) return 'Today';
  if (eventDay.getTime() === tomorrow.getTime()) return 'Tomorrow';
  return date.toLocaleDateString(undefined, {
    weekday: 'short',
    month: 'short',
    day: 'numeric',
  });
}

/**
 * Group events into chronological per-day buckets for the multi-day agenda.
 * Events are sorted by start first, so both the day order and the within-day
 * order come out chronological (Map preserves first-seen insertion order).
 * Events with no parseable start are dropped — they can't be placed on a day.
 */
export function groupByDay(events: MeetingEvent[], now = new Date()): DayGroup[] {
  const byLabel = new Map<string, MeetingEvent[]>();
  for (const event of [...events].sort(sortByStart)) {
    const start = eventStart(event);
    if (!start) continue;
    const label = dayLabel(start, now);
    const bucket = byLabel.get(label);
    if (bucket) bucket.push(event);
    else byLabel.set(label, [event]);
  }
  return Array.from(byLabel, ([label, eventsInDay]) => ({ label, events: eventsInDay }));
}

export function pickLiveMeeting(activeMeetings: ActiveMeeting[]): ActiveMeeting | null {
  const priority: Record<ActiveMeeting['state'], number> = {
    recording: 0,
    starting: 1,
    stopping: 2,
    detected: 3,
    error: 4,
  };
  return (
    [...activeMeetings].sort((a, b) => {
      const stateDelta = priority[a.state] - priority[b.state];
      if (stateDelta !== 0) return stateDelta;
      return Date.parse(b.detectedAt) - Date.parse(a.detectedAt);
    })[0] ?? null
  );
}

export function activeRecordingsFromScheduledBots(
  events: MeetingEvent[],
  botsByEventId: Map<string, ScheduledBot>,
): ActiveMeeting[] {
  const eventsById = new Map(events.map((event) => [event.id, event]));
  const rows: ActiveMeeting[] = [];

  for (const [eventId, bot] of botsByEventId) {
    if (bot.status !== 'recording') continue;
    const event = eventsById.get(eventId);
    rows.push({
      windowId: `scheduled-bot:${bot.botId}`,
      platform: bot.platform,
      meetingUrl: bot.meetingUrl,
      detectedAt:
        bot.scheduledStartTime ??
        event?.start.dateTime ??
        event?.start.date ??
        new Date(0).toISOString(),
      state: 'recording',
      recordingId: bot.botId,
      companyUid: event?.sourceCompanyUid ?? null,
      summary: bot.meetingTitle ?? event?.summary,
      sourceEventId: bot.calendarEventId ?? eventId,
    });
  }

  return rows.sort((a, b) => Date.parse(b.detectedAt) - Date.parse(a.detectedAt));
}

export function totalSignalCounts(events: MeetingEvent[]): SignalCounts {
  return events.reduce(
    (totals, event) => {
      const counts = signalCounts(event);
      totals.actions += counts.actions;
      totals.decisions += counts.decisions;
      totals.risks += counts.risks;
      return totals;
    },
    { actions: 0, decisions: 0, risks: 0 },
  );
}

export function signalCounts(event: MeetingEvent): SignalCounts {
  const signals = normalizeSignals(event.signals);
  return {
    actions: countSignalKind(signals, 'action'),
    decisions: countSignalKind(signals, 'decision'),
    risks: countSignalKind(signals, 'risk'),
  };
}

/**
 * Compact, human signal summary for a meeting row — only non-zero kinds,
 * pluralized, joined with " · " (e.g. "2 actions · 1 decision"). Empty string
 * when the meeting has no extracted signals, so the `.msig` cell stays blank
 * rather than rendering "0 actions · 0 decisions".
 */
export function signalSummary(counts: SignalCounts): string {
  const parts: string[] = [];
  if (counts.actions) parts.push(`${counts.actions} action${counts.actions === 1 ? '' : 's'}`);
  if (counts.decisions)
    parts.push(`${counts.decisions} decision${counts.decisions === 1 ? '' : 's'}`);
  if (counts.risks) parts.push(`${counts.risks} risk${counts.risks === 1 ? '' : 's'}`);
  return parts.join(' · ');
}

export function extractedSignalLabels(event: MeetingEvent): string[] {
  return normalizeSignals(event.signals)
    .map((signal) => signal.title ?? signal.summary ?? signal.text ?? signal.type ?? signal.kind)
    .filter((label): label is string => typeof label === 'string' && label.trim().length > 0)
    .slice(0, 3);
}

export function buildConnectedCalendarRows(
  accounts: GoogleAccount[],
  calendarsByAccount: Map<string, GoogleCalendar[]>,
  enabledCalIdsByAccount: Map<string, Set<string>>,
  events: MeetingEvent[],
  memberships: CompanyMembership[],
): ConnectedCalendarRow[] {
  const membershipByUid = new Map(memberships.map((row) => [row.companyUid, row]));
  const companyByCalendar = new Map<string, string | null>();
  for (const event of events) {
    if (!event.sourceAccountId || !event.sourceCalendarId) continue;
    companyByCalendar.set(
      `${event.sourceAccountId}|${event.sourceCalendarId}`,
      event.sourceCompanyUid ?? null,
    );
  }

  const rows: ConnectedCalendarRow[] = [];
  const hasCalendarInputs = accounts.some(
    (account) =>
      enabledCalIdsByAccount.has(account.accountId) ||
      (calendarsByAccount.get(account.accountId)?.length ?? 0) > 0,
  );
  for (const account of accounts) {
    const enabled = enabledCalIdsByAccount.get(account.accountId);
    for (const calendar of calendarsByAccount.get(account.accountId) ?? []) {
      if (enabled && !enabled.has(calendar.id)) continue;
      const key = `${account.accountId}|${calendar.id}`;
      const companyUid = companyByCalendar.get(key) ?? null;
      const membership = companyUid ? membershipByUid.get(companyUid) : undefined;
      rows.push({
        key,
        email: account.email ?? account.accountId,
        calendar: calendar.summary,
        routingTarget: membership?.companyName ?? (companyUid ? 'Company' : 'Personal'),
        status: membership?.status ?? 'active',
      });
    }
  }

  if (rows.length === 0 && !hasCalendarInputs) {
    return memberships.map((membership) => ({
      key: membership.companyUid,
      email: 'Calendar routing',
      calendar: membership.companyName ?? 'Company',
      routingTarget: membership.companyName ?? 'Company',
      status: membership.status,
    }));
  }

  return rows.sort((a, b) => `${a.email}${a.calendar}`.localeCompare(`${b.email}${b.calendar}`));
}

/**
 * Resolve a usable meeting URL for an event. Server-side extraction (BE-5)
 * populates `meetingUrl` from hangoutLink/conferenceData/description; fall back
 * to the raw `hangoutLink` for events served by a pre-BE-5 backend. Pure mirror
 * of the classic MeetingsWindow helper so row actions resolve identically.
 */
export function eventMeetingUrl(e: MeetingEvent): string | null {
  return e.meetingUrl ?? e.hangoutLink ?? null;
}

/**
 * Human platform label for a meeting row — "Google Meet" / "Zoom" / "Teams" /
 * "Webex", else empty string when the URL is missing or unrecognized.
 */
export function platformLabel(e: MeetingEvent): string {
  const url = eventMeetingUrl(e) ?? '';
  if (url.includes('meet.google.com')) return 'Google Meet';
  if (url.includes('zoom.us')) return 'Zoom';
  if (url.includes('teams.microsoft.com')) return 'Teams';
  if (url.includes('webex.com')) return 'Webex';
  return '';
}

/**
 * True when `url` looks like a real join link for a supported platform
 * (Zoom / Google Meet / Teams / Webex). Gates the row's invite/join affordances
 * so we never schedule a bot against a bogus URL.
 */
export function isPlausibleMeetingUrl(url: string): boolean {
  if (!url) return false;
  return (
    /^https:\/\/[^\s/]*\.zoom\.us\/j\/[^\s]+/i.test(url) ||
    /^https:\/\/meet\.google\.com\/[a-z-]+/i.test(url) ||
    /^https:\/\/teams\.microsoft\.com\/l\/meetup-join\/[^\s]+/i.test(url) ||
    /^https:\/\/[^\s/]*\.webex\.com\/[^\s]+/i.test(url)
  );
}

/**
 * Row action button lifecycle, driven by the scheduled bot's status:
 *   no bot     → "invite"     (CTA)
 *   scheduled  → "invited"    (click to cancel)
 *   joining    → "joining"    (transient)
 *   recording  → "in-call"    (live indicator, click to stop)
 *   processing → "processing" (non-cancellable, transient)
 *   completed  → "done" ONLY once the transcript has really landed as a vault
 *                source (`sourceLanded`); a completed-but-not-yet-landed bot
 *                stays "processing" — see below.
 */
export type RowButtonKind = 'invite' | 'invited' | 'joining' | 'in-call' | 'processing' | 'done';

/**
 * US-010 — "Done — transcript saved" must reflect a transcript that ACTUALLY
 * landed in the vault as a source, not merely a bot whose lifecycle status is
 * "completed". hq-pro's bot status flips on the Recall webhook / retry path,
 * but the per-company source write is a separate S3 PUT that can hard-fail
 * (the 2026-06-02 KMS-grant drift dead-lettered transcripts for ~13 days while
 * bots still read "completed"). hq-pro now exposes `sourceLanded` (a HEAD on
 * the real meeting source object); we render `done` ONLY when it's true.
 *
 * A `completed` bot WITHOUT a confirmed source is shown as `processing`, not a
 * false `done`: from the user's POV the transcript is still being finalised /
 * recovered (the hourly retry-pipeline + dead-letter drainer re-drive it), so
 * the honest state is "Processing", never "Done — transcript saved".
 */
export function rowButtonKind(bot: ScheduledBot | undefined): RowButtonKind {
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
      // Gate the terminal "done" on the real source-landed confirmation. Until
      // hq-pro confirms the transcript persisted as a vault source, keep
      // showing "processing" rather than a premature "Done — transcript saved".
      return bot.sourceLanded === true ? 'done' : 'processing';
    default:
      // Defensive fallback — failed bots aren't in the active map, so hitting
      // here means an unknown status. Render as Invite so the user can recover.
      return 'invite';
  }
}

export function rowButtonLabel(kind: RowButtonKind, pending: boolean): string {
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

/**
 * Map a raw invoke rejection to friendly, recoverable copy. Tries to parse a
 * JSON `{ error | message }` payload first, then falls back to HTTP-status
 * heuristics (409 already-scheduled, 401 re-auth, 403 forbidden, 5xx server),
 * else the caller-supplied fallback. Mirrors the classic MeetingsWindow.
 */
export function friendlyError(err: unknown, fallback: string): string {
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

function normalizeSignals(raw: unknown): MeetingSignal[] {
  if (!raw) return [];
  if (Array.isArray(raw)) return raw.filter(isSignalLike);
  if (typeof raw !== 'object') return [];
  const record = raw as Record<string, unknown>;
  const fromBuckets = ['actions', 'decisions', 'risks', 'actionItems'].flatMap((key) => {
    const value = record[key];
    if (!Array.isArray(value)) return [];
    return value.map((item) =>
      typeof item === 'object' && item !== null
        ? ({ ...item, type: key } as MeetingSignal)
        : ({ title: String(item), type: key } as MeetingSignal),
    );
  });
  if (fromBuckets.length > 0) return fromBuckets;
  if (Array.isArray(record.signals)) return record.signals.filter(isSignalLike);
  return [];
}

function countSignalKind(signals: MeetingSignal[], kind: 'action' | 'decision' | 'risk'): number {
  return signals.filter((signal) => {
    const label = `${signal.type ?? signal.kind ?? signal.category ?? ''}`.toLowerCase();
    if (kind === 'action') return label.includes('action');
    if (kind === 'decision') return label.includes('decision');
    return label.includes('risk');
  }).length;
}

function isSignalLike(value: unknown): value is MeetingSignal {
  return typeof value === 'object' && value !== null;
}

