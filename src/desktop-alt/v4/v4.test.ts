import { describe, expect, it } from 'vitest';
import type { Workspace } from '../../lib/workspaces';
import {
  getV4SidebarModel,
  getV4TitleBarModel,
  V4_NAV_ITEMS,
  V4_SIDEBAR_MAX_COMPANIES,
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

  it('renders nav rows in the SPEC order Home/Companies/Messages/Meetings/Library', () => {
    const model = getV4SidebarModel({ kind: 'home' }, workspaces);
    expect(model.nav.map((row) => row.label)).toEqual([
      'Home',
      'Companies',
      'Messages',
      'Meetings',
      'Library',
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
  it('renders one row per workspace with the display name and status dot tone', () => {
    const model = getV4SidebarModel({ kind: 'home' }, [
      company({ slug: 'synced', displayName: 'Synced Co', state: 'synced' }),
      company({ slug: 'broken', displayName: 'Broken Co', state: 'broken' }),
      company({ slug: 'local', displayName: 'Local Co', state: 'local-only', cloudUid: null }),
      company({ slug: 'cloud', displayName: 'Cloud Co', state: 'cloud-only', hasLocalFolder: false }),
      personal,
    ]);

    expect(model.companies.map((row) => [row.slug, row.label, row.tone])).toEqual([
      ['synced', 'Synced Co', 'ok'],
      ['broken', 'Broken Co', 'error'],
      ['local', 'Local Co', 'idle'],
      ['cloud', 'Cloud Co', 'idle'],
      ['personal', 'Personal', 'ok'],
    ]);
    expect(model.overflowCount).toBe(0);
  });

  it('maps workspace state to dot tone (gray dot = paused, red = broken)', () => {
    expect(v4CompanyDotTone(company({ state: 'synced' }))).toBe('ok');
    expect(v4CompanyDotTone(personal)).toBe('ok');
    expect(v4CompanyDotTone(company({ state: 'broken' }))).toBe('error');
    expect(v4CompanyDotTone(company({ state: 'local-only' }))).toBe('idle');
    expect(v4CompanyDotTone(company({ state: 'cloud-only' }))).toBe('idle');
  });

  it('caps the visible list and reports the "N more…" overflow count', () => {
    const many = Array.from({ length: 9 }, (_, index) =>
      company({ slug: `co-${index}`, displayName: `Co ${index}` }),
    );
    const model = getV4SidebarModel({ kind: 'home' }, many);

    expect(model.companies).toHaveLength(V4_SIDEBAR_MAX_COMPANIES);
    expect(model.overflowCount).toBe(9 - V4_SIDEBAR_MAX_COMPANIES);
  });

  it('promotes an active overflow company into the visible window', () => {
    const many = Array.from({ length: 9 }, (_, index) =>
      company({ slug: `co-${index}`, displayName: `Co ${index}` }),
    );
    const model = getV4SidebarModel({ kind: 'company', slug: 'co-8' }, many);

    expect(model.companies).toHaveLength(V4_SIDEBAR_MAX_COMPANIES);
    expect(model.companies.at(-1)?.slug).toBe('co-8');
    expect(model.companies.at(-1)?.active).toBe(true);
    expect(model.overflowCount).toBe(9 - V4_SIDEBAR_MAX_COMPANIES);
    expect(activeRowCount(model)).toBe(1);
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
