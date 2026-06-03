import { describe, expect, it } from 'vitest';
import { shareTitle } from './share-path';

describe('shareTitle', () => {
  it('names the directory (with trailing slash) for a wildcard directory share', () => {
    // Regression: this used to collapse to the literal "*".
    expect(shareTitle('projects/client-stats-redesign/*')).toBe(
      'client-stats-redesign/',
    );
  });

  it('handles recursive `/**` directory shares', () => {
    expect(shareTitle('projects/foo/**')).toBe('foo/');
  });

  it('leaves plain file shares unchanged', () => {
    expect(shareTitle('docs/a.md')).toBe('a.md');
    expect(shareTitle('README.md')).toBe('README.md');
  });

  it('ignores a trailing slash on a directory path without a wildcard', () => {
    expect(shareTitle('projects/foo/')).toBe('foo');
  });

  it('falls back to "All files" for a whole-vault wildcard', () => {
    expect(shareTitle('*')).toBe('All files');
    expect(shareTitle('**')).toBe('All files');
  });
});
