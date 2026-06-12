import { describe, expect, it } from 'vitest';
import { getCompaniesPageModel } from '../../src/desktop-alt/v4/companies-model';
import type { Workspace } from '../../src/lib/workspaces';
import { readRepoFile } from './harness';

/**
 * US-004 — V4 Companies overview (companies.png).
 *
 * Source-contract + model harness, matching the existing desktop-alt spec
 * style. Story E2E scenario: given a workspace with a needs-connect state
 * (a local directory that is not yet cloud-backed), when Companies renders,
 * then its row shows a Connect action.
 */

function workspace(overrides: Partial<Workspace>): Workspace {
  return {
    slug: 'indigo',
    displayName: 'Indigo',
    kind: 'company',
    state: 'synced',
    cloudUid: 'cmp_1',
    bucketName: 'bucket',
    hasLocalFolder: true,
    localPath: '/tmp/HQ/companies/indigo',
    membershipStatus: 'active',
    role: 'owner',
    lastSyncedAt: null,
    brokenReason: null,
    invitedBy: null,
    invitedAt: null,
    ...overrides,
  };
}

describe('desktop-alt V4 Companies (US-004)', () => {
  it('a workspace that needs Connect renders a NOT CONNECTED row with a Connect action', () => {
    const model = getCompaniesPageModel({
      workspaces: [
        workspace({}),
        workspace({
          slug: 'holler-mgmt',
          displayName: 'Holler Mgmt',
          state: 'local-only',
          cloudUid: null,
          membershipStatus: null,
          role: null,
        }),
      ],
    });

    const row = model.notConnected.find((entry) => entry.slug === 'holler-mgmt');
    expect(row).toBeDefined();
    expect(row?.kind).toBe('local');
    expect(row?.actions).toEqual(['connect']);
    // The synced workspace stays in the CONNECTED table.
    expect(model.connected.map((entry) => entry.slug)).toEqual(['indigo']);

    // The Connect affordance is wired to the real backend command.
    const companiesPage = readRepoFile('src/desktop-alt/pages/CompaniesPage.svelte');
    expect(companiesPage).toContain("await invoke('connect_workspace_to_cloud', { slug })");
    expect(companiesPage).toContain('Connect');
  });

  it('renders connection state lanes, provisioning rows, error rows with Retry, and invites', () => {
    const model = getCompaniesPageModel({
      workspaces: [
        workspace({}),
        workspace({ slug: 'moonflow', displayName: 'Moonflow', state: 'local-only', cloudUid: null, membershipStatus: null }),
        workspace({ slug: 'liverecover', displayName: 'Liverecover', state: 'broken', brokenReason: 'token expired' }),
        workspace({
          slug: 'sender-agency',
          displayName: 'Sender Agency',
          state: 'cloud-only',
          hasLocalFolder: false,
          membershipStatus: 'pending',
          role: null,
          invitedBy: 'geoff@westbound.co',
          invitedAt: null,
        }),
      ],
      connectingSlugs: ['moonflow'],
      syncModes: { indigo: 'all' },
      autoSyncOn: true,
    });

    // CONNECTED: resting rows, then provisioning (amber), then errors (red + Retry).
    expect(model.connected.map((row) => [row.slug, row.tone, row.retry])).toEqual([
      ['indigo', 'ok', false],
      ['moonflow', 'warn', false],
      ['liverecover', 'error', true],
    ]);
    expect(model.connected[0].sync).toBe('Auto · all paths');
    expect(model.connected[1].sub).toBe('provisioning cloud storage…');

    // NOT CONNECTED: the invite row shows inviter context + Accept/Decline.
    const invite = model.notConnected.find((row) => row.kind === 'invite');
    expect(invite?.sub).toContain('Invite from geoff@westbound.co');
    expect(invite?.actions).toEqual(['decline', 'accept']);
  });

  it('DesktopApp mounts CompaniesPage on the companies route and the page footnote points at per-company settings', () => {
    const desktopApp = readRepoFile('src/desktop-alt/DesktopApp.svelte');
    expect(desktopApp).toContain('<CompaniesPage');
    expect(desktopApp).toContain("onopencompany={(slug) => navigate({ kind: 'company', slug })}");

    const companiesPage = readRepoFile('src/desktop-alt/pages/CompaniesPage.svelte');
    expect(companiesPage).toContain(
      'Per-company sync rules, excluded paths, and member roles live inside each company',
    );
  });
});
