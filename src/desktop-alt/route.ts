import type { Workspace } from '../lib/workspaces';

export type DesktopRoute = {
  kind: 'board' | 'sync' | 'meetings' | 'company';
  /** Company slug — set for `company` routes, and (optionally) for `board`
   *  routes to pre-filter the project list to one company. */
  slug?: string;
};

export interface DesktopSidebarRow {
  route: DesktopRoute;
  label: string;
  shortcut?: string;
  active: boolean;
}

export const DESKTOP_SHELL_LAYOUT = {
  sidebarWidthPx: 216,
  statusBarHeightPx: 26,
} as const;

export const initialDesktopRoute: DesktopRoute = { kind: 'sync' };

export function getDesktopCompanies(workspaces: Workspace[]): Workspace[] {
  return workspaces.filter(
    (workspace) => workspace.kind === 'company' && workspace.state === 'synced',
  );
}

export function getDesktopRouteKey(route: DesktopRoute): string {
  if (route.kind === 'company') return `company:${route.slug ?? ''}`;
  // The board surface keys on its pre-filter slug so the page remounts when the
  // pre-filter changes (e.g. entering from a company context); a bare board nav
  // keeps the stable `board` key so it doesn't remount on every visit.
  if (route.kind === 'board') return route.slug ? `board:${route.slug}` : 'board';
  return route.kind;
}

export function isDesktopRouteActive(route: DesktopRoute, candidate: DesktopRoute): boolean {
  if (route.kind !== candidate.kind) return false;
  // The Board sidebar row is slug-agnostic — any board route lights it up.
  return route.kind !== 'company' || route.slug === candidate.slug;
}

export function getDesktopSidebarRows(
  route: DesktopRoute,
  companies: Workspace[],
): DesktopSidebarRow[] {
  const primaryRows: DesktopSidebarRow[] = [
    {
      route: { kind: 'board' },
      label: 'Board',
      shortcut: '⌘1',
      active: isDesktopRouteActive(route, { kind: 'board' }),
    },
    {
      route: { kind: 'sync' },
      label: 'Sync',
      shortcut: '⌘2',
      active: isDesktopRouteActive(route, { kind: 'sync' }),
    },
    {
      route: { kind: 'meetings' },
      label: 'Meetings',
      shortcut: '⌘3',
      active: isDesktopRouteActive(route, { kind: 'meetings' }),
    },
  ];

  return primaryRows.concat(
    companies.map((company, index) => {
      const companyRoute: DesktopRoute = { kind: 'company', slug: company.slug };
      return {
        route: companyRoute,
        label: company.displayName,
        shortcut: index < 4 ? `⌘${index + 4}` : undefined,
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

  if (event.key === '1') return { kind: 'board' };
  if (event.key === '2') return { kind: 'sync' };
  if (event.key === '3') return { kind: 'meetings' };

  if (['4', '5', '6', '7'].includes(event.key)) {
    const company = companies[Number.parseInt(event.key, 10) - 4];
    if (company) return { kind: 'company', slug: company.slug };
  }

  return null;
}
