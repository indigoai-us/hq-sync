import { describe, expect, it } from 'vitest';
import type { Workspace } from '../../lib/workspaces';
import {
  getV4SidebarModel,
  getV4TitleBarModel,
  sortV4CompaniesConnectedFirst,
  V4_NAV_ITEMS,
  v4CompanyConnected,
  v4CompanyDotTone,
  type V4Route,
  type V4SidebarModel,
} from './model';

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
  return { ...baseCompany, ...overrides, kind: 'company' };
}

const personal: Workspace = {
  ...baseCompany,
  slug: 'personal',
  displayName: 'Personal',
  kind: 'personal',
  state: 'personal',
};

function activeRowCount(model: V4SidebarModel): number {
  return (
    model.nav.filter((row) => row.active).length +
    model.companies.filter((row) => row.active).length +
    (model.settingsActive ? 1 : 0)
  );
}

describe('US-001 V4 sidebar active-state mapping', () => {
  const workspaces = [
    company({ slug: 'indigo', displayName: 'Indigo' }),
    company({ slug: 'hpo', displayName: 'hpo' }),
    personal,
  ];

  it('maps each primary destination to its own nav row', () => {
    for (const item of V4_NAV_ITEMS) {
      const model = getV4SidebarModel({ kind: item.id }, workspaces);
      const active = model.nav.filter((row) => row.active);
      expect(active.map((row) => row.id)).toEqual([item.id]);
      expect(activeRowCount(model)).toBe(1);
    }
  });

  it('renders nav rows in the SPEC order Home/Mission Control/Companies/Messages/Meetings/Library/Files', () => {
    const model = getV4SidebarModel({ kind: 'home' }, workspaces);
    expect(model.nav.map((row) => row.label)).toEqual([
      'Home',
      'Mission Control',
      'Companies',
      'Messages',
      'Meetings',
      'Library',
      'Files',
    ]);
  });

  it('highlights the company row — not a nav item — on company routes', () => {
    const model = getV4SidebarModel({ kind: 'company', slug: 'hpo' }, workspaces);
    expect(model.nav.every((row) => !row.active)).toBe(true);
    expect(model.companies.filter((row) => row.active).map((row) => row.slug)).toEqual(['hpo']);
    expect(model.settingsActive).toBe(false);
    expect(activeRowCount(model)).toBe(1);
  });

  it('highlights the Settings footer — and nothing else — on the settings route', () => {
    const model = getV4SidebarModel({ kind: 'settings' }, workspaces);
    expect(model.settingsActive).toBe(true);
    expect(model.nav.every((row) => !row.active)).toBe(true);
    expect(model.companies.every((row) => !row.active)).toBe(true);
    expect(activeRowCount(model)).toBe(1);
  });

  it('falls back to the Companies nav row for a company route with no matching row', () => {
    const model = getV4SidebarModel({ kind: 'company', slug: 'ghost' }, workspaces);
    expect(model.nav.filter((row) => row.active).map((row) => row.id)).toEqual(['companies']);
    expect(activeRowCount(model)).toBe(1);
  });

  it('keeps exactly one active row on every route, including unknown kinds', () => {
    const routes: V4Route[] = [
      { kind: 'home' },
      { kind: 'companies' },
      { kind: 'messages' },
      { kind: 'meetings' },
      { kind: 'library' },
      { kind: 'settings' },
      { kind: 'company', slug: 'indigo' },
      { kind: 'company', slug: 'missing' },
      { kind: 'some-future-kind' },
    ];
    for (const route of routes) {
      expect(activeRowCount(getV4SidebarModel(route, workspaces))).toBe(1);
    }
  });
});

