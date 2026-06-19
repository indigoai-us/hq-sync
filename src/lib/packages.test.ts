import { describe, expect, it } from 'vitest';

import {
  isMarketplaceOrigin,
  isPromptRenderable,
  shortSource,
  type InstalledPack,
  type PackInitialization,
} from './packages';

/**
 * US-009 — the suppress/show gate for a pack's author-written
 * `initialization.prompt`. The prose is renderable for copy/paste ONLY when the
 * pack came from the MODERATED marketplace/registry origin AND it carries the
 * explicit server-set `promptModerated === true` approval signal. Everything
 * else stays suppressed — including, deliberately, every pack today (the server
 * does not emit `promptModerated` yet, so the safe default is "hidden").
 */

function pack(overrides: Partial<InstalledPack> = {}): InstalledPack {
  return {
    name: 'hq-pack-example',
    transport: null,
    hqCoreSatisfied: true,
    contributes: { skill: 1 },
    links: { live: 1, broken: 0, missing: 0, foreign: 0 },
    brokenLinks: [],
    inCatalog: true,
    updateAvailable: false,
    ...overrides,
  };
}

function init(overrides: Partial<PackInitialization> = {}): PackInitialization {
  return { entrypoint: 'example', prompt: 'Run the example to get going.', ...overrides };
}

describe('isMarketplaceOrigin', () => {
  it('recognises the moderated marketplace + registry scheme prefixes', () => {
    expect(isMarketplaceOrigin('marketplace:impeccable')).toBe(true);
    expect(isMarketplaceOrigin('marketplace:impeccable@1.2.0')).toBe(true);
    expect(isMarketplaceOrigin('registry:hq-pack-engineering')).toBe(true);
    // Case / whitespace tolerant.
    expect(isMarketplaceOrigin('  MARKETPLACE:Foo  ')).toBe(true);
  });

  it('rejects local-path and git-URL origins (un-moderated)', () => {
    expect(isMarketplaceOrigin('github:owner/repo#packages/hq-pack-x')).toBe(false);
    expect(isMarketplaceOrigin('git+https://example.com/foo.git')).toBe(false);
    expect(isMarketplaceOrigin('https://example.com/foo')).toBe(false);
    expect(isMarketplaceOrigin('/Users/me/dev/my-pack')).toBe(false);
    expect(isMarketplaceOrigin('./local-pack')).toBe(false);
    expect(isMarketplaceOrigin(undefined)).toBe(false);
    expect(isMarketplaceOrigin('')).toBe(false);
  });
});

describe('isPromptRenderable', () => {
  it('renders prose for a marketplace pack with an approved prompt', () => {
    expect(
      isPromptRenderable(
        pack({ source: 'marketplace:impeccable', initialization: init({ promptModerated: true }) }),
      ),
    ).toBe(true);
    // Registry origin is also a moderated path.
    expect(
      isPromptRenderable(
        pack({ source: 'registry:hq-pack-x', initialization: init({ promptModerated: true }) }),
      ),
    ).toBe(true);
  });

  it('SUPPRESSES prose for a local or git install even when the prompt claims approval', () => {
    // A non-marketplace origin must NEVER render prose, regardless of any flag —
    // it never went through moderation.
    expect(
      isPromptRenderable(
        pack({
          source: 'github:owner/repo#packages/hq-pack-x',
          initialization: init({ promptModerated: true }),
        }),
      ),
    ).toBe(false);
    expect(
      isPromptRenderable(
        pack({
          source: '/Users/me/dev/my-pack',
          initialization: init({ promptModerated: true }),
        }),
      ),
    ).toBe(false);
  });

  it('SUPPRESSES prose when the approval flag is missing (conservative default)', () => {
    // Marketplace origin but no `promptModerated` → suppressed. This is the
    // real-world state today: the server does not emit the flag yet.
    expect(
      isPromptRenderable(pack({ source: 'marketplace:impeccable', initialization: init() })),
    ).toBe(false);
    // An explicit false is likewise suppressed.
    expect(
      isPromptRenderable(
        pack({ source: 'marketplace:impeccable', initialization: init({ promptModerated: false }) }),
      ),
    ).toBe(false);
  });

  it('SUPPRESSES when there is no prose or no initialization at all', () => {
    expect(isPromptRenderable(pack({ source: 'marketplace:impeccable' }))).toBe(false);
    expect(
      isPromptRenderable(
        pack({
          source: 'marketplace:impeccable',
          initialization: { entrypoint: 'example', promptModerated: true },
        }),
      ),
    ).toBe(false);
    // Whitespace-only prose is treated as empty.
    expect(
      isPromptRenderable(
        pack({
          source: 'marketplace:impeccable',
          initialization: init({ prompt: '   \n  ', promptModerated: true }),
        }),
      ),
    ).toBe(false);
    expect(isPromptRenderable(undefined)).toBe(false);
  });
});

describe('shortSource (unchanged regression guard)', () => {
  it('drops the long git prefix to the trailing pack name', () => {
    expect(shortSource('github:owner/repo#packages/hq-pack-impeccable')).toBe('hq-pack-impeccable');
    expect(shortSource(undefined)).toBe('unknown source');
  });
});
