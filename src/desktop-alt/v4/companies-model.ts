import type { Workspace } from '../../lib/workspaces';
import { formatRelativeTime } from '../route';

/**
 * V4 Companies model — pure derivations for the one-page connection-state
 * surface (docs/design/v4/SPEC.md section 5, companies.png). All logic lives
 * here (node-testable); CompaniesPage.svelte only renders.
 *
 * The page splits every workspace from `list_syncable_workspaces` into two
 * groups:
 *  - CONNECTED — the table with role / members / last change / sync lanes,
 *    plus provisioning rows (amber dot, Connect in flight) and error rows
 *    (red dot + Retry, `state === 'broken'`).
 *  - NOT CONNECTED — local directories awaiting Connect
 *    (`connect_workspace_to_cloud`) and pending membership invites (opened via
 *    the `/accept` workflow because the desktop row has invite metadata, not the
 *    magic-link token).
 */

export type CompaniesDotTone = 'ok' | 'idle' | 'warn' | 'error';

/** Resolved per-membership sync mode (`get_sync_mode`); null while loading. */
export type CompanySyncMode = 'all' | 'shared' | 'custom' | null;

export interface ConnectedCompanyRow {
  slug: string;
  name: string;
  /** Second line under the name — role context or provisioning/error note. */
  sub: string;
  tone: CompaniesDotTone;
  /** MEMBERS lane. "—" until the workspaces command exposes member counts. */
  members: string;
  /** LAST CHANGE lane — relative `lastSyncedAt`, or "—". */
  lastChange: string;
  /** SYNC lane — "Auto · all paths", "Manual", "Setting up", "—". */
  sync: string;
  /**
   * Resolved per-membership footprint mode (`get_sync_mode`), null while
   * loading / when not applicable. Drives the interactive Shared/All control.
   */
  syncMode: CompanySyncMode;
  /**
   * True only when the Sync lane should render the interactive Shared/All
   * control instead of the static `sync` label: a synced company row whose
   * mode has resolved to a toggleable value (`all` or `shared`). Loading
   * (`null`), CLI-only `custom`, personal vaults, cloud-only/provisioning/
   * error rows all stay label-only (`false`) and fall back to `sync`.
   */
  canToggleSyncMode: boolean;
  /** Error rows surface an inline Retry (re-runs connect to reconcile). */
  retry: boolean;
  /** Row navigates to the company workspace (synced local vaults only). */
  open: boolean;
}

export interface NotConnectedCompanyRow {
  slug: string;
  name: string;
  sub: string;
  kind: 'local' | 'invite';
  /** Inline note (e.g. a failed Connect attempt) rendered under the sub. */
  note: string | null;
  actions: Array<'open' | 'connect' | 'open-invite'>;
}

export interface CompaniesPageModel {
  /** Muted line under the view title. */
  summary: string;
  connected: ConnectedCompanyRow[];
  notConnected: NotConnectedCompanyRow[];
}

export interface CompaniesModelInput {
  workspaces: Workspace[];
  /** Slugs with an in-flight Connect — rendered as amber provisioning rows. */
  connectingSlugs?: ReadonlyArray<string>;
  /** slug → message from a failed Connect attempt (inline note + retry). */
  connectErrors?: Record<string, string>;
  /** slug → per-membership sync mode (`get_sync_mode`), absent while loading. */
  syncModes?: Record<string, CompanySyncMode>;
  /** `realtimeSync` preference; null while loading. */
  autoSyncOn?: boolean | null;
}

const EM_DASH = '—';

function roleLabel(role: string | null): string {
  if (!role) return 'Member';
  return role.charAt(0).toUpperCase() + role.slice(1);
}

function syncLaneLabel(mode: CompanySyncMode, autoSyncOn: boolean | null | undefined): string {
  const prefix = autoSyncOn === false ? 'Manual' : 'Auto';
  switch (mode) {
    case 'all':
      return `${prefix} · all paths`;
    case 'shared':
      return `${prefix} · shared paths`;
    case 'custom':
      return `${prefix} · custom paths`;
    default:
      return prefix;
  }
}

/**
 * Meta line for a pending-invite row. The membership row carries who created
 * the invite (`invitedBy`, a prs_* person uid) and when (`invitedAt`); invites
 * don't expire server-side today, so the line shows the invite age instead of
 * a made-up expiry. A human-readable inviter (email) renders verbatim;
 * an opaque person uid reads as "a teammate".
 */
export function getInviteMetaLine(workspace: Workspace): string {
  const inviter =
    workspace.invitedBy && workspace.invitedBy.includes('@')
      ? workspace.invitedBy
      : 'a teammate';
  const invitedAgo = formatRelativeTime(workspace.invitedAt);
  return invitedAgo
    ? `Invite from ${inviter} · invited ${invitedAgo}`
    : `Invite from ${inviter} · pending`;
}