describe('US-001 V4 sidebar companies-list rendering', () => {
  it('renders one row per workspace with the display name and status dot tone (connected-first, alpha within group)', () => {
    const model = getV4SidebarModel({ kind: 'home' }, [
      company({ slug: 'synced', displayName: 'Synced Co', state: 'synced' }),
      company({ slug: 'broken', displayName: 'Broken Co', state: 'broken' }),
      company({ slug: 'local', displayName: 'Local Co', state: 'local-only', cloudUid: null }),
      company({ slug: 'cloud', displayName: 'Cloud Co', state: 'cloud-only', hasLocalFolder: false }),
      personal,
    ]);

    // Connected (synced / cloud-only / personal) lead, alphabetical by name;
    // the rest (broken, local-only) follow, alphabetical. Tones are unchanged.
    expect(model.companies.map((row) => [row.slug, row.label, row.tone])).toEqual([
      ['cloud', 'Cloud Co', 'idle'],
      ['personal', 'Personal', 'ok'],
      ['synced', 'Synced Co', 'ok'],
      ['broken', 'Broken Co', 'error'],
      ['local', 'Local Co', 'idle'],
    ]);
  });

  it('maps workspace state to dot tone (gray dot = paused, red = broken)', () => {
    expect(v4CompanyDotTone(company({ state: 'synced' }))).toBe('ok');
    expect(v4CompanyDotTone(personal)).toBe('ok');
    expect(v4CompanyDotTone(company({ state: 'broken' }))).toBe('error');
    expect(v4CompanyDotTone(company({ state: 'local-only' }))).toBe('idle');
    expect(v4CompanyDotTone(company({ state: 'cloud-only' }))).toBe('idle');
  });

  it('renders every workspace directly instead of truncating behind an overflow row', () => {
    const many = Array.from({ length: 9 }, (_, index) =>
      company({ slug: `co-${index}`, displayName: `Co ${index}` }),
    );
    const model = getV4SidebarModel({ kind: 'home' }, many);

    expect(model.companies).toHaveLength(9);
    expect(model.companies.map((row) => row.slug)).toEqual([
      'co-0',
      'co-1',
      'co-2',
      'co-3',
      'co-4',
      'co-5',
      'co-6',
      'co-7',
      'co-8',
    ]);
  });

  it('deduplicates repeated workspace slugs so cached local data cannot blank the app', () => {
    const model = getV4SidebarModel({ kind: 'company', slug: 'dupe' }, [
      company({ slug: 'dupe', displayName: 'Dupe Local', state: 'local-only' }),
      company({ slug: 'dupe', displayName: 'Dupe Cloud', state: 'cloud-only' }),
      company({ slug: 'next', displayName: 'Next', state: 'synced' }),
    ]);

    // First occurrence wins the dedupe (Dupe Local, local-only → idle/not
    // connected), so the connected-first sort puts 'next' (synced) ahead of it.
    expect(model.companies.map((row) => row.slug)).toEqual(['next', 'dupe']);
    expect(model.companies.find((row) => row.slug === 'dupe')?.label).toBe('Dupe Local');
    expect(model.companies.filter((row) => row.active).map((row) => row.slug)).toEqual([
      'dupe',
    ]);
    expect(activeRowCount(model)).toBe(1);
  });

  it('keeps later companies selectable and active because every row renders', () => {
    const many = Array.from({ length: 9 }, (_, index) =>
      company({ slug: `co-${index}`, displayName: `Co ${index}` }),
    );
    const model = getV4SidebarModel({ kind: 'company', slug: 'co-8' }, many);

    expect(model.companies).toHaveLength(9);
    expect(model.companies.find((row) => row.slug === 'co-8')?.active).toBe(true);
    expect(activeRowCount(model)).toBe(1);
  });
});

describe('US-007 V4 sidebar connected-first sort', () => {
  it('sorts cloud-connected companies (synced / cloud-only) above idle ones, alphabetical within group', () => {
    const model = getV4SidebarModel({ kind: 'home' }, [
      company({ slug: 'zed', displayName: 'Zed', state: 'local-only' }),
      company({ slug: 'acme', displayName: 'Acme', state: 'synced' }),
      company({ slug: 'beta', displayName: 'Beta', state: 'local-only' }),
      company({ slug: 'cloudco', displayName: 'CloudCo', state: 'cloud-only', hasLocalFolder: false }),
      company({ slug: 'orbit', displayName: 'Orbit', state: 'synced' }),
    ]);

    // Connected (Acme synced, CloudCo cloud-only, Orbit synced) lead in alpha
    // order; idle (Beta, Zed local-only) follow in alpha order.
    expect(model.companies.map((row) => row.label)).toEqual([
      'Acme',
      'CloudCo',
      'Orbit',
      'Beta',
      'Zed',
    ]);
  });

  it('sorts case-insensitively within each group', () => {
    const model = getV4SidebarModel({ kind: 'home' }, [
      company({ slug: 'b', displayName: 'banana', state: 'synced' }),
      company({ slug: 'a', displayName: 'Apple', state: 'synced' }),
      company({ slug: 'c', displayName: 'Cherry', state: 'synced' }),
    ]);
    expect(model.companies.map((row) => row.label)).toEqual(['Apple', 'banana', 'Cherry']);
  });

  it('treats personal as connected (green dot) and groups it with synced/cloud-only', () => {
    const model = getV4SidebarModel({ kind: 'home' }, [
      company({ slug: 'idle1', displayName: 'Idle One', state: 'local-only' }),
      personal,
      company({ slug: 'sync1', displayName: 'Aardvark', state: 'synced' }),
    ]);
    // personal + synced lead (alpha: Aardvark, Personal), then the idle row.
    expect(model.companies.map((row) => row.slug)).toEqual(['sync1', 'personal', 'idle1']);
  });

  it('keeps the active company highlighted after the connected-first reorder', () => {
    const model = getV4SidebarModel({ kind: 'company', slug: 'idle-active' }, [
      company({ slug: 'idle-active', displayName: 'Idle Active', state: 'local-only' }),
      company({ slug: 'conn', displayName: 'Connected', state: 'synced' }),
    ]);
    // Connected row sorts first, but the idle active row stays the only active one.
    expect(model.companies.map((row) => row.slug)).toEqual(['conn', 'idle-active']);
    expect(model.companies.filter((row) => row.active).map((row) => row.slug)).toEqual([
      'idle-active',
    ]);
    expect(activeRowCount(model)).toBe(1);
  });

  it('exposes v4CompanyConnected as the grouping predicate (synced/cloud-only/personal)', () => {
    expect(v4CompanyConnected(company({ state: 'synced' }))).toBe(true);
    expect(v4CompanyConnected(company({ state: 'cloud-only' }))).toBe(true);
    expect(v4CompanyConnected(personal)).toBe(true);
    expect(v4CompanyConnected(company({ state: 'local-only' }))).toBe(false);
    expect(v4CompanyConnected(company({ state: 'broken' }))).toBe(false);
  });
});

