import type { Workspace } from '../lib/workspaces';
import {
  v4CompanyDotTone,
  type V4DotTone,
  type V4Route,
  type V4SecondaryFooter,
  type V4SecondaryItem,
} from './v4/model';

/**
 * V4 information architecture (docs/design/v4/SPEC.md section 4).
 *
 * Five primary destinations — Home, Companies, Messages, Meetings, Library —
 * plus companies as first-class sidebar rows, a Settings footer route, and the
 * admin-only Moderation surface (no sidebar row; reachable via the ⌘K palette).
 * Company pages and the Library carry their sections in the secondary sidebar
 * rather than in-page segmented controls.
 */

/**
 * Library sub-surfaces — rows of the Library secondary sidebar. They all share
 * the `library` page + LibraryBrowser body, differing only by which tab is
 * forced. Defaults to 'skills' when a library route carries no tab.
 */
export type LibraryTab = 'skills' | 'workers' | 'installed' | 'marketplace' | 'profile';

export const DEFAULT_LIBRARY_TAB: LibraryTab = 'skills';

/**
 * Company page sections — rows of the company secondary sidebar (SPEC section
 * 4: Overview / Goals / Projects / Tasks / Activity / Deployments / Secrets /
 * Library). Defaults to 'overview' when a company route carries no tab.
 */
export type CompanyTab =
  | 'overview'
  | 'goals'
  | 'projects'
  | 'tasks'
  | 'activity'
  | 'deployments'
  | 'secrets'
  | 'library';

export const DEFAULT_COMPANY_TAB: CompanyTab = 'overview';

/** Settings sections — rows of the Settings secondary sidebar (US-013 fills the bodies). */
export type SettingsTab = 'sync' | 'notifications' | 'updates' | 'general' | 'meetings';

export const DEFAULT_SETTINGS_TAB: SettingsTab = 'sync';

export type DesktopRoute =
  | { kind: 'home' | 'companies' | 'messages' | 'meetings' | 'moderation' }
  | { kind: 'library'; tab?: LibraryTab }
  | { kind: 'settings'; tab?: SettingsTab }
  | { kind: 'company'; slug: string; tab?: CompanyTab };

export type DesktopRouteKind = DesktopRoute['kind'];

export const initialDesktopRoute: DesktopRoute = { kind: 'home' };

/** The eight company secondary-sidebar rows, in SPEC display order. */
export const COMPANY_SECTIONS: ReadonlyArray<{ id: CompanyTab; label: string }> = [
  { id: 'overview', label: 'Overview' },
  { id: 'goals', label: 'Goals' },
  { id: 'projects', label: 'Projects' },
  { id: 'tasks', label: 'Tasks' },
  { id: 'activity', label: 'Activity' },
  { id: 'deployments', label: 'Deployments' },
  { id: 'secrets', label: 'Secrets' },
  { id: 'library', label: 'Library' },
];

/** The five Library secondary-sidebar rows, in SPEC display order. */
export const LIBRARY_SECTIONS: ReadonlyArray<{ id: LibraryTab; label: string }> = [
  { id: 'skills', label: 'Skills' },
  { id: 'workers', label: 'Workers' },
  { id: 'installed', label: 'Installed' },
  { id: 'marketplace', label: 'Marketplace' },
  { id: 'profile', label: 'Profile' },
];

/** Settings secondary-sidebar rows; Meetings carries the 11px "gated" note. */
export const SETTINGS_SECTIONS: ReadonlyArray<{
  id: SettingsTab;
  label: string;
  note?: string;
}> = [
  { id: 'sync', label: 'Sync' },
  { id: 'notifications', label: 'Notifications' },
  { id: 'updates', label: 'Updates' },
  { id: 'general', label: 'General' },
  { id: 'meetings', label: 'Meetings', note: 'gated' },
];

export function getDesktopCompanies(workspaces: Workspace[]): Workspace[] {
  // Desktop is local-first. If a company folder exists on this machine, it must
  // be navigable even when it is not cloud-backed yet. Cloud-only memberships
  // also stay visible so an invite/download state does not disappear from the
  // desktop shell.
  return workspaces.filter(
    (workspace) =>
      workspace.kind === 'personal' ||
      (workspace.kind === 'company' &&
        (workspace.hasLocalFolder || workspace.state === 'cloud-only')),
  );
}

/**
 * Remount key for the routed page. Company pages key on the slug only — the
 * eight sections swap panels inside the page (keyed there), so switching
 * sections never tears down the page chrome. The library likewise keys on the
 * surface, not the tab, so tab switches don't refetch the library tree.
 */
