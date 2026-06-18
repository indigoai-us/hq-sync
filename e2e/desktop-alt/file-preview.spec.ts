import { describe, expect, it } from 'vitest';
import { readRepoFile } from './harness';

/**
 * US-004 — File preview pane + open-in-Claude-Code / reveal-in-Finder
 *
 * Source-contract harness (same style as open-in-claude-code.spec.ts /
 * board-surface.spec.ts). Does NOT mount components — asserts on source text
 * to lock down implementation contracts and prevent regressions.
 *
 * Acceptance criteria covered:
 *   1. Markdown files are detected by extension and rendered as HTML via
 *      renderMarkdown, not shown as raw text.
 *   2. Open-in-Claude-Code reuses OpenFileInClaudeCode.svelte (claude:// path)
 *      — FilePreviewPane does NOT hand-roll a claude:// scheme and does NOT
 *      route that action through plugin-shell open().
 *   3. Binary / oversized files drive the unsupported placeholder via .catch();
 *      the open actions render in the header independent of preview success.
 *   4. get_company_file_content is invoked with { path } (binary/oversized
 *      triggers the catch path, which drives the unsupported state).
 *   5. Reveal in Finder uses plugin-shell open() (shell:allow-open grant) for
 *      reveal only — it is not used for Open-in-Claude-Code.
 *   6. Files mode wires the tree + preview: FilesModeSidebar owns the tree and
 *      file select, DesktopApp renders FilePreviewPane in the main area driven
 *      by the selected path (US-009 moved this off the per-company panel).
 *   7. CompanyFileTree accepts selectedPath prop and highlights the selected
 *      row with .selected + aria-current="true".
 *   8. No purple and no hardcoded hex in FilePreviewPane's <style> block.
 */

