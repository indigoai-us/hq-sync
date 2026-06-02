import pkg from '../../package.json' with { type: 'json' };
import type { Workspace } from '../../src/lib/workspaces';
import {
  DESKTOP_SHELL_LAYOUT,
  getDesktopCompanies,
  getDesktopActiveCompany,
  getDesktopHotkeyRoute,
  getDesktopRouteKey,
  getDesktopSidebarRows,
  initialDesktopRoute,
  isDesktopRouteActive,
  type DesktopRoute,
} from '../../src/desktop-alt/route';

function workspace(overrides: Partial<Workspace>): Workspace {
  return {
    slug: 'personal',
    displayName: 'Personal',
    kind: 'personal',
    state: 'personal',
    cloudUid: null,
    bucketName: null,
    hasLocalFolder: true,
    localPath: '/Users/test/HQ',
    membershipStatus: null,
    lastSyncedAt: null,
    brokenReason: null,
    ...overrides,
  };
}

const workspaces: Workspace[] = [
  workspace({ slug: 'personal', displayName: 'Personal', kind: 'personal' }),
  workspace({
    slug: 'acme',
    displayName: 'Acme Corp',
    kind: 'company',
    state: 'synced',
    cloudUid: 'cloud-acme',
    bucketName: 'hq-acme',
    membershipStatus: 'active',
  }),
  workspace({
    slug: 'globex',
    displayName: 'Globex',
    kind: 'company',
    state: 'cloud-only',
    cloudUid: 'cloud-globex',
    bucketName: 'hq-globex',
    hasLocalFolder: false,
    localPath: null,
    membershipStatus: 'active',
  }),
];

describe('US-003: Desktop-alt Svelte 5 app shell — sidebar, route state, ⌘K hotkeys', () => {
  it('shows the 216px desktop sidebar with Board, Sync, Meetings, and company rows on mount', () => {
    const companies = getDesktopCompanies(workspaces);
    const rows = getDesktopSidebarRows(initialDesktopRoute, companies);

    expect(DESKTOP_SHELL_LAYOUT).toEqual({
      sidebarWidthPx: 216,
      statusBarHeightPx: 26,
    });
    expect(pkg.version).toMatch(/^\d+\.\d+\.\d+/);
    // US-007 added Board as the first top-level destination; Sync/Meetings
    // renumbered after it, and company rows now start at ⌘4.
    expect(rows.map((row) => row.label)).toEqual(['Board', 'Sync', 'Meetings', 'Acme Corp']);
    expect(rows.map((row) => row.shortcut)).toEqual(['⌘1', '⌘2', '⌘3', '⌘4']);
    expect(rows[1]).toMatchObject({ active: true, route: { kind: 'sync' } });
    // Sync/Meetings are real pages now (US-005) — no active company resolves.
    expect(getDesktopActiveCompany(initialDesktopRoute, companies)).toBeNull();
  });

  it('switches the main pane to Meetings when the user presses ⌘3', () => {
    const companies = getDesktopCompanies(workspaces);
    const nextRoute = getDesktopHotkeyRoute(
      { key: '3', metaKey: true, ctrlKey: false },
      companies,
    );

    expect(nextRoute).toEqual({ kind: 'meetings' });
    expect(getDesktopRouteKey(nextRoute as DesktopRoute)).toBe('meetings');
    // Meetings is a non-company route — no active company resolves.
    expect(getDesktopActiveCompany(nextRoute as DesktopRoute, companies)).toBeNull();
  });

  it('switches to the Company page and marks the clicked company row active', () => {
    const companies = getDesktopCompanies(workspaces);
    const acmeRow = getDesktopSidebarRows(initialDesktopRoute, companies).find(
      (row) => row.label === 'Acme Corp',
    );

    expect(acmeRow?.route).toEqual({ kind: 'company', slug: 'acme' });

    const nextRoute = acmeRow!.route;
    const rowsAfterClick = getDesktopSidebarRows(nextRoute, companies);

    expect(getDesktopRouteKey(nextRoute)).toBe('company:acme');
    expect(getDesktopActiveCompany(nextRoute, companies)).toMatchObject({ slug: 'acme' });
    expect(isDesktopRouteActive(nextRoute, { kind: 'company', slug: 'acme' })).toBe(true);
    expect(rowsAfterClick.find((row) => row.label === 'Acme Corp')).toMatchObject({
      active: true,
      shortcut: '⌘4',
    });
    expect(rowsAfterClick.find((row) => row.label === 'Globex')).toBeUndefined();
  });
});
