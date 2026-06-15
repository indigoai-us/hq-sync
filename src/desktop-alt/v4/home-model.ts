import type { Workspace } from '../../lib/workspaces';
import { formatRelativeTime } from '../route';
import type { Project } from '../lib/projects-model';
import {
  companyLabel,
  eventStart,
  isToday,
  sortByStart,
  timeLabel,
  type MeetingEvent,
} from '../lib/meetings-model';
import {
  formatBytes,
  friendlySyncError,
  timeAgo,
  workspaceDisplayName,
  type ActivityEntry,
  type SyncCompanyRef,
  type SyncProgress,
  type SyncState,
  type WorkspaceSyncStats,
} from '../lib/sync-model';

/**
 * V4 Home model — pure derivations for the exception-based Home surface
 * (docs/design/v4/SPEC.md section 5, home-healthy/syncing/error.png).
 *
 * Home answers "is anything wrong, what needs me, what happened": a compact
 * meta line under the title, a NEEDS YOU queue of inline-action cards
 * (conflicts + core drift), a syncing progress card with per-company fanout,
 * a plain-language error card, and an actor-grouped narrative digest. All
 * logic lives here (node-testable); the Svelte components only render.
 */

// ── Conflicts (from the `sync:conflict` event stream) ───────────────────────

export interface HomeConflict {
  path: string;
  canAutoResolve: boolean;
  status: 'pending' | 'resolving' | 'error';
  /** When the conflict event arrived (ms epoch). */
  at: number;
  error?: string;
}

// ── Core drift (subset of the popover's `check_core_state` payload) ─────────

export interface HomeDriftEntry {
  path: string;
  size: number;
  gitShaLocal: string | null;
  gitShaUpstream: string | null;
}

export interface HomeDriftReport {
  /** USER-EDIT drift count — THE drift number (missing/added don't count). */
  count: number;
  modified: HomeDriftEntry[];
  missing: HomeDriftEntry[];
  added: HomeDriftEntry[];
  hqVersion: string;
  targetRepo: string;
  targetRef: string;
}

export interface HomeCoreState {
  targetVersion: string;
  driftReport: HomeDriftReport;
}

// ── Inline-action cards ──────────────────────────────────────────────────────

export type HomeCardTone = 'warn' | 'error' | 'neutral';

export interface HomeCardAction {
  id: string;
  label: string;
  kind: 'primary' | 'secondary' | 'text';
  disabled?: boolean;
}

export interface HomeCardModel {
  title: string;
  sub: string | null;
  tone: HomeCardTone;
  actions: HomeCardAction[];
}

/** Conflict card — Keep mine / Take theirs / Compare (home-healthy.png). */
export function getConflictCardModel(conflict: HomeConflict, now = Date.now()): HomeCardModel {
  const age = timeAgo(conflict.at).toLowerCase();
  const resolving = conflict.status === 'resolving';
  const subParts = [age];
  if (conflict.error) subParts.push(`could not resolve — ${conflict.error}`);
  else if (resolving) subParts.push('resolving…');
  else subParts.push(conflict.canAutoResolve ? 'can auto-resolve' : 'auto-resolve not possible');
  void now;
  return {
    title: `Conflict — ${conflict.path} was edited here and in the cloud`,
    sub: subParts.join(' · '),
    tone: 'warn',
    actions: [
      { id: 'keep-local', label: 'Keep mine', kind: 'primary', disabled: resolving },
      { id: 'keep-remote', label: 'Take theirs', kind: 'secondary', disabled: resolving },
      { id: 'compare', label: 'Compare', kind: 'text', disabled: false },
    ],
  };
}

/** Drift card — Restore / Keep edit / View diff (home-healthy.png). */
export function getDriftCardModel(core: HomeCoreState, restoring = false): HomeCardModel | null {
  const report = core.driftReport;
  if (!report || report.count <= 0) return null;
  const first = report.modified[0]?.path ?? report.missing[0]?.path ?? null;
  const version = core.targetVersion || report.hqVersion;
  const title =
    report.count === 1
      ? `1 core file drifted from v${version}${first ? ` — ${first} edited locally` : ''}`
      : `${report.count} core files drifted from v${version}${
          first ? ` — ${first} + ${report.count - 1} more edited locally` : ''
        }`;
  return {
    title,
    sub:
      report.count === 1
        ? 'restore will overwrite your local change'
        : 'restore will overwrite your local changes',
    tone: 'neutral',
    actions: [
      { id: 'restore', label: restoring ? 'Restoring…' : 'Restore', kind: 'primary', disabled: restoring },
      { id: 'keep-edit', label: 'Keep edit', kind: 'secondary', disabled: restoring },
      { id: 'view-diff', label: 'View diff', kind: 'text', disabled: false },
    ],
  };
}

