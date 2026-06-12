/**
 * Thin adapter over the US-003 Rust commands (`get_local_projects` /
 * `get_local_project_prd`) that normalises their wire shapes into the pure
 * US-004 model types (`Project` / `Story`).
 *
 * The Rust `LocalProject` is camelCase-serialised as
 * `{ id, title, description, company, status, prdPath?, storyCount, storiesComplete }`
 * and `LocalStory.priority` is an optional *string* — the US-004 `Story` type
 * wants `storiesTotal`/`storiesComplete` on the project and a numeric
 * `priority`, so this module owns that coercion in one place. No Svelte runes
 * here — just data mapping, so it stays trivially unit-testable.
 */

import { invoke } from '@tauri-apps/api/core';
import type { Project, Story } from './projects-model';

/** Raw `LocalProject` wire shape from `get_local_projects`. */
export interface LocalProjectWire {
  id: string;
  title: string;
  description?: string;
  company: string;
  status?: string;
  prdPath?: string;
  storyCount: number;
  storiesComplete: number;
}

/** Raw `LocalStory` wire shape (stories inside `get_local_project_prd`). */
export interface LocalStoryWire {
  id: string;
  title: string;
  description?: string;
  acceptanceCriteria?: string[];
  passes?: boolean;
  priority?: string | number | null;
  labels?: string[];
  dependsOn?: string[];
  notes?: string | null;
  files?: string[];
  model_hint?: string | null;
}

/** Raw `LocalProjectPrd` wire shape from `get_local_project_prd`. */
export interface LocalProjectPrdWire {
  name: string;
  description?: string;
  branchName?: string | null;
  userStories?: LocalStoryWire[];
  metadata?: unknown;
}

/** Coerce a string|number priority to the numeric `Story.priority`. */
export function coercePriority(raw: string | number | null | undefined): number | undefined {
  if (raw == null) return undefined;
  if (typeof raw === 'number') return Number.isFinite(raw) ? raw : undefined;
  // Accept "P1", "1", "high"-style strings — pull the first integer if present.
  const match = raw.match(/\d+/);
  if (match) return Number.parseInt(match[0], 10);
  return undefined;
}

/** Map one `LocalProject` wire object into the US-004 `Project` shape. */
export function toProject(wire: LocalProjectWire): Project {
  return {
    id: wire.id,
    title: wire.title,
    name: wire.title,
    description: wire.description ?? '',
    company: wire.company,
    status: wire.status ?? '',
    prdPath: wire.prdPath ?? '',
    storiesTotal: Math.max(0, wire.storyCount ?? 0),
    storiesComplete: Math.max(0, wire.storiesComplete ?? 0),
  };
}

/** Map one `LocalStory` wire object into the US-004 `Story` shape. */
export function toStory(wire: LocalStoryWire): Story {
  return {
    id: wire.id,
    title: wire.title,
    description: wire.description ?? '',
    acceptanceCriteria: wire.acceptanceCriteria ?? [],
    passes: wire.passes ?? false,
    priority: coercePriority(wire.priority),
    labels: wire.labels ?? [],
    dependsOn: wire.dependsOn ?? [],
    notes: wire.notes ?? null,
    files: wire.files ?? [],
    model_hint: wire.model_hint ?? null,
  };
}

/** Load + normalise every local project across companies. */
export async function loadLocalProjects(): Promise<Project[]> {
  const wire = await invoke<LocalProjectWire[]>('get_local_projects');
  return (wire ?? []).map(toProject);
}

// ---------------------------------------------------------------------------
// Company goals (objectives + initiatives) — get_local_company_goals
// ---------------------------------------------------------------------------

/**
 * One key result under an objective. The current board.json data carries
 * `keyResults: []`, so every field is optional (mirrors the permissive Rust
 * `KeyResult`). When populated, `target`/`current` are arbitrary JSON values, so
 * they stay loosely typed here — the card coerces them to numbers for the bar.
 */
export interface KeyResult {
  id?: string;
  title?: string;
  metric?: string;
  target?: number | string | null;
  current?: number | string | null;
  unit?: string;
  status?: string;
}

/** One objective from a company `board.json` `objectives[]` entry. */
export interface Objective {
  id: string;
  title: string;
  description: string;
  status: string;
  timeframe: string;
  owner?: string | null;
  keyResults: KeyResult[];
  initiativeIds: string[];
  linearInitiativeId?: string | null;
}

