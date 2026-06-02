import {
  saveLocalProjectStatus,
  saveLocalStoryPasses,
} from './local-projects';
import type { Project } from './projects-model';

// ---------------------------------------------------------------------------
// Projects mutation store (US-010) — optimistic local status/passes writes.
//
// Why this exists: the US-009 detail view rendered the status control read-only.
// US-010 makes the board a control center — selecting a status persists to the
// company `board.json` (and a story-passes toggle persists to the project's
// prd.json) through the new Rust write commands. Persistence is slow enough
// (disk read → parse → mutate → atomic rename) that a blocking await would make
// the dropdown feel laggy, so this store follows the stale-while-revalidate
// shape of company-store: it applies the change to an in-memory overlay
// IMMEDIATELY (optimistic paint), fires the write, and on failure ROLLS BACK the
// overlay to the prior value and surfaces a clear error string.
//
// Like company-store these overlays are intentionally NON-reactive plain Maps:
// the page owns its own $state for the rendered value and reads/writes the
// overlay imperatively from its handler, so a background concern can never
// retrigger an effect into a write loop. The store is pure logic + a tiny
// override cache — no Svelte runes — so it stays trivially unit-testable.
// ---------------------------------------------------------------------------

/** Optimistically-applied status overrides, keyed by `${company}:${id}`. */
const statusOverride = new Map<string, string>();
/** Optimistically-applied story-passes overrides, keyed by `${prdPath}:${id}`. */
const passesOverride = new Map<string, boolean>();

function projectKey(company: string, projectId: string): string {
  return `${company}:${projectId}`;
}

function storyKey(prdPath: string, storyId: string): string {
  return `${prdPath}:${storyId}`;
}

/**
 * Derive a project's company `board.json` HQ-relative path. The board lives at
 * `companies/<company>/board.json` — the same tree the readers scan. Returns
 * null when the project has no company (an unlinked prd-only project), in which
 * case there is nothing to persist a board status to.
 */
export function boardPathFor(project: Pick<Project, 'company'>): string | null {
  const company = (project.company ?? '').trim();
  if (!company) return null;
  return `companies/${company}/board.json`;
}

export interface StatusWriteResult {
  ok: boolean;
  /** The value the caller should now show (new on success, previous on rollback). */
  status: string;
  /** A clear, user-facing error string when `ok === false`. */
  error: string | null;
}

/**
 * Optimistically set a project's status and persist it.
 *
 * The caller updates its rendered value to `next` BEFORE awaiting this (the
 * optimistic paint). On success the overlay records `next`; on failure the
 * overlay is rolled back to `previous` and the returned result tells the caller
 * to restore `previous` and show `error`.
 */
export async function setProjectStatus(
  project: Pick<Project, 'id' | 'company'>,
  previous: string,
  next: string,
): Promise<StatusWriteResult> {
  const key = projectKey(project.company, project.id);
  const boardPath = boardPathFor(project);
  if (!boardPath) {
    return {
      ok: false,
      status: previous,
      error: 'This project has no company board to save to.',
    };
  }

  // Optimistic overlay first — a concurrent read sees the new value immediately.
  statusOverride.set(key, next);

  try {
    await saveLocalProjectStatus(boardPath, project.id, next);
    return { ok: true, status: next, error: null };
  } catch (err) {
    // Roll back to the prior value and surface a clear error.
    statusOverride.set(key, previous);
    console.error('set_local_project_status failed:', err);
    return {
      ok: false,
      status: previous,
      error: 'Could not save the status change. Please try again.',
    };
  }
}

export interface PassesWriteResult {
  ok: boolean;
  passes: boolean;
  error: string | null;
}

/** Optimistically toggle a story's `passes` and persist it to the prd.json. */
export async function setStoryPasses(
  prdPath: string,
  storyId: string,
  previous: boolean,
  next: boolean,
): Promise<PassesWriteResult> {
  const key = storyKey(prdPath, storyId);
  passesOverride.set(key, next);
  try {
    await saveLocalStoryPasses(prdPath, storyId, next);
    return { ok: true, passes: next, error: null };
  } catch (err) {
    passesOverride.set(key, previous);
    console.error('set_local_story_passes failed:', err);
    return {
      ok: false,
      passes: previous,
      error: 'Could not save the story change. Please try again.',
    };
  }
}

/** Read surface — the last optimistically-applied overlay, if any. */
export const projectsStore = {
  statusOverride(company: string, projectId: string): string | null {
    return statusOverride.get(projectKey(company, projectId)) ?? null;
  },
  passesOverride(prdPath: string, storyId: string): boolean | null {
    const v = passesOverride.get(storyKey(prdPath, storyId));
    return v === undefined ? null : v;
  },
  setProjectStatus,
  setStoryPasses,
  boardPathFor,
  /** Test-only reset of the overlays between runs. */
  _reset(): void {
    statusOverride.clear();
    passesOverride.clear();
  },
};