export function getDesktopRouteKey(route: DesktopRoute): string {
  if (route.kind === 'company') return `company:${route.slug}`;
  return route.kind;
}

export function isDesktopRouteActive(route: DesktopRoute, candidate: DesktopRoute): boolean {
  if (route.kind !== candidate.kind) return false;
  if (route.kind === 'company' && candidate.kind === 'company') {
    return route.slug === candidate.slug;
  }
  return true;
}

export function getDesktopActiveCompany(
  route: DesktopRoute,
  companies: Workspace[],
): Workspace | null {
  if (route.kind !== 'company') return null;
  return companies.find((company) => company.slug === route.slug) ?? null;
}

/** First ⌘ hotkey assigned to a company row (after the five primary destinations). */
const COMPANY_HOTKEY_BASE = 6;

/**
 * ⌘1–⌘5 map to the five primary destinations in sidebar order (Home /
 * Companies / Messages / Meetings / Library); ⌘6–⌘9 map to the first four
 * companies. Mirrors `companyHotkey` below for the palette/sidebar labels.
 */
export function getDesktopHotkeyRoute(
  event: Pick<KeyboardEvent, 'key' | 'metaKey' | 'ctrlKey'>,
  companies: Workspace[],
): DesktopRoute | null {
  if (!(event.metaKey || event.ctrlKey)) return null;

  if (event.key === '1') return { kind: 'home' };
  if (event.key === '2') return { kind: 'companies' };
  if (event.key === '3') return { kind: 'messages' };
  if (event.key === '4') return { kind: 'meetings' };
  if (event.key === '5') return { kind: 'library' };

  const companyIndex = Number.parseInt(event.key, 10) - COMPANY_HOTKEY_BASE;
  if (companyIndex >= 0 && companyIndex <= 9 - COMPANY_HOTKEY_BASE) {
    const company = companies[companyIndex];
    if (company) return { kind: 'company', slug: company.slug };
  }

  return null;
}

/** ⌘ hotkey label for the company at `index`, or undefined past ⌘9. */
export function companyHotkey(index: number): string | undefined {
  const hotkeyNumber = COMPANY_HOTKEY_BASE + index;
  return hotkeyNumber <= 9 ? `⌘${hotkeyNumber}` : undefined;
}

/**
 * Resolve a backend navigation intent (desktop_alt_consume_pending_route /
 * the `desktop:navigate` event) to a route. Legacy aliases stay functional:
 * 'sync' deep-links — the pre-V4 home surface — land on Home.
 */
