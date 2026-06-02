import { describe, expect, it } from 'vitest';
import { readRepoFile } from './harness';

/**
 * US-008 — story detail slide-over.
 *
 * Source-contract (non-render) harness, matching the existing desktop-alt spec
 * style (board-surface.spec.ts). Asserts that StoryDetailPanel wires the
 * slide-over affordances (backdrop, close button, Escape), the section content
 * (AC checklist + progress, dependency chips, labels, notes, files), and the
 * graceful agent-activity empty state — and that CompanyBoardPanel opens/closes
 * the panel through its selection state.
 */

describe('desktop-alt story detail slide-over (US-008)', () => {
  const panel = readRepoFile(
    'src/desktop-alt/components/StoryDetailPanel.svelte',
  );
  const page = readRepoFile('src/desktop-alt/panels/CompanyBoardPanel.svelte');

  it('declares the US-008 props contract', () => {
    // story (Story | null), onclose, onselectDependency(storyId).
    expect(panel).toContain('story: StoryWithExtras | null');
    expect(panel).toContain('onclose: () => void');
    expect(panel).toContain('onselectDependency?: (storyId: string) => void');
    // Renders nothing when story is null.
    expect(panel).toContain('{#if story}');
  });

  it('is a right-side slide-over with a backdrop and close affordances', () => {
    // Fixed-to-the-right panel + dark translucent backdrop.
    expect(panel).toContain('data-testid="story-detail-panel"');
    expect(panel).toContain('data-testid="story-detail-backdrop"');
    expect(panel).toContain('role="dialog"');
    expect(panel).toContain('aria-modal="true"');

    // Closes on: backdrop click, explicit X button, and Escape.
    expect(panel).toContain('data-testid="story-detail-close"');
    expect(panel).toContain('onclick={onclose}');
    expect(panel).toContain("event.key === 'Escape'");
    expect(panel).toContain('onkeydown={story ? handleKeydown : undefined}');

    // Slide-in transition lives in the style block, gated for reduced motion.
    const styleBlock = panel.split('<style>')[1] ?? '';
    expect(styleBlock).toContain('panel-slide-in');
    expect(styleBlock).toContain('@media (prefers-reduced-motion: reduce)');
  });

  it('renders the AC checklist with a progress indicator', () => {
    expect(panel).toContain('data-testid="ac-checklist"');
    expect(panel).toContain('data-testid="ac-progress-count"');
    expect(panel).toContain('role="progressbar"');
    expect(panel).toContain('aria-valuenow={acComplete}');
    // The checklist iterates the acceptance criteria.
    expect(panel).toContain('{#each acItems as criterion');
  });

  it('renders dependency chips as a clickable reselect callback', () => {
    expect(panel).toContain('data-testid="dependency-chips"');
    expect(panel).toContain('data-testid="dependency-chip"');
    expect(panel).toContain('onclick={() => selectDependency(depId)}');
    expect(panel).toContain('onselectDependency?.(depId)');
  });

  it('reuses the US-005 LabelChip and renders notes + files sections', () => {
    expect(panel).toContain(
      "import LabelChip from './LabelChip.svelte'",
    );
    expect(panel).toContain('<LabelChip {label} />');
    // Monospace files list.
    expect(panel).toContain('class="file-list"');
    expect(panel).toContain('{#each files as file');
    // Notes section.
    expect(panel).toContain('{#if story.notes}');
  });

  it('degrades agent activity gracefully to a calm empty state with a seam', () => {
    // Calm "No active run" empty state — present, not faked.
    expect(panel).toContain('data-testid="agent-activity-empty"');
    expect(panel).toContain('No active run');
    // Seam for later orchestrator wiring: optional `activity` prop + TODO.
    expect(panel).toContain('activity?: AgentActivity | null');
    expect(panel).toContain('activity = null');
    expect(panel).toContain('TODO');
  });

  it('keeps the panel token-driven (no hardcoded hex)', () => {
    const styleBlock = panel.split('<style>')[1] ?? '';
    // The only allowed literal colors are the neutral black scrim/shadow rgba()s,
    // which carry no hex. Assert there are no hex literals at all.
    expect(styleBlock).not.toMatch(/#[0-9a-fA-F]{3,8}\b/);
  });

  it('wires the panel into CompanyBoardPanel through its selection state', () => {
    expect(page).toContain(
      "import StoryDetailPanel from '../components/StoryDetailPanel.svelte'",
    );
    // The Kanban lives inside ProjectDetailView: CompanyBoardPanel threads
    // openStory into the detail view via onselectStory, which forwards it to the
    // embedded StoryKanban's onselect. The panel itself lives in CompanyBoardPanel.
    expect(page).toContain('onselectStory={openStory}');
    const detail = readRepoFile('src/desktop-alt/pages/ProjectDetailView.svelte');
    expect(detail).toContain('onselect={onselectStory}');
    expect(page).toContain('story={selectedStory}');
    expect(page).toContain('onclose={closeStory}');
    expect(page).toContain('onselectDependency={selectStoryById}');
    // Leaving the project / going back clears the open story.
    expect(page).toContain('selectedStoryId = null');
  });
});
