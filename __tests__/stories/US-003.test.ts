import pkg from '../../package.json' with { type: 'json' };
import type { Workspace } from '../../src/lib/workspaces';
import {
  getDesktopCompanies,
  getDesktopActiveCompany,
  getDesktopHotkeyRoute,
  getDesktopRouteKey,
  initialDesktopRoute,
  isDesktopRouteActive,
  type DesktopRoute,
} from '../../src/desktop-alt/route';
import { getV4SidebarModel, V4_CHROME_LAYOUT } from '../../src/desktop-alt/v4/model';

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
    invitedBy: null,
    invitedAt: null,
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

describe('US-003: Desktop-alt app shell — sidebar, route state, ⌘ hotkeys (V4 IA since US-002)', () => {
  it('shows the V4 sidebar with the five nav destinations and the COMPANIES section on mount', () => {
    // The V4 window redesign (US-001/US-002) replaced the 216px rail + 42px
    // titlebar with the 220px raised sidebar + 40px title bar + 200px
    // contextual secondary sidebar.
    expect(V4_CHROME_LAYOUT).toEqual({
      titleBarHeightPx: 40,
      primarySidebarWidthPx: 220,
      secondarySidebarWidthPx: 200,
    });
    expect(pkg.version).toMatch(/^\d+\.\d+\.\d+/);

    const model = getV4SidebarModel(initialDesktopRoute, workspaces);
    expect(model.nav.map((row) => row.label)).toEqual([
      'Home',
      'Companies',
      'Messages',
      'Meetings',
      'Library',
    ]);
    // Home is the initial route and the only active row.
    expect(model.nav.find((row) => row.active)?.id).toBe('home');
    expect(model.companies.map((row) => row.label)).toEqual([
      'Personal',
      'Acme Corp',
      'Globex',
    ]);
    // The initial route is a non-company surface — no active company resolves.
    expect(
      getDesktopActiveCompany(initialDesktopRoute, getDesktopCompanies(workspaces)),
    ).toBeNull();
  });

  it('switches the main pane to Meetings when the user presses ⌘4', () => {
    const companies = getDesktopCompanies(workspaces);
    const nextRoute = getDesktopHotkeyRoute(
      { key: '4', metaKey: true, ctrlKey: false },
      companies,
    );

    expect(nextRoute).toEqual({ kind: 'meetings' });
    expect(getDesktopRouteKey(nextRoute as DesktopRoute)).toBe('meetings');
    // Meetings is a non-company route — no active company resolves.
    expect(getDesktopActiveCompany(nextRoute as DesktopRoute, companies)).toBeNull();
  });

  it('gives personal a navigable page and marks a clicked company row active', () => {
    const companies = getDesktopCompanies(workspaces);

    // Personal is local-first and keeps its own desktop page — first company
    // hotkey (⌘6, after the five primary destinations).
    expect(
      getDesktopHotkeyRoute({ key: '6', metaKey: true, ctrlKey: false }, companies),
    ).toEqual({ kind: 'company', slug: 'personal' });
    expect(
      getDesktopHotkeyRoute({ key: '7', metaKey: true, ctrlKey: false }, companies),
    ).toEqual({ kind: 'company', slug: 'acme' });

    const nextRoute: DesktopRoute = { kind: 'company', slug: 'acme' };
    expect(getDesktopRouteKey(nextRoute)).toBe('company:acme');
    expect(getDesktopActiveCompany(nextRoute, companies)).toMatchObject({ slug: 'acme' });
    expect(isDesktopRouteActive(nextRoute, { kind: 'company', slug: 'acme' })).toBe(true);

    // The sidebar highlights the clicked company row — and nothing else.
    const model = getV4SidebarModel(nextRoute, workspaces);
    expect(model.companies.filter((row) => row.active).map((row) => row.slug)).toEqual(['acme']);
    expect(model.nav.every((row) => !row.active)).toBe(true);

    // Globex is cloud-only (no synced local vault) but still gets a desktop
    // page so the user can see and act on the membership instead of losing it.
    expect(companies.find((company) => company.slug === 'globex')).toMatchObject({
      slug: 'globex',
      state: 'cloud-only',
    });
    expect(
      getDesktopActiveCompany({ kind: 'company', slug: 'globex' }, companies),
    ).toMatchObject({ slug: 'globex' });
  });
});