export function resolvePendingDesktopRoute(name: string | null | undefined): DesktopRoute | null {
  const normalized = name?.trim().replace(/\//g, ':');
  if (!normalized) return null;

  const [kind, first, second] = normalized.split(':');

  if (kind === 'company' && first) {
    const tab = isCompanyTab(second) ? second : undefined;
    return tab ? { kind: 'company', slug: first, tab } : { kind: 'company', slug: first };
  }

  if (kind === 'library') {
    const tab = isLibraryTab(first) ? first : undefined;
    return tab ? { kind: 'library', tab } : { kind: 'library' };
  }

  if (kind === 'settings') {
    const tab = isSettingsTab(first) ? first : undefined;
    return tab ? { kind: 'settings', tab } : { kind: 'settings' };
  }

  switch (normalized) {
    case 'home':
    case 'sync':
      return { kind: 'home' };
    case 'companies':
      return { kind: 'companies' };
    case 'messages':
      return { kind: 'messages' };
    case 'meetings':
      return { kind: 'meetings' };
    case 'library':
      return { kind: 'library' };
    case 'settings':
      return { kind: 'settings' };
    default:
      return null;
  }
}

function isCompanyTab(value: string | undefined): value is CompanyTab {
  return COMPANY_SECTIONS.some((section) => section.id === value);
}

function isLibraryTab(value: string | undefined): value is LibraryTab {
  return LIBRARY_SECTIONS.some((section) => section.id === value);
}

function isSettingsTab(value: string | undefined): value is SettingsTab {
  return SETTINGS_SECTIONS.some((section) => section.id === value);
}

/**
 * Narrow a V4Sidebar navigation payload (open-ended V4Route) back into the
 * app's DesktopRoute union. Unknown kinds land on Home — the exception surface.
 */
export function fromV4Route(route: V4Route): DesktopRoute {
  if (route.kind === 'company' && route.slug) {
    return { kind: 'company', slug: route.slug };
  }
  switch (route.kind) {
    case 'home':
      return { kind: 'home' };
    case 'companies':
      return { kind: 'companies' };
    case 'messages':
      return { kind: 'messages' };
    case 'meetings':
      return { kind: 'meetings' };
    case 'library':
      return { kind: 'library' };
    case 'settings':
      return { kind: 'settings' };
    default:
      return { kind: 'home' };
  }
}

/** Secondary (contextual) sidebar render model — null on surfaces without one. */
export interface DesktopSecondarySidebar {
  surface: 'company' | 'library' | 'settings';
  header: string;
  headerTone: V4DotTone | null;
  meta: string | null;
  items: V4SecondaryItem[];
  activeId: string;
  footer: V4SecondaryFooter | null;
}

export interface DesktopSecondarySidebarOptions {
  /** App version for the Settings header meta line. */
  version?: string | null;
}

/**
 * SPEC section 4: the secondary sidebar exists ONLY on company, Library, and
 * Settings surfaces. Home, Companies, Meetings, and Moderation have none, and
 * Messages keeps its own 300px conversation list instead. A company route
 * whose slug isn't connected yet renders no secondary column either (the body
 * shows the not-synced placeholder).
 */
export function getDesktopSecondarySidebar(
  route: DesktopRoute,
  companies: Workspace[],
  options: DesktopSecondarySidebarOptions = {},
): DesktopSecondarySidebar | null {
  if (route.kind === 'company') {
    const company = getDesktopActiveCompany(route, companies);
    if (!company) return null;
    const stateMeta = formatCompanyStateMeta(company);
    return {
      surface: 'company',
      header: company.displayName,
      headerTone: v4CompanyDotTone(company),
      meta:
        [formatCompanyRole(company), stateMeta]
          .filter(Boolean)
          .join(' · ') || null,
      items: COMPANY_SECTIONS.map(({ id, label }) => ({ id, label })),
      activeId: route.tab ?? DEFAULT_COMPANY_TAB,
      footer: { label: 'Company settings', meta: 'sync rules · members · roles' },
    };
  }

  if (route.kind === 'library') {
    return {
      surface: 'library',
      header: 'Library',
      headerTone: null,
      meta: '~/.hq',
      items: LIBRARY_SECTIONS.map(({ id, label }) => ({ id, label })),
      activeId: route.tab ?? DEFAULT_LIBRARY_TAB,
      footer: { label: 'Publish a pack' },
    };
  }

  if (route.kind === 'settings') {
    return {
      surface: 'settings',
      header: 'Settings',
      headerTone: null,
      meta: options.version ? `HQ Sync v${options.version}` : null,
      items: SETTINGS_SECTIONS.map(({ id, label, note }) => ({ id, label, note: note ?? null })),
      activeId: route.tab ?? DEFAULT_SETTINGS_TAB,
      // The "Sign out" footer ships with the V4 Settings surface (US-013).
      footer: null,
    };
  }

  return null;
}

function formatCompanyRole(company: Workspace): string | null {
  if (company.kind === 'personal') return 'Personal';
  const role = company.role?.trim();
  if (!role) return 'Member';
  return role.slice(0, 1).toUpperCase() + role.slice(1).toLowerCase();
}

function formatCompanyStateMeta(company: Workspace): string | null {
  if (company.kind === 'personal') {
    const lastSync = formatRelativeTime(company.lastSyncedAt);
    return lastSync ? `synced ${lastSync}` : 'local vault';
  }
  switch (company.state) {
    case 'synced': {
      const lastSync = formatRelativeTime(company.lastSyncedAt);
      return lastSync ? `synced ${lastSync}` : 'synced just now';
    }
    case 'local-only':
      return 'local only';
    case 'cloud-only':
      return 'not on this Mac';
    case 'broken':
      return 'needs reconnect';
    default:
      return null;
  }
}

/** Human relative timestamp ("just now", "5m ago") for status meta lines. */
export function formatRelativeTime(iso: string | null | undefined): string | null {
  if (!iso) return null;
  const then = new Date(iso).getTime();
  if (Number.isNaN(then)) return null;
  const secs = Math.max(0, Math.round((Date.now() - then) / 1000));
  if (secs < 60) return 'just now';
  const mins = Math.round(secs / 60);
  if (mins < 60) return `${mins}m ago`;
  const hrs = Math.round(mins / 60);
  if (hrs < 24) return `${hrs}h ago`;
  return `${Math.round(hrs / 24)}d ago`;
}
