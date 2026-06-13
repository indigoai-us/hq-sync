import { describe, expect, it } from 'vitest';
import {
  classifyStories,
  classifyStory,
  groupByState,
  labelColor,
  labelColorIndex,
  LABEL_PALETTE,
  LABEL_PALETTE_SIZE,
  deriveProjectState,
  projectProgress,
  projectProgressFromStories,
  effectiveProjectStatus,
  projectListStatus,
  projectRecencyTime,
  compareProjectsByRecency,
  matchesStatusFilter,
  projectDisplayName,
  type Story,
  type Project,
} from '../projects-model';

function story(id: string, overrides: Partial<Story> = {}): Story {
  return {
    id,
    title: `Story ${id}`,
    description: '',
    acceptanceCriteria: [],
    passes: false,
    labels: [],
    dependsOn: [],
    ...overrides,
  };
}

describe('classifyStories', () => {
  it('marks passing stories complete', () => {
    const result = classifyStories([
      story('US-001', { passes: true }),
      story('US-002', { passes: true }),
    ]);
    expect(result.map((c) => c.state)).toEqual(['complete', 'complete']);
  });

  it('assigns the first eligible story in-progress and the rest pending', () => {
    const result = classifyStories([
      story('US-001', { passes: false }),
      story('US-002', { passes: false }),
      story('US-003', { passes: false }),
    ]);
    expect(result.map((c) => c.state)).toEqual([
      'in-progress',
      'pending',
      'pending',
    ]);
  });

  it('blocks a story with an unmet dependency', () => {
    const result = classifyStories([
      story('US-001', { passes: false }),
      // US-002 depends on the not-yet-complete US-001 → blocked
      story('US-002', { passes: false, dependsOn: ['US-001'] }),
    ]);
    const byId = Object.fromEntries(result.map((c) => [c.story.id, c.state]));
    expect(byId['US-001']).toBe('in-progress');
    expect(byId['US-002']).toBe('blocked');
  });

  it('unblocks a story once its dependency passes', () => {
    const result = classifyStories([
      story('US-001', { passes: true }),
      story('US-002', { passes: false, dependsOn: ['US-001'] }),
    ]);
    const byId = Object.fromEntries(result.map((c) => [c.story.id, c.state]));
    expect(byId['US-001']).toBe('complete');
    // dependency met → US-002 is now the first eligible → in-progress
    expect(byId['US-002']).toBe('in-progress');
  });

  it('only assigns in-progress to one story even with multiple eligible', () => {
    const result = classifyStories([
      story('US-001', { passes: true }),
      story('US-002', { passes: false }),
      story('US-003', { passes: false }),
      story('US-004', { passes: false, dependsOn: ['US-009'] }),
    ]);
    const states = result.map((c) => c.state);
    expect(states.filter((s) => s === 'in-progress')).toHaveLength(1);
    expect(states).toEqual(['complete', 'in-progress', 'pending', 'blocked']);
  });
});

describe('classifyStory', () => {
  it('matches classifyStories for a complete story', () => {
    const all = [story('US-001', { passes: true })];
    expect(classifyStory(all[0], all)).toBe('complete');
  });

  it('reports blocked when a dependency is unmet', () => {
    const all = [
      story('US-001', { passes: false }),
      story('US-002', { passes: false, dependsOn: ['US-001'] }),
    ];
    expect(classifyStory(all[1], all)).toBe('blocked');
  });

  it('distinguishes in-progress from pending via isFirstEligible', () => {
    const all = [story('US-001', { passes: false })];
    expect(classifyStory(all[0], all, true)).toBe('in-progress');
    expect(classifyStory(all[0], all, false)).toBe('pending');
  });
});

describe('groupByState', () => {
  it('buckets each classified story into its state', () => {
    const groups = groupByState(
      classifyStories([
        story('US-001', { passes: true }),
        story('US-002', { passes: false }),
        story('US-003', { passes: false, dependsOn: ['US-009'] }),
      ]),
    );
    expect(groups.complete.map((c) => c.story.id)).toEqual(['US-001']);
    expect(groups['in-progress'].map((c) => c.story.id)).toEqual(['US-002']);
    expect(groups.blocked.map((c) => c.story.id)).toEqual(['US-003']);
    expect(groups.pending).toEqual([]);
  });
});

