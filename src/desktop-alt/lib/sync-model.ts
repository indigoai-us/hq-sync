import type { Workspace } from '../../lib/workspaces';

export type SyncState = 'idle' | 'syncing' | 'error' | 'conflict' | 'setup-needed' | 'auth-error';

export interface SyncProgress {
  company: string;
  path: string;
  bytes: number;
}

export interface SyncCompanyRef {
  uid: string;
  slug: string;
  name?: string;
}

export interface SyncStatus {
  lastSyncAt: string | null;
  pendingFiles: number;
  conflicts: number;
  daemonRunning: boolean;
  source: string;
}

export interface DaemonStatus {
  running: boolean;
  pid: number | null;
  startedAt: string | null;
  watchPath: string | null;
  source: string;
}

export interface ActivityEntry {
  company: string;
  path: string;
  bytes: number;
  direction: string;
  author?: string;
  isNew?: boolean;
  at: number;
}

export interface WorkspaceSyncStats {
  progressedFiles: number;
  plannedFiles: number;
  plannedBytes: number;
  transferredBytes: number;
  completedBytes: number;
  completedFiles: number;
  skippedFiles: number;
  conflicts: number;
  aborted: boolean;
  lastEventAt: number | null;
  errorMessage: string | null;
}

export type SourceLiveState = 'ok' | 'syncing' | 'warn' | 'paused';
export type SourceAction = 'Up to date' | 'Syncing' | 'Reauth' | 'Paused' | 'Needs attention';

export interface SourceViewModel {
  key: string;
  slug: string;
  kind: Workspace['kind'];
  state: Workspace['state'];
  isPersonal: boolean;
  showSyncMode: boolean;
  name: string;
  detail: string;
  liveState: SourceLiveState;
  action: SourceAction;
  lastSyncLabel: string;
  transferredLabel: string;
  progressPct: number | null;
  warning: string | null;
}

// Mirror classic WorkspaceList ordering: personal first, then synced,
// cloud-only, broken, local-only. Stable sort preserves backend order within
// each group.
const SOURCE_STATE_ORDER: Record<Workspace['state'], number> = {
  personal: 0,
  synced: 1,
  'cloud-only': 2,
  broken: 3,
  'local-only': 4,
};

// Hover shared/All footprint toggle only applies to cloud-backed company rows.
export function showSyncModeFor(workspace: Workspace): boolean {
  return (
    workspace.kind === 'company' &&
    (workspace.state === 'synced' || workspace.state === 'cloud-only')
  );
}

export interface AttentionItem {
  key: string;
  title: string;
  detail: string;
  tone: 'warn' | 'paused';
  actionLabel?: string;
}

export function emptyWorkspaceStats(): WorkspaceSyncStats {
  return {
    progressedFiles: 0,
    plannedFiles: 0,
    plannedBytes: 0,
    transferredBytes: 0,
    completedBytes: 0,
    completedFiles: 0,
    skippedFiles: 0,
    conflicts: 0,
    aborted: false,
    lastEventAt: null,
    errorMessage: null,
  };
}

export function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes <= 0) return '0 B';
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  if (bytes < 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  return `${(bytes / (1024 * 1024 * 1024)).toFixed(2)} GB`;
}

export function timeAgo(input: string | number | null | undefined): string {
  if (input == null) return 'Never';
  const then = typeof input === 'number' ? input : new Date(input).getTime();
  if (!Number.isFinite(then)) return 'Unknown';

  const seconds = Math.max(0, Math.floor((Date.now() - then) / 1000));
  if (seconds < 60) return 'Just now';
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes}m ago`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours}h ago`;
  const days = Math.floor(hours / 24);
  if (days < 30) return `${days}d ago`;
  return new Date(then).toLocaleDateString([], { month: 'short', day: 'numeric' });
}

