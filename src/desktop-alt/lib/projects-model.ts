/**
 * Pure story/project classification logic for the desktop-alt projects view.
 *
 * Ported from hq-desktop's prd-utils.ts / board-utils.ts / kanban-board.tsx,
 * with the indigo Tailwind label palette replaced by HQ Sync's monochrome-glass
 * token system. These are pure functions — no Svelte runes, no Tauri imports —
 * so they are trivially unit-testable.
 */

// ---------------------------------------------------------------------------
// Types (match the US-003 Rust command output shape)
// ---------------------------------------------------------------------------

/** A single user story, as surfaced by the get_company_projects Rust command. */
export interface Story {
  /** Unique story identifier (e.g. "US-001"). */
  id: string;
  /** Short title describing the story. */
  title: string;
  /** Full description of the story. */
  description: string;
  /** List of acceptance criteria. */
  acceptanceCriteria: string[];
  /** Whether all acceptance criteria pass (story is complete). */
  passes: boolean;
  /** Priority level (1 = highest). Optional. */
  priority?: number;
  /** Labels for categorization. */
  labels: string[];
  /** IDs of stories this one depends on. */
  dependsOn: string[];
}

/** A project, as surfaced by the get_company_projects Rust command. */
export interface Project {
  /** Unique project identifier (usually the directory name). */
  id: string;
  /** Display name. Either `name` or `title` may be populated upstream. */
  name?: string;
  /** Alternate display name field. */
  title?: string;
  /** Project description. */
  description: string;
  /** Owning company slug. */
  company: string;
  /** Raw status from board.json (e.g. "active", "archived", "planned"). */
  status: string;
  /** Absolute or HQ-relative path to the prd.json file. */
  prdPath: string;
  /** Total number of stories in the PRD. */
  storiesTotal: number;
  /** Number of stories that pass. */
  storiesComplete: number;
}

// ---------------------------------------------------------------------------
// Story state classification (mirrors prd-utils.classifyStories)
// ---------------------------------------------------------------------------

/** Kanban column state for a user story. */
export type StoryState = 'complete' | 'blocked' | 'in-progress' | 'pending';

/** A user story enriched with its derived kanban state. */
export interface ClassifiedStory {
  story: Story;
  state: StoryState;
}

/** All kanban states, in display order. */
export const STORY_STATES: StoryState[] = [
  'pending',
  'blocked',
  'in-progress',
  'complete',
];

/**
 * Classify all stories into kanban column states.
 *
 * Classification logic (matches hq-desktop's prd-utils):
 * - complete:    passes === true
 * - blocked:     passes === false AND at least one dependency is not complete
 * - in-progress: the FIRST eligible (deps met, not complete) story
 * - pending:     all remaining eligible stories
 */
export function classifyStories(stories: Story[]): ClassifiedStory[] {
  const completedIds = new Set(stories.filter((s) => s.passes).map((s) => s.id));

  const classified: ClassifiedStory[] = [];
  let inProgressAssigned = false;

  for (const story of stories) {
    if (story.passes) {
      classified.push({ story, state: 'complete' });
      continue;
    }

    const deps = story.dependsOn ?? [];
    const hasUnmetDeps = deps.some((depId) => !completedIds.has(depId));

    if (hasUnmetDeps) {
      classified.push({ story, state: 'blocked' });
      continue;
    }

    if (!inProgressAssigned) {
      classified.push({ story, state: 'in-progress' });
      inProgressAssigned = true;
    } else {
      classified.push({ story, state: 'pending' });
    }
  }

  return classified;
}

/**
 * Classify a single story against the full story set (needed for dependency
 * resolution). The `isFirstEligible` flag distinguishes in-progress from
 * pending — callers that don't care can pass `false` and treat any eligible
 * story as pending.
 */
export function classifyStory(
  story: Story,
  allStories: Story[],
  isFirstEligible = false,
): StoryState {
  if (story.passes) return 'complete';

  const completedIds = new Set(
    allStories.filter((s) => s.passes).map((s) => s.id),
  );
  const deps = story.dependsOn ?? [];
  const hasUnmetDeps = deps.some((depId) => !completedIds.has(depId));
  if (hasUnmetDeps) return 'blocked';

  return isFirstEligible ? 'in-progress' : 'pending';
}

/** Group classified stories by their kanban state. */
export function groupByState(
  classified: ClassifiedStory[],
): Record<StoryState, ClassifiedStory[]> {
  const groups: Record<StoryState, ClassifiedStory[]> = {
    pending: [],
    blocked: [],
    'in-progress': [],
    complete: [],
  };
  for (const item of classified) {
    groups[item.state].push(item);
  }
  return groups;
}

// ---------------------------------------------------------------------------
// Deterministic label color (monochrome-glass adaptation of kanban-board.tsx)
// ---------------------------------------------------------------------------

/**
 * A label chip's resolved color, expressed against HQ Sync's monochrome-glass
 * identity. Rather than the indigo Tailwind palette used in hq-desktop, we map
 * each label to one entry of a small neutral/translucent palette plus a stable
 * index. All values are CSS-var-friendly translucent monochrome tones with a
 * single controlled-saturation hue, preserving the monochrome look while still
 * giving each label a stable, distinguishable shade.
 */
