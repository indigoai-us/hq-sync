import type { Workspace } from '../../lib/workspaces';
import type { SyncState } from '../lib/sync-model';

/**
 * V4 chrome model — pure derivations for the shared chrome components
 * (V4Sidebar / V4SecondarySidebar / V4TitleBar). The route restructure that
 * adopts these kinds app-wide lands in US-002; until then this module defines
 * the V4 information architecture the chrome renders against
 * (docs/design/v4/SPEC.md section 4 + chrome-master.png).
 */

/**
 * The primary-nav destinations, in display order. Mission Control sits directly
 * under Home as a global, cross-company surface (US-006).
 */
export type V4NavId =
  | 'home'
  | 'mission-control'
  | 'companies'
  | 'messages'
  | 'meetings'
  | 'library'
  | 'files';

/**
 * Route shape the V4 chrome maps to an active row. `kind` is open-ended
 * (`string & {}` keeps autocomplete on the known kinds while accepting the
 * richer kinds US-002 introduces); `slug` is set for company routes.
 */
export interface V4Route {
  kind: V4NavId | 'settings' | 'company' | (string & {});
  slug?: string;
}

export const V4_NAV_ITEMS: ReadonlyArray<{ id: V4NavId; label: string }> = [
  { id: 'home', label: 'Home' },
  { id: 'mission-control', label: 'Mission Control' },
  { id: 'companies', label: 'Companies' },
  { id: 'messages', label: 'Messages' },
  { id: 'meetings', label: 'Meetings' },
  { id: 'library', label: 'Library' },
  { id: 'files', label: 'Files' },
];

/** Chrome metrics (SPEC section 4) — exported for shell composition in US-002. */
export const V4_CHROME_LAYOUT = {
  titleBarHeightPx: 40,
  primarySidebarWidthPx: 220,
  secondarySidebarWidthPx: 200,
} as const;

/** Status-dot tones — the only color in the app, almost always as 6px dots. */
export type V4DotTone = 'ok' | 'warn' | 'error' | 'idle';

export interface V4SidebarNavRow {
  id: V4NavId;
  label: string;
  active: boolean;
}

export interface V4SidebarCompanyRow {
  slug: string;
  label: string;
  tone: V4DotTone;
  active: boolean;
}

export interface V4SidebarModel {
  nav: V4SidebarNavRow[];
  companies: V4SidebarCompanyRow[];
  /** Settings highlights the footer, not a nav item (SPEC section 4). */
  settingsActive: boolean;
}

/**
 * Sidebar dot tone for a workspace row: green = connected/syncing fine
 * (personal is local-first and always live), red = broken, gray = paused /
 * not yet connected (SPEC: "gray dot = paused").
 */
export function v4CompanyDotTone(workspace: Workspace): V4DotTone {
  if (workspace.kind === 'personal') return 'ok';
  if (workspace.state === 'synced') return 'ok';
  if (workspace.state === 'broken') return 'error';
  return 'idle';
}

/**
 * Cloud-connected = a live vault link the user can sync against right now:
 * `synced` (local + cloud in step) or `cloud-only` (cloud membership, not yet
 * pulled). Personal is local-first and always live, so it counts as connected
 * too (it renders the same green dot). These rows sort to the TOP of the
 * COMPANIES list (US-007) so the user's active workspaces lead; idle/broken
 * rows follow. Mirrors the "green dot = connected" product framing.
 */
export function v4CompanyConnected(workspace: Workspace): boolean {
  return (
    workspace.kind === 'personal' ||
    workspace.state === 'synced' ||
    workspace.state === 'cloud-only'
  );
}

/**
 * Derive the primary-sidebar render model from the route + the
 * `list_syncable_workspaces` result. Invariant (locked by v4.test.ts):
 * EXACTLY ONE active row per sidebar — a nav item, a company row, or the
 * Settings footer. Company pages highlight the company row, not a nav item;
 * all local-first/cloud-visible companies render directly in the sidebar; a
 * company route whose slug isn't in the list falls back to the Companies nav row.
 */
/**
 * Shared dedupe + connected-first + alpha sort for the COMPANIES list (US-007).
 * Both the primary V4Sidebar (via getV4SidebarModel) and the FilesModeSidebar
 * mini company list consume this, so their ordering matches exactly. Pass
 * `activeSlug` to mark one row active; the dedupe keeps the first occurrence per
 * slug and the sort is stable so the active row and survivor are untouched.
 */
export function sortV4CompaniesConnectedFirst(
  workspaces: Workspace[],
  activeSlug?: string | null,
): V4SidebarCompanyRow[] {
  const seenCompanySlugs = new Set<string>();

  // Dedupe by slug (first occurrence wins), capturing the connected flag so the
  // sort below can group without re-reading the source workspace.
  const deduped: Array<{ row: V4SidebarCompanyRow; connected: boolean }> = [];
  for (const workspace of workspaces) {
    if (seenCompanySlugs.has(workspace.slug)) continue;
    seenCompanySlugs.add(workspace.slug);
    deduped.push({
      connected: v4CompanyConnected(workspace),
      row: {
        slug: workspace.slug,
        label: workspace.displayName,
        tone: v4CompanyDotTone(workspace),
        active: activeSlug != null && activeSlug === workspace.slug,
      },
    });
  }

  // Connected-first (US-007): cloud-connected companies (synced / cloud-only,
  // plus always-live personal) lead; everything else follows. Alphabetical by
  // display name (case-insensitive) WITHIN each group. Sort is stable so the
  // active-highlight and dedupe survivor are untouched — only order changes.
  deduped.sort((a, b) => {
    if (a.connected !== b.connected) return a.connected ? -1 : 1;
    return a.row.label.localeCompare(b.row.label, undefined, { sensitivity: 'base' });
  });

  return deduped.map((entry) => entry.row);
}