/** NEEDS YOU queue count — pending/errored conflicts + the drift card. */
export function getNeedsYouCount(
  conflicts: HomeConflict[],
  core: HomeCoreState | null,
  driftDismissed: boolean,
): number {
  const conflictCount = conflicts.length;
  const driftCount =
    !driftDismissed && core && core.driftReport && core.driftReport.count > 0 ? 1 : 0;
  return conflictCount + driftCount;
}

// ── Meta line under the Home title ──────────────────────────────────────────

export interface HomeMetaInput {
  syncState: SyncState;
  /** `realtimeSync` preference — null while the settings fetch is pending. */
  autoSyncOn: boolean | null;
  daemonRunning: boolean | null;
  /** Relative last-sync label ("just now", "5m ago"), null when never. */
  lastSyncLabel: string | null;
  /** Local hq-core version ("15.0.15"), null when core.yaml is unreadable. */
  hqVersion: string | null;
  /** Wall-clock label for when the running sync started ("11:32 AM"). */
  syncStartedLabel?: string | null;
}

export function getHomeMetaLine(input: HomeMetaInput): string {
  const version = input.hqVersion ? `HQ v${input.hqVersion}` : null;

  if (input.syncState === 'syncing') {
    return joinMeta([
      'sync in progress',
      input.syncStartedLabel ? `started ${input.syncStartedLabel}` : null,
      input.autoSyncOn ? 'auto-sync on' : null,
      version,
    ]);
  }

  if (input.syncState === 'error' || input.syncState === 'auth-error') {
    return joinMeta([
      'auto-sync paused after failure',
      input.autoSyncOn ? 'retries on the next change' : 'retry from the title bar',
      version,
    ]);
  }

  return joinMeta([
    input.autoSyncOn == null ? null : input.autoSyncOn ? 'auto-sync on' : 'auto-sync off',
    input.autoSyncOn ? 'syncs on every change' : null,
    input.daemonRunning == null ? null : input.daemonRunning ? 'daemon running' : 'daemon idle',
    input.lastSyncLabel ? `last sync ${input.lastSyncLabel.toLowerCase()}` : 'no syncs yet',
    version,
  ]);
}

function joinMeta(parts: Array<string | null>): string {
  return parts.filter((part): part is string => Boolean(part)).join(' · ');
}

// ── Syncing progress card (home-syncing.png) ────────────────────────────────

export interface HomeFanoutRow {
  slug: string;
  name: string;
  state: 'done' | 'active' | 'queued';
  /** Right-hand detail ("done · 97 files", "downloading … · 76 of 301"). */
  detail: string;
}

export interface HomeProgressModel {
  /** "187 of 412 files" (falls back to "Preparing sync…" pre-totals). */
  headline: string;
  /** 0–100, null before totals are known (indeterminate). */
  pct: number | null;
  /** Right-hand meta: "2.1 MB transferred". */
  meta: string;
  rows: HomeFanoutRow[];
  /** Collapsed "N more queued" row; null when everything is visible. */
  queued: { count: number; names: string } | null;
}

export interface HomeProgressInput {
  filesProgressed: number;
  totalFiles: number;
  transferredBytes: number;
  progress: SyncProgress | null;
  companies: SyncCompanyRef[];
  statsBySlug: Record<string, WorkspaceSyncStats>;
  workspaces: Workspace[];
}

