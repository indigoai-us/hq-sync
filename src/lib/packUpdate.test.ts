import { describe, expect, it } from 'vitest';

import { packUpdateTitle } from './packUpdate';

describe('packUpdateTitle', () => {
  it('uses singular copy for one pack', () => {
    expect(packUpdateTitle(1)).toBe('1 pack has an update available');
  });

  it('uses plural copy for two packs', () => {
    expect(packUpdateTitle(2)).toBe('2 packs have updates available');
  });

  it('uses plural copy for three packs', () => {
    expect(packUpdateTitle(3)).toBe('3 packs have updates available');
  });

  it('uses plural copy for zero packs defensively', () => {
    expect(packUpdateTitle(0)).toBe('0 packs have updates available');
  });
});