export function getV4SidebarModel(route: V4Route, workspaces: Workspace[]): V4SidebarModel {
  const settingsActive = route.kind === 'settings';

  const companies: V4SidebarCompanyRow[] = sortV4CompaniesConnectedFirst(
    workspaces,
    route.kind === 'company' ? route.slug : null,
  );

  const companyRowActive = companies.some((row) => row.active);
  // Company route with no matching row (e.g. not connected yet) → fall back
  // to the Companies nav destination so exactly one row stays active.
  const companiesFallback = route.kind === 'company' && !companyRowActive && !settingsActive;

  const activeNavId: V4NavId | null =
    settingsActive || companyRowActive
      ? null
      : companiesFallback
        ? 'companies'
        : V4_NAV_ITEMS.some((item) => item.id === route.kind)
          ? (route.kind as V4NavId)
          : // Unknown kinds (transitional routes) read as Home — the exception
            // surface — rather than leaving the sidebar with no "you are here".
            'home';

  return {
    nav: V4_NAV_ITEMS.map((item) => ({
      id: item.id,
      label: item.label,
      active: item.id === activeNavId,
    })),
    companies,
    settingsActive,
  };
}

/** Secondary-sidebar item (contextual menu row). */
export interface V4SecondaryItem {
  id: string;
  label: string;
  /** Optional muted trailing note, e.g. "gated" on the Meetings settings row. */
  note?: string | null;
}

/** Secondary-sidebar footer, e.g. "Company settings" + "sync rules · members · roles". */
export interface V4SecondaryFooter {
  label: string;
  meta?: string | null;
}

/** Title-bar primary action — contextual, always exactly one. */
export interface V4TitleBarAction {
  id: 'sync' | 'cancel' | 'retry';
  label: 'Sync Now' | 'Cancel' | 'Retry';
}

export interface V4TitleBarModel {
  tone: V4DotTone;
  /** The live sync status sentence, 13px text-1 ("All synced", …). */
  sentence: string;
  /** Trailing text-3 detail ("12 watched · just now"), null when empty. */
  meta: string | null;
  action: V4TitleBarAction;
}

export interface V4TitleBarInput {
  syncState: SyncState;
  /** Connected workspaces being watched (companies + personal). */
  watchedCount: number;
  /** Human relative last-sync label ("just now", "5m ago"). */
  lastSyncLabel?: string | null;
  /** Company currently transferring, while syncing. */
  syncingCompany?: string | null;
  fanoutDone?: number;
  fanoutTotal?: number;
  /** Plain-language error summary, for error states. */
  errorSummary?: string | null;
}

/**
 * Title-bar render model: 6px dot + status sentence + text-3 meta + ONE
 * contextual primary action (Sync Now / Cancel / Retry) per SPEC section 4.
 */
export function getV4TitleBarModel(input: V4TitleBarInput): V4TitleBarModel {
  const syncNow: V4TitleBarAction = { id: 'sync', label: 'Sync Now' };

  switch (input.syncState) {
    case 'syncing': {
      const parts: string[] = [];
      if (input.syncingCompany) parts.push(input.syncingCompany);
      if (input.fanoutTotal && input.fanoutTotal > 0) {
        parts.push(`${input.fanoutDone ?? 0}/${input.fanoutTotal} companies`);
      }
      return {
        tone: 'ok',
        sentence: 'Syncing…',
        meta: parts.length > 0 ? parts.join(' · ') : null,
        action: { id: 'cancel', label: 'Cancel' },
      };
    }
    case 'error':
      return {
        tone: 'error',
        sentence: 'Sync failed',
        meta: input.errorSummary ?? 'check your connection',
        action: { id: 'retry', label: 'Retry' },
      };
    case 'auth-error':
      return {
        tone: 'error',
        sentence: 'Signed out',
        meta: 'sign in to resume sync',
        action: { id: 'retry', label: 'Retry' },
      };
    case 'conflict':
      return {
        tone: 'warn',
        sentence: 'Needs your review',
        meta: 'resolve conflicts to continue',
        action: syncNow,
      };
    case 'setup-needed':
      return {
        tone: 'idle',
        sentence: 'Sync not set up',
        meta: null,
        action: syncNow,
      };
    default: {
      const watched = `${input.watchedCount} watched`;
      return {
        tone: 'ok',
        sentence: 'All synced',
        meta: input.lastSyncLabel ? `${watched} · ${input.lastSyncLabel}` : watched,
        action: syncNow,
      };
    }
  }
}
