import { describe, expect, it } from 'vitest';
import type { Workspace } from '../lib/workspaces';
import {
  getDesktopCompanies,
  getDesktopHotkeyRoute,
  getDesktopSidebarRows,
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
};

function company(overrides: Partial<Workspace>): Workspace {
  return {
    ...baseCompany,
    ...overrides,
    kind: 'company',
  };
}

describe('desktop-alt routes', () => {
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

  it('maps the five library hotkeys (⌘3–⌘7) to their tabs', () => {
    const companies = getDesktopCompanies([
      company({ slug: 'synced', displayName: 'Synced', state: 'synced' }),
    ]);

    // Sync ⌘1 / Meetings ⌘2, then the broken-out library destinations at ⌘3–⌘7.
    expect(getDesktopHotkeyRoute({ key: '3', metaKey: true, ctrlKey: false }, companies)).toEqual({
      kind: 'library',
      tab: 'skills',
    });
    expect(getDesktopHotkeyRoute({ key: '4', metaKey: true, ctrlKey: false }, companies)).toEqual({
      kind: 'library',
      tab: 'workers',
    });
    expect(getDesktopHotkeyRoute({ key: '5', metaKey: true, ctrlKey: false }, companies)).toEqual({
      kind: 'library',
      tab: 'installed',
    });
    expect(getDesktopHotkeyRoute({ key: '6', metaKey: true, ctrlKey: false }, companies)).toEqual({
      kind: 'library',
      tab: 'marketplace',
    });
    expect(getDesktopHotkeyRoute({ key: '7', metaKey: true, ctrlKey: false }, companies)).toEqual({
      kind: 'library',
      tab: 'profile',
    });
  });

  it('maps company hotkeys at ⌘8+ over the filtered synced company list', () => {
    const companies = getDesktopCompanies([
      company({ slug: 'unsynced', displayName: 'Unsynced', state: 'local-only' }),
      company({ slug: 'synced', displayName: 'Synced', state: 'synced' }),
    ]);

    // Seven primary destinations (Sync, Meetings, Skills, Workers, Installed,
    // Marketplace, Profile) consume ⌘1–⌘7, so companies start at ⌘8.
    expect(getDesktopHotkeyRoute({ key: '8', metaKey: true, ctrlKey: false }, companies)).toEqual({
      kind: 'company',
      slug: 'synced',
    });
  });

  it('exposes the five library tabs as top-level sidebar rows with ⌘3–⌘7', () => {
    const rows = getDesktopSidebarRows({ kind: 'sync' }, []);
    const library = rows.filter((row) => row.route.kind === 'library');
    expect(library.map((row) => [row.label, row.shortcut, row.route.tab])).toEqual([
      ['Skills', '⌘3', 'skills'],
      ['Workers', '⌘4', 'workers'],
      ['Installed', '⌘5', 'installed'],
      ['Marketplace', '⌘6', 'marketplace'],
      ['Profile', '⌘7', 'profile'],
    ]);
    // The old single "Library" row is gone.
    expect(rows.map((row) => row.label)).not.toContain('Library');
  });

  it('marks only the active library tab as current', () => {
    const rows = getDesktopSidebarRows({ kind: 'library', tab: 'workers' }, []);
    const active = rows.filter((row) => row.active).map((row) => row.label);
    expect(active).toEqual(['Workers']);
  });
});

describe('desktop-alt sidebar rows — admin-only Moderation entry', () => {
  const route: DesktopRoute = { kind: 'sync' };
  const synced = [company({ slug: 'synced', displayName: 'Synced', state: 'synced' })];

  it('hides the Moderation row for a non-admin (default-deny)', () => {
    const labelsDefault = getDesktopSidebarRows(route, synced).map((row) => row.label);
    const labelsFalse = getDesktopSidebarRows(route, synced, { isAdmin: false }).map(
      (row) => row.label,
    );
    // Default (no options) and explicit false both omit Moderation — the row
    // only appears on an explicit true.
    expect(labelsDefault).not.toContain('Moderation');
    expect(labelsFalse).not.toContain('Moderation');
  });

  it('shows the Moderation row for an admin, after the standing primary rows', () => {
    const rows = getDesktopSidebarRows(route, synced, { isAdmin: true });
    const labels = rows.map((row) => row.label);
    expect(labels).toContain('Moderation');
    // Sits after the last primary library row (Profile) and before any company.
    const moderationIndex = labels.indexOf('Moderation');
    expect(moderationIndex).toBe(labels.indexOf('Profile') + 1);
    expect(moderationIndex).toBeLessThan(labels.indexOf('Synced'));
    // The Moderation row routes to the moderation kind and carries no hotkey
    // (so company ⌘-hotkeys are unaffected by the admin gate).
    const moderationRow = rows[moderationIndex];
    expect(moderationRow.route).toEqual({ kind: 'moderation' });
    expect(moderationRow.shortcut).toBeUndefined();
  });

  it('keeps company hotkeys at ⌘8 whether or not the admin row is present', () => {
    const withAdmin = getDesktopSidebarRows(route, synced, { isAdmin: true });
    const companyRow = withAdmin.find((row) => row.route.kind === 'company');
    // The admin Moderation row carries no hotkey, so the company keeps ⌘8.
    expect(companyRow?.shortcut).toBe('⌘8');
  });
});
