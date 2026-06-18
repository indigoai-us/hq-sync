import { describe, expect, it } from 'vitest';
import { readRepoFile } from './harness';

/**
 * US-006 — Company "Files" surface: tree renders + a file select drives preview.
 *
 * Source-contract harness (same style as file-preview.spec.ts /
 * open-in-claude-code.spec.ts). Does NOT mount components — asserts on source
 * text to lock down the end-to-end wiring contract of the Files surface so the
 * explorer and its select→preview data flow don't regress.
 *
 * This spec is COMPLEMENTARY to file-preview.spec.ts (US-004), which exercises
 * FilePreviewPane internals (markdown rendering, claude:// reuse, unsupported
 * placeholder). Here the focus is the PANEL-LEVEL integration the US-006
 * e2eTest calls out:
 *   1. The Files surface fetches the tree via get_company_file_tree(slug) and
 *      renders CompanyFileTree only when the tree has children.
 *   2. A file select in the tree (onselect) sets selectedPath, which drives
 *      FilePreviewPane — i.e. selecting a file shows preview content.
 *   3. The tree's onselect contract: files fire onselect(path); folders toggle
 *      expansion only (no select), so previewing is driven by file rows only.
 *   4. The Files surface is reachable: 'files' is a company tab + secondary
 *      sidebar section in route.ts (the surface exists in the IA).
 */

describe('desktop-alt company Files surface — tree + select-drives-preview (US-006)', () => {
  const panel = readRepoFile('src/desktop-alt/panels/CompanyFilesPanel.svelte');
  const tree = readRepoFile('src/desktop-alt/components/CompanyFileTree.svelte');
  const route = readRepoFile('src/desktop-alt/route.ts');

  // -------------------------------------------------------------------------
  // US-006 e2eTest: the Files surface loads the tree and renders it
  // -------------------------------------------------------------------------
  it('fetches the company file tree via get_company_file_tree(slug) and renders CompanyFileTree when it has children', () => {
    // The panel imports the presentational tree component.
    expect(panel).toContain(
      "import CompanyFileTree from '../components/CompanyFileTree.svelte'",
    );

    // It fetches the tree for the active company slug.
    expect(panel).toContain(
      "invoke<FileNode>('get_company_file_tree', { slug })",
    );

    // The tree is rendered with the fetched root.
    expect(panel).toContain('<CompanyFileTree');
    expect(panel).toContain('root={tree}');

    // The tree only renders when there is content; otherwise the empty/loading
    // states show (so an empty company doesn't render a bare tree).
    expect(panel).toContain('tree && tree.children.length > 0');
    expect(panel).toContain("No files yet");
  });

  // -------------------------------------------------------------------------
  // US-006 e2eTest: selecting a file shows preview content (select→preview)
  // -------------------------------------------------------------------------
  it('a file select sets selectedPath which drives FilePreviewPane (select → preview)', () => {
    // The panel imports the preview pane.
    expect(panel).toContain(
      "import FilePreviewPane from '../components/FilePreviewPane.svelte'",
    );

    // selectedPath state drives the preview.
    expect(panel).toContain('let selectedPath = $state<string | null>(null)');

    // The tree's onselect is bound to a handler that sets selectedPath.
    expect(panel).toContain('onselect={handleSelect}');
    expect(panel).toMatch(
      /function handleSelect\(path: string\): void \{\s*selectedPath = path;\s*\}/,
    );

    // The preview pane is rendered with the selected path (the data flow:
    // tree select → selectedPath → FilePreviewPane path).
    expect(panel).toContain('path={selectedPath}');
    expect(panel).toContain('<FilePreviewPane path={selectedPath}');

    // The preview is conditional on a selection — before any select, an empty
    // prompt shows instead of a preview.
    expect(panel).toContain('{#if selectedPath}');
    expect(panel).toContain('Select a file to preview it');

    // The selected path is also passed back to the tree for row highlighting,
    // closing the loop (selection is reflected in both panes).
    expect(panel).toContain('<CompanyFileTree root={tree} onselect={handleSelect} {selectedPath} />');
  });

  // -------------------------------------------------------------------------
  // US-006 e2eTest: files drive select; folders only toggle (no preview)
  // -------------------------------------------------------------------------
  it('CompanyFileTree fires onselect for file rows only — folders toggle expansion without selecting', () => {
    // onselect is an optional callback carrying the node path.
    expect(tree).toContain('onselect?: (path: string) => void');

    // Row click routes dirs to toggle() and files to onselect() — the two are
    // mutually exclusive (folders never fire a select, so they never preview).
    expect(tree).toMatch(
      /function onRowClick\(node: FileNode\): void \{[\s\S]*if \(node\.isDir\) \{[\s\S]*toggle\(node\.path\);[\s\S]*\} else \{[\s\S]*onselect\?\.\(node\.path\);[\s\S]*\}[\s\S]*\}/,
    );

    // File rows are buttons that invoke onRowClick (the select path).
    expect(tree).toContain('class="ft-row ft-file"');
    expect(tree).toContain('onclick={() => onRowClick(node)}');

    // The tree renders rows from the flattenTree display order (US-002 lib).
    expect(tree).toContain(
      "import { flattenTree, type FileNode } from '../lib/file-tree'",
    );
    expect(tree).toContain('flattenTree(root?.children ?? []');
  });

  // -------------------------------------------------------------------------
  // US-006: the Files surface is reachable in the desktop-alt IA
  // -------------------------------------------------------------------------
  it("exposes a 'files' company tab + secondary-sidebar section so the surface is reachable", () => {
    // 'files' is a valid company tab.
    expect(route).toMatch(/'files'/);
    // The Files surface is listed among the company sections (secondary sidebar)
    // with the 'files' tab id + 'Files' label.
    expect(route).toMatch(/id: 'files', label: 'Files'/);
  });
});
