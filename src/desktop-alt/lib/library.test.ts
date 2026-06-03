import { describe, expect, it } from 'vitest';
import {
  filterLibraryItems,
  libraryItemKey,
  toLibraryItems,
  type LibraryItems,
  type LibrarySkill,
  type LibraryWorker,
} from './library';

const worker = (overrides: Partial<LibraryWorker> = {}): LibraryWorker => ({
  id: 'architect',
  name: 'architect',
  type: 'CodeWorker',
  description: 'System design and architecture decisions',
  scope: 'root',
  status: 'active',
  path: 'core/workers/public/dev-team/architect/',
  team: 'dev-team',
  ...overrides,
});

const skill = (overrides: Partial<LibrarySkill> = {}): LibrarySkill => ({
  name: 'signals',
  description: 'Surface action items from meetings',
  scope: 'company',
  company: 'indigo',
  path: 'companies/indigo/skills/signals/SKILL.md',
  allowedTools: [],
  ...overrides,
});

describe('toLibraryItems', () => {
  it('flattens workers then skills into a discriminated list', () => {
    const items: LibraryItems = {
      workers: [worker()],
      skills: [skill()],
    };
    const flat = toLibraryItems(items);
    expect(flat).toHaveLength(2);
    expect(flat[0].kind).toBe('worker');
    expect(flat[1].kind).toBe('skill');
  });

  it('tolerates missing arrays', () => {
    // @ts-expect-error — exercising the defensive `?? []` paths.
    expect(toLibraryItems({})).toEqual([]);
  });
});

describe('libraryItemKey', () => {
  it('namespaces worker vs skill keys by path', () => {
    expect(libraryItemKey({ kind: 'worker', worker: worker() })).toBe(
      'worker:core/workers/public/dev-team/architect/',
    );
    expect(libraryItemKey({ kind: 'skill', skill: skill() })).toBe(
      'skill:companies/indigo/skills/signals/SKILL.md',
    );
  });
});

describe('filterLibraryItems', () => {
  const flat = toLibraryItems({
    workers: [worker(), worker({ id: 'backend-dev', name: 'backend-dev', description: 'APIs', type: 'CodeWorker', path: 'core/workers/public/dev-team/backend-dev/' })],
    skills: [skill()],
  });

  it('returns everything for an empty query', () => {
    expect(filterLibraryItems(flat, '')).toHaveLength(3);
    expect(filterLibraryItems(flat, '   ')).toHaveLength(3);
  });

  it('matches name, description, type and scope case-insensitively', () => {
    expect(filterLibraryItems(flat, 'architect').map((i) => libraryItemKey(i))).toEqual([
      'worker:core/workers/public/dev-team/architect/',
    ]);
    // description match on the skill
    expect(filterLibraryItems(flat, 'action items')).toHaveLength(1);
    // scope match
    expect(filterLibraryItems(flat, 'indigo')).toHaveLength(1);
    // type match hits both CodeWorkers
    expect(filterLibraryItems(flat, 'codeworker')).toHaveLength(2);
  });

  it('returns nothing when no item matches', () => {
    expect(filterLibraryItems(flat, 'zzz-nope')).toHaveLength(0);
  });
});
