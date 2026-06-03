/**
 * Thin adapter over the Library Rust commands (`get_library_root` /
 * `get_library_company` / `get_library_worker_detail` /
 * `get_library_skill_detail`).
 *
 * The Rust structs are camelCase-serialised, so the wire shapes map almost 1:1
 * to these TS types. This module owns the `invoke()` calls and a small
 * discriminated `LibraryItem` union so the list + detail components can treat
 * workers and skills uniformly. No Svelte runes here — just data, so it stays
 * trivially unit-testable.
 */

import { invoke } from '@tauri-apps/api/core';

/** A worker row (`LibraryWorker` wire shape). */
export interface LibraryWorker {
  id: string;
  name: string;
  type: string;
  description: string;
  scope: 'root' | 'company';
  company?: string;
  status: string;
  path: string;
  team?: string | null;
}

/** A skill row (`LibrarySkill` wire shape). */
export interface LibrarySkill {
  name: string;
  description: string;
  scope: 'root' | 'personal' | 'company';
  company?: string;
  path: string;
  allowedTools: string[];
}

/** Combined library payload for one scope. */
export interface LibraryItems {
  workers: LibraryWorker[];
  skills: LibrarySkill[];
}

/** One named skill reference inside a worker's detail. */
export interface WorkerSkillRef {
  name: string;
  description?: string | null;
}

/** Full worker detail (`get_library_worker_detail`). */
export interface WorkerDetail {
  id: string;
  name: string;
  type: string;
  description: string;
  team?: string | null;
  skills: WorkerSkillRef[];
  instructions: string;
}

/** Full skill detail (`get_library_skill_detail`). */
export interface SkillDetail {
  name: string;
  description: string;
  allowedTools: string[];
  body: string;
}

/**
 * A unified list item. The list + detail panel switch on `kind`; both variants
 * carry the underlying row (with its `path` for the lazy detail load).
 */
export type LibraryItem =
  | { kind: 'worker'; worker: LibraryWorker }
  | { kind: 'skill'; skill: LibrarySkill };

/** Flatten a {@link LibraryItems} payload into a single ordered item list. */
export function toLibraryItems(items: LibraryItems): LibraryItem[] {
  const workers: LibraryItem[] = (items.workers ?? []).map((worker) => ({
    kind: 'worker',
    worker,
  }));
  const skills: LibraryItem[] = (items.skills ?? []).map((skill) => ({
    kind: 'skill',
    skill,
  }));
  return [...workers, ...skills];
}

/** Stable key + searchable text for a list item. */
export function libraryItemKey(item: LibraryItem): string {
  return item.kind === 'worker'
    ? `worker:${item.worker.path}`
    : `skill:${item.skill.path}`;
}

export function libraryItemName(item: LibraryItem): string {
  return item.kind === 'worker' ? item.worker.name : item.skill.name;
}

export function libraryItemDescription(item: LibraryItem): string {
  return item.kind === 'worker'
    ? item.worker.description
    : item.skill.description;
}

/** Lowercased haystack for the text filter (name + description + scope/type). */
export function libraryItemHaystack(item: LibraryItem): string {
  if (item.kind === 'worker') {
    return [
      item.worker.name,
      item.worker.description,
      item.worker.type,
      item.worker.team ?? '',
      item.worker.scope,
      item.worker.company ?? '',
    ]
      .join(' ')
      .toLowerCase();
  }
  return [
    item.skill.name,
    item.skill.description,
    item.skill.scope,
    item.skill.company ?? '',
  ]
    .join(' ')
    .toLowerCase();
}

/** Filter items by a free-text query (matches name/description/type/scope). */
export function filterLibraryItems(items: LibraryItem[], query: string): LibraryItem[] {
  const q = query.trim().toLowerCase();
  if (q === '') return items;
  return items.filter((item) => libraryItemHaystack(item).includes(q));
}

// ---- loaders ---------------------------------------------------------------

/** Load the root/shared library (public workers + root + personal skills). */
export async function loadLibraryRoot(): Promise<LibraryItems> {
  const wire = await invoke<LibraryItems>('get_library_root');
  return { workers: wire?.workers ?? [], skills: wire?.skills ?? [] };
}

/** Load a single company's library (its private workers + company skills). */
export async function loadLibraryCompany(slug: string): Promise<LibraryItems> {
  // The Rust command param is `company_slug`; Tauri v2 exposes it camelCased.
  const wire = await invoke<LibraryItems>('get_library_company', { companySlug: slug });
  return { workers: wire?.workers ?? [], skills: wire?.skills ?? [] };
}

/** Load a worker's full detail by its HQ-relative directory path. */
export async function loadWorkerDetail(workerPath: string): Promise<WorkerDetail> {
  const wire = await invoke<WorkerDetail>('get_library_worker_detail', { workerPath });
  return {
    id: wire?.id ?? '',
    name: wire?.name ?? '',
    type: wire?.type ?? '',
    description: wire?.description ?? '',
    team: wire?.team ?? null,
    skills: wire?.skills ?? [],
    instructions: wire?.instructions ?? '',
  };
}

/** Load a skill's full detail by its HQ-relative SKILL.md path. */
export async function loadSkillDetail(skillPath: string): Promise<SkillDetail> {
  const wire = await invoke<SkillDetail>('get_library_skill_detail', { skillPath });
  return {
    name: wire?.name ?? '',
    description: wire?.description ?? '',
    allowedTools: wire?.allowedTools ?? [],
    body: wire?.body ?? '',
  };
}
