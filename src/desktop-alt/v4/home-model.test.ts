import { describe, expect, it } from 'vitest';
import type { Workspace } from '../../lib/workspaces';
import type { ActivityEntry, SyncCompanyRef, WorkspaceSyncStats } from '../lib/sync-model';
import { emptyWorkspaceStats } from '../lib/sync-model';
import type { Project } from '../lib/projects-model';
import type { MeetingEvent } from '../lib/meetings-model';
import {
  activityFileVerb,
  getConflictCardModel,
  getDriftCardModel,
  getHomeCompanyRows,
  getHomeDigestGroups,
  getHomeErrorModel,
  getHomeMetaLine,
  getHomePortfolioStats,
  getHomeProgressModel,
  getHomeTodayAgenda,
  getNeedsYouCount,
  type HomeConflict,
  type HomeCoreState,
} from './home-model';

const baseWorkspace: Workspace = {
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
  invitedBy: null,
  invitedAt: null,
};

function workspace(overrides: Partial<Workspace>): Workspace {
  return { ...baseWorkspace, ...overrides };
}

function conflict(overrides: Partial<HomeConflict> = {}): HomeConflict {
  return {
    path: 'policies/slack-channel.md',
    canAutoResolve: false,
    status: 'pending',
    at: Date.now() - 15 * 60_000,
    ...overrides,
  };
}

function coreState(overrides: Partial<HomeCoreState['driftReport']> = {}): HomeCoreState {
  return {
    targetVersion: '15.0.15',
    driftReport: {
      count: 1,
      modified: [
        { path: 'core/hooks/hook-gate.sh', size: 10, gitShaLocal: 'aaa', gitShaUpstream: 'bbb' },
      ],
      missing: [],
      added: [],
      hqVersion: '15.0.15',
      targetRepo: 'indigoai-us/hq-core',
      targetRef: 'v15.0.15',
      ...overrides,
    },
  };
}

function entry(overrides: Partial<ActivityEntry>): ActivityEntry {
  return {
    company: 'hpo',
    path: 'policies/hpo-review-cadence.md',
    bytes: 900,
    direction: 'down',
    at: Date.now(),
    ...overrides,
  };
}

describe('US-003 Home meta line', () => {
  it('healthy: cadence, daemon, last sync, and version', () => {
    const line = getHomeMetaLine({
      syncState: 'idle',
      autoSyncOn: true,
      daemonRunning: true,
      lastSyncLabel: 'Just now',
      hqVersion: '15.0.15',
    });
    expect(line).toBe(
      'auto-sync on · syncs on every change · daemon running · last sync just now · HQ v15.0.15',
    );
  });

  it('syncing: start time and auto-sync note', () => {
    const line = getHomeMetaLine({
      syncState: 'syncing',
      autoSyncOn: true,
      daemonRunning: true,
      lastSyncLabel: 'Just now',
      hqVersion: '15.0.15',
      syncStartedLabel: '11:32 AM',
    });
    expect(line).toBe('sync in progress · started 11:32 AM · auto-sync on · HQ v15.0.15');
  });

  it('error: pause + retry note in the meta line', () => {
    const line = getHomeMetaLine({
      syncState: 'error',
      autoSyncOn: true,
      daemonRunning: false,
      lastSyncLabel: '5m ago',
      hqVersion: '15.0.15',
    });
    expect(line).toBe('auto-sync paused after failure · retries on the next change · HQ v15.0.15');
  });

  it('omits unknown facts instead of guessing', () => {
    const line = getHomeMetaLine({
      syncState: 'idle',
      autoSyncOn: null,
      daemonRunning: null,
      lastSyncLabel: null,
      hqVersion: null,
    });
    expect(line).toBe('no syncs yet');
  });
});