describe('desktop-alt file preview pane + open actions (US-004 file-explorer)', () => {
  const preview = readRepoFile(
    'src/desktop-alt/components/FilePreviewPane.svelte',
  );
  const sidebar = readRepoFile(
    'src/desktop-alt/v4/FilesModeSidebar.svelte',
  );
  const desktopApp = readRepoFile('src/desktop-alt/DesktopApp.svelte');
  const tree = readRepoFile(
    'src/desktop-alt/components/CompanyFileTree.svelte',
  );

  // -------------------------------------------------------------------------
  // US-004 e2eTest 1: Markdown detection + renderMarkdown rendering
  // -------------------------------------------------------------------------
  it('detects markdown by extension and renders via renderMarkdown into file-preview-markdown (not raw text)', () => {
    // Imports renderMarkdown from the shared lib (no reimplementation).
    expect(preview).toContain(
      "import { renderMarkdown } from '../lib/markdown'",
    );

    // Markdown detection regex matches .md / .markdown (case-insensitive).
    expect(preview).toContain('/\\.(md|markdown)$/i');

    // The markdown derived-state drives renderMarkdown when isMarkdown is true.
    expect(preview).toContain('renderMarkdown(content)');

    // Markdown result is rendered into the article via {@html ...} —
    // Svelte auto-escaping is intentionally bypassed for HTML rendering.
    expect(preview).toContain('{@html markdownHtml}');

    // The markdown article carries the correct testid.
    expect(preview).toContain('data-testid="file-preview-markdown"');

    // The article has the markdown-body class (mirrors LibraryDetailPanel).
    expect(preview).toContain('class="markdown-body"');

    // Non-markdown text branch uses the monospace <pre> testid — it is the
    // OTHER branch, confirming the two paths are mutually exclusive.
    expect(preview).toContain('data-testid="file-preview-monospace"');

    // The monospace pre does NOT use {@html ...} (Svelte auto-escapes it).
    expect(preview).not.toMatch(
      /file-preview-monospace[\s\S]{0,80}\{@html/,
    );
  });

  // -------------------------------------------------------------------------
  // US-004 e2eTest 2: Open-in-Claude-Code reuses the shared component
  // -------------------------------------------------------------------------
  it('reuses OpenFileInClaudeCode for open actions — does NOT hand-roll claude:// or route through plugin-shell', () => {
    // Imports the shared component (not a re-implementation).
    expect(preview).toContain(
      "import OpenFileInClaudeCode from './OpenFileInClaudeCode.svelte'",
    );

    // Renders it, passing the selected file path + hqFolderPath as folder.
    expect(preview).toContain('<OpenFileInClaudeCode');
    expect(preview).toContain('file={path}');
    expect(preview).toContain('folder={hqFolderPath}');

    // The open-in-claude-code testid originates from the reused component.
    // FilePreviewPane does NOT independently produce this testid — it comes
    // from the imported component.  The panel source does NOT contain a
    // hand-rolled data-testid="open-in-claude-code" string:
    const previewWithoutImportLine = preview
      .split('\n')
      .filter((l) => !l.includes('OpenFileInClaudeCode'))
      .join('\n');
    expect(previewWithoutImportLine).not.toContain(
      'data-testid="open-in-claude-code"',
    );

    // No hand-rolled claude:// query string in FilePreviewPane source.
    expect(preview).not.toMatch(/claude:\/\/[\w/]*\?/);

    // plugin-shell open() is imported — but used ONLY for Reveal in Finder,
    // not for Open-in-Claude-Code.  Assert it is present (reveal needs it):
    expect(preview).toContain("from '@tauri-apps/plugin-shell'");
    // And that it is NOT invoked near the open-in-claude-code path — the
    // import is used exclusively in the revealInFinder function.
    expect(preview).toContain('async function revealInFinder');
    expect(preview).toContain('await open(absolutePath)');
  });

  // -------------------------------------------------------------------------
  // US-004 e2eTest 3: Binary / oversized drives unsupported placeholder;
  //                   open actions render in header regardless of state
  // -------------------------------------------------------------------------
  it('drives file-preview-unsupported via .catch() and keeps open actions in the header independent of preview state', () => {
    // The .catch() on the invoke sets unsupported = true.
    expect(preview).toContain('.catch(');
    expect(preview).toContain('unsupported = true');

    // The unsupported placeholder is guarded by the unsupported state.
    expect(preview).toContain('data-testid="file-preview-unsupported"');
    expect(preview).toContain('{:else if unsupported}');

    // The preview-actions div (containing open buttons) is inside the header,
    // OUTSIDE the preview-body conditional block — it renders regardless.
    expect(preview).toContain('class="preview-actions"');
    expect(preview).toContain('<header class="preview-header">');

    // Verify structural order: header (with actions) comes BEFORE preview-body.
    const headerIdx = preview.indexOf('<header class="preview-header">');
    const bodyIdx = preview.indexOf('<div class="preview-body">');
    expect(headerIdx).toBeGreaterThan(-1);
    expect(bodyIdx).toBeGreaterThan(-1);
    expect(headerIdx).toBeLessThan(bodyIdx);

    // preview-actions is inside the header, before preview-body.
    const actionsIdx = preview.indexOf('class="preview-actions"');
    expect(actionsIdx).toBeGreaterThan(headerIdx);
    expect(actionsIdx).toBeLessThan(bodyIdx);

    // Reveal in Finder button is also inside the header section
    // (testid is before the preview-body div).
    const revealIdx = preview.indexOf('data-testid="reveal-in-finder"');
    expect(revealIdx).toBeGreaterThan(headerIdx);
    expect(revealIdx).toBeLessThan(bodyIdx);
  });

  // -------------------------------------------------------------------------
  // Additional acceptance criteria: get_company_file_content invocation
  // -------------------------------------------------------------------------
  it('invokes get_company_file_content with { path } and handles rejection as unsupported state', () => {
    // Correct invoke signature: { path } (the path variable, not a literal).
    expect(preview).toContain(
      "invoke<string>('get_company_file_content', { path:",
    );

    // On success, content is set; unsupported = false.
    expect(preview).toContain('content = text');
    expect(preview).toContain('unsupported = false');

    // On rejection, content = null; unsupported = true (binary/oversized path).
    expect(preview).toContain('content = null');
    expect(preview).toContain('unsupported = true');

    // A cancel flag guards against out-of-order completions.
    expect(preview).toContain('let cancelled = false');
    expect(preview).toContain('cancelled = true');
  });

  // -------------------------------------------------------------------------
  // Additional acceptance criteria: Reveal in Finder uses plugin-shell open()
  // -------------------------------------------------------------------------
  it('uses plugin-shell open() for Reveal in Finder only — not for claude:// dispatch', () => {
    // plugin-shell open() is imported.
    expect(preview).toContain("import { open } from '@tauri-apps/plugin-shell'");

    // Reveal button carries the correct testid.
    expect(preview).toContain('data-testid="reveal-in-finder"');

    // open() is called inside the revealInFinder function (the absolute path).
    expect(preview).toContain('await open(absolutePath)');

    // absolutePath is built from hqFolderPath + '/' + path.
    expect(preview).toContain('hqFolderPath');
    expect(preview).toContain('absolutePath');

    // Reveal self-suppresses when hqFolderPath is empty ({#if absolutePath}).
    expect(preview).toContain('{#if absolutePath}');

    // open() is NOT used for any claude:// dispatch in FilePreviewPane.
    expect(preview).not.toMatch(/open\(['"]claude:\/\//);
  });

  // -------------------------------------------------------------------------
  // Additional acceptance criteria (US-009): Files mode wires tree → preview
  // The tree lives in the explorer sidebar; the preview fills the MAIN area.
  // -------------------------------------------------------------------------
  it('Files mode wires the tree (sidebar) and FilePreviewPane (main area) via the selected path', () => {
    // The explorer sidebar owns the tree + file select.
    expect(sidebar).toContain(
      "import CompanyFileTree from '../components/CompanyFileTree.svelte'",
    );
    expect(sidebar).toContain('<CompanyFileTree');
    expect(sidebar).toContain('onselectfile?: (path: string) => void');

    // The shell renders FilePreviewPane in the main content area, driven by the
    // route-carried selected path + the loaded HQ root.
    expect(desktopApp).toContain(
      "import FilePreviewPane from './components/FilePreviewPane.svelte'",
    );
    expect(desktopApp).toContain('<FilePreviewPane path={filesSelectedPath}');
    expect(desktopApp).toContain('hqFolderPath={hqFolderPath ?? \'\'}');

    // A file select flows: tree onselect → onselectfile → files route path.
    expect(desktopApp).toContain('onselectfile={(path) =>');
    expect(desktopApp).toContain(
      "navigate({ kind: 'files', slug: filesActiveSlug ?? undefined, path })",
    );
  });

  // -------------------------------------------------------------------------
  // Additional acceptance criteria: CompanyFileTree selectedPath prop
  // -------------------------------------------------------------------------
  it('CompanyFileTree accepts selectedPath prop, marks selected row with .selected and aria-current', () => {
    // Optional selectedPath prop accepted.
    expect(tree).toContain('selectedPath?');

    // Highlights the row with .selected class.
    expect(tree).toContain('class:selected={node.path === selectedPath}');

    // aria-current="true" on the selected file row.
    expect(tree).toContain("aria-current={node.path === selectedPath ? 'true' : undefined}");
  });

  // -------------------------------------------------------------------------
  // Additional acceptance criteria: no purple, no hardcoded hex in style block
  // (mirrors open-in-claude-code.spec.ts "token-driven" test)
  // -------------------------------------------------------------------------
  it('FilePreviewPane style block is token-driven — no hardcoded hex colors', () => {
    const styleBlock = preview.split('<style>')[1] ?? '';

    // No hardcoded hex color literals (3, 4, 6, or 8 hex digits).
    expect(styleBlock).not.toMatch(/#[0-9a-fA-F]{3,8}\b/);

    // No purple keyword in any form (hard Indigo policy).
    expect(styleBlock.toLowerCase()).not.toContain('purple');
  });

  it('FilesModeSidebar style block is token-driven — no hardcoded hex colors (except the mask sentinel)', () => {
    const styleBlock = sidebar.split('<style>')[1] ?? '';

    // No purple anywhere (hard Indigo policy).
    expect(styleBlock.toLowerCase()).not.toContain('purple');

    // The only #hex allowed is the opaque mask sentinel (#000) used by the
    // name fade-out mask — same pattern as V4Sidebar. No other hex literals.
    const hexMatches = styleBlock.match(/#[0-9a-fA-F]{3,8}\b/g) ?? [];
    expect(hexMatches.every((hex) => hex === '#000')).toBe(true);
  });
});