export function getCompaniesPageModel(input: CompaniesModelInput): CompaniesPageModel {
  const connecting = new Set(input.connectingSlugs ?? []);
  const connectErrors = input.connectErrors ?? {};
  const syncModes = input.syncModes ?? {};
  const autoSyncOn = input.autoSyncOn ?? null;

  const active: ConnectedCompanyRow[] = [];
  const provisioning: ConnectedCompanyRow[] = [];
  const errored: ConnectedCompanyRow[] = [];
  const notConnected: NotConnectedCompanyRow[] = [];

  // `list_syncable_workspaces` is the UNION of manifest companies and cloud
  // memberships, so a company present in both arrives twice under the same
  // slug. CompaniesPage keys its rows by slug; a duplicate key throws
  // `each_key_duplicate` and strands the whole page (the body freezes on the
  // previous route). Collapse to the first occurrence per slug — same
  // first-wins dedupe the sidebar already applies via `getDesktopCompanies`.
  const seenSlugs = new Set<string>();

  for (const workspace of input.workspaces) {
    if (seenSlugs.has(workspace.slug)) continue;
    seenSlugs.add(workspace.slug);

    const lastChange = formatRelativeTime(workspace.lastSyncedAt) ?? EM_DASH;

    if (workspace.kind === 'personal') {
      active.push({
        slug: workspace.slug,
        name: workspace.displayName,
        sub: 'Personal vault · private',
        tone: workspace.hasLocalFolder ? 'ok' : 'idle',
        members: EM_DASH,
        lastChange,
        sync: syncLaneLabel(null, autoSyncOn),
        syncMode: null,
        canToggleSyncMode: false,
        retry: false,
        open: true,
      });
      continue;
    }

    // An in-flight Connect renders as the amber provisioning row regardless
    // of the workspace's resting state — companies.png "Setting up".
    if (connecting.has(workspace.slug)) {
      provisioning.push({
        slug: workspace.slug,
        name: workspace.displayName,
        sub: 'provisioning cloud storage…',
        tone: 'warn',
        members: EM_DASH,
        lastChange: EM_DASH,
        sync: 'Setting up',
        syncMode: null,
        canToggleSyncMode: false,
        retry: false,
        open: false,
      });
      continue;
    }

    if (workspace.membershipStatus === 'pending') {
      notConnected.push({
        slug: workspace.slug,
        name: workspace.displayName,
        sub: getInviteMetaLine(workspace),
        kind: 'invite',
        note: null,
        actions: ['open-invite'],
      });
      continue;
    }

    if (workspace.state === 'local-only') {
      notConnected.push({
        slug: workspace.slug,
        name: workspace.displayName,
        sub: 'Local directory exists · not cloud-backed',
        kind: 'local',
        note: connectErrors[workspace.slug] ?? null,
        actions: ['open', 'connect'],
      });
      continue;
    }

    if (workspace.state === 'broken') {
      errored.push({
        slug: workspace.slug,
        name: workspace.displayName,
        sub: 'connection error · needs reconnect',
        tone: 'error',
        members: EM_DASH,
        lastChange: EM_DASH,
        sync: EM_DASH,
        syncMode: null,
        canToggleSyncMode: false,
        retry: true,
        open: false,
      });
      continue;
    }

    if (workspace.state === 'cloud-only') {
      active.push({
        slug: workspace.slug,
        name: workspace.displayName,
        sub: `${roleLabel(workspace.role)} · not on this Mac`,
        tone: 'idle',
        members: EM_DASH,
        lastChange,
        sync: EM_DASH,
        syncMode: null,
        canToggleSyncMode: false,
        retry: false,
        open: false,
      });
      continue;
    }

    // state === 'synced'
    const syncedMode = syncModes[workspace.slug] ?? null;
    active.push({
      slug: workspace.slug,
      name: workspace.displayName,
      sub: roleLabel(workspace.role),
      tone: 'ok',
      members: EM_DASH,
      lastChange,
      sync: syncLaneLabel(syncedMode, autoSyncOn),
      syncMode: syncedMode,
      // Toggleable only once the mode resolves to a value the popover/CLI
      // expose as a binary control. `custom` is CLI-only (needs a path list),
      // and a still-loading `null` falls back to the plain "Auto" label until
      // get_sync_mode lands — never render a control whose state we don't know.
      canToggleSyncMode: syncedMode === 'all' || syncedMode === 'shared',
      retry: false,
      open: true,
    });
  }

  // Resting rows in workspace order (personal is already first), then
  // provisioning, then error rows — matching companies.png.
  const connected = [...active, ...provisioning, ...errored];

  const summaryParts = [
    `${connected.length} connected`,
    notConnected.length > 0 ? `${notConnected.length} available` : null,
    autoSyncOn === false ? 'manual sync' : 'syncing on every change',
  ].filter((part): part is string => Boolean(part));

  return {
    summary: summaryParts.join(' · '),
    connected,
    notConnected,
  };
}
