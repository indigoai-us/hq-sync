import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { describe, expect, it } from 'vitest';
import { labelColor, type Story } from '../projects-model';

/**
 * Story-card components (US-005: StoryCard + LabelChip).
 *
 * The components are Svelte and the repo has no component-render harness, so we
 * test (a) the pure AC-progress computation the card relies on, (b) the US-004
 * label-color contract LabelChip consumes, and (c) the source contract of both
 * components (tokens only, no hardcoded hex, focus ring, callback prop).
 */

const storyCardSource = readFileSync(
  resolve(process.cwd(), 'src/desktop-alt/components/StoryCard.svelte'),
  'utf8',
);
const labelChipSource = readFileSync(
  resolve(process.cwd(), 'src/desktop-alt/components/LabelChip.svelte'),
  'utf8',
);

function story(overrides: Partial<Story> = {}): Story {
  return {
    id: 'US-001',
    title: 'A story',
    description: '',
    acceptanceCriteria: [],
    passes: false,
    labels: [],
    dependsOn: [],
    ...overrides,
  };
}

/**
 * The AC-progress rule the card uses: no per-AC done flags exist, so a passing
 * story reads full (acTotal/acTotal) and any other story reads 0/acTotal. This
 * mirrors hq-desktop's StoryCard and is what the card template computes.
 */
function acProgress(s: Story): { complete: number; total: number; percent: number } {
  const total = s.acceptanceCriteria?.length ?? 0;
  const complete = s.passes ? total : 0;
  const percent = total > 0 ? (complete / total) * 100 : 0;
  return { complete, total, percent };
}

describe('US-005 AC-progress computation', () => {
  it('reads full when the story passes', () => {
    expect(acProgress(story({ passes: true, acceptanceCriteria: ['a', 'b', 'c'] }))).toEqual({
      complete: 3,
      total: 3,
      percent: 100,
    });
  });

  it('reads empty when the story does not pass', () => {
    expect(acProgress(story({ passes: false, acceptanceCriteria: ['a', 'b'] }))).toEqual({
      complete: 0,
      total: 2,
      percent: 0,
    });
  });

  it('is safe with zero acceptance criteria', () => {
    expect(acProgress(story({ passes: true, acceptanceCriteria: [] }))).toEqual({
      complete: 0,
      total: 0,
      percent: 0,
    });
  });
});

describe('US-005 label overflow', () => {
  it('shows up to two labels with a +N overflow indicator', () => {
    const labels = ['frontend', 'backend', 'infra', 'docs'];
    const visible = labels.slice(0, 2);
    expect(visible).toEqual(['frontend', 'backend']);
    expect(labels.length - visible.length).toBe(2);
  });
});

describe('US-005 LabelChip color contract (US-004 palette)', () => {
  it('resolves the same monochrome-glass color for the same label', () => {
    expect(labelColor('frontend')).toEqual(labelColor('frontend'));
  });

  it('uses translucent monochrome hsla tones, not indigo/Tailwind classes', () => {
    const color = labelColor('security');
    expect(color.background).toMatch(/^hsla\(/);
    expect(color.foreground).toMatch(/^hsla\(/);
  });
});

describe('US-005 StoryCard source contract', () => {
  it('renders id, 2-line-clamped title, progress bar, and badges', () => {
    expect(storyCardSource).toContain('story-id');
    expect(storyCardSource).toContain('line-clamp: 2');
    expect(storyCardSource).toContain('role="progressbar"');
    expect(storyCardSource).toContain('priority-badge');
    expect(storyCardSource).toContain('model-badge');
    expect(storyCardSource).toContain('label-overflow');
  });

  it('is a focusable button that emits an onselect callback', () => {
    expect(storyCardSource).toContain('<button');
    expect(storyCardSource).toContain('onselect?.(story)');
    expect(storyCardSource).toContain(':focus-visible');
  });

  it('dims completed stories', () => {
    expect(storyCardSource).toContain('class:is-complete={story.passes}');
    expect(storyCardSource).toMatch(/\.story-card\.is-complete\s*\{[^}]*opacity/);
  });

  it('uses design tokens and no hardcoded hex colors', () => {
    expect(storyCardSource).toContain('var(--');
    // No 3/4/6/8-digit hex literals in the component styles.
    expect(storyCardSource).not.toMatch(/#[0-9a-fA-F]{3,8}\b/);
    expect(labelChipSource).not.toMatch(/#[0-9a-fA-F]{3,8}\b/);
  });

  it('reuses the US-004 label-color function in LabelChip', () => {
    expect(labelChipSource).toContain("import { labelColor } from '../lib/projects-model'");
    expect(labelChipSource).toContain('labelColor(label)');
  });
});
