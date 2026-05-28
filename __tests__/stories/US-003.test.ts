import pkg from '../../package.json' with { type: 'json' };
import type { Workspace } from '../../src/lib/workspaces';
import {
  DESKTOP_SHELL_LAYOUT,
  getDesktopCompanies,
  getDesktopHotkeyRoute,
  getDesktopPage,
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
  it('shows the 216px desktop sidebar with Sync, Meetings, and company rows on mount', () => {
    const companies = getDesktopCompanies(workspaces);
    const rows = getDesktopSidebarRows(initialDesktopRoute, companies);
    const page = getDesktopPage(initialDesktopRoute, companies);

    expect(DESKTOP_SHELL_LAYOUT).toEqual({
      sidebarWidthPx: 216,
      statusBarHeightPx: 26,
    });
    expect(pkg.version).toMatch(/^\d+\.\d+\.\d+/);
    expect(rows.map((row) => row.label)).toEqual(['Sync', 'Meetings', 'Acme Corp']);
    expect(rows.map((row) => row.shortcut)).toEqual(['⌘1', '⌘2', '⌘3']);
    expect(rows[0]).toMatchObject({ active: true, route: { kind: 'sync' } });
    expect(page).toMatchObject({ title: 'Sync', placeholder: 'Sync page - wired in US-005' });
  });

  it('switches the main pane to the Meetings placeholder when the user presses ⌘2', () => {
    const companies = getDesktopCompanies(workspaces);
    const nextRoute = getDesktopHotkeyRoute(
      { key: '2', metaKey: true, ctrlKey: false },
      companies,
    );

    expect(nextRoute).toEqual({ kind: 'meetings' });
    expect(getDesktopRouteKey(nextRoute as DesktopRoute)).toBe('meetings');
    expect(getDesktopPage(nextRoute as DesktopRoute, companies)).toMatchObject({
      title: 'Meetings',
      placeholder: 'Meetings page - wired in US-005',
    });
  });

  it('switches to the Company placeholder and marks the clicked company row active', () => {
    const companies = getDesktopCompanies(workspaces);
    const acmeRow = getDesktopSidebarRows(initialDesktopRoute, companies).find(
      (row) => row.label === 'Acme Corp',
    );

    expect(acmeRow?.route).toEqual({ kind: 'company', slug: 'acme' });

    const nextRoute = acmeRow!.route;
    const rowsAfterClick = getDesktopSidebarRows(nextRoute, companies);

    expect(getDesktopRouteKey(nextRoute)).toBe('company:acme');
    expect(getDesktopPage(nextRoute, companies)).toMatchObject({
      title: 'Acme Corp',
      placeholder: 'Company page - wired in US-005',
      activeCompany: expect.objectContaining({ slug: 'acme' }),
    });
    expect(isDesktopRouteActive(nextRoute, { kind: 'company', slug: 'acme' })).toBe(true);
    expect(rowsAfterClick.find((row) => row.label === 'Acme Corp')).toMatchObject({
      active: true,
      shortcut: '⌘3',
    });
    expect(rowsAfterClick.find((row) => row.label === 'Globex')).toBeUndefined();
  });
});
