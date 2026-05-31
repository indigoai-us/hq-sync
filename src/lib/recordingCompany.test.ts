import { describe, expect, it } from 'vitest';
import {
  activeMemberships,
  resolveStartCompany,
  resolveValidDefault,
  shouldBackfill,
  type RecordingMembership,
} from './recordingCompany';

// Two companies the user can record for. `resolveValidDefault` validates a
// stored default against this list, so anything not present here is "stale".
const members: RecordingMembership[] = [
  { companyUid: 'cmp_a', companyName: 'Alpha', status: 'active' },
  { companyUid: 'cmp_b', companyName: 'Beta', status: 'active' },
];

describe('resolveValidDefault', () => {
  it('returns the default when it matches a current membership', () => {
    expect(resolveValidDefault('cmp_a', members)).toBe('cmp_a');
  });

  it('drops a stale default that no longer matches any membership', () => {
    // Company left / membership revoked: must NOT keep attributing to it.
    expect(resolveValidDefault('cmp_gone', members)).toBeNull();
  });

  it('returns null when there is no stored default', () => {
    expect(resolveValidDefault(null, members)).toBeNull();
  });
});

describe('resolveStartCompany', () => {
  it('honours an explicit user choice over the default', () => {
    const row = { companyUid: 'cmp_b', companyUserSet: true };
    expect(resolveStartCompany(row, 'cmp_a', members)).toBe('cmp_b');
  });

  it('honours an explicit Personal (null) choice over the default', () => {
    // The user deliberately chose Personal — a default must not override it.
    const row = { companyUid: null, companyUserSet: true };
    expect(resolveStartCompany(row, 'cmp_a', members)).toBeNull();
  });

  it('falls back to the valid default when the row is not user-set', () => {
    const row = { companyUid: null, companyUserSet: false };
    expect(resolveStartCompany(row, 'cmp_a', members)).toBe('cmp_a');
  });

  it('falls back to the row company when the default is stale', () => {
    const row = { companyUid: 'cmp_b', companyUserSet: false };
    expect(resolveStartCompany(row, 'cmp_gone', members)).toBe('cmp_b');
  });

  it('yields null when there is no row and no valid default', () => {
    expect(resolveStartCompany(undefined, null, members)).toBeNull();
  });
});

describe('activeMemberships', () => {
  it('drops memberships that are not active', () => {
    const list: RecordingMembership[] = [
      ...members,
      { companyUid: 'cmp_c', companyName: 'Gamma', status: 'invited' },
    ];
    expect(activeMemberships(list)).toEqual(members);
  });
});

describe('shouldBackfill', () => {
  it('back-fills an open, non-user-set row that differs from the default', () => {
    expect(shouldBackfill({ companyUid: null, companyUserSet: false }, 'cmp_a')).toBe(true);
  });

  it('never touches an explicit user choice', () => {
    expect(shouldBackfill({ companyUid: null, companyUserSet: true }, 'cmp_a')).toBe(false);
  });

  it('does nothing when there is no valid default', () => {
    expect(shouldBackfill({ companyUid: null, companyUserSet: false }, null)).toBe(false);
  });

  it('does nothing when the row already matches the default', () => {
    expect(shouldBackfill({ companyUid: 'cmp_a', companyUserSet: false }, 'cmp_a')).toBe(false);
  });
});
