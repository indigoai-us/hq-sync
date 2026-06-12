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
    expect(row.sub).toBe('Local directory exists, not yet cloud-backed');
    expect(row.actions).toEqual(['connect']);
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
    expect(model.notConnected[0].actions).toEqual(['connect']);
  });

  it('renders pending invites with inviter context and Accept/Decline actions', () => {
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
    expect(invite.actions).toEqual(['decline', 'accept']);
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
});
