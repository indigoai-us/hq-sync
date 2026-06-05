import { describe, expect, it } from 'vitest';
import type { Workspace } from '../lib/workspaces';
import {
  getDesktopCompanies,
  getDesktopHotkeyRoute,
  getDesktopSidebarRows,
  isDesktopRouteActive,
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

  it('maps the five library hotkeys (⌘4–⌘8) to their tabs', () => {
    const companies = getDesktopCompanies([
      company({ slug: 'synced', displayName: 'Synced', state: 'synced' }),
    ]);

    // Sync ⌘1 / Meetings ⌘2 / Messages ⌘3, then the broken-out library
    // destinations at ⌘4–⌘8.
    expect(getDesktopHotkeyRoute({ key: '4', metaKey: true, ctrlKey: false }, companies)).toEqual({
      kind: 'library',
      tab: 'skills',
    });
    expect(getDesktopHotkeyRoute({ key: '5', metaKey: true, ctrlKey: false }, companies)).toEqual({
      kind: 'library',
      tab: 'workers',
    });
    expect(getDesktopHotkeyRoute({ key: '6', metaKey: true, ctrlKey: false }, companies)).toEqual({
      kind: 'library',
      tab: 'installed',
    });
    expect(getDesktopHotkeyRoute({ key: '7', metaKey: true, ctrlKey: false }, companies)).toEqual({
      kind: 'library',
      tab: 'marketplace',
    });
    expect(getDesktopHotkeyRoute({ key: '8', metaKey: true, ctrlKey: false }, companies)).toEqual({
      kind: 'library',
      tab: 'profile',
    });
  });

  it('maps company hotkeys at ⌘9 over the filtered synced company list', () => {
    const companies = getDesktopCompanies([
      company({ slug: 'unsynced', displayName: 'Unsynced', state: 'local-only' }),
      company({ slug: 'synced', displayName: 'Synced', state: 'synced' }),
    ]);

    // Eight primary destinations (Sync, Meetings, Messages, Skills, Workers,
    // Installed, Marketplace, Profile) consume ⌘1–⌘8, so companies start at ⌘9.
    expect(getDesktopHotkeyRoute({ key: '9', metaKey: true, ctrlKey: false }, companies)).toEqual({
      kind: 'company',
      slug: 'synced',
    });
  });

  it('exposes the five library tabs as top-level sidebar rows with ⌘4–⌘8', () => {
    const rows = getDesktopSidebarRows({ kind: 'sync' }, []);
    const library = rows.filter((row) => row.route.kind === 'library');
    expect(library.map((row) => [row.label, row.shortcut, row.route.tab])).toEqual([
      ['Skills', '⌘4', 'skills'],
      ['Workers', '⌘5', 'workers'],
      ['Installed', '⌘6', 'installed'],
      ['Marketplace', '⌘7', 'marketplace'],
      ['Profile', '⌘8', 'profile'],
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

  it('keeps company hotkeys at ⌘9 whether or not the admin row is present', () => {
    const withAdmin = getDesktopSidebarRows(route, synced, { isAdmin: true });
    const companyRow = withAdmin.find((row) => row.route.kind === 'company');
    // The admin Moderation row carries no hotkey, so the company keeps ⌘9.
    expect(companyRow?.shortcut).toBe('⌘9');
  });
});

describe('desktop-alt routes — Messages (US-019)', () => {
  it('resolves the Messages route via ⌘3 and marks it active', () => {
    expect(
      getDesktopHotkeyRoute({ key: '3', metaKey: true, ctrlKey: false }, []),
    ).toEqual({ kind: 'messages' });
    expect(
      getDesktopHotkeyRoute({ key: '3', metaKey: false, ctrlKey: true }, []),
    ).toEqual({ kind: 'messages' });

    expect(isDesktopRouteActive({ kind: 'messages' }, { kind: 'messages' })).toBe(true);
    expect(isDesktopRouteActive({ kind: 'library' }, { kind: 'messages' })).toBe(false);
  });

  it('includes a Messages sidebar row at ⌘3 ahead of the library tabs and companies', () => {
    const companies = getDesktopCompanies([
      company({ slug: 'synced', displayName: 'Synced', state: 'synced' }),
    ]);
    const rows = getDesktopSidebarRows({ kind: 'messages' }, companies);

    const labelsAndShortcuts = rows.map((row) => ({
      label: row.label,
      shortcut: row.shortcut,
    }));
    expect(labelsAndShortcuts).toEqual([
      { label: 'Sync', shortcut: '⌘1' },
      { label: 'Meetings', shortcut: '⌘2' },
      { label: 'Messages', shortcut: '⌘3' },
      { label: 'Skills', shortcut: '⌘4' },
      { label: 'Workers', shortcut: '⌘5' },
      { label: 'Installed', shortcut: '⌘6' },
      { label: 'Marketplace', shortcut: '⌘7' },
      { label: 'Profile', shortcut: '⌘8' },
      { label: 'Synced', shortcut: '⌘9' },
    ]);

    const messagesRow = rows.find((row) => row.label === 'Messages');
    expect(messagesRow?.route).toEqual({ kind: 'messages' });
    expect(messagesRow?.active).toBe(true);
  });
});
