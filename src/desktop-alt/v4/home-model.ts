import type { Workspace } from '../../lib/workspaces';
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
 * Home answers "is anything wrong, what needs me, what happened": an 11px
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

// ── 11px meta line under the Home title ─────────────────────────────────────

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
  /** 11px right-hand detail ("done · 97 files", "downloading … · 76 of 301"). */
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
