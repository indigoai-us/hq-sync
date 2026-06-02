import { describe, expect, it } from 'vitest';
import type { Workspace } from '../../lib/workspaces';
import {
  buildAttentionItems,
  buildSourceRows,
  currentSyncLabel,
  emptyWorkspaceStats,
  type WorkspaceSyncStats,
} from './sync-model';
import { CORE_SETUP_LABEL } from '../../lib/progressLabel';

const personalWorkspace: Workspace = {
  slug: 'personal',
  displayName: 'Personal',
  kind: 'personal',
  state: 'personal',
  cloudUid: 'person_1',
  bucketName: 'bucket',
  hasLocalFolder: true,
  localPath: '/tmp/HQ/personal',
  membershipStatus: null,
  role: null,
  lastSyncedAt: null,
  brokenReason: null,
};

const companyWorkspace: Workspace = {
  slug: 'acme',
  displayName: 'Acme',
  kind: 'company',
  state: 'synced',
  cloudUid: 'cmp_1',
  bucketName: 'bucket',
  hasLocalFolder: true,
  localPath: '/tmp/HQ/companies/acme',
  membershipStatus: 'active',
  role: 'member',
  lastSyncedAt: null,
  brokenReason: null,
};

function stats(overrides: Partial<WorkspaceSyncStats>): WorkspaceSyncStats {
  return {
    ...emptyWorkspaceStats(),
    ...overrides,
  };
}

describe('currentSyncLabel core-noise collapsing', () => {
  const workspaces = [personalWorkspace, companyWorkspace];

  it('returns "Preparing sync" with no progress', () => {
    expect(currentSyncLabel(null, workspaces, [])).toBe('Preparing sync');
  });

  it('collapses core/ paths into the calm setup label', () => {
    const label = currentSyncLabel(
      { company: 'personal', path: 'core/policies/x.md', bytes: 1 },
      workspaces,
      [],
    );
    expect(label).toBe(`Personal / ${CORE_SETUP_LABEL}`);
  });

  it('shows the real path for the user\'s own files', () => {
    const label = currentSyncLabel(
      { company: 'acme', path: 'knowledge/strategy.md', bytes: 1 },
      workspaces,
      [],
    );
    expect(label).toBe('Acme / knowledge/strategy.md');
  });
});

describe('desktop-alt sync model attention states', () => {
  it('marks source rows with sync errors as needing attention', () => {
    const [row] = buildSourceRows({
      workspaces: [companyWorkspace],
      syncState: 'error',
      progress: null,
      statsBySlug: {
        acme: stats({ errorMessage: 'Access denied' }),
      },
      cloudReachable: true,
    });

    expect(row.liveState).toBe('warn');
    expect(row.action).toBe('Needs attention');
    expect(row.warning).toBe('Access denied');
  });

  it('surfaces conflict-aborted syncs in source rows and attention items', () => {
    const statsBySlug = {
      personal: stats({ conflicts: 1, aborted: true }),
    };
    const [row] = buildSourceRows({
      workspaces: [personalWorkspace],
      syncState: 'conflict',
      progress: null,
      statsBySlug,
      cloudReachable: true,
    });
    const attention = buildAttentionItems({
      workspaces: [personalWorkspace],
      syncState: 'conflict',
      syncErrorMessage: '',
      cloudReachable: true,
      cloudError: null,
      manifestError: null,
      statsBySlug,
    });

    expect(row.liveState).toBe('warn');
    expect(row.action).toBe('Needs attention');
    expect(attention).toEqual(
      expect.arrayContaining([
        expect.objectContaining({
          key: 'source-attention:personal',
          title: 'Personal needs attention',
        }),
      ]),
    );
  });

  it('shows personal first-push progress on the personal source row', () => {
    const [row] = buildSourceRows({
      workspaces: [personalWorkspace],
      syncState: 'syncing',
      progress: { company: 'personal', path: 'Notes/intro.md', bytes: 0 },
      statsBySlug: {
        personal: stats({ progressedFiles: 2, plannedFiles: 5 }),
      },
      cloudReachable: true,
    });

    expect(row.liveState).toBe('syncing');
    expect(row.action).toBe('Syncing');
    expect(row.progressPct).toBe(40);
  });

  it('surfaces reauth, paused, cloud, manifest, and top-level error attention', () => {
    const brokenWorkspace: Workspace = {
      ...companyWorkspace,
      slug: 'broken',
      displayName: 'Broken Co',
      state: 'broken',
      brokenReason: 'Token expired',
    };
    const pausedWorkspace: Workspace = {
      ...companyWorkspace,
      slug: 'paused',
      displayName: 'Paused Co',
      membershipStatus: 'paused',
    };

    const attention = buildAttentionItems({
      workspaces: [brokenWorkspace, pausedWorkspace],
      syncState: 'error',
      syncErrorMessage: 'Runner failed',
      cloudReachable: false,
      cloudError: 'Cloud timeout',
      manifestError: 'Manifest parse failed',
      statsBySlug: {},
    });

    expect(attention).toEqual(
      expect.arrayContaining([
        expect.objectContaining({ key: 'sync-error', title: 'Sync needs attention' }),
        expect.objectContaining({ key: 'cloud-unreachable', detail: 'Cloud timeout' }),
        expect.objectContaining({ key: 'manifest-error', detail: 'Manifest parse failed' }),
        expect.objectContaining({ key: 'reauth:broken', title: 'Broken Co needs reconnect' }),
        expect.objectContaining({ key: 'paused:paused', title: 'Paused Co is paused' }),
      ]),
    );
  });
});
