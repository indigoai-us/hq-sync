import { describe, expect, it } from 'vitest';
import type { Workspace } from '../lib/workspaces';
import {
  COMPANY_SECTIONS,
  companyHotkey,
  fromV4Route,
  getDesktopCompanies,
  getDesktopHotkeyRoute,
  getDesktopRouteKey,
  getDesktopSecondarySidebar,
  initialDesktopRoute,
  isDesktopRouteActive,
  LIBRARY_SECTIONS,
  resolvePendingDesktopRoute,
  SETTINGS_SECTIONS,
  type DesktopRoute,
} from './route';

const baseCompany: Workspace = {
  slug: 'indigo',
  displayName: 'Indigo',
  kind: 'company',
  state: 'synced',
  cloudUid: 'cmp_1',
  bucketName: 'bucket',
  hasLocalFolder: true,
  localPath: '/tmp/HQ/companies/indigo',
  membershipStatus: 'active',
  role: 'member',
  lastSyncedAt: null,
  brokenReason: null,
  invitedBy: null,
  invitedAt: null,
};

function company(overrides: Partial<Workspace>): Workspace {
  return {
    ...baseCompany,
    ...overrides,
    kind: 'company',
  };
}

describe('US-002 V4 desktop routes', () => {
  it('starts on Home — the exception surface', () => {
    expect(initialDesktopRoute).toEqual({ kind: 'home' });
  });

  it('exposes local-first companies plus the personal page in desktop navigation', () => {
    const visible = getDesktopCompanies([
      company({
        slug: 'synced',
        displayName: 'Synced',
        state: 'synced',
        hasLocalFolder: false,
      }),
      company({
        slug: 'local',
        displayName: 'Local',
        state: 'local-only',
        cloudUid: null,
      }),
      company({ slug: 'cloud', displayName: 'Cloud', state: 'cloud-only', hasLocalFolder: false }),
      company({ slug: 'broken', displayName: 'Broken', state: 'broken' }),
      {
        ...baseCompany,
        slug: 'personal',
        displayName: 'Personal',
        kind: 'personal',
        state: 'personal',
      },
    ]);

    // Local folders get a page even before they are cloud-backed. Cloud-only
    // memberships stay visible too. A stale/missing hasLocalFolder flag must
    // not erase a workspace returned by the backend.
    expect(visible.map((workspace) => workspace.slug)).toEqual([
      'synced',
      'local',
      'cloud',
      'broken',
      'personal',
    ]);
  });

  it('deduplicates repeated slugs before they reach keyed sidebar rendering', () => {
    const visible = getDesktopCompanies([
      company({ slug: 'dupe', displayName: 'Dupe Local', state: 'local-only' }),
      company({ slug: 'dupe', displayName: 'Dupe Cloud', state: 'cloud-only' }),
      company({ slug: 'next', displayName: 'Next' }),
    ]);

    expect(visible.map((workspace) => workspace.slug)).toEqual(['dupe', 'next']);
    expect(visible.map((workspace) => workspace.displayName)).toEqual(['Dupe Local', 'Next']);
  });

  it('declares the company sections in SPEC order with Overview first and Accounts second', () => {
    expect(COMPANY_SECTIONS.map((section) => section.id)).toEqual([
      'overview',
      'accounts',
      'goals',
      'projects',
      'tasks',
      'activity',
      'deployments',
      'secrets',
      'library',
    ]);
  });

  it('resolves an accounts company deep-link to the accounts tab', () => {
    expect(resolvePendingDesktopRoute('company:indigo:accounts')).toEqual({
      kind: 'company',
      slug: 'indigo',
      tab: 'accounts',
    });
  });

  it('declares the five library sections in SPEC order', () => {
    expect(LIBRARY_SECTIONS.map((section) => section.id)).toEqual([
      'skills',
      'workers',
      'installed',
      'marketplace',
      'profile',
    ]);
  });

  it('keys company pages by slug only so section switches never remount the page', () => {
    expect(getDesktopRouteKey({ kind: 'company', slug: 'indigo', tab: 'overview' })).toBe(
      'company:indigo',
    );
    expect(getDesktopRouteKey({ kind: 'company', slug: 'indigo', tab: 'secrets' })).toBe(
      'company:indigo',
    );
    expect(getDesktopRouteKey({ kind: 'library', tab: 'workers' })).toBe('library');
    expect(getDesktopRouteKey({ kind: 'home' })).toBe('home');
  });

  it('treats every section of a company as the same active sidebar destination', () => {
    const overview: DesktopRoute = { kind: 'company', slug: 'indigo', tab: 'overview' };
    const secrets: DesktopRoute = { kind: 'company', slug: 'indigo', tab: 'secrets' };
    expect(isDesktopRouteActive(overview, secrets)).toBe(true);
    expect(
      isDesktopRouteActive(overview, { kind: 'company', slug: 'other', tab: 'overview' }),
    ).toBe(false);
    expect(isDesktopRouteActive({ kind: 'library' }, { kind: 'library', tab: 'profile' })).toBe(
      true,
    );
  });
});