export function getHomeProgressModel(input: HomeProgressInput): HomeProgressModel {
  const { filesProgressed, totalFiles, transferredBytes, progress, companies, statsBySlug } =
    input;

  const headline =
    totalFiles > 0
      ? `${filesProgressed.toLocaleString()} of ${totalFiles.toLocaleString()} files`
      : 'Preparing sync…';
  const pct =
    totalFiles > 0 ? Math.min(100, Math.max(0, (filesProgressed / totalFiles) * 100)) : null;

  const rows: HomeFanoutRow[] = [];
  const queuedNames: string[] = [];

  for (const company of companies) {
    const name = workspaceDisplayName(company.slug, input.workspaces, companies);
    const stats = statsBySlug[company.slug];
    const isActive = progress?.company === company.slug;
    const isDone =
      !isActive && Boolean(stats && (stats.completedFiles > 0 || stats.completedBytes > 0 || stats.aborted));

    if (isActive) {
      const planned = stats?.plannedFiles ?? 0;
      const done = stats?.progressedFiles ?? 0;
      const file = progress?.path ?? '';
      rows.push({
        slug: company.slug,
        name,
        state: 'active',
        detail: planned > 0 ? `downloading ${file} · ${done} of ${planned}` : `downloading ${file}`,
      });
    } else if (isDone) {
      const files = stats?.completedFiles ?? 0;
      rows.push({
        slug: company.slug,
        name,
        state: 'done',
        detail: `done · ${files.toLocaleString()} file${files === 1 ? '' : 's'}`,
      });
    } else {
      queuedNames.push(name);
    }
  }

  return {
    headline,
    pct,
    meta: `${formatBytes(transferredBytes)} transferred`,
    rows,
    queued:
      queuedNames.length > 0
        ? {
            count: queuedNames.length,
            names: `${queuedNames.slice(0, 3).join(', ').toLowerCase()}${
              queuedNames.length > 3 ? '…' : ''
            }`,
          }
        : null,
  };
}

// ── Error card (home-error.png) ──────────────────────────────────────────────

export interface HomeErrorModel {
  title: string;
  sub: string;
  /** Auth-shaped failures surface "Sign in again" as the primary action. */
  showSignIn: boolean;
  /** Collapsible "Technical details" inset lines. */
  techLines: string[];
}

export interface HomeErrorInput {
  syncState: SyncState;
  syncErrorMessage: string;
  /** Company the failing run reported, when the error event carried one. */
  errorCompany: string | null;
  workspaces: Workspace[];
  companies: SyncCompanyRef[];
  appVersion: string;
  lastSyncLabel: string | null;
}

export function getHomeErrorModel(input: HomeErrorInput): HomeErrorModel | null {
  if (input.syncState !== 'error' && input.syncState !== 'auth-error') return null;

  const friendly = friendlySyncError(input.syncErrorMessage);
  const company = input.errorCompany
    ? workspaceDisplayName(input.errorCompany, input.workspaces, input.companies)
    : null;
  const showSignIn =
    input.syncState === 'auth-error' || /sign in/i.test(friendly.summary);

  const techLines: string[] = [];
  if (friendly.detail) techLines.push(friendly.detail);
  else if (input.syncErrorMessage.trim()) techLines.push(input.syncErrorMessage.trim());
  techLines.push(`runner: hq-sync v${input.appVersion} · journal ~/.hq/sync-journal.log`);
  techLines.push(
    `${
      input.lastSyncLabel ? `last good sync ${input.lastSyncLabel.toLowerCase()}` : 'no prior sync'
    } · log ~/.hq/sync-debug.log`,
  );

  return {
    title: company ? `Sync failed for ${company} — ${friendly.summary}` : `Sync failed — ${friendly.summary}`,
    sub: showSignIn
      ? 'your vault sign-in likely expired'
      : 'sync paused — nothing was lost',
    showSignIn,
    techLines,
  };
}

// ── Actor-grouped digest (home-healthy.png) ─────────────────────────────────

export type HomeFileVerb = 'ADD' | 'UPD' | 'DEL';

export interface HomeDigestFile {
  verb: HomeFileVerb;
  path: string;
  sizeLabel: string;
  at: number;
}

export interface HomeDigestGroup {
  id: string;
  /** Actor display ("Geoff", "You", or the company name as fallback). */
  actor: string;
  /** Two-letter avatar initials. */
  initials: string;
  /** "Geoff added 2 files to hpo". */
  headline: string;
  /** "10:58 AM · geoff@westbound.co". */
  meta: string;
  files: HomeDigestFile[];
  latestAt: number;
}

export function activityFileVerb(entry: ActivityEntry): HomeFileVerb {
  if (entry.direction === 'deleted') return 'DEL';
  if (entry.isNew) return 'ADD';
  return 'UPD';
}

/** "geoff@westbound.co" → "Geoff"; uploads (direction up) read as "You". */
function actorLabel(entry: ActivityEntry, companyName: string): string {
  if (entry.direction === 'up') return 'You';
  if (entry.author) {
    const local = entry.author.split('@')[0] ?? entry.author;
    const word = local.split(/[._-]/)[0] || local;
    return word.charAt(0).toUpperCase() + word.slice(1);
  }
  return companyName;
}

function actorKey(entry: ActivityEntry): string {
  if (entry.direction === 'up') return 'you';
  return entry.author ? `author:${entry.author}` : `company:${entry.company}`;
}