describe('US-003 NEEDS YOU queue', () => {
  it('conflict card offers Keep mine / Take theirs / Compare', () => {
    const card = getConflictCardModel(conflict());
    expect(card.title).toContain('policies/slack-channel.md');
    expect(card.sub).toContain('auto-resolve not possible');
    expect(card.tone).toBe('warn');
    expect(card.actions.map((action) => [action.id, action.label])).toEqual([
      ['keep-local', 'Keep mine'],
      ['keep-remote', 'Take theirs'],
      ['compare', 'Compare'],
    ]);
  });

  it('resolving conflicts disable the resolution actions', () => {
    const card = getConflictCardModel(conflict({ status: 'resolving' }));
    expect(card.sub).toContain('resolving…');
    expect(card.actions.find((action) => action.id === 'keep-local')?.disabled).toBe(true);
    expect(card.actions.find((action) => action.id === 'keep-remote')?.disabled).toBe(true);
  });

  it('drift card names the drifted file and offers Restore / Keep edit / View diff', () => {
    const card = getDriftCardModel(coreState());
    expect(card?.title).toBe(
      '1 core file drifted from v15.0.15 — core/hooks/hook-gate.sh edited locally',
    );
    expect(card?.sub).toBe('restore will overwrite your local change');
    expect(card?.actions.map((action) => action.id)).toEqual(['restore', 'keep-edit', 'view-diff']);
  });

  it('drift card collapses multiple files into a "+ N more" title', () => {
    const card = getDriftCardModel(
      coreState({
        count: 3,
        modified: [
          { path: 'core/hooks/hook-gate.sh', size: 1, gitShaLocal: 'a', gitShaUpstream: 'b' },
          { path: 'core/scripts/reindex.sh', size: 1, gitShaLocal: 'c', gitShaUpstream: 'd' },
          { path: 'core/docs/INDEX.md', size: 1, gitShaLocal: 'e', gitShaUpstream: 'f' },
        ],
      }),
    );
    expect(card?.title).toBe(
      '3 core files drifted from v15.0.15 — core/hooks/hook-gate.sh + 2 more edited locally',
    );
  });

  it('no drift card when the USER-EDIT count is zero', () => {
    expect(getDriftCardModel(coreState({ count: 0, modified: [] }))).toBeNull();
  });

  it('counts pending conflicts plus the drift card; Keep edit dismisses drift', () => {
    expect(getNeedsYouCount([conflict()], coreState(), false)).toBe(2);
    expect(getNeedsYouCount([conflict()], coreState(), true)).toBe(1);
    expect(getNeedsYouCount([], null, false)).toBe(0);
  });
});

describe('US-003 syncing progress card', () => {
  const companies: SyncCompanyRef[] = [
    { uid: 'cmp_1', slug: 'corey-epstein', name: 'corey-epstein' },
    { uid: 'cmp_2', slug: 'hpo', name: 'hpo' },
    { uid: 'cmp_3', slug: 'indigo', name: 'indigo' },
    { uid: 'cmp_4', slug: 'amass', name: 'amass' },
    { uid: 'cmp_5', slug: 'keptwork', name: 'keptwork' },
  ];
  const statsBySlug: Record<string, WorkspaceSyncStats> = {
    'corey-epstein': { ...emptyWorkspaceStats(), completedFiles: 97 },
    hpo: { ...emptyWorkspaceStats(), completedFiles: 14 },
    indigo: { ...emptyWorkspaceStats(), plannedFiles: 301, progressedFiles: 76 },
  };

  it('renders the file counts, per-company fanout rows, and the queued row', () => {
    const model = getHomeProgressModel({
      filesProgressed: 187,
      totalFiles: 412,
      transferredBytes: 2.1 * 1024 * 1024,
      progress: { company: 'indigo', path: 'policies/indigo-hq-slack-channel.md', bytes: 1 },
      companies,
      statsBySlug,
      workspaces: [],
    });

    expect(model.headline).toBe('187 of 412 files');
    expect(model.pct).toBeCloseTo((187 / 412) * 100, 5);
    expect(model.meta).toBe('2.1 MB transferred');
    expect(model.rows.map((row) => [row.slug, row.state])).toEqual([
      ['corey-epstein', 'done'],
      ['hpo', 'done'],
      ['indigo', 'active'],
    ]);
    expect(model.rows[0].detail).toBe('done · 97 files');
    expect(model.rows[2].detail).toBe(
      'downloading policies/indigo-hq-slack-channel.md · 76 of 301',
    );
    expect(model.queued).toEqual({ count: 2, names: 'amass, keptwork' });
  });

  it('is indeterminate before totals are known', () => {
    const model = getHomeProgressModel({
      filesProgressed: 0,
      totalFiles: 0,
      transferredBytes: 0,
      progress: null,
      companies: [],
      statsBySlug: {},
      workspaces: [],
    });
    expect(model.headline).toBe('Preparing sync…');
    expect(model.pct).toBeNull();
    expect(model.rows).toEqual([]);
    expect(model.queued).toBeNull();
  });
});