describe('US-002 hotkeys — ⌘1..9 over the V4 destinations', () => {
  const companies = getDesktopCompanies([
    company({ slug: 'first', displayName: 'First', state: 'synced' }),
    company({ slug: 'second', displayName: 'Second', state: 'synced' }),
  ]);

  it('maps ⌘1–⌘6 to the six primary destinations in sidebar order, with Mission Control under Home', () => {
    const meta = (key: string) => getDesktopHotkeyRoute({ key, metaKey: true, ctrlKey: false }, companies);
    expect(meta('1')).toEqual({ kind: 'home' });
    expect(meta('2')).toEqual({ kind: 'mission-control' });
    expect(meta('3')).toEqual({ kind: 'companies' });
    expect(meta('4')).toEqual({ kind: 'messages' });
    expect(meta('5')).toEqual({ kind: 'meetings' });
    expect(meta('6')).toEqual({ kind: 'library' });
  });

  it('maps ⌘7+ to companies in list order, ctrl works too, and unmodified keys do nothing', () => {
    expect(
      getDesktopHotkeyRoute({ key: '7', metaKey: true, ctrlKey: false }, companies),
    ).toEqual({ kind: 'company', slug: 'first' });
    expect(
      getDesktopHotkeyRoute({ key: '8', metaKey: false, ctrlKey: true }, companies),
    ).toEqual({ kind: 'company', slug: 'second' });
    // No company at ⌘9 here → no route.
    expect(getDesktopHotkeyRoute({ key: '9', metaKey: true, ctrlKey: false }, companies)).toBeNull();
    expect(getDesktopHotkeyRoute({ key: '1', metaKey: false, ctrlKey: false }, companies)).toBeNull();
  });

  it('labels company hotkeys ⌘7–⌘9 and none past the ninth slot', () => {
    expect(companyHotkey(0)).toBe('⌘7');
    expect(companyHotkey(2)).toBe('⌘9');
    expect(companyHotkey(3)).toBeUndefined();
  });
});

describe('US-002 pending-route aliases (desktop_alt_consume_pending_route)', () => {
  it("keeps the legacy 'sync' deep-link functional by landing it on Home", () => {
    expect(resolvePendingDesktopRoute('sync')).toEqual({ kind: 'home' });
  });

  it('resolves the V4 destinations and rejects unknown intents', () => {
    expect(resolvePendingDesktopRoute('meetings')).toEqual({ kind: 'meetings' });
    expect(resolvePendingDesktopRoute('messages')).toEqual({ kind: 'messages' });
    expect(resolvePendingDesktopRoute('home')).toEqual({ kind: 'home' });
    expect(resolvePendingDesktopRoute('mission-control')).toEqual({ kind: 'mission-control' });
    expect(resolvePendingDesktopRoute('companies')).toEqual({ kind: 'companies' });
    expect(resolvePendingDesktopRoute('library')).toEqual({ kind: 'library' });
    expect(resolvePendingDesktopRoute('settings')).toEqual({ kind: 'settings' });
    expect(resolvePendingDesktopRoute('bogus')).toBeNull();
    expect(resolvePendingDesktopRoute(null)).toBeNull();
  });

  it('resolves deep links into company sections, library tabs, and settings tabs', () => {
    expect(resolvePendingDesktopRoute('company:indigo:projects')).toEqual({
      kind: 'company',
      slug: 'indigo',
      tab: 'projects',
    });
    expect(resolvePendingDesktopRoute('company/indigo/secrets')).toEqual({
      kind: 'company',
      slug: 'indigo',
      tab: 'secrets',
    });
    expect(resolvePendingDesktopRoute('company:indigo:not-real')).toEqual({
      kind: 'company',
      slug: 'indigo',
    });
    expect(resolvePendingDesktopRoute('library:marketplace')).toEqual({
      kind: 'library',
      tab: 'marketplace',
    });
    expect(resolvePendingDesktopRoute('settings:meetings')).toEqual({
      kind: 'settings',
      tab: 'meetings',
    });
  });
});

describe('US-002 V4Sidebar payload narrowing', () => {
  it('maps sidebar payloads onto the DesktopRoute union', () => {
    expect(fromV4Route({ kind: 'company', slug: 'indigo' })).toEqual({
      kind: 'company',
      slug: 'indigo',
    });
    expect(fromV4Route({ kind: 'settings' })).toEqual({ kind: 'settings' });
    expect(fromV4Route({ kind: 'library' })).toEqual({ kind: 'library' });
    // Unknown kinds land on Home, mirroring the sidebar model's fallback.
    expect(fromV4Route({ kind: 'mystery' })).toEqual({ kind: 'home' });
  });
});

