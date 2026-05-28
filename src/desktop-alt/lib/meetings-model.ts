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
  for (const account of accounts) {
    const enabled = enabledCalIdsByAccount.get(account.accountId) ?? new Set<string>();
    for (const calendar of calendarsByAccount.get(account.accountId) ?? []) {
      if (enabled.size > 0 && !enabled.has(calendar.id)) continue;
      const key = `${account.accountId}|${calendar.id}`;
      const companyUid = companyByCalendar.get(key) ?? null;
      const membership = companyUid ? membershipByUid.get(companyUid) : undefined;
      rows.push({
        key,
        email: account.email ?? account.accountId,
        calendar: calendar.summary,
        routingTarget: membership?.companyName ?? (companyUid ? shortUid(companyUid) : 'Personal'),
        status: membership?.status ?? 'active',
      });
    }
  }

  if (rows.length === 0) {
    return memberships.map((membership) => ({
      key: membership.companyUid,
      email: 'Calendar routing',
      calendar: membership.companyName ?? shortUid(membership.companyUid),
      routingTarget: membership.companyName ?? shortUid(membership.companyUid),
      status: membership.status,
    }));
  }

  return rows.sort((a, b) => `${a.email}${a.calendar}`.localeCompare(`${b.email}${b.calendar}`));
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

function shortUid(uid: string): string {
  const short = uid.slice(0, 12);
  return short.length === 12 ? `${short}...` : short;
}
