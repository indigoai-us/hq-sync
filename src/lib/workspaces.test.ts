import { describe, expect, it } from 'vitest';
import { dedupeWorkspaces, type Workspace } from './workspaces';

const base: Workspace = {
  slug: 'acme',
  displayName: 'Acme',
  kind: 'company',
  state: 'synced',
  cloudUid: 'cmp_1',
  bucketName: 'bucket',
  hasLocalFolder: true,
  localPath: '/tmp/HQ/companies/acme',
  membershipStatus: 'active',
  role: 'owner',
  lastSyncedAt: null,
  brokenReason: null,
  invitedBy: null,
  invitedAt: null,
};

const ws = (overrides: Partial<Workspace>): Workspace => ({ ...base, ...overrides });

describe('dedupeWorkspaces', () => {
  it('collapses a company present in both manifest and cloud to one row', () => {
    // The exact shape that froze the Companies page / popover: the same
    // company returned twice by the manifest+cloud union.
    const result = dedupeWorkspaces([
      ws({ slug: 'liverecover', displayName: 'Liverecover' }),
      ws({ slug: 'liverecover', displayName: 'Liverecover', state: 'cloud-only' }),
      ws({ slug: 'indigo', displayName: 'Indigo' }),
    ]);

    expect(result.map((w) => w.slug)).toEqual(['liverecover', 'indigo']);
  });

  it('keeps the first occurrence (preserves backend ordering of the survivor)', () => {
    const result = dedupeWorkspaces([
      ws({ slug: 'dup', displayName: 'First', role: 'owner' }),
      ws({ slug: 'dup', displayName: 'Second', role: 'member' }),
    ]);

    expect(result).toHaveLength(1);
    expect(result[0].displayName).toBe('First');
    expect(result[0].role).toBe('owner');
  });

  it('treats a personal and a company sharing a slug as distinct (key is kind:slug)', () => {
    const result = dedupeWorkspaces([
      ws({ slug: 'personal', kind: 'personal', state: 'personal' }),
      ws({ slug: 'personal', kind: 'company', state: 'synced' }),
    ]);

    expect(result).toHaveLength(2);
  });

  it('produces collision-free `kind:slug` keys for a keyed each block', () => {
    const result = dedupeWorkspaces([
      ws({ slug: 'a' }),
      ws({ slug: 'a' }),
      ws({ slug: 'b' }),
      ws({ slug: 'b', kind: 'personal' }),
    ]);

    const keys = result.map((w) => `${w.kind}:${w.slug}`);
    expect(new Set(keys).size).toBe(keys.length);
  });

  it('does not mutate the input array', () => {
    const input = [ws({ slug: 'x' }), ws({ slug: 'x' })];
    const snapshotLength = input.length;
    dedupeWorkspaces(input);
    expect(input).toHaveLength(snapshotLength);
  });
});
