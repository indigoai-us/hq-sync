import { describe, expect, it } from 'vitest';

import {
  BUNDLED_PACK_COVERS,
  coverFallback,
  coverForListing,
} from './pack-covers';
import type { MarketplaceListing } from './marketplace';

/** Minimal listing factory — only the fields the cover resolver reads. */
function listing(partial: Partial<MarketplaceListing>): MarketplaceListing {
  return {
    id: 'lst_test',
    type: 'skill',
    name: 'Test Pack',
    slug: 'test-pack',
    version: '1.0.0',
    author: 'tester',
    createdAt: '2026-01-01T00:00:00.000Z',
    ...partial,
  };
}

describe('coverForListing — pack cover resolution', () => {
  it('returns the bundled cover for each of the five shipped packs', () => {
    for (const slug of [
      'engineering',
      'gstack',
      'pocock-skills',
      'impeccable',
      'magicpath-agent-skills',
    ]) {
      const url = coverForListing(listing({ slug }));
      expect(typeof url).toBe('string');
      expect((url ?? '').length).toBeGreaterThan(0);
    }
  });

  it('every bundled-cover key resolves to a non-empty asset URL', () => {
    for (const [slug, url] of Object.entries(BUNDLED_PACK_COVERS)) {
      expect(url, `cover for ${slug}`).toBeTruthy();
    }
  });

  it('returns null for a pack with no bundled art and no hosted cover', () => {
    expect(coverForListing(listing({ slug: 'some-unknown-pack' }))).toBeNull();
  });

  it('prefers a server-provided coverImageUrl over the bundled map', () => {
    const hosted = 'https://cdn.example.com/cover.png';
    // Even for a slug that HAS bundled art, the hosted URL wins (forward-compat).
    const url = coverForListing(listing({ slug: 'gstack', coverImageUrl: hosted }));
    expect(url).toBe(hosted);
  });

  it('ignores a blank/whitespace coverImageUrl and falls back to bundled art', () => {
    const url = coverForListing(listing({ slug: 'gstack', coverImageUrl: '   ' }));
    expect(url).not.toBe('   ');
    expect((url ?? '').length).toBeGreaterThan(0);
  });

  it('ignores a null coverImageUrl and falls back to bundled art', () => {
    const url = coverForListing(listing({ slug: 'impeccable', coverImageUrl: null }));
    expect((url ?? '').length).toBeGreaterThan(0);
  });
});

describe('coverFallback — deterministic branded placeholder', () => {
  it('derives a stable gradient + monogram from the slug', () => {
    const a = coverFallback(listing({ slug: 'foo-bar', name: 'Foo Bar' }));
    const b = coverFallback(listing({ slug: 'foo-bar', name: 'Foo Bar' }));
    expect(a.gradient).toBe(b.gradient);
    expect(a.gradient).toMatch(/^linear-gradient\(/);
    expect(a.monogram).toBe('F');
  });

  it('gives different slugs different gradients (no single flat color)', () => {
    const a = coverFallback(listing({ slug: 'alpha' }));
    const b = coverFallback(listing({ slug: 'zulu' }));
    expect(a.gradient).not.toBe(b.gradient);
  });

  it('falls back to a placeholder monogram when name and slug are empty', () => {
    const fb = coverFallback(listing({ slug: '', name: '' }));
    expect(fb.monogram).toBe('?');
    expect(fb.gradient).toMatch(/^linear-gradient\(/);
  });
});
