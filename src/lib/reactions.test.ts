import { describe, it, expect } from 'vitest';
import {
  type ReactionAggregate,
  type ReactionMap,
  CURATED_EMOJI,
  dmScope,
  channelScope,
  findAggregate,
  hasReacted,
  toggleIsAdd,
  toggleReaction,
  sortAggregates,
  applyReactionEvent,
  setMessageReactions,
  buildReactionMap,
} from './reactions';

function agg(over: Partial<ReactionAggregate> = {}): ReactionAggregate {
  return { emoji: '👍', count: 1, reactedByMe: false, ...over };
}

describe('scope builders', () => {
  it('dmScope prefixes dm: and trims', () => {
    expect(dmScope('prs_peer')).toBe('dm:prs_peer');
    expect(dmScope('  prs_x  ')).toBe('dm:prs_x');
  });

  it('channelScope prefixes chan: and trims', () => {
    expect(channelScope('chn_1')).toBe('chan:chn_1');
    expect(channelScope('  chn_2 ')).toBe('chan:chn_2');
  });

  it('the two scope kinds never collide', () => {
    expect(dmScope('x')).not.toBe(channelScope('x'));
  });
});

describe('CURATED_EMOJI', () => {
  it('is a compact, de-duplicated set (no heavy emoji-data dep)', () => {
    // ~24 curated emoji — keep it small for the <15MB bundle budget.
    expect(CURATED_EMOJI.length).toBeGreaterThanOrEqual(20);
    expect(CURATED_EMOJI.length).toBeLessThanOrEqual(30);
    expect(new Set(CURATED_EMOJI).size).toBe(CURATED_EMOJI.length);
    // The most common reactions are present.
    expect(CURATED_EMOJI).toContain('👍');
    expect(CURATED_EMOJI).toContain('❤️');
    expect(CURATED_EMOJI).toContain('🎉');
  });
});

describe('findAggregate / hasReacted / toggleIsAdd', () => {
  it('finds an emoji or returns undefined', () => {
    const list = [agg({ emoji: '👍' }), agg({ emoji: '🎉' })];
    expect(findAggregate(list, '🎉')?.emoji).toBe('🎉');
    expect(findAggregate(list, '🔥')).toBeUndefined();
    expect(findAggregate(undefined, '👍')).toBeUndefined();
  });

  it('hasReacted reflects reactedByMe', () => {
    const list = [agg({ emoji: '👍', reactedByMe: true }), agg({ emoji: '🎉' })];
    expect(hasReacted(list, '👍')).toBe(true);
    expect(hasReacted(list, '🎉')).toBe(false);
    expect(hasReacted(list, '🔥')).toBe(false);
    expect(hasReacted(undefined, '👍')).toBe(false);
  });

  it('toggleIsAdd is the inverse of hasReacted', () => {
    const list = [agg({ emoji: '👍', reactedByMe: true })];
    expect(toggleIsAdd(list, '👍')).toBe(false); // already reacted → remove
    expect(toggleIsAdd(list, '🔥')).toBe(true); // not reacted → add
  });
});

describe('toggleReaction (optimistic, immutable)', () => {
  it('adds a brand-new emoji with count 1, reactedByMe true', () => {
    const list: ReactionAggregate[] = [];
    const next = toggleReaction(list, '🔥');
    expect(next).toEqual([{ emoji: '🔥', count: 1, reactedByMe: true }]);
    // input untouched
    expect(list).toEqual([]);
  });

  it('joins an emoji others already used (count +1, reactedByMe true)', () => {
    const list = [agg({ emoji: '👍', count: 3, reactedByMe: false })];
    const next = toggleReaction(list, '👍');
    expect(next).toEqual([{ emoji: '👍', count: 4, reactedByMe: true }]);
  });

  it('removes the caller from a shared emoji (count -1, reactedByMe false)', () => {
    const list = [agg({ emoji: '👍', count: 3, reactedByMe: true })];
    const next = toggleReaction(list, '👍');
    expect(next).toEqual([{ emoji: '👍', count: 2, reactedByMe: false }]);
  });

  it('drops the pill entirely when the caller was the only reactor', () => {
    const list = [agg({ emoji: '🎉', count: 1, reactedByMe: true })];
    const next = toggleReaction(list, '🎉');
    expect(next).toEqual([]);
  });

  it('round-trips: add then remove returns to the original set', () => {
    const start = [agg({ emoji: '❤️', count: 2, reactedByMe: false })];
    const added = toggleReaction(start, '❤️');
    expect(added).toEqual([{ emoji: '❤️', count: 3, reactedByMe: true }]);
    const removed = toggleReaction(added, '❤️');
    expect(removed).toEqual([{ emoji: '❤️', count: 2, reactedByMe: false }]);
  });

  it('handles an undefined list (first reaction on a message)', () => {
    expect(toggleReaction(undefined, '👍')).toEqual([
      { emoji: '👍', count: 1, reactedByMe: true },
    ]);
  });
});

