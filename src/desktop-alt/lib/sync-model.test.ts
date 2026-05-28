import { describe, expect, it } from 'vitest';
import type { Workspace } from '../../lib/workspaces';
import {
  buildAttentionItems,
  buildSourceRows,
  emptyWorkspaceStats,
  type WorkspaceSyncStats,
} from './sync-model';

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
  lastSyncedAt: null,
  brokenReason: null,
};

function stats(overrides: Partial<WorkspaceSyncStats>): WorkspaceSyncStats {
  return {
    ...emptyWorkspaceStats(),
    ...overrides,
  };
}

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
});