describe('US-003 error card', () => {
  it('auth failures get plain language, Sign in again, and technical details', () => {
    const model = getHomeErrorModel({
      syncState: 'auth-error',
      syncErrorMessage: 'VaultError 403 TokenExpired for cmp_01J8',
      errorCompany: 'indigo',
      workspaces: [workspace({})],
      companies: [],
      appVersion: '0.7.3',
      lastSyncLabel: '11:18 AM',
    });

    expect(model?.title).toBe('Sync failed for Indigo — Your session needs a refresh — sign in again.');
    expect(model?.showSignIn).toBe(true);
    expect(model?.techLines.join('\n')).toContain('runner: hq-sync v0.7.3');
    expect(model?.techLines.join('\n')).toContain('~/.hq/sync-journal.log');
    expect(model?.techLines.join('\n')).toContain('last good sync 11:18 am');
  });

  it('non-auth failures offer Retry only and keep the raw message in details', () => {
    const model = getHomeErrorModel({
      syncState: 'error',
      syncErrorMessage: 'NET_FAIL connection reset',
      errorCompany: null,
      workspaces: [],
      companies: [],
      appVersion: '0.7.3',
      lastSyncLabel: null,
    });
    expect(model?.title).toBe("Sync failed — Couldn't reach the sync server — check your connection.");
    expect(model?.showSignIn).toBe(false);
    expect(model?.techLines.join('\n')).toContain('NET_FAIL connection reset');
  });

  it('returns null outside error states', () => {
    expect(
      getHomeErrorModel({
        syncState: 'idle',
        syncErrorMessage: '',
        errorCompany: null,
        workspaces: [],
        companies: [],
        appVersion: '0.7.3',
        lastSyncLabel: null,
      }),
    ).toBeNull();
  });
});

