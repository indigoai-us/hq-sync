import { describe, expect, it } from 'vitest';
import {
  dedupeWorkspaces,
  joinableMemberships,
  type Workspace,
} from './workspaces';

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

describe('joinableMemberships', () => {
  // An accepted-but-not-yet-pulled company: active membership in the cloud,
  // no local folder. This is the row the "You've been added — Sync to pull it"
  // prompt is for (the field case: a client accepted via HQ Console, the menubar
  // had the membership but never surfaced it).
  const joinable = (overrides: Partial<Workspace> = {}): Workspace =>
    ws({
      slug: 'sender-agency',
      displayName: 'Sender Agency',
      state: 'cloud-only',
      membershipStatus: 'active',
      hasLocalFolder: false,
      localPath: null,
      ...overrides,
    });

  it('returns an active cloud-only membership (accepted, not yet pulled)', () => {
    expect(joinableMemberships([joinable()]).map((w) => w.slug)).toEqual([
      'sender-agency',
    ]);
  });

  it('excludes pending invites — not accepted/granted yet, nothing to pull', () => {
    expect(joinableMemberships([joinable({ membershipStatus: 'pending' })])).toEqual(
      [],
    );
  });

  it('excludes companies already on this machine (synced / local-only)', () => {
    expect(
      joinableMemberships([
        ws({ slug: 'indigo', state: 'synced', hasLocalFolder: true }),
        ws({
          slug: 'scratch',
          state: 'local-only',
          membershipStatus: null,
          hasLocalFolder: true,
        }),
      ]),
    ).toEqual([]);
  });

  it('excludes personal and broken workspaces', () => {
    expect(
      joinableMemberships([
        ws({ slug: 'personal', kind: 'personal', state: 'personal', membershipStatus: null }),
        ws({ slug: 'oops', state: 'broken', brokenReason: 'uid mismatch' }),
      ]),
    ).toEqual([]);
  });

  it('offers a company present in both manifest and cloud only once (deduped)', () => {
    const dup = joinable();
    expect(joinableMemberships([dup, { ...dup }])).toHaveLength(1);
  });

  it('picks only the joinable rows out of a mixed union', () => {
    const result = joinableMemberships([
      ws({ slug: 'personal', kind: 'personal', state: 'personal', membershipStatus: null }),
      ws({ slug: 'indigo', state: 'synced', hasLocalFolder: true }),
      joinable(),
      joinable({ slug: 'pending-co', membershipStatus: 'pending' }),
    ]);
    expect(result.map((w) => w.slug)).toEqual(['sender-agency']);
  });
});
