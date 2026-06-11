import { describe, expect, it } from 'vitest';
import type { Workspace } from '../../lib/workspaces';
import {
  buildAttentionItems,
  buildSourceRows,
  currentSyncLabel,
  emptyWorkspaceStats,
  friendlySyncError,
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

describe('friendlySyncError', () => {
  it('summarizes SCOPE_SHRINK_BLOCKED without leaking UIDs and keeps the count', () => {
    const raw =
      'ScopeShrinkBlockedError code=SCOPE_SHRINK_BLOCKED Sync scope shrank for ' +
      'cmp_01KQ7P52H2T70HAWX9E65Z2BZV (all → shared); 4150 dirty file(s) outside the new scope';
    const { summary, detail } = friendlySyncError(raw);

    expect(summary.length).toBeLessThanOrEqual(90);
    expect(summary).not.toContain('cmp_');
    expect(summary).not.toContain('code=');
    expect(summary).not.toContain('ScopeShrinkBlockedError');
    expect(summary).toBe(
      "A workspace's sharing scope shrank, so sync paused to avoid removing files outside it.",
    );
    expect(detail).not.toBeNull();
    expect(detail).toContain('4,150');
    expect(detail).not.toContain('cmp_');
  });

  it('maps network failures to a connection message with no detail', () => {
    const { summary, detail } = friendlySyncError(
      'NetworkError code=NET_FAIL Connection reset by peer',
    );
    expect(summary).toBe("Couldn't reach the sync server — check your connection.");
    expect(detail).toBeNull();
  });

  it('maps auth failures to a sign-in message', () => {
    const { summary, detail } = friendlySyncError('HttpError code=AUTH 401 token expired');
    expect(summary).toBe('Your session needs a refresh — sign in again.');
    expect(detail).toBeNull();
  });

  it('truncates an unknown long message and exposes the full cleaned detail', () => {
    const long =
      'SomethingWeirdError code=MYSTERY the sync runner produced an unusually long ' +
      'diagnostic line that goes well beyond ninety characters and just keeps describing internal state';
    const { summary, detail } = friendlySyncError(long);

    expect(summary.length).toBeLessThanOrEqual(91); // 90 chars + ellipsis
    expect(summary.endsWith('…')).toBe(true);
    expect(summary).not.toContain('code=');
    expect(summary).not.toContain('SomethingWeirdError');
    expect(detail).not.toBeNull();
    expect(detail?.length ?? 0).toBeGreaterThan(summary.length);
  });

  it('returns no detail when the summary already says it all', () => {
    const { summary, detail } = friendlySyncError('Disk full.');
    expect(summary).toBe('Disk full.');
    expect(detail).toBeNull();
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

describe('desktop-alt sources Connect affordance', () => {
  // The Sources tab gained an inline Connect for rows that can be reconciled
  // to the cloud in place (mirrors the menubar WorkspaceList). `connectable`
  // gates that button; it must light up only for local-only / broken rows and
  // stay off for rows that already have a cloud presence (which instead get the
  // hover Shared/All toggle).
  const localOnly: Workspace = {
    ...companyWorkspace,
    slug: 'local-only',
    displayName: 'Local Only Co',
    state: 'local-only',
    cloudUid: null,
  };
  const broken: Workspace = {
    ...companyWorkspace,
    slug: 'broken',
    displayName: 'Broken Co',
    state: 'broken',
    brokenReason: 'Manifest out of sync',
  };
  const cloudOnly: Workspace = {
    ...companyWorkspace,
    slug: 'cloud-only',
    displayName: 'Cloud Only Co',
    state: 'cloud-only',
  };

  function rowsBySlug(workspaces: Workspace[]) {
    const rows = buildSourceRows({
      workspaces,
      syncState: 'idle',
      progress: null,
      statsBySlug: {},
      cloudReachable: true,
    });
    return Object.fromEntries(rows.map((r) => [r.slug, r]));
  }

  it('marks local-only and broken rows as connectable', () => {
    const rows = rowsBySlug([localOnly, broken]);
    expect(rows['local-only'].connectable).toBe(true);
    expect(rows['broken'].connectable).toBe(true);
  });

  it('does not mark cloud-backed or personal rows as connectable', () => {
    const rows = rowsBySlug([personalWorkspace, companyWorkspace, cloudOnly]);
    expect(rows['personal'].connectable).toBe(false);
    expect(rows['acme'].connectable).toBe(false);
    expect(rows['cloud-only'].connectable).toBe(false);
  });

  it('keeps Connect and the Shared/All toggle mutually exclusive per row', () => {
    // connectable rows never show the sync-mode toggle, and vice versa — the
    // action cell renders one or the other, never both.
    const rows = rowsBySlug([localOnly, broken, companyWorkspace, cloudOnly]);
    for (const row of Object.values(rows)) {
      expect(row.connectable && row.showSyncMode).toBe(false);
    }
    expect(rows['acme'].showSyncMode).toBe(true);
    expect(rows['cloud-only'].showSyncMode).toBe(true);
  });
});
