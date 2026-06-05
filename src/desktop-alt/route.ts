import type { Workspace } from '../lib/workspaces';

export type DesktopRoute = {
  kind: 'sync' | 'meetings' | 'library' | 'company';
  /** Company slug — set for `company` routes. */
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
  return route.kind;
}

export function isDesktopRouteActive(route: DesktopRoute, candidate: DesktopRoute): boolean {
  if (route.kind !== candidate.kind) return false;
  return route.kind !== 'company' || route.slug === candidate.slug;
}

export function getDesktopSidebarRows(
  route: DesktopRoute,
  companies: Workspace[],
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
    {
      route: { kind: 'library' },
      label: 'Library',
      shortcut: '⌘3',
      active: isDesktopRouteActive(route, { kind: 'library' }),
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

  if (event.key === '1') return { kind: 'sync' };
  if (event.key === '2') return { kind: 'meetings' };
  if (event.key === '3') return { kind: 'library' };

  if (['4', '5', '6', '7'].includes(event.key)) {
    const company = companies[Number.parseInt(event.key, 10) - 4];
    if (company) return { kind: 'company', slug: company.slug };
  }

  return null;
}