describe('US-002 secondary sidebar — company / library / settings only', () => {
  const companies = [
    company({ slug: 'indigo', displayName: 'Indigo', state: 'synced', role: 'owner' }),
  ];

  it('shows the company sections with Overview active and Accounts second on a fresh company route', () => {
    const model = getDesktopSecondarySidebar({ kind: 'company', slug: 'indigo' }, companies);
    expect(model?.surface).toBe('company');
    expect(model?.header).toBe('Indigo');
    expect(model?.headerTone).toBe('ok');
    expect(model?.meta).toBe('Owner · synced just now');
    expect(model?.items.map((item) => item.label)).toEqual([
      'Overview',
      'Accounts',
      'Goals',
      'Projects',
      'Tasks',
      'Activity',
      'Deployments',
      'Secrets',
      'Library',
    ]);
    expect(model?.activeId).toBe('overview');
    expect(model?.footer).toEqual({
      label: 'Company settings',
      meta: 'sync rules · members · roles',
    });
  });

  it('labels local-only company pages honestly instead of pretending they just synced', () => {
    const companies = [
      company({
        slug: 'holler-mgmt',
        displayName: 'Holler Mgmt',
        state: 'local-only',
        cloudUid: null,
        role: null,
      }),
    ];
    const model = getDesktopSecondarySidebar({ kind: 'company', slug: 'holler-mgmt' }, companies);
    expect(model?.meta).toBe('Member · local only');
  });

  it('marks the routed company section active', () => {
    const model = getDesktopSecondarySidebar(
      { kind: 'company', slug: 'indigo', tab: 'deployments' },
      companies,
    );
    expect(model?.activeId).toBe('deployments');
  });

  it('renders no secondary column for a company route with no connected workspace', () => {
    expect(getDesktopSecondarySidebar({ kind: 'company', slug: 'ghost' }, companies)).toBeNull();
  });

  it('shows the five library sections with the routed tab active', () => {
    const configuredPath = ['', 'Users', 'corey', 'Documents', 'HQ'].join('/');
    const model = getDesktopSecondarySidebar(
      { kind: 'library', tab: 'marketplace' },
      companies,
      { hqFolderPath: configuredPath },
    );
    expect(model?.surface).toBe('library');
    expect(model?.meta).toBe('~/Documents/HQ');
    expect(model?.items.map((item) => item.id)).toEqual(LIBRARY_SECTIONS.map((s) => s.id));
    expect(model?.activeId).toBe('marketplace');
    expect(getDesktopSecondarySidebar({ kind: 'library' }, companies)?.activeId).toBe('skills');
  });

  it('shows the settings sections with the gated Meetings note and a version meta', () => {
    const model = getDesktopSecondarySidebar({ kind: 'settings' }, companies, {
      version: '1.2.3',
    });
    expect(model?.surface).toBe('settings');
    expect(model?.meta).toBe('HQ Sync v1.2.3');
    expect(model?.items.map((item) => item.id)).toEqual(SETTINGS_SECTIONS.map((s) => s.id));
    expect(model?.items.find((item) => item.id === 'meetings')?.note).toBe('gated');
    expect(model?.activeId).toBe('sync');
  });

  it('has no secondary sidebar on Home, Mission Control, Companies, Messages, Meetings, or Moderation', () => {
    for (const kind of [
      'home',
      'mission-control',
      'companies',
      'messages',
      'meetings',
      'moderation',
    ] as const) {
      expect(getDesktopSecondarySidebar({ kind }, companies)).toBeNull();
    }
  });
});