describe('US-003 actor-grouped digest', () => {
  it('verb lanes map deleted/new/updated to DEL/ADD/UPD', () => {
    expect(activityFileVerb(entry({ direction: 'deleted' }))).toBe('DEL');
    expect(activityFileVerb(entry({ direction: 'down', isNew: true }))).toBe('ADD');
    expect(activityFileVerb(entry({ direction: 'down' }))).toBe('UPD');
    expect(activityFileVerb(entry({ direction: 'up' }))).toBe('UPD');
  });

  it('groups by author, newest first, with expandable file rows', () => {
    const now = Date.now();
    const groups = getHomeDigestGroups(
      [
        entry({
          author: 'geoff@westbound.co',
          path: 'policies/hpo-meta-relaunch-gate.md',
          bytes: 1331,
          isNew: true,
          at: now - 60_000,
        }),
        entry({
          author: 'geoff@westbound.co',
          path: 'policies/hpo-review-cadence.md',
          bytes: 921,
          isNew: true,
          at: now - 120_000,
        }),
        entry({ author: 'hassaan@hpo.co', path: 'docs/a.md', at: now - 30_000 }),
      ],
      [workspace({ slug: 'hpo', displayName: 'hpo' })],
    );

    expect(groups).toHaveLength(2);
    // Hassaan's entry is newest, so his group sorts first.
    expect(groups[0].actor).toBe('Hassaan');
    expect(groups[1].actor).toBe('Geoff');
    expect(groups[1].initials).toBe('GE');
    expect(groups[1].headline).toBe('Geoff added 2 files to hpo');
    expect(groups[1].meta).toContain('geoff@westbound.co');
    expect(groups[1].files.map((file) => file.verb)).toEqual(['ADD', 'ADD']);
    expect(groups[1].files[0].sizeLabel).toBe('1.3 KB');
  });

  it('attributes uploads to You and falls back to the company for anonymous downloads', () => {
    const groups = getHomeDigestGroups(
      [
        entry({ direction: 'up', company: 'indigo', author: undefined }),
        entry({ direction: 'down', company: 'indigo', author: undefined }),
      ],
      [workspace({})],
    );
    expect(groups.map((group) => group.actor).sort()).toEqual(['Indigo', 'You']);
  });

  it('spans companies in the headline when an actor touched more than one', () => {
    const groups = getHomeDigestGroups(
      [
        entry({ author: 'a@b.co', company: 'indigo' }),
        entry({ author: 'a@b.co', company: 'hpo' }),
      ],
      [workspace({}), workspace({ slug: 'hpo', displayName: 'hpo' })],
    );
    expect(groups[0].headline).toContain('across');
    expect(groups[0].headline).toContain('Indigo');
    expect(groups[0].headline).toContain('hpo');
  });
});

// ── Merged-Home real-data sections ──────────────────────────────────────────

function project(overrides: Partial<Project> = {}): Project {
  return {
    id: overrides.id ?? 'p1',
    title: overrides.title ?? 'Project',
    name: overrides.name ?? overrides.title ?? 'Project',
    description: overrides.description ?? '',
    company: overrides.company ?? 'indigo',
    status: overrides.status ?? '',
    prdPath: overrides.prdPath ?? '',
    createdAt: overrides.createdAt ?? null,
    updatedAt: overrides.updatedAt ?? null,
    storiesTotal: overrides.storiesTotal ?? 0,
    storiesComplete: overrides.storiesComplete ?? 0,
  };
}

function meeting(overrides: Partial<MeetingEvent> & { startISO?: string } = {}): MeetingEvent {
  const start = overrides.startISO ?? new Date().toISOString();
  return {
    id: overrides.id ?? 'm1',
    summary: overrides.summary,
    start: overrides.start ?? { dateTime: start },
    end: overrides.end ?? { dateTime: start },
    status: overrides.status ?? 'confirmed',
    sourceCompanyUid: overrides.sourceCompanyUid,
  };
}

describe('getHomePortfolioStats', () => {
  it('counts companies (not personal), active projects, and open stories — all real', () => {
    const stats = getHomePortfolioStats({
      workspaces: [
        workspace({ slug: 'personal', kind: 'personal', state: 'personal' }),
        workspace({ slug: 'indigo' }),
        workspace({ slug: 'amass' }),
      ],
      projects: [
        project({ id: 'a', company: 'indigo', storiesTotal: 6, storiesComplete: 2 }), // active, 4 open
        project({ id: 'b', company: 'indigo', status: 'done', storiesTotal: 3, storiesComplete: 3 }), // terminal
        project({ id: 'c', company: 'amass', storiesTotal: 0 }), // active (no stories tracked)
      ],
    });
    expect(stats).toEqual([
      { label: 'Companies', value: '2' },
      { label: 'Active projects', value: '2' },
      { label: 'Open stories', value: '4' },
    ]);
  });

  it('dedupes the manifest+cloud union so a doubled company is counted once', () => {
    const stats = getHomePortfolioStats({
      workspaces: [workspace({ slug: 'indigo' }), workspace({ slug: 'indigo' })],
      projects: [],
    });
    expect(stats[0]).toEqual({ label: 'Company', value: '1' });
  });

  it('never invents storage / latency / sparkline tiles', () => {
    const stats = getHomePortfolioStats({ workspaces: [], projects: [] });
    const labels = stats.map((s) => s.label.toLowerCase()).join(' ');
    expect(labels).not.toMatch(/storage|gb|p95|latency|spark|edits|shipped/);
  });
});