export function formatUptime(daemon: DaemonStatus | null): string {
  if (!daemon?.running) return 'Not running';
  if (!daemon.startedAt) return 'Running';

  const then = new Date(daemon.startedAt).getTime();
  if (!Number.isFinite(then)) return 'Running';

  const minutes = Math.max(0, Math.floor((Date.now() - then) / 60000));
  if (minutes < 1) return 'Just started';
  if (minutes < 60) return `${minutes}m`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours}h ${minutes % 60}m`;
  return `${Math.floor(hours / 24)}d ${hours % 24}h`;
}

export function workspaceDisplayName(
  slug: string,
  workspaces: Workspace[],
  companies: SyncCompanyRef[],
): string {
  return (
    workspaces.find((workspace) => workspace.slug === slug)?.displayName ??
    companies.find((company) => company.slug === slug)?.name ??
    slug
  );
}

export function currentSyncLabel(
  progress: SyncProgress | null,
  workspaces: Workspace[],
  companies: SyncCompanyRef[],
): string {
  if (!progress) return 'Preparing sync';
  const company = workspaceDisplayName(progress.company, workspaces, companies);
  return `${company} / ${progress.path}`;
}

export function isPausedWorkspace(workspace: Workspace, cloudReachable: boolean): boolean {
  const status = workspace.membershipStatus?.toLowerCase() ?? '';
  return (
    !cloudReachable ||
    workspace.state === 'cloud-only' ||
    workspace.state === 'local-only' ||
    status.includes('paused') ||
    status.includes('inactive') ||
    status.includes('disabled') ||
    status.includes('suspended')
  );
}

export function needsReauthWorkspace(workspace: Workspace, syncState: SyncState): boolean {
  return syncState === 'auth-error' || workspace.state === 'broken';
}

export function workspaceWarning(
  stats: WorkspaceSyncStats,
  workspace: Workspace,
): string | null {
  if (stats.errorMessage) return stats.errorMessage;
  if (stats.conflicts > 0) {
    return `${stats.conflicts.toLocaleString()} conflict${stats.conflicts === 1 ? '' : 's'} need review.`;
  }
  if (stats.aborted) return 'Sync stopped because a conflict needs attention.';
  return workspace.brokenReason;
}

export function buildSourceRows(args: {
  workspaces: Workspace[];
  syncState: SyncState;
  progress: SyncProgress | null;
  statsBySlug: Record<string, WorkspaceSyncStats>;
  cloudReachable: boolean;
}): SourceViewModel[] {
  const { workspaces, syncState, progress, statsBySlug, cloudReachable } = args;

  const ordered = [...workspaces].sort(
    (a, b) => SOURCE_STATE_ORDER[a.state] - SOURCE_STATE_ORDER[b.state],
  );

  return ordered.map((workspace) => {
    const stats = statsBySlug[workspace.slug] ?? emptyWorkspaceStats();
    const syncing = syncState === 'syncing' && progress?.company === workspace.slug;
    const reauth = needsReauthWorkspace(workspace, syncState);
    const sourceAttention = Boolean(stats.errorMessage) || stats.aborted || stats.conflicts > 0;
    const paused = !reauth && !sourceAttention && isPausedWorkspace(workspace, cloudReachable);
    const plannedFiles = Math.max(stats.plannedFiles, 0);
    const progressPct =
      syncing && plannedFiles > 0
        ? Math.min(100, Math.max(4, (stats.progressedFiles / plannedFiles) * 100))
        : syncing
          ? 18
          : null;

    const detail =
      workspace.kind === 'personal'
        ? workspace.cloudUid
          ? 'Personal vault'
          : 'Personal vault · cloud pending'
        : workspace.membershipStatus
          ? `Membership · ${workspace.membershipStatus}`
          : workspace.cloudUid
            ? 'Company membership'
            : workspace.localPath
              ? 'Local source'
              : 'Cloud source';

    const completedOrTransferred = Math.max(stats.transferredBytes, stats.completedBytes);

    return {
      key: `${workspace.kind}:${workspace.slug}`,
      slug: workspace.slug,
      kind: workspace.kind,
      state: workspace.state,
      isPersonal: workspace.kind === 'personal',
      showSyncMode: showSyncModeFor(workspace),
      name: workspace.displayName,
      detail,
      liveState: syncing
        ? 'syncing'
        : reauth || sourceAttention
          ? 'warn'
          : paused
            ? 'paused'
            : 'ok',
      action: syncing
        ? 'Syncing'
        : reauth
          ? 'Reauth'
          : sourceAttention
            ? 'Needs attention'
            : paused
              ? 'Paused'
              : 'Up to date',
      lastSyncLabel:
        workspace.lastSyncedAt != null
          ? timeAgo(workspace.lastSyncedAt)
          : stats.lastEventAt != null
            ? timeAgo(stats.lastEventAt)
            : 'Never',
      transferredLabel: formatBytes(completedOrTransferred),
      progressPct,
      warning: workspaceWarning(stats, workspace),
    };
  });
}

export function buildAttentionItems(args: {
  workspaces: Workspace[];
  syncState: SyncState;
  syncErrorMessage: string;
  cloudReachable: boolean;
  cloudError: string | null;
  manifestError: string | null;
  statsBySlug?: Record<string, WorkspaceSyncStats>;
}): AttentionItem[] {
  const {
    workspaces,
    syncState,
    syncErrorMessage,
    cloudReachable,
    cloudError,
    manifestError,
    statsBySlug = {},
  } = args;
  const items: AttentionItem[] = [];

  const hasSourceConflict = workspaces.some((workspace) => {
    const stats = statsBySlug[workspace.slug];
    return stats ? stats.aborted || stats.conflicts > 0 : false;
  });

  if (syncState === 'auth-error') {
    items.push({
      key: 'auth-error',
      title: 'Sign-in expired',
      detail: syncErrorMessage || 'Reconnect your account before the next sync can run.',
      tone: 'warn',
      actionLabel: 'Open settings',
    });
  }

  if (syncState === 'conflict' && !hasSourceConflict) {
    items.push({
      key: 'sync-conflict',
      title: 'Sync conflict needs review',
      detail: syncErrorMessage || 'A sync stopped because a conflict needs attention.',
      tone: 'warn',
      actionLabel: 'Open settings',
    });
  }

  if (syncState === 'error' && syncErrorMessage) {
    items.push({
      key: 'sync-error',
      title: 'Sync needs attention',
      detail: syncErrorMessage,
      tone: 'warn',
      actionLabel: 'Open settings',
    });
  }

  if (!cloudReachable) {
    items.push({
      key: 'cloud-unreachable',
      title: 'Cloud unreachable',
      detail: cloudError || 'Showing local workspace state until the vault is reachable again.',
      tone: 'paused',
    });
  }

  if (manifestError) {
    items.push({
      key: 'manifest-error',
      title: 'Workspace manifest could not be read',
      detail: manifestError,
      tone: 'warn',
    });
  }

  for (const workspace of workspaces) {
    const stats = statsBySlug[workspace.slug] ?? emptyWorkspaceStats();
    const warning = workspaceWarning(stats, workspace);
    if (stats.errorMessage || stats.aborted || stats.conflicts > 0) {
      items.push({
        key: `source-attention:${workspace.slug}`,
        title: `${workspace.displayName} needs attention`,
        detail: warning || 'Review this source before the next sync.',
        tone: 'warn',
        actionLabel: 'Open settings',
      });
    } else if (workspace.state === 'broken') {
      items.push({
        key: `reauth:${workspace.slug}`,
        title: `${workspace.displayName} needs reconnect`,
        detail: workspace.brokenReason || 'Manifest and cloud membership are out of sync.',
        tone: 'warn',
        actionLabel: 'Open settings',
      });
    } else if (isPausedWorkspace(workspace, cloudReachable)) {
      items.push({
        key: `paused:${workspace.slug}`,
        title: `${workspace.displayName} is paused`,
        detail:
          workspace.state === 'local-only'
            ? 'Local folder is not connected to a cloud vault yet.'
            : workspace.state === 'cloud-only'
              ? 'Cloud source is not present on this machine yet.'
              : workspace.membershipStatus
                ? `Membership status: ${workspace.membershipStatus}`
                : 'Sync will resume when the source is available.',
        tone: 'paused',
        actionLabel: workspace.state === 'local-only' ? 'Open settings' : undefined,
      });
    }
  }

  return items;
}

export function latestFullSync(workspaces: Workspace[], status: SyncStatus | null): string | null {
  if (status?.lastSyncAt) return status.lastSyncAt;

  const times = workspaces
    .map((workspace) => (workspace.lastSyncedAt ? new Date(workspace.lastSyncedAt).getTime() : 0))
    .filter((time) => Number.isFinite(time) && time > 0);
  if (times.length === 0) return null;
  return new Date(Math.max(...times)).toISOString();
}
