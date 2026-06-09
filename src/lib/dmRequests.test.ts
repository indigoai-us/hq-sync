import { describe, it, expect } from 'vitest';
import {
  type DmRequest,
  requestDisplayName,
  requestInitials,
  addRequest,
  removeRequest,
  requestBannerTitle,
  requestBannerBody,
} from './dmRequests';

function mk(over: Partial<DmRequest> = {}): DmRequest {
  return {
    pairKey: 'pair_1',
    fromPersonUid: 'prs_1',
    fromEmail: 'ada@example.com',
    fromDisplayName: 'Ada Lovelace',
    message: null,
    sharedCompany: null,
    createdAt: '2026-06-05T00:00:00Z',
    ...over,
  };
}

describe('requestDisplayName', () => {
  it('prefers display name, falls back to email then personUid', () => {
    expect(requestDisplayName(mk())).toBe('Ada Lovelace');
    expect(requestDisplayName(mk({ fromDisplayName: '  ' }))).toBe(
      'ada@example.com'
    );
    expect(
      requestDisplayName(mk({ fromDisplayName: '', fromEmail: '' }))
    ).toBe('prs_1');
  });
});

describe('requestInitials', () => {
  it('uses first letters of two name parts', () => {
    expect(requestInitials(mk())).toBe('AL');
  });
  it('falls back to first two chars for a single token', () => {
    expect(requestInitials(mk({ fromDisplayName: 'Solo' }))).toBe('SO');
  });
});

describe('addRequest', () => {
  it('prepends a new request', () => {
    const list = [mk({ pairKey: 'a' })];
    const next = addRequest(list, mk({ pairKey: 'b' }));
    expect(next.map((r) => r.pairKey)).toEqual(['b', 'a']);
  });
  it('dedupes by pairKey (re-emit is a no-op)', () => {
    const list = [mk({ pairKey: 'a' })];
    const next = addRequest(list, mk({ pairKey: 'a' }));
    expect(next).toBe(list); // same reference — nothing added
    expect(next).toHaveLength(1);
  });
});

describe('removeRequest', () => {
  it('prunes the resolved request by pairKey and decrements the count', () => {
    const list = [mk({ pairKey: 'a' }), mk({ pairKey: 'b' })];
    const next = removeRequest(list, 'a');
    expect(next.map((r) => r.pairKey)).toEqual(['b']);
    // Accept-then-prune: the count (list length) drops, driving the badge.
    expect(next).toHaveLength(1);
  });
  it('is a no-op for an unknown pairKey', () => {
    const list = [mk({ pairKey: 'a' })];
    expect(removeRequest(list, 'zzz')).toHaveLength(1);
  });
});

describe('request banner copy', () => {
  it('uses distinct "wants to connect" title copy', () => {
    expect(requestBannerTitle(mk())).toBe('Ada Lovelace wants to connect');
  });
  it('quotes the held message when present, else a fallback prompt', () => {
    expect(requestBannerBody(mk({ message: 'hey, can we connect?' }))).toBe(
      'hey, can we connect?'
    );
    expect(requestBannerBody(mk({ message: '   ' }))).toBe(
      'Open Messages to accept, decline, or block this request.'
    );
  });
});
