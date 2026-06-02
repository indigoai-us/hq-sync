import { describe, expect, it } from 'vitest';
import { CORE_SETUP_LABEL, displayLabel, isCorePath } from './progressLabel';

describe('isCorePath', () => {
  it('matches root-level core paths', () => {
    expect(isCorePath('core/policies/x.md')).toBe(true);
    expect(isCorePath('core')).toBe(true);
    expect(isCorePath('/core/skills/y.md')).toBe(true);
  });

  it('matches nested core paths', () => {
    expect(isCorePath('repos/public/foo/core/bar.ts')).toBe(true);
  });

  it('does not match non-core paths', () => {
    expect(isCorePath('companies/indigo/knowledge/a.md')).toBe(false);
    expect(isCorePath('personal/policies/b.md')).toBe(false);
    expect(isCorePath('hardcore/notes.md')).toBe(false); // not a core/ segment
    expect(isCorePath('mycore/x')).toBe(false);
  });

  it('handles empty / nullish input', () => {
    expect(isCorePath('')).toBe(false);
    expect(isCorePath(null)).toBe(false);
    expect(isCorePath(undefined)).toBe(false);
  });
});

describe('displayLabel', () => {
  it('collapses core paths to the setup label', () => {
    expect(displayLabel('core/hooks/master-hook.sh')).toBe(CORE_SETUP_LABEL);
    expect(displayLabel('/core/docs/INDEX.md')).toBe(CORE_SETUP_LABEL);
  });

  it('passes through non-core paths unchanged', () => {
    expect(displayLabel('companies/indigo/projects/x.md')).toBe(
      'companies/indigo/projects/x.md',
    );
  });

  it('returns empty string for nullish input', () => {
    expect(displayLabel(null)).toBe('');
    expect(displayLabel(undefined)).toBe('');
  });
});