describe('US-009 top-level Files mode', () => {
  const companies = [company({ slug: 'indigo', displayName: 'Indigo', state: 'synced' })];

  it('resolves the files pending-route, with and without a slug + path', () => {
    expect(resolvePendingDesktopRoute('files')).toEqual({ kind: 'files' });
    expect(resolvePendingDesktopRoute('files:indigo')).toEqual({ kind: 'files', slug: 'indigo' });
    // File paths contain '/', which the normaliser turns into ':'. The path
    // remainder after the slug must survive intact (restored to slashes).
    expect(resolvePendingDesktopRoute('files:indigo:companies/indigo/a.md')).toEqual({
      kind: 'files',
      slug: 'indigo',
      path: 'companies/indigo/a.md',
    });
  });

  it('keys Files mode on its kind only so company/file changes never remount the shell', () => {
    expect(getDesktopRouteKey({ kind: 'files' })).toBe('files');
    expect(getDesktopRouteKey({ kind: 'files', slug: 'indigo' })).toBe('files');
    expect(getDesktopRouteKey({ kind: 'files', slug: 'indigo', path: 'a/b.md' })).toBe('files');
  });

  it('treats every Files-mode route as the same active destination', () => {
    const route: DesktopRoute = { kind: 'files', slug: 'indigo', path: 'a/b.md' };
    expect(isDesktopRouteActive(route, { kind: 'files' })).toBe(true);
    expect(isDesktopRouteActive(route, { kind: 'files', slug: 'other' })).toBe(true);
    expect(isDesktopRouteActive(route, { kind: 'home' })).toBe(false);
  });

  it('renders no secondary sidebar in Files mode', () => {
    expect(getDesktopSecondarySidebar({ kind: 'files', slug: 'indigo' }, companies)).toBeNull();
    expect(
      getDesktopSecondarySidebar({ kind: 'files', slug: 'indigo', path: 'a/b.md' }, companies),
    ).toBeNull();
  });

  it('narrows the Files nav payload onto the DesktopRoute union with no slug', () => {
    expect(fromV4Route({ kind: 'files' })).toEqual({ kind: 'files' });
  });

  it('has dropped the company Files secondary-sidebar section', () => {
    expect(COMPANY_SECTIONS.some((section) => (section.id as string) === 'files')).toBe(false);
    const model = getDesktopSecondarySidebar({ kind: 'company', slug: 'indigo' }, companies);
    expect(model?.items.some((item) => item.label === 'Files')).toBe(false);
  });
});

describe('US-006 Mission Control destination', () => {
  it('is a primary destination route keyed on its own kind', () => {
    const route: DesktopRoute = { kind: 'mission-control' };
    expect(getDesktopRouteKey(route)).toBe('mission-control');
  });

  it('treats every Mission Control route as the same active sidebar destination', () => {
    const route: DesktopRoute = { kind: 'mission-control' };
    expect(isDesktopRouteActive(route, { kind: 'mission-control' })).toBe(true);
    expect(isDesktopRouteActive(route, { kind: 'home' })).toBe(false);
    expect(isDesktopRouteActive({ kind: 'home' }, route)).toBe(false);
  });

  it('resolves the backend navigation intent to the Mission Control route', () => {
    expect(resolvePendingDesktopRoute('mission-control')).toEqual({ kind: 'mission-control' });
  });

  it('narrows a V4 sidebar payload for Mission Control onto the DesktopRoute union', () => {
    expect(fromV4Route({ kind: 'mission-control' })).toEqual({ kind: 'mission-control' });
  });

  it('places Mission Control directly under Home on the ⌘2 hotkey', () => {
    const companies = getDesktopCompanies([
      company({ slug: 'first', displayName: 'First', state: 'synced' }),
    ]);
    expect(getDesktopHotkeyRoute({ key: '1', metaKey: true, ctrlKey: false }, companies)).toEqual({
      kind: 'home',
    });
    expect(getDesktopHotkeyRoute({ key: '2', metaKey: true, ctrlKey: false }, companies)).toEqual({
      kind: 'mission-control',
    });
  });
});

describe('US-012 Mission Control destination — routing coverage gate', () => {
  const companies = [company({ slug: 'indigo', displayName: 'Indigo', state: 'synced' })];

  it('resolves the destination intent and trims surrounding whitespace', () => {
    expect(resolvePendingDesktopRoute('mission-control')).toEqual({ kind: 'mission-control' });
    // The slash→colon normaliser must not split the hyphenated kind into a bogus
    // 'mission'/'control' pair — the whole token has to survive as one kind.
    expect(resolvePendingDesktopRoute('  mission-control  ')).toEqual({ kind: 'mission-control' });
  });

  it('renders no secondary sidebar — Mission Control is a full-width global surface', () => {
    expect(getDesktopSecondarySidebar({ kind: 'mission-control' }, companies)).toBeNull();
  });

  it('is a distinct primary destination, not aliased onto any other surface', () => {
    const route: DesktopRoute = { kind: 'mission-control' };
    for (const other of [
      { kind: 'home' },
      { kind: 'companies' },
      { kind: 'messages' },
      { kind: 'meetings' },
      { kind: 'library' },
      { kind: 'settings' },
    ] as DesktopRoute[]) {
      expect(isDesktopRouteActive(route, other)).toBe(false);
      expect(getDesktopRouteKey(other)).not.toBe(getDesktopRouteKey(route));
    }
    // And it round-trips through the V4 sidebar payload narrowing unchanged.
    expect(fromV4Route({ kind: 'mission-control' })).toEqual(route);
  });
});
