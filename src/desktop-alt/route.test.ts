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

  it('maps company hotkeys over the filtered synced company list', () => {
    const companies = getDesktopCompanies([
      company({ slug: 'unsynced', displayName: 'Unsynced', state: 'local-only' }),
      company({ slug: 'synced', displayName: 'Synced', state: 'synced' }),
    ]);

    // Sync ⌘1 / Meetings ⌘2 / Library ⌘3 / Messages ⌘4 are the four top-level
    // destinations (the board lives per-company on the company page), so company
    // hotkeys start at ⌘5.
    expect(getDesktopHotkeyRoute({ key: '3', metaKey: true, ctrlKey: false }, companies)).toEqual({
      kind: 'library',
    });
    expect(getDesktopHotkeyRoute({ key: '4', metaKey: true, ctrlKey: false }, companies)).toEqual({
      kind: 'messages',
    });
    expect(getDesktopHotkeyRoute({ key: '5', metaKey: true, ctrlKey: false }, companies)).toEqual({
      kind: 'company',
      slug: 'synced',
    });
  });

  it('resolves the Messages route via ⌘4 and marks it active', () => {
    expect(
      getDesktopHotkeyRoute({ key: '4', metaKey: true, ctrlKey: false }, []),
    ).toEqual({ kind: 'messages' });
    expect(
      getDesktopHotkeyRoute({ key: '4', metaKey: false, ctrlKey: true }, []),
    ).toEqual({ kind: 'messages' });

    expect(isDesktopRouteActive({ kind: 'messages' }, { kind: 'messages' })).toBe(true);
    expect(isDesktopRouteActive({ kind: 'library' }, { kind: 'messages' })).toBe(false);
  });

  it('includes a Messages sidebar row at ⌘4 ahead of companies renumbered to ⌘5+', () => {
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
      { label: 'Library', shortcut: '⌘3' },
      { label: 'Messages', shortcut: '⌘4' },
      { label: 'Synced', shortcut: '⌘5' },
    ]);

    const messagesRow = rows.find((row) => row.label === 'Messages');
    expect(messagesRow?.route).toEqual({ kind: 'messages' });
    expect(messagesRow?.active).toBe(true);
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
    // Sits after the last standing primary row (Messages) and before companies.
    const moderationIndex = labels.indexOf('Moderation');
    expect(moderationIndex).toBe(labels.indexOf('Messages') + 1);
    expect(moderationIndex).toBeLessThan(labels.indexOf('Synced'));
    // The Moderation row routes to the moderation kind and carries no hotkey
    // (so company ⌘-hotkeys are unaffected by the admin gate).
    const moderationRow = rows[moderationIndex];
    expect(moderationRow.route).toEqual({ kind: 'moderation' });
    expect(moderationRow.shortcut).toBeUndefined();
  });

  it('keeps company hotkeys at ⌘5 whether or not the admin row is present', () => {
    const withAdmin = getDesktopSidebarRows(route, synced, { isAdmin: true });
    const companyRow = withAdmin.find((row) => row.route.kind === 'company');
    // Four primary rows (Sync/Meetings/Library/Messages) → companies start at ⌘5;
    // the admin Moderation row carries no hotkey, so the company keeps ⌘5.
    expect(companyRow?.shortcut).toBe('⌘5');
  });
});