/** One initiative from a company `board.json` `initiatives[]` entry. */
export interface Initiative {
  id: string;
  title: string;
  description: string;
  status: string;
}

/** A company's GOALS surface, returned by `get_local_company_goals`. */
export interface CompanyGoals {
  objectives: Objective[];
  initiatives: Initiative[];
}

/** Raw wire shape from `get_local_company_goals` (camelCase from serde). */
interface CompanyGoalsWire {
  objectives?: Partial<Objective>[];
  initiatives?: Partial<Initiative>[];
}

/** Normalise one raw objective into the {@link Objective} shape. */
function toObjective(wire: Partial<Objective>): Objective {
  return {
    id: typeof wire.id === 'string' ? wire.id : '',
    title: typeof wire.title === 'string' ? wire.title : '',
    description: typeof wire.description === 'string' ? wire.description : '',
    status: typeof wire.status === 'string' ? wire.status : '',
    timeframe: typeof wire.timeframe === 'string' ? wire.timeframe : '',
    owner: typeof wire.owner === 'string' ? wire.owner : null,
    keyResults: Array.isArray(wire.keyResults) ? wire.keyResults : [],
    initiativeIds: Array.isArray(wire.initiativeIds) ? wire.initiativeIds : [],
    linearInitiativeId:
      typeof wire.linearInitiativeId === 'string' ? wire.linearInitiativeId : null,
  };
}

/** Normalise one raw initiative into the {@link Initiative} shape. */
function toInitiative(wire: Partial<Initiative>): Initiative {
  return {
    id: typeof wire.id === 'string' ? wire.id : '',
    title: typeof wire.title === 'string' ? wire.title : '',
    description: typeof wire.description === 'string' ? wire.description : '',
    status: typeof wire.status === 'string' ? wire.status : '',
  };
}

/**
 * Load + normalise a single company's goals (objectives + initiatives) from its
 * `board.json` via the US-011 Rust command. The command param is `company_slug`;
 * Tauri v2 exposes it camelCased. A company with no board (or an empty goals
 * block) resolves to empty arrays rather than throwing — the caller renders the
 * "No goals yet" empty state.
 */
export async function loadCompanyGoals(slug: string): Promise<CompanyGoals> {
  const wire = await invoke<CompanyGoalsWire>('get_local_company_goals', {
    companySlug: slug,
  });
  return {
    objectives: (wire?.objectives ?? []).map(toObjective),
    initiatives: (wire?.initiatives ?? []).map(toInitiative),
  };
}

/** Load + normalise the stories for a single project's prd.json. */
export async function loadLocalProjectStories(prdPath: string): Promise<Story[]> {
  // The Rust command param is `prd_path`; Tauri v2 exposes it camelCased.
  const prd = await invoke<LocalProjectPrdWire>('get_local_project_prd', { prdPath });
  return (prd?.userStories ?? []).map(toStory);
}

/** Load the raw PRD content needed by the project detail surface. */
export async function loadLocalProjectPrd(prdPath: string): Promise<LocalProjectPrdWire> {
  return invoke<LocalProjectPrdWire>('get_local_project_prd', { prdPath });
}

/**
 * Load a project's sibling README.md by its prd path (US-009). Returns `null`
 * when the project has no README — the Rust command returns `Option<String>`,
 * so a missing README is a normal `null`, not a thrown error.
 */
export async function loadLocalProjectReadme(prdPath: string): Promise<string | null> {
  const content = await invoke<string | null>('get_local_project_readme', { prdPath });
  return content ?? null;
}

/**
 * Persist a project's status to its company `board.json` (US-010). The Rust
 * command params are `board_path` / `project_id` / `status`; Tauri v2 exposes
 * them camelCased. Rejects (throws) on a path-escape, a non-board.json target,
 * an unknown project id, or any write failure — the caller treats a throw as the
 * optimistic-update rollback signal.
 */
export async function saveLocalProjectStatus(
  boardPath: string,
  projectId: string,
  status: string,
): Promise<void> {
  await invoke('set_local_project_status', { boardPath, projectId, status });
}

/**
 * Persist a story's `passes` toggle to the project's prd.json (US-010). Same
 * throw-on-failure contract as {@link saveLocalProjectStatus}.
 */
export async function saveLocalStoryPasses(
  prdPath: string,
  storyId: string,
  passes: boolean,
): Promise<void> {
  await invoke('set_local_story_passes', { prdPath, storyId, passes });
}