export function actorInitials(actor: string): string {
  const words = actor.trim().split(/\s+/).filter(Boolean);
  if (words.length >= 2) return `${words[0][0]}${words[1][0]}`.toUpperCase();
  return actor.slice(0, 2).toUpperCase();
}

export function formatClock(at: number): string {
  return new Date(at).toLocaleTimeString([], { hour: 'numeric', minute: '2-digit' });
}

export function getHomeDigestGroups(
  activity: ActivityEntry[],
  workspaces: Workspace[],
  companies: SyncCompanyRef[] = [],
): HomeDigestGroup[] {
  const groups = new Map<string, { entries: ActivityEntry[]; label: string }>();

  for (const entry of activity) {
    const key = actorKey(entry);
    const group = groups.get(key);
    if (group) group.entries.push(entry);
    else {
      const companyName = workspaceDisplayName(entry.company, workspaces, companies);
      groups.set(key, { entries: [entry], label: actorLabel(entry, companyName) });
    }
  }

  const result: HomeDigestGroup[] = [];
  for (const [key, group] of groups) {
    const entries = [...group.entries].sort((a, b) => b.at - a.at);
    const latest = entries[0];
    const verbs = new Set(entries.map(activityFileVerb));
    const verbWord =
      verbs.size === 1 && verbs.has('ADD')
        ? 'added'
        : verbs.size === 1 && verbs.has('DEL')
          ? 'deleted'
          : 'updated';

    const companyNames = [
      ...new Set(entries.map((entry) => workspaceDisplayName(entry.company, workspaces, companies))),
    ];
    const where =
      companyNames.length === 1
        ? `to ${companyNames[0]}`
        : `across ${companyNames.slice(0, 2).join(' + ')}${companyNames.length > 2 ? ' + more' : ''}`;
    const fileCount = entries.length;
    const headline = `${group.label} ${verbWord} ${fileCount.toLocaleString()} file${
      fileCount === 1 ? '' : 's'
    } ${where}`;

    const metaParts = [formatClock(latest.at)];
    if (latest.direction !== 'up' && latest.author) metaParts.push(latest.author);
    else metaParts.push(companyNames.join(' + '));

    result.push({
      id: key,
      actor: group.label,
      initials: actorInitials(group.label),
      headline,
      meta: metaParts.join(' · '),
      files: entries.map((entry) => ({
        verb: activityFileVerb(entry),
        path: entry.path,
        sizeLabel: formatBytes(entry.bytes),
        at: entry.at,
      })),
      latestAt: latest.at,
    });
  }

  return result.sort((a, b) => b.latestAt - a.latestAt);
}

// ── Portfolio overview (merged Home — real, local-only, NO vault fan-out) ────
//
// Everything below derives from data DesktopApp already holds: the deduped
// workspace union, the single `get_local_projects` scan, and the meetings cache.
// We deliberately do NOT surface vault storage, sync-latency, per-company
// sparklines, goals done/total, "shipped this week", or "due today" — none of
// those has a real source yet (the deferred vault-enrichment work), and
// inventing them would violate the no-fabricated-data rule.

/** A project is "active" when work remains: a non-terminal status and, when
 *  stories are tracked, at least one still open. Terminal statuses never count. */
function isActiveProject(p: Project): boolean {
  const status = (p.status ?? '').toLowerCase();
  if (['done', 'complete', 'completed', 'archived', 'shipped', 'cancelled', 'canceled'].includes(status)) {
    return false;
  }
  if (p.storiesTotal > 0) return p.storiesComplete < p.storiesTotal;
  return true;
}

function roleLabel(role: string | null | undefined): string {
  if (!role) return 'Member';
  return role.charAt(0).toUpperCase() + role.slice(1);
}

function toneForWorkspace(w: Workspace): HomeCompanyTone {
  if (w.kind === 'personal') return w.hasLocalFolder ? 'ok' : 'idle';
  switch (w.state) {
    case 'synced':
      return 'ok';
    case 'broken':
      return 'error';
    case 'cloud-only':
    case 'local-only':
    default:
      return 'idle';
  }
}

/** Collapse the manifest+cloud union to one entry per slug (first wins) — the
 *  same invariant every keyed `{#each (slug)}` in this app relies on. */
function dedupeBySlug(workspaces: Workspace[]): Workspace[] {
  const seen = new Set<string>();
  const out: Workspace[] = [];
  for (const w of workspaces) {
    if (seen.has(w.slug)) continue;
    seen.add(w.slug);
    out.push(w);
  }
  return out;
}