export interface LabelColor {
  /** Stable palette index (0..LABEL_PALETTE_SIZE-1). */
  index: number;
  /** Translucent background fill (CSS color, monochrome with low saturation). */
  background: string;
  /** Border color (slightly stronger than the fill). */
  border: string;
  /** Foreground/text color. */
  foreground: string;
}

/** Number of distinct monochrome shades a label can resolve to. */
export const LABEL_PALETTE_SIZE = 8;

/**
 * The monochrome-glass label palette. Each entry is a hsla() built around a
 * single near-neutral hue (210, a cool slate) with a deliberately low, fixed
 * saturation so the chips read as monochrome glass rather than a rainbow. Only
 * lightness/alpha vary across the palette, which is what makes adjacent labels
 * distinguishable without breaking the monochrome identity.
 */
export const LABEL_PALETTE: LabelColor[] = Array.from(
  { length: LABEL_PALETTE_SIZE },
  (_, i): LabelColor => {
    // Lightness sweeps 58%..86% across the palette; saturation stays low (12%).
    const lightness = 58 + i * 4;
    return {
      index: i,
      background: `hsla(210, 12%, ${lightness}%, 0.12)`,
      border: `hsla(210, 12%, ${lightness}%, 0.24)`,
      foreground: `hsla(210, 14%, ${lightness}%, 0.82)`,
    };
  },
);

/**
 * Deterministically hash a label string to a stable palette index.
 *
 * Uses the same `hash = (hash * 31 + charCode) | 0` rolling hash as
 * hq-desktop's kanban-board.tsx, so the mapping is stable and well-distributed.
 * Same input string always yields the same index.
 */
export function labelColorIndex(label: string): number {
  let hash = 0;
  for (let i = 0; i < label.length; i++) {
    hash = (hash * 31 + label.charCodeAt(i)) | 0;
  }
  return Math.abs(hash) % LABEL_PALETTE_SIZE;
}

/**
 * Resolve a label string to its deterministic monochrome-glass color.
 * Same string → identical LabelColor every time.
 */
export function labelColor(label: string): LabelColor {
  return LABEL_PALETTE[labelColorIndex(label)];
}

// ---------------------------------------------------------------------------
// Project progress + effective status (mirrors board-utils / prd-types)
// ---------------------------------------------------------------------------

/** Derived project-level rollup state. */
export type ProjectState = 'complete' | 'in-progress' | 'pending';

/** Project progress derived from prd.json story completion. */
export interface ProjectProgress {
  /** Number of complete stories. */
  complete: number;
  /** Total number of stories. */
  total: number;
  /** Completion percentage, 0–100 (0 when there are no stories). */
  percent: number;
  /** Rollup state derived from story completion. */
  state: ProjectState;
}

/**
 * Derive the project rollup state from story completion counts.
 *
 * - complete:    every story passes (and there is at least one)
 * - in-progress: some — but not all — stories pass
 * - pending:     no stories pass (or there are no stories)
 */
export function deriveProjectState(
  complete: number,
  total: number,
): ProjectState {
  if (total === 0) return 'pending';
  if (complete >= total) return 'complete';
  if (complete > 0) return 'in-progress';
  return 'pending';
}

/**
 * Compute project progress from explicit complete/total counts (as carried on
 * the Project shape from the US-003 Rust command).
 */
export function projectProgress(
  storiesComplete: number,
  storiesTotal: number,
): ProjectProgress {
  const total = Math.max(0, storiesTotal);
  const complete = Math.max(0, Math.min(storiesComplete, total));
  const percent = total === 0 ? 0 : Math.round((complete / total) * 100);
  return {
    complete,
    total,
    percent,
    state: deriveProjectState(complete, total),
  };
}

/**
 * Compute project progress directly from a story list (when the raw stories,
 * rather than precomputed counts, are available).
 */
export function projectProgressFromStories(stories: Story[]): ProjectProgress {
  const total = stories.length;
  const complete = stories.filter((s) => s.passes).length;
  return projectProgress(complete, total);
}

/**
 * Derive an effective, display-ready project status by combining the raw
 * board.json `status` with the prd.json story rollup.
 *
 * Rules (mirroring how board-utils treats archived projects as terminal):
 * - An "archived" board status is terminal and always wins.
 * - Otherwise the story rollup drives the effective status, so a board marked
 *   "active" but with every story passing reads as "complete", and one with no
 *   passing stories reads as "pending".
 */
export function effectiveProjectStatus(
  project: Pick<Project, 'status' | 'storiesComplete' | 'storiesTotal'>,
): ProjectState | 'archived' {
  if (project.status === 'archived') return 'archived';
  return deriveProjectState(project.storiesComplete, project.storiesTotal);
}

/** Best-effort display name for a project (`name` wins, then `title`, then id). */
export function projectDisplayName(project: Project): string {
  return project.name ?? project.title ?? project.id;
}
