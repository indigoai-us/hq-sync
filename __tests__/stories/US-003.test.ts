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
  it('shows the 216px desktop sidebar with Sync, Meetings, the personal page, and company rows on mount', () => {
    const companies = getDesktopCompanies(workspaces);
    const rows = getDesktopSidebarRows(initialDesktopRoute, companies);

    // Window redesign (#175) added a titlebar and grew the status bar; assert
    // the shipped layout contract.
    expect(DESKTOP_SHELL_LAYOUT).toEqual({
      sidebarWidthPx: 216,
      titleBarHeightPx: 42,
      statusBarHeightPx: 32,
    });
    expect(pkg.version).toMatch(/^\d+\.\d+\.\d+/);
    // The top-level Board surface was removed — the board lives on each
    // company/personal page now. Top-level destinations are Sync (⌘1),
    // Meetings (⌘2), and Library (⌘3); the personal page + synced companies
    // follow from ⌘4.
    expect(rows.map((row) => row.label)).toEqual([
      'Sync',
      'Meetings',
      'Library',
      'Personal',
      'Acme Corp',
    ]);
    expect(rows.map((row) => row.shortcut)).toEqual(['⌘1', '⌘2', '⌘3', '⌘4', '⌘5']);
    expect(rows[0]).toMatchObject({ active: true, route: { kind: 'sync' } });
    // Sync/Meetings are real pages — no active company resolves.
    expect(getDesktopActiveCompany(initialDesktopRoute, companies)).toBeNull();
  });

  it('switches the main pane to Meetings when the user presses ⌘2', () => {
    const companies = getDesktopCompanies(workspaces);
    const nextRoute = getDesktopHotkeyRoute(
      { key: '2', metaKey: true, ctrlKey: false },
      companies,
    );

    expect(nextRoute).toEqual({ kind: 'meetings' });
    expect(getDesktopRouteKey(nextRoute as DesktopRoute)).toBe('meetings');
    // Meetings is a non-company route — no active company resolves.
    expect(getDesktopActiveCompany(nextRoute as DesktopRoute, companies)).toBeNull();
  });

  it('gives personal a navigable page and marks a clicked company row active', () => {
    const companies = getDesktopCompanies(workspaces);
    const rows = getDesktopSidebarRows(initialDesktopRoute, companies);

    // Personal is local-first and now gets its own desktop page (⌘4, after the
    // Sync/Meetings/Library top-level rows).
    expect(rows.find((row) => row.label === 'Personal')).toMatchObject({
      route: { kind: 'company', slug: 'personal' },
      shortcut: '⌘4',
    });

    const acmeRow = rows.find((row) => row.label === 'Acme Corp');
    expect(acmeRow?.route).toEqual({ kind: 'company', slug: 'acme' });

    const nextRoute = acmeRow!.route;
    const rowsAfterClick = getDesktopSidebarRows(nextRoute, companies);

    expect(getDesktopRouteKey(nextRoute)).toBe('company:acme');
    expect(getDesktopActiveCompany(nextRoute, companies)).toMatchObject({ slug: 'acme' });
    expect(isDesktopRouteActive(nextRoute, { kind: 'company', slug: 'acme' })).toBe(true);
    expect(rowsAfterClick.find((row) => row.label === 'Acme Corp')).toMatchObject({
      active: true,
      shortcut: '⌘5',
    });
    // Globex is cloud-only (no synced local vault) → no desktop page.
    expect(rowsAfterClick.find((row) => row.label === 'Globex')).toBeUndefined();
  });
});