describe('labelColor determinism', () => {
  it('returns an identical color for the same input', () => {
    const a = labelColor('frontend');
    const b = labelColor('frontend');
    expect(a).toEqual(b);
    expect(a.index).toBe(b.index);
  });

  it('produces a stable index across calls', () => {
    expect(labelColorIndex('frontend')).toBe(labelColorIndex('frontend'));
    expect(labelColorIndex('backend')).toBe(labelColorIndex('backend'));
  });

  it('keeps the index within the palette bounds', () => {
    const samples = ['a', 'bug', 'frontend', 'backend', 'infra', '', 'P1', '🔥'];
    for (const s of samples) {
      const idx = labelColorIndex(s);
      expect(idx).toBeGreaterThanOrEqual(0);
      expect(idx).toBeLessThan(LABEL_PALETTE_SIZE);
      expect(labelColor(s)).toBe(LABEL_PALETTE[idx]);
    }
  });

  it('emits CSS-var-friendly hsla tokens across a multi-hue palette', () => {
    const hues = new Set<string>();
    for (const entry of LABEL_PALETTE) {
      // Each token is a well-formed hsla() string.
      expect(entry.background).toMatch(/^hsla\(\d+, \d+%, \d+%, [\d.]+\)$/);
      expect(entry.border).toMatch(/^hsla\(\d+, \d+%, \d+%, [\d.]+\)$/);
      expect(entry.foreground).toMatch(/^hsla\(\d+, \d+%, \d+%, [\d.]+\)$/);
      hues.add(entry.background.split(',')[0]);
    }
    // The palette spans multiple distinct hues (not a single monochrome shade).
    expect(hues.size).toBeGreaterThan(1);
  });

  it('distributes a realistic label set across multiple shades', () => {
    const labels = [
      'frontend',
      'backend',
      'infra',
      'bug',
      'feature',
      'docs',
      'test',
      'design',
      'security',
      'perf',
      'ci',
      'release',
    ];
    const indices = new Set(labels.map(labelColorIndex));
    // With 12 labels over 8 buckets we expect more than one bucket used.
    expect(indices.size).toBeGreaterThan(3);
  });
});

describe('deriveProjectState', () => {
  it('is pending when there are no stories', () => {
    expect(deriveProjectState(0, 0)).toBe('pending');
  });
  it('is complete when all stories pass', () => {
    expect(deriveProjectState(4, 4)).toBe('complete');
  });
  it('is in-progress when some stories pass', () => {
    expect(deriveProjectState(1, 4)).toBe('in-progress');
  });
  it('is pending when no stories pass', () => {
    expect(deriveProjectState(0, 4)).toBe('pending');
  });
});

describe('projectProgress', () => {
  it('derives complete/total/percent/state', () => {
    expect(projectProgress(3, 6)).toEqual({
      complete: 3,
      total: 6,
      percent: 50,
      state: 'in-progress',
    });
  });

  it('returns 0% with pending state for an empty project', () => {
    expect(projectProgress(0, 0)).toEqual({
      complete: 0,
      total: 0,
      percent: 0,
      state: 'pending',
    });
  });

  it('rounds the percentage', () => {
    expect(projectProgress(1, 3).percent).toBe(33);
    expect(projectProgress(2, 3).percent).toBe(67);
  });

  it('clamps complete to total and floors negatives', () => {
    expect(projectProgress(9, 4)).toEqual({
      complete: 4,
      total: 4,
      percent: 100,
      state: 'complete',
    });
    expect(projectProgress(-2, 4).complete).toBe(0);
  });

  it('computes progress directly from stories', () => {
    const stories = [
      story('US-001', { passes: true }),
      story('US-002', { passes: true }),
      story('US-003', { passes: false }),
    ];
    expect(projectProgressFromStories(stories)).toEqual({
      complete: 2,
      total: 3,
      percent: 67,
      state: 'in-progress',
    });
  });
});

describe('effectiveProjectStatus', () => {
  const base: Pick<Project, 'status' | 'storiesComplete' | 'storiesTotal'> = {
    status: 'active',
    storiesComplete: 0,
    storiesTotal: 0,
  };

  it('treats archived as terminal regardless of story rollup', () => {
    expect(
      effectiveProjectStatus({ ...base, status: 'archived', storiesComplete: 5, storiesTotal: 5 }),
    ).toBe('archived');
  });

  it('reads as complete when an active board has all stories passing', () => {
    expect(
      effectiveProjectStatus({ ...base, storiesComplete: 4, storiesTotal: 4 }),
    ).toBe('complete');
  });

  it('reads as in-progress when some stories pass', () => {
    expect(
      effectiveProjectStatus({ ...base, storiesComplete: 2, storiesTotal: 4 }),
    ).toBe('in-progress');
  });

  it('reads as pending when no stories pass', () => {
    expect(
      effectiveProjectStatus({ ...base, storiesComplete: 0, storiesTotal: 4 }),
    ).toBe('pending');
  });
});