describe('US-009 Files nav row + shared connected-first sort', () => {
  const workspaces = [
    company({ slug: 'indigo', displayName: 'Indigo' }),
    company({ slug: 'hpo', displayName: 'hpo' }),
    personal,
  ];

  it('includes Files as the last primary nav row', () => {
    expect(V4_NAV_ITEMS.at(-1)).toEqual({ id: 'files', label: 'Files' });
  });

  it('marks the Files nav row active in Files mode with exactly one active row', () => {
    const model = getV4SidebarModel({ kind: 'files' }, workspaces);
    expect(model.nav.filter((row) => row.active).map((row) => row.id)).toEqual(['files']);
    expect(model.companies.every((row) => !row.active)).toBe(true);
    expect(model.settingsActive).toBe(false);
    expect(activeRowCount(model)).toBe(1);
  });

  it('sortV4CompaniesConnectedFirst groups connected-first, alpha within group', () => {
    const rows = sortV4CompaniesConnectedFirst([
      company({ slug: 'zed', displayName: 'Zed', state: 'local-only' }),
      company({ slug: 'acme', displayName: 'Acme', state: 'synced' }),
      company({ slug: 'beta', displayName: 'Beta', state: 'local-only' }),
      company({ slug: 'cloudco', displayName: 'CloudCo', state: 'cloud-only', hasLocalFolder: false }),
      personal,
    ]);
    // Connected (Acme synced, CloudCo cloud-only, Personal) lead alpha; idle follow.
    expect(rows.map((row) => row.label)).toEqual(['Acme', 'CloudCo', 'Personal', 'Beta', 'Zed']);
    expect(rows.every((row) => !row.active)).toBe(true);
  });

  it('marks the passed activeSlug row active (the FilesModeSidebar contract)', () => {
    const rows = sortV4CompaniesConnectedFirst(workspaces, 'hpo');
    expect(rows.filter((row) => row.active).map((row) => row.slug)).toEqual(['hpo']);
  });
});

describe('US-001 V4 title bar model', () => {
  it('shows the healthy sentence with watched count + last sync and the Sync Now action', () => {
    const model = getV4TitleBarModel({
      syncState: 'idle',
      watchedCount: 12,
      lastSyncLabel: 'just now',
    });
    expect(model).toEqual({
      tone: 'ok',
      sentence: 'All synced',
      meta: '12 watched · just now',
      action: { id: 'sync', label: 'Sync Now' },
    });
  });

  it('switches the primary action to Cancel while syncing, with fanout meta', () => {
    const model = getV4TitleBarModel({
      syncState: 'syncing',
      watchedCount: 12,
      syncingCompany: 'indigo',
      fanoutDone: 2,
      fanoutTotal: 5,
    });
    expect(model.action).toEqual({ id: 'cancel', label: 'Cancel' });
    expect(model.sentence).toBe('Syncing…');
    expect(model.meta).toBe('indigo · 2/5 companies');
  });

  it('switches the primary action to Retry on sync and auth errors', () => {
    const error = getV4TitleBarModel({
      syncState: 'error',
      watchedCount: 3,
      errorSummary: 'Connection lost',
    });
    expect(error.tone).toBe('error');
    expect(error.meta).toBe('Connection lost');
    expect(error.action).toEqual({ id: 'retry', label: 'Retry' });

    const auth = getV4TitleBarModel({ syncState: 'auth-error', watchedCount: 3 });
    expect(auth.action).toEqual({ id: 'retry', label: 'Retry' });
    expect(auth.tone).toBe('error');
  });

  it('flags conflicts as a warn state that keeps Sync Now as the action', () => {
    const model = getV4TitleBarModel({ syncState: 'conflict', watchedCount: 3 });
    expect(model.tone).toBe('warn');
    expect(model.action).toEqual({ id: 'sync', label: 'Sync Now' });
  });
});
