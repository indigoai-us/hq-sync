import { describe, expect, it } from 'vitest';
import { readRepoFile } from './harness';

/**
 * US-012 — Open-in-Claude-Code + drill-ins across surfaces.
 *
 * Source-contract harness (same style as board-surface.spec.ts /
 * deployments-actions.spec.ts). Asserts that:
 *
 *   1. A shared desktop-alt affordance (OpenFileInClaudeCode) REUSES the
 *      existing `buildClaudeCodeUrl` util + the `open_claude_code_link` Tauri
 *      command — it does NOT reimplement link building, and does NOT widen to
 *      plugin-shell open().
 *   2. The US-008 StoryDetailPanel files section wires that affordance per file.
 *   3. ActivityPanel recent entries become a clickable drill-in over the file
 *      the entry already carries (no invented fields).
 *   4. The drill-in affordances (hover / pointer cursor / focus-visible ring)
 *      are consistent with the board + deployments rows, and stay token-driven.
 */

describe('desktop-alt open-in-Claude-Code + activity drill-ins (US-012)', () => {
  const affordance = readRepoFile(
    'src/desktop-alt/components/OpenFileInClaudeCode.svelte',
  );
  const panel = readRepoFile(
    'src/desktop-alt/components/StoryDetailPanel.svelte',
  );
  const activity = readRepoFile('src/desktop-alt/panels/ActivityPanel.svelte');

  it('reuses the claude-code-link util + open_claude_code_link command (no reimplementation)', () => {
    // Builds the URL through the shared util — not a hand-rolled claude:// string.
    expect(affordance).toContain(
      "import { buildClaudeCodeUrl } from '../../lib/claude-code-link'",
    );
    expect(affordance).toContain('buildClaudeCodeUrl({ folder, prompt })');
    // Dispatches through the dedicated Tauri command, same as
    // OpenInClaudeCodeButton — NOT plugin-shell open().
    expect(affordance).toContain("invoke('open_claude_code_link', { url })");
    expect(affordance).not.toContain("from '@tauri-apps/plugin-shell'");
    // No hand-rolled claude:// scheme assembly in code (only the util builds it).
    // A `claude://.../?<query>` string would be reimplementation; assert none
    // exists (bare prose mentions of the scheme in comments are fine).
    expect(affordance).not.toMatch(/claude:\/\/[\w/]*\?/);
    // Suppresses itself when the HQ folder isn't loaded yet (button contract).
    expect(affordance).toContain('{#if folder}');
    expect(affordance).toContain('data-testid="open-in-claude-code"');
  });

  it('wires Open-in-Claude-Code into the story-files section (US-008 panel)', () => {
    expect(panel).toContain(
      "import OpenFileInClaudeCode from './OpenFileInClaudeCode.svelte'",
    );
    // The files list renders the affordance per file, passing the HQ root.
    expect(panel).toContain('data-testid="story-files"');
    expect(panel).toContain('{#each files as file');
    expect(panel).toContain(
      '<OpenFileInClaudeCode {file} folder={hqFolderPath} variant="compact" />',
    );
    // HQ root resolved via the same get_config command App.svelte uses.
    expect(panel).toContain("invoke<{ hqFolderPath?: string }>('get_config')");
  });

  it('makes activity entries a clickable drill-in over the file they already carry', () => {
    expect(activity).toContain(
      "import OpenFileInClaudeCode from '../components/OpenFileInClaudeCode.svelte'",
    );
    expect(activity).toContain('data-testid="activity-row"');
    // Drill-in uses ONLY the file the entry data already carries — no invented
    // fields (the ActivityEntry shape is who/what/file/when).
    expect(activity).toContain('file={entry.file}');
    expect(activity).toContain('folder={hqFolderPath}');
    // Guarded behind a real file name (normalizer falls back to "Untitled file").
    expect(activity).toContain("entry.file !== 'Untitled file'");
    // No fabricated activity fields leaked into the drill-in.
    expect(activity).not.toMatch(/entry\.(path|url|repo|sha|commit|line)\b/);
  });

  it('gives both surfaces consistent drill-in affordances (cursor, hover, focus ring)', () => {
    const affordanceStyle = affordance.split('<style>')[1] ?? '';
    expect(affordanceStyle).toContain('cursor: pointer');
    expect(affordanceStyle).toContain('.open-claude-btn:hover');
    expect(affordanceStyle).toContain('.open-claude-btn:focus-visible');
    expect(affordanceStyle).toContain('outline: 2px solid var(--blue)');

    // Story files row + activity row both reveal the affordance on hover/focus.
    const panelStyle = panel.split('<style>')[1] ?? '';
    expect(panelStyle).toContain('.file-item:hover');
    expect(panelStyle).toContain(':focus-visible)');

    const activityStyle = activity.split('<style>')[1] ?? '';
    expect(activityStyle).toContain('.recent-row:hover :global(.open-claude-btn)');
    expect(activityStyle).toContain(':focus-visible)');
  });

  it('keeps every US-012 surface token-driven (no hardcoded hex)', () => {
    for (const src of [affordance, panel, activity]) {
      const styleBlock = src.split('<style>')[1] ?? '';
      expect(styleBlock).not.toMatch(/#[0-9a-fA-F]{3,8}\b/);
    }
  });
});
