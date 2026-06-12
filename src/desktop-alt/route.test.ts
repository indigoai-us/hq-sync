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

  it('exposes synced companies plus the local-first personal page in desktop navigation', () => {
    const visible = getDesktopCompanies([
      company({ slug: 'synced', displayName: 'Synced', state: 'synced' }),
      company({ slug: 'local', displayName: 'Local', state: 'local-only', cloudUid: null }),
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

    // Synced companies get a page; non-synced companies don't; personal is
    // local-first (state 'personal') and always navigable so it gets a board too.
    expect(visible.map((workspace) => workspace.slug)).toEqual(['synced', 'personal']);
  });

  it('declares the eight company sections in SPEC order with Overview first', () => {
    expect(COMPANY_SECTIONS.map((section) => section.id)).toEqual([
      'overview',
      'goals',
      'projects',
      'tasks',
      'activity',
      'deployments',
      'secrets',
      'library',
    ]);
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

  it('maps ⌘1–⌘5 to the five primary destinations in sidebar order', () => {
    const meta = (key: string) => getDesktopHotkeyRoute({ key, metaKey: true, ctrlKey: false }, companies);
    expect(meta('1')).toEqual({ kind: 'home' });
    expect(meta('2')).toEqual({ kind: 'companies' });
    expect(meta('3')).toEqual({ kind: 'messages' });
    expect(meta('4')).toEqual({ kind: 'meetings' });
    expect(meta('5')).toEqual({ kind: 'library' });
  });

  it('maps ⌘6+ to companies in list order, ctrl works too, and unmodified keys do nothing', () => {
    expect(
      getDesktopHotkeyRoute({ key: '6', metaKey: true, ctrlKey: false }, companies),
    ).toEqual({ kind: 'company', slug: 'first' });
    expect(
      getDesktopHotkeyRoute({ key: '7', metaKey: false, ctrlKey: true }, companies),
    ).toEqual({ kind: 'company', slug: 'second' });
    // No company at ⌘8/⌘9 here → no route.
    expect(getDesktopHotkeyRoute({ key: '8', metaKey: true, ctrlKey: false }, companies)).toBeNull();
    expect(getDesktopHotkeyRoute({ key: '1', metaKey: false, ctrlKey: false }, companies)).toBeNull();
  });

  it('labels company hotkeys ⌘6–⌘9 and none past the ninth slot', () => {
    expect(companyHotkey(0)).toBe('⌘6');
    expect(companyHotkey(3)).toBe('⌘9');
    expect(companyHotkey(4)).toBeUndefined();
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
    expect(resolvePendingDesktopRoute('companies')).toEqual({ kind: 'companies' });
    expect(resolvePendingDesktopRoute('library')).toEqual({ kind: 'library' });
    expect(resolvePendingDesktopRoute('settings')).toEqual({ kind: 'settings' });
    expect(resolvePendingDesktopRoute('bogus')).toBeNull();
    expect(resolvePendingDesktopRoute(null)).toBeNull();
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

  it('shows the 8 company sections with Overview active on a fresh company route', () => {
    const model = getDesktopSecondarySidebar({ kind: 'company', slug: 'indigo' }, companies);
    expect(model?.surface).toBe('company');
    expect(model?.header).toBe('Indigo');
    expect(model?.headerTone).toBe('ok');
    expect(model?.items.map((item) => item.label)).toEqual([
      'Overview',
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
    const model = getDesktopSecondarySidebar({ kind: 'library', tab: 'marketplace' }, companies);
    expect(model?.surface).toBe('library');
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

  it('has no secondary sidebar on Home, Companies, Messages, Meetings, or Moderation', () => {
    for (const kind of ['home', 'companies', 'messages', 'meetings', 'moderation'] as const) {
      expect(getDesktopSecondarySidebar({ kind }, companies)).toBeNull();
    }
  });
});