describe('sortAggregates', () => {
  it('orders by count desc, then emoji for stable ties', () => {
    const list = [
      agg({ emoji: '🎉', count: 1 }),
      agg({ emoji: '👍', count: 5 }),
      agg({ emoji: '🔥', count: 1 }),
    ];
    const sorted = sortAggregates(list);
    expect(sorted.map((r) => r.emoji)).toEqual(['👍', '🎉', '🔥']);
    // input untouched
    expect(list[0].emoji).toBe('🎉');
  });
});

describe('applyReactionEvent (live reconcile)', () => {
  it('replaces a message entry wholesale with the server truth, sorted', () => {
    const map: ReactionMap = { e1: [agg({ emoji: '👍', count: 1, reactedByMe: true })] };
    const next = applyReactionEvent(map, 'dm:prs_x', {
      messageScope: 'dm:prs_x',
      messageId: 'e1',
      reactions: [
        agg({ emoji: '👍', count: 2, reactedByMe: true }),
        agg({ emoji: '🔥', count: 3, reactedByMe: false }),
      ],
    });
    expect(next.e1.map((r) => r.emoji)).toEqual(['🔥', '👍']); // sorted by count
    expect(next.e1[1]).toEqual({ emoji: '👍', count: 2, reactedByMe: true });
    // original map untouched
    expect(map.e1[0].count).toBe(1);
  });

  it('drops the key when the message has no reactions left', () => {
    const map: ReactionMap = { e1: [agg({ emoji: '👍', count: 1, reactedByMe: true })] };
    const next = applyReactionEvent(map, 'dm:prs_x', {
      messageScope: 'dm:prs_x',
      messageId: 'e1',
      reactions: [],
    });
    expect(next).toEqual({});
    expect('e1' in next).toBe(false);
  });

  it('ignores events for a different scope (host only reconciles its own)', () => {
    const map: ReactionMap = { e1: [agg()] };
    const next = applyReactionEvent(map, 'dm:prs_x', {
      messageScope: 'chan:chn_9',
      messageId: 'e1',
      reactions: [agg({ count: 99 })],
    });
    expect(next).toBe(map); // returned unchanged (same reference)
  });
});

describe('setMessageReactions', () => {
  it('sets a message entry immutably', () => {
    const map: ReactionMap = {};
    const next = setMessageReactions(map, 'e1', [agg({ emoji: '🔥', count: 1, reactedByMe: true })]);
    expect(next.e1).toEqual([{ emoji: '🔥', count: 1, reactedByMe: true }]);
    expect(map).toEqual({});
  });

  it('deletes the key when set to an empty list', () => {
    const map: ReactionMap = { e1: [agg()] };
    const next = setMessageReactions(map, 'e1', []);
    expect('e1' in next).toBe(false);
  });
});

describe('buildReactionMap', () => {
  it('keys aggregates by messageId, sorted, dropping empties', () => {
    const map = buildReactionMap([
      { messageId: 'e1', reactions: [agg({ emoji: '👍', count: 1 }), agg({ emoji: '🔥', count: 4 })] },
      { messageId: 'e2', reactions: [] },
    ]);
    expect(Object.keys(map)).toEqual(['e1']);
    expect(map.e1.map((r) => r.emoji)).toEqual(['🔥', '👍']);
  });
});
