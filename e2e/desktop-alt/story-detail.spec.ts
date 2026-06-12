import { describe, expect, it } from 'vitest';
import { readRepoFile } from './harness';

/**
 * US-008 — story detail slide-over.
 *
 * Source-contract (non-render) harness, matching the existing desktop-alt spec
 * style (board-surface.spec.ts). Asserts that StoryPanel wires the
 * slide-over affordances (backdrop, close button, Escape), the section content
 * (AC checklist + progress, dependency chips, labels, notes, files), and the
 * graceful agent-activity empty state — and that CompanyBoardPanel opens/closes
 * the panel through its selection state.
 */

describe('desktop-alt story detail slide-over (US-008)', () => {
  const panel = readRepoFile(
    'src/desktop-alt/v4/StoryPanel.svelte',
  );
  const page = readRepoFile('src/desktop-alt/panels/CompanyBoardPanel.svelte');

  it('declares the US-008 props contract', () => {
    // story (Story | null), project, prdPath, onclose, and dependency/status callbacks.
    expect(panel).toContain('story: Story | null');
    expect(panel).toContain('project: Project | null');
    expect(panel).toContain('prdPath: string');
    expect(panel).toContain('onclose: () => void');
    expect(panel).toContain('onselectDependency?: (storyId: string) => void');
    expect(panel).toContain('onStoryPassesChange?: (storyId: string, passes: boolean) => void');
    // Renders nothing when story is null.
    expect(panel).toContain('{#if story}');
  });

  it('is a right-side slide-over with a backdrop and close affordances', () => {
    // Fixed-to-the-right panel + dark translucent backdrop.
    expect(panel).toContain('class="story-backdrop"');
    expect(panel).toContain('class="story-panel"');
    expect(panel).toContain('data-testid="v4-story-panel"');
    expect(panel).toContain('width: min(420px, 100vw)');

    // Closes on: backdrop click, explicit X button, and Escape.
    expect(panel).toContain('aria-label="Close story"');
    expect(panel).toContain('onclick={onclose}');
    expect(panel).toContain("event.key === 'Escape'");
    expect(panel).toContain('onkeydown={story ? handleKeydown : undefined}');
  });

  it('renders the AC checklist with a progress indicator', () => {
    expect(panel).toContain("import { setStoryPasses } from '../lib/projects-store.svelte'");
    expect(panel).toContain('class="status-control"');
    expect(panel).toContain('Acceptance criteria');
    expect(panel).toContain('class="progress-track"');
    expect(panel).toContain('style={`width: ${progress}%`}');
    // The checklist iterates the acceptance criteria.
    expect(panel).toContain('{#each acItems as item, index (index)}');
  });

  it('renders dependency chips as a clickable reselect callback', () => {
    expect(panel).toContain('{#if story.dependsOn.length > 0}');
    expect(panel).toContain('class="chip-row"');
    expect(panel).toContain('{#each story.dependsOn as dep (dep)}');
    expect(panel).toContain('onclick={() => onselectDependency?.(dep)}');
  });

  it('renders label chips, description, files, and footer actions', () => {
    expect(panel).toContain('{#each story.labels as label (label)}');
    expect(panel).toContain('<span class="label-chip">{label}</span>');
    expect(panel).toContain('{#if story.description}');
    expect(panel).toContain('<p>{story.description}</p>');
    // Monospace files list.
    expect(panel).toContain('class="file-list"');
    expect(panel).toContain('{#each story.files as file (file)}');
    expect(panel).toContain('Open PRD');
    expect(panel).toContain('Run story');
    expect(panel).toContain('onclick={() => void openPrd()}');
    expect(panel).toContain('onclick={() => void runStory()}');
    expect(panel).toContain("invoke('open_in_editor', { path: prdPath })");
    expect(panel).toContain('buildClaudeCodeUrl({ folder: config.hqFolderPath ?? \'\', prompt })');
    expect(panel).toContain("invoke('open_claude_code_link', { url })");
  });

  it('keeps the panel token-driven (no hardcoded hex)', () => {
    const styleBlock = panel.split('<style>')[1] ?? '';
    // The only allowed literal colors are the neutral black scrim/shadow rgba()s,
    // which carry no hex. Assert there are no hex literals at all.
    expect(styleBlock).not.toMatch(/#[0-9a-fA-F]{3,8}\b/);
  });

  it('wires the panel into CompanyBoardPanel through its selection state', () => {
    expect(page).toContain(
      "import StoryPanel from '../v4/StoryPanel.svelte'",
    );
    // The Kanban lives inside ProjectDetailView: CompanyBoardPanel threads
    // openStory into the detail view via onselectStory, which forwards it to the
    // embedded StoryKanban's onselect. The panel itself lives in CompanyBoardPanel.
    expect(page).toContain('onselectStory={openStory}');
    const detail = readRepoFile('src/desktop-alt/pages/ProjectDetailView.svelte');
    expect(detail).toContain('onselect={onselectStory}');
    expect(page).toContain('<StoryPanel');
    expect(page).toContain('story={selectedStory}');
    expect(page).toContain('onclose={closeStory}');
    expect(page).toContain('onselectDependency={selectStoryById}');
    expect(page).toContain('{onStoryPassesChange}');
    // Leaving the project / going back clears the open story.
    expect(page).toContain('selectedStoryId = null');
  });
});
