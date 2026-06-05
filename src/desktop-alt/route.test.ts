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

  it('maps company hotkeys over the filtered synced company list', () => {
    const companies = getDesktopCompanies([
      company({ slug: 'unsynced', displayName: 'Unsynced', state: 'local-only' }),
      company({ slug: 'synced', displayName: 'Synced', state: 'synced' }),
    ]);

    // Sync ⌘1 / Meetings ⌘2 / Library ⌘3 are the three top-level destinations
    // (the board lives per-company on the company page), so company hotkeys start
    // at ⌘4.
    expect(getDesktopHotkeyRoute({ key: '3', metaKey: true, ctrlKey: false }, companies)).toEqual({
      kind: 'library',
    });
    expect(getDesktopHotkeyRoute({ key: '4', metaKey: true, ctrlKey: false }, companies)).toEqual({
      kind: 'company',
      slug: 'synced',
    });
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
    // Sits after Library and before any company row.
    const moderationIndex = labels.indexOf('Moderation');
    expect(moderationIndex).toBe(labels.indexOf('Library') + 1);
    expect(moderationIndex).toBeLessThan(labels.indexOf('Synced'));
    // The Moderation row routes to the moderation kind and carries no hotkey
    // (so company ⌘-hotkeys are unaffected by the admin gate).
    const moderationRow = rows[moderationIndex];
    expect(moderationRow.route).toEqual({ kind: 'moderation' });
    expect(moderationRow.shortcut).toBeUndefined();
  });

  it('keeps company hotkeys at ⌘4 whether or not the admin row is present', () => {
    const withAdmin = getDesktopSidebarRows(route, synced, { isAdmin: true });
    const companyRow = withAdmin.find((row) => row.route.kind === 'company');
    // The admin Moderation row does not consume ⌘4 — the company keeps it.
    expect(companyRow?.shortcut).toBe('⌘4');
  });
});