export interface HomeStat {
  label: string;
  value: string;
}

/**
 * The Home stat strip — three glanceable, fully-real counts:
 *   • Companies     — connected company workspaces (excludes the personal vault)
 *   • Active projects — local projects with work remaining
 *   • Open stories  — unfinished stories summed across those active projects
 * No storage / latency / "edits 7d" tiles: there is no honest source for them.
 */
export function getHomePortfolioStats(input: {
  workspaces: Workspace[];
  projects: Project[];
}): HomeStat[] {
  const companies = dedupeBySlug(input.workspaces).filter((w) => w.kind === 'company');
  const active = input.projects.filter(isActiveProject);
  const openStories = active.reduce(
    (sum, p) => sum + Math.max(0, p.storiesTotal - p.storiesComplete),
    0,
  );
  return [
    { label: companies.length === 1 ? 'Company' : 'Companies', value: companies.length.toLocaleString() },
    { label: 'Active projects', value: active.length.toLocaleString() },
    { label: 'Open stories', value: openStories.toLocaleString() },
  ];
}

// ── Company portfolio table ─────────────────────────────────────────────────

export type HomeCompanyTone = 'ok' | 'idle' | 'warn' | 'error';

export interface HomeCompanyRow {
  slug: string;
  name: string;
  /** Second line — role, or "Personal vault". */
  sub: string;
  tone: HomeCompanyTone;
  /** "3 active" project count, or "—" when none are local. */
  projects: string;
  /** "12 / 18 stories" rollup, or "—" when no stories are tracked. */
  stories: string;
  /** Relative last-synced, or "—". */
  lastChange: string;
}

/**
 * Per-company portfolio rows for the Home table. Project + story counts come
 * from the single `get_local_projects` scan grouped by company slug; role,
 * tone, and last-change come from the (deduped) workspace union. Goals,
 * members, and activity sparklines are intentionally absent — no real source.
 */
export function getHomeCompanyRows(input: {
  workspaces: Workspace[];
  projects: Project[];
}): HomeCompanyRow[] {
  const byCompany = new Map<string, { active: number; storiesTotal: number; storiesComplete: number }>();
  for (const p of input.projects) {
    const agg = byCompany.get(p.company) ?? { active: 0, storiesTotal: 0, storiesComplete: 0 };
    if (isActiveProject(p)) agg.active += 1;
    agg.storiesTotal += Math.max(0, p.storiesTotal);
    agg.storiesComplete += Math.max(0, p.storiesComplete);
    byCompany.set(p.company, agg);
  }

  return dedupeBySlug(input.workspaces).map((w) => {
    const agg = byCompany.get(w.slug);
    const projects =
      agg && agg.active > 0 ? `${agg.active.toLocaleString()} active` : agg ? 'no active' : '—';
    const stories =
      agg && agg.storiesTotal > 0
        ? `${agg.storiesComplete.toLocaleString()} / ${agg.storiesTotal.toLocaleString()} stories`
        : '—';
    return {
      slug: w.slug,
      name: w.displayName,
      sub: w.kind === 'personal' ? 'Personal vault' : roleLabel(w.role),
      tone: toneForWorkspace(w),
      projects,
      stories,
      lastChange: formatRelativeTime(w.lastSyncedAt) ?? '—',
    };
  });
}

// ── Today agenda (meetings only — action items have no due-date source) ──────

export interface HomeAgendaItem {
  id: string;
  /** "10:00 AM" or "Time pending". */
  time: string;
  title: string;
  /** Routed company name, or "Personal". */
  company: string;
}

/**
 * Today's meetings, chronological. Filtered to the current day from the
 * already-loaded meetings cache; capped so the rail stays calm. Action items
 * are NOT included — board cards carry an age string, not a due date, so
 * "due today" can't be derived honestly yet.
 */
export function getHomeTodayAgenda(input: {
  events: MeetingEvent[];
  companyNamesByUid: Map<string, string>;
  now?: Date;
  limit?: number;
}): HomeAgendaItem[] {
  const now = input.now ?? new Date();
  const limit = input.limit ?? 6;
  return input.events
    .filter((e) => isToday(e, now) && eventStart(e) !== null)
    .sort(sortByStart)
    .slice(0, limit)
    .map((e) => ({
      id: e.id,
      time: timeLabel(e),
      title: e.summary?.trim() || 'Untitled meeting',
      company: companyLabel(e, input.companyNamesByUid),
    }));
}
