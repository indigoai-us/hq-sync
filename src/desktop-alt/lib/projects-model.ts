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
  /** Optional implementation notes from prd.json. */
  notes?: string | null;
  /** Optional declared files from prd.json. */
  files?: string[];
  /** Optional model hint from prd.json. */
  model_hint?: string | null;
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

/** Number of distinct hues a label can resolve to. */
export const LABEL_PALETTE_SIZE = 8;

/**
 * The label palette — a low-saturation 8-hue set mirroring hq-desktop's
 * blue/purple/teal/pink/orange/cyan/lime/rose chips, tuned for the dark desktop
 * surface (translucent fill + matching border + a brighter readable foreground).
 * Hues are spaced around the wheel so adjacent labels stay distinguishable. The
 * deterministic hash (below) maps each label string to a stable entry.
 */
const LABEL_HUES = [217, 270, 173, 330, 25, 190, 84, 350];

export const LABEL_PALETTE: LabelColor[] = LABEL_HUES.map(
  (hue, i): LabelColor => ({
    index: i,
    background: `hsla(${hue}, 65%, 58%, 0.15)`,
    border: `hsla(${hue}, 65%, 60%, 0.30)`,
    foreground: `hsla(${hue}, 70%, 74%, 0.92)`,
  }),
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

// ---------------------------------------------------------------------------
// Board list-surface helpers (US-007 — ported from hq-desktop project-types)
// ---------------------------------------------------------------------------

/**
 * The "effective" status a project resolves to for the Board list surface. This
 * is the union of {@link effectiveProjectStatus}'s output, plus the synthetic
 * `live` state used to give actively-running projects visual emphasis.
 *
 * - `live`        — board status is "active"/"live" AND there is in-flight work
 *                   (some but not all stories complete). Gets the glow + pulse.
 * - `in-progress` — story rollup says work is underway, but the board isn't live.
 * - `complete`    — every story passes.
 * - `pending`     — no stories pass yet (planned / not started).
 * - `archived`    — board status is archived (terminal).
 */
export type ProjectListStatus =
  | 'live'
  | 'in-progress'
  | 'complete'
  | 'pending'
  | 'archived';

/** The status-filter pills shown above the Board project list. */
export type StatusFilter = 'all' | 'active' | 'in-progress' | 'complete' | 'archived';

/** How the Board project list groups its rows. */
export type ProjectGroupMode = 'status' | 'company';

/** The status-filter pill options, in display order. */
export const STATUS_FILTER_OPTIONS: { value: StatusFilter; label: string }[] = [
  { value: 'all', label: 'All' },
  { value: 'active', label: 'Active' },
  { value: 'in-progress', label: 'In Progress' },
  { value: 'complete', label: 'Completed' },
  { value: 'archived', label: 'Archived' },
];

/** Raw board statuses that indicate a project is live/running (earns the
 * pulsing "live" emphasis). */
const LIVE_BOARD_STATUSES = new Set(['live', 'active', 'running']);

/**
 * Raw board statuses that mark a project as actively in progress. HQ's
 * `board.json` uses `in_progress` (snake_case) for projects under active work —
 * distinct from the `live`/`active`/`running` set above that earns the pulsing
 * "live" emphasis. A project carrying this status reads as In Progress even
 * before its first story completes, so a project the user has explicitly
 * started doesn't get mis-grouped under "Planned".
 */
const IN_PROGRESS_BOARD_STATUSES = new Set([
  'in_progress',
  'in-progress',
  'inprogress',
]);

/**
 * Resolve a project's effective Board-list status by combining its raw board
 * `status` with the prd.json story rollup. Mirrors hq-desktop's board scanner:
 * an archived board status is terminal; a live board status with in-flight work
 * surfaces as `live` (emphasised); otherwise the story rollup drives it.
 */
export function projectListStatus(
  project: Pick<Project, 'status' | 'storiesComplete' | 'storiesTotal'>,
): ProjectListStatus {
  const rollup = effectiveProjectStatus(project);
  if (rollup === 'archived') return 'archived';
  if (rollup === 'complete') return 'complete';

  const raw = (project.status ?? '').toLowerCase();
  const isLiveBoard = LIVE_BOARD_STATUSES.has(raw);
  const isInProgressBoard = IN_PROGRESS_BOARD_STATUSES.has(raw);
  if (rollup === 'in-progress') {
    return isLiveBoard ? 'live' : 'in-progress';
  }
  // pending rollup (no completed stories yet): a live board still earns `live`
  // emphasis; a board the user explicitly marked in-progress reads as
  // in-progress so it surfaces under "In Progress"; otherwise it's planned.
  if (isLiveBoard) return 'live';
  if (isInProgressBoard) return 'in-progress';
  return 'pending';
}

/** Whether a project's effective list status passes the given filter pill. */
export function matchesStatusFilter(
  status: ProjectListStatus,
  filter: StatusFilter,
): boolean {
  switch (filter) {
    case 'all':
      return true;
    case 'active':
      // "Active" hides the terminal states (completed + archived).
      return status === 'live' || status === 'in-progress' || status === 'pending';
    case 'in-progress':
      return status === 'live' || status === 'in-progress';
    case 'complete':
      return status === 'complete';
    case 'archived':
      return status === 'archived';
    default:
      return true;
  }
}

/** Display order for the status-grouped Board sections (lower sorts first). */
export const PROJECT_LIST_STATUS_ORDER: Record<ProjectListStatus, number> = {
  live: 0,
  'in-progress': 1,
  pending: 2,
  complete: 3,
  archived: 4,
};

/** Human label for each effective list status (section headers + badges). */
export const PROJECT_LIST_STATUS_LABEL: Record<ProjectListStatus, string> = {
  live: 'Running',
  'in-progress': 'In Progress',
  pending: 'Planned',
  complete: 'Completed',
  archived: 'Archived',
};

/** Does a project's effective status text match a free-text query token? */
function projectStatusMatches(project: Project, query: string): boolean {
  return PROJECT_LIST_STATUS_LABEL[projectListStatus(project)]
    .toLowerCase()
    .includes(query);
}

/**
 * Filter a project list by a free-text query, matching the title, description,
 * id, and company slug (case-insensitive). An empty/whitespace query is a no-op.
 */
export function filterProjectsByQuery(projects: Project[], rawQuery: string): Project[] {
  const query = rawQuery.toLowerCase().trim();
  if (!query) return projects;
  return projects.filter((project) => {
    const name = projectDisplayName(project).toLowerCase();
    return (
      name.includes(query) ||
      (project.description ?? '').toLowerCase().includes(query) ||
      project.id.toLowerCase().includes(query) ||
      project.company.toLowerCase().includes(query) ||
      projectStatusMatches(project, query)
    );
  });
}

/** A grouped section of projects for the Board list (status- or company-keyed). */
export interface ProjectSection {
  /** Stable section key (status id or company slug). */
  key: string;
  /** Display label for the section header. */
  label: string;
  /** Projects in this section, pre-sorted for display. */
  projects: Project[];
}

/** Sort comparator: live first, then by status order, then by name. */
function compareProjects(a: Project, b: Project): number {
  const aStatus = PROJECT_LIST_STATUS_ORDER[projectListStatus(a)] ?? 99;
  const bStatus = PROJECT_LIST_STATUS_ORDER[projectListStatus(b)] ?? 99;
  return (
    aStatus - bStatus ||
    projectDisplayName(a).localeCompare(projectDisplayName(b))
  );
}

/**
 * Group + sort a project list into display sections.
 *
 * - `status`  — one section per effective list status, in status order, empty
 *               sections omitted.
 * - `company` — one section per company slug, alphabetical, projects within a
 *               section sorted live-first then by name.
 */
export function groupProjects(
  projects: Project[],
  mode: ProjectGroupMode,
): ProjectSection[] {
  if (mode === 'status') {
    const buckets = new Map<ProjectListStatus, Project[]>();
    for (const project of projects) {
      const status = projectListStatus(project);
      const list = buckets.get(status) ?? [];
      list.push(project);
      buckets.set(status, list);
    }
    return (Object.keys(PROJECT_LIST_STATUS_ORDER) as ProjectListStatus[])
      .sort(
        (a, b) => PROJECT_LIST_STATUS_ORDER[a] - PROJECT_LIST_STATUS_ORDER[b],
      )
      .filter((status) => (buckets.get(status)?.length ?? 0) > 0)
      .map((status) => ({
        key: status,
        label: PROJECT_LIST_STATUS_LABEL[status],
        projects: (buckets.get(status) ?? []).slice().sort(compareProjects),
      }));
  }

  const buckets = new Map<string, Project[]>();
  for (const project of projects) {
    const key = project.company || 'hq';
    const list = buckets.get(key) ?? [];
    list.push(project);
    buckets.set(key, list);
  }
  return [...buckets.keys()]
    .sort((a, b) => a.localeCompare(b))
    .map((key) => ({
      key,
      label: key,
      projects: (buckets.get(key) ?? []).slice().sort(compareProjects),
    }));
}

// ---------------------------------------------------------------------------
// Editable project statuses (US-009 detail-view status control)
// ---------------------------------------------------------------------------

/**
 * The statuses a user may manually assign to a project, in display order
 * (ported from hq-desktop's EDITABLE_STATUSES). Excludes the synthetic `live`
 * state — that is set by the orchestrator, not the user. The detail-view status
 * control (US-009) renders these read-only for now; persisting a change is
 * wired in US-010.
 */
export const EDITABLE_PROJECT_STATUSES = [
  'planned',
  'prd_created',
  'in_progress',
  'completed',
  'archived',
] as const;

/** One of the user-assignable project statuses. */
export type EditableProjectStatus = (typeof EDITABLE_PROJECT_STATUSES)[number];

/** Human-readable label for each editable project status. */
export const EDITABLE_PROJECT_STATUS_LABEL: Record<EditableProjectStatus, string> = {
  planned: 'Planned',
  prd_created: 'PRD Created',
  in_progress: 'In Progress',
  completed: 'Completed',
  archived: 'Archived',
};

/**
 * Resolve a project's raw board `status` to the editable-status enum used by the
 * detail-view control. Raw board statuses are messy (`active`, `live`,
 * `running`, `complete`, …); this maps them onto the canonical editable set so
 * the control always has a defined current value. Unknown statuses fall back to
 * `planned`.
 */
export function toEditableStatus(rawStatus: string | undefined | null): EditableProjectStatus {
  const s = (rawStatus ?? '').toLowerCase().trim();
  switch (s) {
    case 'planned':
    case 'pending':
    case '':
      return 'planned';
    case 'prd_created':
    case 'prd':
      return 'prd_created';
    case 'in_progress':
    case 'in-progress':
    case 'active':
    case 'live':
    case 'running':
      return 'in_progress';
    case 'completed':
    case 'complete':
    case 'done':
      return 'completed';
    case 'archived':
      return 'archived';
    default:
      return 'planned';
  }
}

/** Distinct company slugs present in a project list, alphabetical. */
export function projectCompanies(projects: Project[]): string[] {
  return [...new Set(projects.map((project) => project.company).filter(Boolean))].sort(
    (a, b) => a.localeCompare(b),
  );
}