describe('getHomeCompanyRows', () => {
  it('rolls up local projects/stories per company and reads role + last change from the workspace', () => {
    const fifteenMinutesAgo = new Date(Date.now() - 15 * 60_000).toISOString();
    const rows = getHomeCompanyRows({
      workspaces: [
        workspace({ slug: 'personal', displayName: 'Corey', kind: 'personal', state: 'personal' }),
        workspace({ slug: 'indigo', displayName: 'Indigo', role: 'owner', lastSyncedAt: fifteenMinutesAgo }),
      ],
      projects: [
        project({ id: 'a', company: 'indigo', storiesTotal: 6, storiesComplete: 2 }),
        project({ id: 'b', company: 'indigo', status: 'done', storiesTotal: 4, storiesComplete: 4 }),
      ],
    });
    const indigo = rows.find((r) => r.slug === 'indigo')!;
    expect(indigo.sub).toBe('Owner');
    expect(indigo.tone).toBe('ok');
    expect(indigo.projects).toBe('1 active'); // the done one doesn't count
    expect(indigo.stories).toBe('6 / 10 stories'); // both projects' stories roll up
    expect(indigo.lastChange).toBe('15m ago');

    const personal = rows.find((r) => r.slug === 'personal')!;
    expect(personal.sub).toBe('Personal vault');
    expect(personal.projects).toBe('—'); // no local projects for personal
    expect(personal.stories).toBe('—');
    expect(personal.lastChange).toBe('—');
  });

  it('maps workspace state to a status tone and dedupes duplicate slugs', () => {
    const rows = getHomeCompanyRows({
      workspaces: [
        workspace({ slug: 'broke', state: 'broken' }),
        workspace({ slug: 'cloudonly', state: 'cloud-only', hasLocalFolder: false }),
        workspace({ slug: 'broke', state: 'broken' }), // dup
      ],
      projects: [],
    });
    expect(rows.map((r) => r.slug)).toEqual(['broke', 'cloudonly']); // deduped
    expect(rows.find((r) => r.slug === 'broke')!.tone).toBe('error');
    expect(rows.find((r) => r.slug === 'cloudonly')!.tone).toBe('idle');
  });
});

describe('getHomeTodayAgenda', () => {
  const NOW = new Date('2026-06-15T12:00:00');

  it("returns only today's meetings, chronological, with time + company", () => {
    const names = new Map([['cmp_indigo', 'Indigo']]);
    const agenda = getHomeTodayAgenda({
      events: [
        meeting({ id: 'late', summary: 'Standup', startISO: '2026-06-15T16:00:00', sourceCompanyUid: 'cmp_indigo' }),
        meeting({ id: 'early', summary: 'Kickoff', startISO: '2026-06-15T09:30:00' }),
        meeting({ id: 'tomorrow', summary: 'Later', startISO: '2026-06-16T09:00:00' }),
      ],
      companyNamesByUid: names,
      now: NOW,
    });
    expect(agenda.map((a) => a.id)).toEqual(['early', 'late']); // today only, sorted
    expect(agenda[0].title).toBe('Kickoff');
    expect(agenda[0].company).toBe('Personal'); // no source company uid
    expect(agenda[1].company).toBe('Indigo');
  });

  it('falls back to a placeholder title and caps the list', () => {
    const events = Array.from({ length: 9 }, (_, i) =>
      meeting({ id: `e${i}`, startISO: `2026-06-15T0${i}:00:00`, summary: i === 0 ? undefined : `M${i}` }),
    );
    const agenda = getHomeTodayAgenda({ events, companyNamesByUid: new Map(), now: NOW, limit: 6 });
    expect(agenda).toHaveLength(6);
    expect(agenda[0].title).toBe('Untitled meeting');
  });
});
