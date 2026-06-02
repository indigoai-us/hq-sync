import { describe, expect, it } from 'vitest';
import {
  classifyStories,
  groupByState,
  STORY_STATES,
  type Story,
  type StoryState,
} from '../../src/desktop-alt/lib/projects-model';
import { readRepoFile } from './harness';

/**
 * US-006 — Story Kanban + List toggle (the restyled board).
 *
 * The desktop-alt harness is a source-contract (non-render) harness, so this
 * spec asserts at two levels, matching the existing style:
 *   1. Logic level — the same classifier the board uses (classifyStories /
 *      groupByState, US-004) places a fixture story set into the right columns.
 *   2. Source contract — StoryKanban.svelte / StoryList.svelte wire the four
 *      columns, the Board/List toggle, StoryCard, the onselect callback, and the
 *      empty + loading states.
 */

// A fixture story set that exercises every column. Order matters for the
// classifier: the first eligible (deps met, not passing) story is in-progress;
// remaining eligible stories are pending; unmet deps are blocked.
const FIXTURE_STORIES: Story[] = [
  {
    id: 'US-001',
    title: 'Foundational schema',
    description: '',
    acceptanceCriteria: ['a', 'b'],
    passes: true,
    priority: 1,
    labels: ['backend'],
    dependsOn: [],
  },
  {
    id: 'US-002',
    title: 'Wire up the API',
    description: '',
    acceptanceCriteria: ['a', 'b', 'c'],
    passes: false,
    priority: 2,
    labels: ['backend', 'api'],
    dependsOn: ['US-001'],
  },
  {
    id: 'US-003',
    title: 'Build the UI shell',
    description: '',
    acceptanceCriteria: ['a'],
    passes: false,
    priority: 2,
    labels: ['frontend'],
    dependsOn: ['US-001'],
  },
  {
    id: 'US-004',
    title: 'Ship the polished board',
    description: '',
    acceptanceCriteria: ['a', 'b'],
    passes: false,
    priority: 3,
    labels: ['frontend', 'polish'],
    dependsOn: ['US-099'], // unmet dependency → blocked
  },
];

describe('desktop-alt story board (US-006)', () => {
  it('classifies the fixture story set into the four kanban columns', () => {
    const grouped = groupByState(classifyStories(FIXTURE_STORIES));

    const idsIn = (state: StoryState) => grouped[state].map((c) => c.story.id);

    expect(idsIn('complete')).toEqual(['US-001']);
    expect(idsIn('in-progress')).toEqual(['US-002']); // first eligible
    expect(idsIn('pending')).toEqual(['US-003']); // remaining eligible
    expect(idsIn('blocked')).toEqual(['US-004']); // unmet dependency

    // Every fixture story lands in exactly one column.
    const placed = STORY_STATES.flatMap((s) => idsIn(s));
    expect(placed.sort()).toEqual(['US-001', 'US-002', 'US-003', 'US-004']);
  });

  it('keeps the list view over the same set in source order', () => {
    // StoryList renders classifyStories(stories) in the original story order,
    // so the list and the board are the same story set, just laid out
    // differently — the toggle never changes which stories are shown.
    const classified = classifyStories(FIXTURE_STORIES);
    expect(classified.map((c) => c.story.id)).toEqual([
      'US-001',
      'US-002',
      'US-003',
      'US-004',
    ]);
  });

  it('renders an empty board when there are no stories', () => {
    const grouped = groupByState(classifyStories([]));
    for (const state of STORY_STATES) {
      expect(grouped[state]).toEqual([]);
    }
  });

  it('wires four collapsible columns, the toggle, StoryCard, and onselect', () => {
    const kanban = readRepoFile('src/desktop-alt/components/StoryKanban.svelte');

    // Reuses US-004's classifier + grouping rather than re-deriving state.
    expect(kanban).toContain('classifyStories');
    expect(kanban).toContain('groupByState');
    expect(kanban).toContain('STORY_STATES');

    // Four named columns.
    for (const label of ['Pending', 'Blocked', 'In Progress', 'Complete']) {
      expect(kanban).toContain(label);
    }

    // Collapsible columns (chevron toggles a per-column collapse map).
    expect(kanban).toContain('toggleColumn');
    expect(kanban).toContain('collapsed');
    expect(kanban).toContain('count-badge');
    expect(kanban).toContain('status-dot');

    // Board/List segmented toggle.
    expect(kanban).toContain('data-testid="view-toggle-board"');
    expect(kanban).toContain('data-testid="view-toggle-list"');
    expect(kanban).toContain("viewMode = 'board'");
    expect(kanban).toContain("viewMode = 'list'");

    // Cards are US-005 StoryCard; rows are US-006 StoryList; onselect threads through.
    expect(kanban).toContain('import StoryCard');
    expect(kanban).toContain('import StoryList');
    expect(kanban).toContain('<StoryCard story={item.story} {onselect}');
    expect(kanban).toContain('<StoryList {stories} {onselect}');

    // Empty + loading states.
    expect(kanban).toContain('No stories');
    expect(kanban).toContain('loading');
    expect(kanban).toContain('skeleton-card');
  });

  it('renders the list rows with id, state badge, priority, AC count, and onselect', () => {
    const list = readRepoFile('src/desktop-alt/components/StoryList.svelte');

    expect(list).toContain('classifyStories');
    expect(list).toContain('import LabelChip');
    expect(list).toContain('story-id');
    expect(list).toContain('state-badge');
    expect(list).toContain('priority-badge');
    expect(list).toContain('ac-count');
    expect(list).toContain('onselect?.(story)');
    expect(list).toContain('No stories'); // empty state
  });

  it('keeps the board token-driven (no hardcoded hex) for light + dark', () => {
    for (const path of [
      'src/desktop-alt/components/StoryKanban.svelte',
      'src/desktop-alt/components/StoryList.svelte',
    ]) {
      const styleBlock = readRepoFile(path).split('<style>')[1] ?? '';
      // No 3/6-digit hex literals anywhere in the component styles — every color
      // resolves from a token, so light + dark both work.
      expect(styleBlock).not.toMatch(/#[0-9a-fA-F]{3,8}\b/);
    }
  });
});
