import { describe, expect, it } from 'vitest';
import { shouldAppendInbound } from './dmThread';

const peer = 'prs_alice';
const msg = (eventId: string) => ({ eventId });
const dm = (eventId: string, fromPersonUid: string) => ({ eventId, fromPersonUid });

describe('shouldAppendInbound (DM detail live thread)', () => {
  it('appends a new DM from the viewed peer', () => {
    expect(shouldAppendInbound([msg('e1')], dm('e2', peer), peer)).toBe(true);
  });

  it('ignores a DM from a different peer (window is one conversation)', () => {
    expect(shouldAppendInbound([], dm('e2', 'prs_bob'), peer)).toBe(false);
  });

  it('ignores a duplicate already in the thread (poll re-surface / thread overlap)', () => {
    expect(shouldAppendInbound([msg('e1'), msg('e2')], dm('e2', peer), peer)).toBe(false);
  });

  it('ignores everything when no peer is set yet (nothing open)', () => {
    expect(shouldAppendInbound([], dm('e2', peer), null)).toBe(false);
    expect(shouldAppendInbound([], dm('e2', peer), undefined)).toBe(false);
    expect(shouldAppendInbound([], dm('e2', peer), '')).toBe(false);
  });

  it('appends into an empty thread from the viewed peer', () => {
    expect(shouldAppendInbound([], dm('first', peer), peer)).toBe(true);
  });
});
