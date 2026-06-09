import type { Workspace } from '../lib/workspaces';

/**
 * Library sub-surfaces. Each is its own top-level sidebar link (Skills /
 * Workers / Installed / Marketplace / Profile) but they all share the `library`
 * page + LibraryBrowser body, differing only by which tab is forced. Defaults
 * to 'skills' when a library route carries no tab.
 */
export type LibraryTab = 'skills' | 'workers' | 'installed' | 'marketplace' | 'profile';

export const DEFAULT_LIBRARY_TAB: LibraryTab = 'skills';

/**
 * The library sub-surfaces promoted to top-level sidebar links, in display
 * order, with their ⌘ hotkeys (⌘3–⌘7). Companies start at ⌘8 (see
 * getDesktopHotkeyRoute + the company-row mapping below).
 */
export const LIBRARY_SIDEBAR_TABS: { tab: LibraryTab; label: string; shortcut: string }[] = [
  { tab: 'skills', label: 'Skills', shortcut: '⌘3' },
  { tab: 'workers', label: 'Workers', shortcut: '⌘4' },
  { tab: 'installed', label: 'Installed', shortcut: '⌘5' },
  { tab: 'marketplace', label: 'Marketplace', shortcut: '⌘6' },
  { tab: 'profile', label: 'Profile', shortcut: '⌘7' },
];

/** First ⌘ hotkey assigned to a company row (after the 7 primary destinations). */
const COMPANY_HOTKEY_BASE = 8;

export type DesktopRoute = {
  kind: 'sync' | 'meetings' | 'library' | 'moderation' | 'company';
  /** Company slug — set for `company` routes. */
  slug?: string;
  /** Library sub-surface — set for `library` routes (defaults to 'skills'). */
  tab?: LibraryTab;
};

export interface DesktopSidebarRow {
  route: DesktopRoute;
  label: string;
  shortcut?: string;
  active: boolean;
}

export const DESKTOP_SHELL_LAYOUT = {
  sidebarWidthPx: 216,
  titleBarHeightPx: 42,
  statusBarHeightPx: 32,
} as const;

export const initialDesktopRoute: DesktopRoute = { kind: 'sync' };

export function getDesktopCompanies(workspaces: Workspace[]): Workspace[] {
  // Personal is local-first (its state is 'personal', never 'synced'), so it
  // gets a page whenever it's present; companies need a synced local vault.
  return workspaces.filter(
    (workspace) =>
      workspace.kind === 'personal' ||
      (workspace.kind === 'company' && workspace.state === 'synced'),
  );
}

export function getDesktopRouteKey(route: DesktopRoute): string {
  if (route.kind === 'company') return `company:${route.slug ?? ''}`;
  if (route.kind === 'library') return `library:${route.tab ?? DEFAULT_LIBRARY_TAB}`;
  return route.kind;
}

export function isDesktopRouteActive(route: DesktopRoute, candidate: DesktopRoute): boolean {
  if (route.kind !== candidate.kind) return false;
  if (route.kind === 'company') return route.slug === candidate.slug;
  if (route.kind === 'library') {
    return (route.tab ?? DEFAULT_LIBRARY_TAB) === (candidate.tab ?? DEFAULT_LIBRARY_TAB);
  }
  return true;
}

/** Options that gate which sidebar rows are visible. */
export interface DesktopSidebarOptions {
  /**
   * Whether the signed-in user is a moderation admin (@getindigo.ai). DEFAULT-
   * DENY: the Moderation row is added ONLY when this is explicitly `true`. While
   * the admin check is still resolving (or on any error) callers pass `false`,
   * so the row stays hidden — it never flashes for a non-admin. The server is
   * the real authorization boundary; this is a UX-only gate.
   */
  isAdmin?: boolean;
}

export function getDesktopSidebarRows(
  route: DesktopRoute,
  companies: Workspace[],
  options: DesktopSidebarOptions = {},
): DesktopSidebarRow[] {
  const primaryRows: DesktopSidebarRow[] = [
    {
      route: { kind: 'sync' },
      label: 'Sync',
      shortcut: '⌘1',
      active: isDesktopRouteActive(route, { kind: 'sync' }),
    },
    {
      route: { kind: 'meetings' },
      label: 'Meetings',
      shortcut: '⌘2',
      active: isDesktopRouteActive(route, { kind: 'meetings' }),
    },
    // The Library surface is broken out into four top-level destinations
    // (Skills / Workers / Marketplace / Profile), each forcing its tab on the
    // shared library page. ⌘3–⌘6; companies pick up at ⌘7.
    ...LIBRARY_SIDEBAR_TABS.map(({ tab, label, shortcut }) => {
      const libraryRoute: DesktopRoute = { kind: 'library', tab };
      return {
        route: libraryRoute,
        label,
        shortcut,
        active: isDesktopRouteActive(route, libraryRoute),
      };
    }),
  ];

  // Admin-only Moderation row (default-deny). Appended after the standing
  // primary destinations; it carries no numbered hotkey, so company hotkeys
  // (⌘4+) are unaffected whether or not the user is an admin.
  if (options.isAdmin === true) {
    primaryRows.push({
      route: { kind: 'moderation' },
      label: 'Moderation',
      active: isDesktopRouteActive(route, { kind: 'moderation' }),
    });
  }

  return primaryRows.concat(
    companies.map((company, index) => {
      const companyRoute: DesktopRoute = { kind: 'company', slug: company.slug };
      // Only ⌘8–⌘9 are addressable (single-digit), so the first two
      // companies get a hotkey; the rest are click-only.
      const hotkeyNumber = COMPANY_HOTKEY_BASE + index;
      return {
        route: companyRoute,
        label: company.displayName,
        shortcut: hotkeyNumber <= 9 ? `⌘${hotkeyNumber}` : undefined,
        active: isDesktopRouteActive(route, companyRoute),
      };
    }),
  );
}

export function getDesktopActiveCompany(
  route: DesktopRoute,
  companies: Workspace[],
): Workspace | null {
  if (route.kind !== 'company') return null;
  return companies.find((company) => company.slug === route.slug) ?? null;
}

export function getDesktopHotkeyRoute(
  event: Pick<KeyboardEvent, 'key' | 'metaKey' | 'ctrlKey'>,
  companies: Workspace[],
): DesktopRoute | null {
  if (!(event.metaKey || event.ctrlKey)) return null;

  if (event.key === '1') return { kind: 'sync' };
  if (event.key === '2') return { kind: 'meetings' };

  // ⌘3–⌘7 → the five library destinations.
  const libraryIndex = Number.parseInt(event.key, 10) - 3;
  if (libraryIndex >= 0 && libraryIndex < LIBRARY_SIDEBAR_TABS.length) {
    return { kind: 'library', tab: LIBRARY_SIDEBAR_TABS[libraryIndex].tab };
  }

  // ⌘8–⌘9 → the first two companies.
  if (['8', '9'].includes(event.key)) {
    const company = companies[Number.parseInt(event.key, 10) - COMPANY_HOTKEY_BASE];
    if (company) return { kind: 'company', slug: company.slug };
  }

  return null;
}
