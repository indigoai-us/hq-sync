import { describe, expect, it } from 'vitest';
import type { Workspace } from '../../lib/workspaces';
import { getCompaniesPageModel, getInviteMetaLine } from './companies-model';

const baseWorkspace: Workspace = {
  slug: 'acme',
  displayName: 'Acme',
  kind: 'company',
  state: 'synced',
  cloudUid: 'cmp_1',
  bucketName: 'bucket',
  hasLocalFolder: true,
  localPath: '/tmp/HQ/companies/acme',
  membershipStatus: 'active',
  role: 'owner',
  lastSyncedAt: null,
  brokenReason: null,
  invitedBy: null,
  invitedAt: null,
};

function workspace(overrides: Partial<Workspace>): Workspace {
  return { ...baseWorkspace, ...overrides };
}

const personal = workspace({
  slug: 'personal',
  displayName: 'Corey Epstein',
  kind: 'personal',
  state: 'personal',
  membershipStatus: null,
  role: null,
});

describe('V4 Companies model (US-004)', () => {
  it('renders connected rows with role, members, last change, and sync lanes', () => {
    const fifteenMinutesAgo = new Date(Date.now() - 15 * 60_000).toISOString();
    const model = getCompaniesPageModel({
      workspaces: [
        personal,
        workspace({ slug: 'indigo', displayName: 'Indigo', lastSyncedAt: fifteenMinutesAgo }),
        workspace({ slug: 'amass', displayName: 'Amass', role: 'member' }),
      ],
      syncModes: { indigo: 'all', amass: 'shared' },
      autoSyncOn: true,
    });

    expect(model.connected.map((row) => row.slug)).toEqual(['personal', 'indigo', 'amass']);

    const [personalRow, indigo, amass] = model.connected;
    expect(personalRow.sub).toBe('Personal vault · private');
    expect(personalRow.tone).toBe('ok');

    expect(indigo.sub).toBe('Owner');
    expect(indigo.tone).toBe('ok');
    expect(indigo.members).toBe('—');
    expect(indigo.lastChange).toBe('15m ago');
    expect(indigo.sync).toBe('Auto · all paths');
    expect(indigo.open).toBe(true);

    expect(amass.sub).toBe('Member');
    expect(amass.sync).toBe('Auto · shared paths');

    expect(model.summary).toBe('3 connected · syncing on every change');
    expect(model.notConnected).toEqual([]);
  });

  it('renders an in-flight Connect as an amber provisioning row with a note', () => {
    const model = getCompaniesPageModel({
      workspaces: [
        workspace({ slug: 'indigo', displayName: 'Indigo' }),
        workspace({
          slug: 'moonflow',
          displayName: 'Moonflow',
          state: 'local-only',
          cloudUid: null,
          membershipStatus: null,
          role: null,
        }),
      ],
      connectingSlugs: ['moonflow'],
    });

    // Provisioning rows sort after resting rows, matching companies.png.
    expect(model.connected.map((row) => row.slug)).toEqual(['indigo', 'moonflow']);
    const provisioning = model.connected[1];
    expect(provisioning.tone).toBe('warn');
    expect(provisioning.sub).toBe('provisioning cloud storage…');
    expect(provisioning.sync).toBe('Setting up');
    expect(provisioning.members).toBe('—');
    expect(provisioning.retry).toBe(false);
  });

  it('renders broken workspaces as red error rows with Retry', () => {
    const model = getCompaniesPageModel({
      workspaces: [
        workspace({ slug: 'indigo', displayName: 'Indigo' }),
        workspace({
          slug: 'liverecover',
          displayName: 'Liverecover',
          state: 'broken',
          brokenReason: 'manifest cloud_uid cmp_x not found in your cloud memberships',
        }),
      ],
    });

    const errorRow = model.connected.find((row) => row.slug === 'liverecover');
    expect(errorRow).toBeDefined();
    expect(errorRow?.tone).toBe('error');
    expect(errorRow?.retry).toBe(true);
    expect(errorRow?.sub).toContain('connection error');
    // Error rows sort last.
    expect(model.connected[model.connected.length - 1].slug).toBe('liverecover');
  });

  // Story e2e scenario: given a workspace that needs Connect, the Companies
  // page row offers a Connect action.
  it('offers Connect for a local directory that is not yet cloud-backed', () => {
    const model = getCompaniesPageModel({
      workspaces: [
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

    expect(model.notConnected).toHaveLength(1);
    const row = model.notConnected[0];
    expect(row.kind).toBe('local');
    expect(row.sub).toBe('Local directory exists · not cloud-backed');
    expect(row.actions).toEqual(['open', 'connect']);
    expect(model.summary).toContain('1 available');
  });

  it('surfaces a failed Connect attempt as an inline note on the local row', () => {
    const model = getCompaniesPageModel({
      workspaces: [
        workspace({
          slug: 'holler-mgmt',
          displayName: 'Holler Mgmt',
          state: 'local-only',
          cloudUid: null,
          membershipStatus: null,
        }),
      ],
      connectErrors: { 'holler-mgmt': 'vault unreachable' },
    });

    expect(model.notConnected[0].note).toBe('vault unreachable');
    expect(model.notConnected[0].actions).toEqual(['open', 'connect']);
  });

  it('renders pending invites with inviter context and the accept-flow handoff action', () => {
    const twoDaysAgo = new Date(Date.now() - 2 * 24 * 3600_000).toISOString();
    const model = getCompaniesPageModel({
      workspaces: [
        workspace({
          slug: 'sender-agency',
          displayName: 'Sender Agency',
          state: 'cloud-only',
          hasLocalFolder: false,
          membershipStatus: 'pending',
          role: null,
          invitedBy: 'prs_01abc',
          invitedAt: twoDaysAgo,
        }),
      ],
    });

    expect(model.connected).toEqual([]);
    const invite = model.notConnected[0];
    expect(invite.kind).toBe('invite');
    expect(invite.sub).toBe('Invite from a teammate · invited 2d ago');
    expect(invite.actions).toEqual(['open-invite']);
  });

  it('shows a human-readable inviter verbatim when the row carries an email', () => {
    const line = getInviteMetaLine(
      workspace({
        membershipStatus: 'pending',
        invitedBy: 'geoff@westbound.co',
        invitedAt: null,
      }),
    );
    expect(line).toBe('Invite from geoff@westbound.co · pending');
  });

  it('renders cloud-only active memberships as idle connected rows without Connect', () => {
    const model = getCompaniesPageModel({
      workspaces: [
        workspace({
          slug: 'keptwork',
          displayName: 'Keptwork',
          state: 'cloud-only',
          hasLocalFolder: false,
          role: 'member',
        }),
      ],
    });

    const row = model.connected[0];
    expect(row.tone).toBe('idle');
    expect(row.sub).toBe('Member · not on this Mac');
    expect(row.open).toBe(false);
    expect(model.notConnected).toEqual([]);
  });

  it('labels every sync lane Manual when auto-sync is off', () => {
    const model = getCompaniesPageModel({
      workspaces: [personal, workspace({ slug: 'indigo', displayName: 'Indigo' })],
      syncModes: { indigo: 'all' },
      autoSyncOn: false,
    });

    expect(model.connected[0].sync).toBe('Manual');
    expect(model.connected[1].sync).toBe('Manual · all paths');
    expect(model.summary).toContain('manual sync');
  });

  it('dedupes companies that appear twice (manifest + cloud union) so rows stay slug-unique', () => {
    // `list_syncable_workspaces` is the UNION of manifest companies and cloud
    // memberships — a company in both arrives twice under the same slug.
    // CompaniesPage keys its rows by slug; a duplicate key throws
    // `each_key_duplicate` in Svelte and freezes the whole page (the Companies
    // tab "loads nothing" — the body stays stuck on the previous route). The
    // model must collapse duplicates to one row per slug.
    const model = getCompaniesPageModel({
      workspaces: [
        personal,
        workspace({ slug: 'liverecover', displayName: 'Liverecover', role: 'member' }),
        // Same slug again — the cloud-membership copy of a manifest company.
        workspace({ slug: 'liverecover', displayName: 'Liverecover', role: 'member' }),
        workspace({ slug: 'indigo', displayName: 'Indigo' }),
      ],
    });

    const allSlugs = [
      ...model.connected.map((row) => row.slug),
      ...model.notConnected.map((row) => row.slug),
    ];
    // No duplicate keys — the invariant CompaniesPage's keyed {#each} relies on.
    expect(new Set(allSlugs).size).toBe(allSlugs.length);
    // The duplicate collapsed to exactly one Liverecover row.
    expect(model.connected.filter((row) => row.slug === 'liverecover')).toHaveLength(1);
    expect(model.connected.map((row) => row.slug)).toEqual(['personal', 'liverecover', 'indigo']);
  });
});

describe('V4 Companies model — per-company sync-mode toggle', () => {
  function syncedRow(slug: string, syncModes: Record<string, 'all' | 'shared' | 'custom' | null>) {
    const model = getCompaniesPageModel({
      workspaces: [workspace({ slug, displayName: slug })],
      syncModes,
      autoSyncOn: true,
    });
    return model.connected.find((row) => row.slug === slug)!;
  }

  it('exposes a toggleable mode for a synced company resolved to "all"', () => {
    const row = syncedRow('indigo', { indigo: 'all' });
    expect(row.syncMode).toBe('all');
    expect(row.canToggleSyncMode).toBe(true);
    // The string label remains as the fallback the lane renders while loading.
    expect(row.sync).toBe('Auto · all paths');
  });

  it('exposes a toggleable mode for a synced company resolved to "shared"', () => {
    const row = syncedRow('amass', { amass: 'shared' });
    expect(row.syncMode).toBe('shared');
    expect(row.canToggleSyncMode).toBe(true);
    expect(row.sync).toBe('Auto · shared paths');
  });

  it('keeps "custom" read-only — it is CLI-only and not a binary control', () => {
    const row = syncedRow('sender-agency', { 'sender-agency': 'custom' });
    expect(row.syncMode).toBe('custom');
    expect(row.canToggleSyncMode).toBe(false);
    expect(row.sync).toBe('Auto · custom paths');
  });

  it('stays label-only while the mode is still loading (no get_sync_mode yet)', () => {
    // No entry in syncModes → mode unresolved → fall back to the plain label,
    // never render a control whose on/off state we do not yet know.
    const row = syncedRow('keptwork', {});
    expect(row.syncMode).toBeNull();
    expect(row.canToggleSyncMode).toBe(false);
    expect(row.sync).toBe('Auto');
  });

  it('never offers the toggle for the personal vault', () => {
    const model = getCompaniesPageModel({ workspaces: [personal], autoSyncOn: true });
    const row = model.connected[0];
    expect(row.syncMode).toBeNull();
    expect(row.canToggleSyncMode).toBe(false);
  });

  it('never offers the toggle for cloud-only, provisioning, or broken rows', () => {
    const model = getCompaniesPageModel({
      workspaces: [
        workspace({ slug: 'cloudonly', displayName: 'CloudOnly', state: 'cloud-only', hasLocalFolder: false, role: 'member' }),
        workspace({ slug: 'mid-connect', displayName: 'Mid Connect', state: 'local-only', cloudUid: null, membershipStatus: null, role: null }),
        workspace({ slug: 'broke', displayName: 'Broke', state: 'broken', brokenReason: 'x' }),
      ],
      connectingSlugs: ['mid-connect'],
      // Even if a stale mode is somehow present for a non-synced row, it must
      // not become toggleable — only the synced branch reads syncModes.
      syncModes: { cloudonly: 'all', 'mid-connect': 'all', broke: 'all' },
    });
    for (const row of model.connected) {
      expect(row.canToggleSyncMode).toBe(false);
    }
  });
});
