import { describe, expect, it } from 'vitest';
import type { Workspace } from '../lib/workspaces';
import { getDesktopCompanies, getDesktopHotkeyRoute } from './route';

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

    // Sync ⌘1 / Meetings ⌘2 are the two top-level destinations (the board lives
    // per-company on the company page now, not as a top-level route), so company
    // hotkeys start at ⌘3.
    expect(getDesktopHotkeyRoute({ key: '3', metaKey: true, ctrlKey: false }, companies)).toEqual({
      kind: 'company',
      slug: 'synced',
    });
  });
});