describe('projectDisplayName', () => {
  const proj = (overrides: Partial<Project>): Project => ({
    id: 'proj-x',
    description: '',
    company: 'indigo',
    status: 'active',
    prdPath: '/x/prd.json',
    storiesTotal: 0,
    storiesComplete: 0,
    ...overrides,
  });

  it('prefers name, then title, then id', () => {
    expect(projectDisplayName(proj({ name: 'Name', title: 'Title' }))).toBe('Name');
    expect(projectDisplayName(proj({ title: 'Title' }))).toBe('Title');
    expect(projectDisplayName(proj({}))).toBe('proj-x');
  });
});

describe('project recency ordering', () => {
  const proj = (overrides: Partial<Project>): Project => ({
    id: 'proj-x',
    description: '',
    company: 'indigo',
    status: 'active',
    prdPath: '/x/prd.json',
    storiesTotal: 0,
    storiesComplete: 0,
    ...overrides,
  });

  it('prefers updatedAt, falling back to createdAt', () => {
    expect(
      projectRecencyTime(
        proj({
          createdAt: '2026-06-10T00:00:00Z',
          updatedAt: '2026-06-12T00:00:00Z',
        }),
      ),
    ).toBe(Date.parse('2026-06-12T00:00:00Z'));

    expect(projectRecencyTime(proj({ createdAt: '2026-06-10T00:00:00Z' }))).toBe(
      Date.parse('2026-06-10T00:00:00Z'),
    );
  });

  it('sorts newest projects first before status/name tie breakers', () => {
    const sorted = [
      proj({ id: 'a', title: 'Alpha', updatedAt: '2026-06-10T00:00:00Z' }),
      proj({ id: 'b', title: 'Beta', updatedAt: '2026-06-12T00:00:00Z' }),
      proj({ id: 'c', title: 'Charlie', createdAt: '2026-06-11T00:00:00Z' }),
    ].sort(compareProjectsByRecency);

    expect(sorted.map((project) => project.id)).toEqual(['b', 'c', 'a']);
  });
});

describe('projectListStatus board-status mapping', () => {
  const base = { storiesComplete: 0, storiesTotal: 0 };

  it('treats HQ board status "in_progress" as In Progress even with no completed stories', () => {
    // Regression: HQ board.json uses `in_progress`, which was previously absent
    // from the recognized board statuses, so manually-started projects fell
    // into "Planned" instead of "In Progress".
    expect(projectListStatus({ ...base, status: 'in_progress' })).toBe('in-progress');
    expect(projectListStatus({ ...base, status: 'in-progress' })).toBe('in-progress');
  });

  it('keeps live/active/running board statuses on the emphasised "live" state', () => {
    expect(projectListStatus({ ...base, status: 'active' })).toBe('live');
    expect(projectListStatus({ ...base, status: 'running' })).toBe('live');
    expect(projectListStatus({ ...base, status: 'live' })).toBe('live');
  });

  it('leaves planned-style statuses as pending (Planned group)', () => {
    expect(projectListStatus({ ...base, status: 'prd_created' })).toBe('pending');
    expect(projectListStatus({ ...base, status: 'exploring' })).toBe('pending');
    expect(projectListStatus({ ...base, status: '' })).toBe('pending');
  });

  it('an in_progress project surfaces under both the Active and In Progress filters', () => {
    const status = projectListStatus({ ...base, status: 'in_progress' });
    expect(matchesStatusFilter(status, 'active')).toBe(true);
    expect(matchesStatusFilter(status, 'in-progress')).toBe(true);
    expect(matchesStatusFilter(status, 'complete')).toBe(false);
  });

  it('archived stays terminal regardless of in-progress aliases', () => {
    expect(projectListStatus({ status: 'archived', storiesComplete: 0, storiesTotal: 5 })).toBe(
      'archived',
    );
  });
});
