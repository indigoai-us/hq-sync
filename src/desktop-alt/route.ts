import type { Workspace } from '../lib/workspaces';

export type DesktopRoute = { kind: 'sync' | 'meetings' | 'company'; slug?: string };

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
  return workspaces.filter((workspace) => workspace.kind === 'company');
}

export function getDesktopRouteKey(route: DesktopRoute): string {
  return route.kind === 'company' ? `company:${route.slug ?? ''}` : route.kind;
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
  ];

  return primaryRows.concat(
    companies.map((company, index) => {
      const companyRoute: DesktopRoute = { kind: 'company', slug: company.slug };
      return {
        route: companyRoute,
        label: company.displayName,
        shortcut: index < 4 ? `⌘${index + 3}` : undefined,
        active: isDesktopRouteActive(route, companyRoute),
      };
    }),
  );
}

export function getDesktopPage(route: DesktopRoute, companies: Workspace[]) {
  if (route.kind === 'sync') {
    return { title: 'Sync', placeholder: 'Sync page - wired in US-005' };
  }

  if (route.kind === 'meetings') {
    return { title: 'Meetings', placeholder: 'Meetings page - wired in US-005' };
  }

  const activeCompany = companies.find((company) => company.slug === route.slug) ?? null;

  return {
    title: activeCompany?.displayName ?? 'Company',
    placeholder: 'Company page - wired in US-005',
    activeCompany,
  };
}

export function getDesktopHotkeyRoute(
  event: Pick<KeyboardEvent, 'key' | 'metaKey' | 'ctrlKey'>,
  companies: Workspace[],
): DesktopRoute | null {
  if (!(event.metaKey || event.ctrlKey)) return null;

  if (event.key === '1') return { kind: 'sync' };
  if (event.key === '2') return { kind: 'meetings' };

  if (['3', '4', '5', '6'].includes(event.key)) {
    const company = companies[Number.parseInt(event.key, 10) - 3];
    if (company) return { kind: 'company', slug: company.slug };
  }

  return null;
}
