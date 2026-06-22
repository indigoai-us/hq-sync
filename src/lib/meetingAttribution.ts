import { invoke } from '@tauri-apps/api/core';

export const UNATTRIBUTED = 'unknown';

export interface ScheduledBotLike {
  botId: string;
  companyId?: string | null;
  meetingTitle?: string | null;
  status?: string | null;
  meetingUrl?: string | null;
  scheduledStartTime?: string | null;
}

export interface CompanyMembershipLike {
  companyUid: string;
  companyName?: string | null;
  role?: string | null;
  status?: string | null;
}

export interface CompanyOption {
  companyUid: string;
  label: string;
}

export interface SetCompanySuccess {
  ok: true;
  meetingId: string;
  companyId: string;
  seriesKey?: string | null;
  appliedToSeries?: boolean;
  refiled?: boolean;
}

export interface SetCompanyError {
  ok: false;
  code?: string;
  error?: string;
}

export function isUnattributed(bot: ScheduledBotLike): boolean {
  const value = bot.companyId?.trim();
  return !value || value.toLowerCase() === UNATTRIBUTED;
}

export function attributionCompanyName(
  bot: ScheduledBotLike,
  memberships: CompanyMembershipLike[],
): string | null {
  const companyId = bot.companyId?.trim();
  if (!companyId || isUnattributed(bot)) return null;
  const match = memberships.find((m) => m.companyUid === companyId);
  if (!match) return null;
  return match.companyName?.trim() || match.companyUid;
}

export function attributionLabel(
  bot: ScheduledBotLike,
  memberships: CompanyMembershipLike[],
): string {
  return attributionCompanyName(bot, memberships) ?? 'Unassigned';
}

export function companyOptions(
  memberships: CompanyMembershipLike[],
): CompanyOption[] {
  const byUid = new Map<string, CompanyOption>();
  for (const membership of memberships) {
    const uid = membership.companyUid?.trim();
    if (!uid || byUid.has(uid)) continue;
    const status = membership.status?.trim().toLowerCase();
    if (status && status !== 'active') continue;
    byUid.set(uid, {
      companyUid: uid,
      label: membership.companyName?.trim() || uid,
    });
  }
  return [...byUid.values()].sort((a, b) =>
    a.label.localeCompare(b.label, undefined, { sensitivity: 'base' }),
  );
}

export function selectUnattributed(
  bots: ScheduledBotLike[],
): ScheduledBotLike[] {
  return bots.filter(
    (bot) =>
      isUnattributed(bot) &&
      bot.status?.trim().toLowerCase() !== 'cancelled',
  );
}

export function buildSetCompanyArgs(
  meetingId: string,
  companyId: string,
  applyToSeries = true,
): { meetingId: string; companyId: string; applyToSeries: boolean } {
  return { meetingId, companyId, applyToSeries };
}

export function parseSetCompanyResult(
  raw: unknown,
): SetCompanySuccess | SetCompanyError {
  if (raw && typeof raw === 'object' && 'ok' in raw) {
    const value = raw as Record<string, unknown>;
    if (value.ok === true) {
      return {
        ok: true,
        meetingId: typeof value.meetingId === 'string' ? value.meetingId : '',
        companyId: typeof value.companyId === 'string' ? value.companyId : '',
        seriesKey:
          typeof value.seriesKey === 'string' || value.seriesKey === null
            ? value.seriesKey
            : undefined,
        appliedToSeries:
          typeof value.appliedToSeries === 'boolean'
            ? value.appliedToSeries
            : undefined,
        refiled: typeof value.refiled === 'boolean' ? value.refiled : undefined,
      };
    }
    if (value.ok === false) {
      return {
        ok: false,
        code: typeof value.code === 'string' ? value.code : undefined,
        error: typeof value.error === 'string' ? value.error : undefined,
      };
    }
  }
  return { ok: false };
}

export function setCompanyErrorMessage(err: SetCompanyError): string {
  if (err.error?.trim()) return err.error;
  switch (err.code) {
    case 'company-access-denied':
      return "You don't have access to that company.";
    case 'meeting-not-found':
      return 'That meeting no longer exists.';
    case 'invalid-company':
    case 'missing-company':
      return 'Pick a valid company.';
    default:
      return "Couldn't update the meeting's company.";
  }
}

export function listScheduledBots(): Promise<ScheduledBotLike[]> {
  return invoke<ScheduledBotLike[]>('meetings_list_scheduled_bots', {
    calendarEventIds: null,
  });
}

export function listMemberships(): Promise<CompanyMembershipLike[]> {
  return invoke<CompanyMembershipLike[]>('meetings_list_memberships');
}

export async function setMeetingCompany(
  meetingId: string,
  companyId: string,
  applyToSeries = true,
): Promise<SetCompanySuccess | SetCompanyError> {
  const raw = await invoke<unknown>(
    'meetings_set_company',
    buildSetCompanyArgs(meetingId, companyId, applyToSeries),
  );
  return parseSetCompanyResult(raw);
}
