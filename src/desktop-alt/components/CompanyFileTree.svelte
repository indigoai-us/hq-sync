<script lang="ts">
  /**
   * CompanyFileTree — Obsidian-style collapsible folder tree for the selected
   * company's local files (US-002).
   *
   * PRESENTATIONAL only: it receives the `get_company_file_tree` result as a
   * `root: FileNode` prop and renders its children as an indented, collapsible
   * tree. It never calls `invoke()` — US-003 wires the data in. Folders expand /
   * collapse with a chevron and clicking a folder toggles expansion only; files
   * are leaf rows and clicking one fires `onselect(node.path)` with the
   * HQ-folder-relative path. Expansion state (a `Set<string>` of expanded paths)
   * is held in component `$state`, so it persists across re-renders within the
   * same company session.
   *
   * Styling mirrors V4SecondarySidebar's `.v4-row` look (28px fixed height, 6px
   * radius, faint hover) using the V4 tokens. No purple anywhere (hard Indigo
   * policy); this tree needs no status color at all.
   */
  import { flattenTree, type FileNode } from '../lib/file-tree';
  import '../v4/tokens.css';

  interface Props {
    /**
     * The tree root (the company node, whose `name` is the slug). Its children
     * are rendered; the root row itself is not shown. The data contract from
     * US-001's `get_company_file_tree` returns exactly this shape.
     */
    root: FileNode;
    /** Fired when a FILE row is activated, with its HQ-folder-relative path. */
    onselect?: (path: string) => void;
    /** The currently-selected file path; the matching row is highlighted. */
    selectedPath?: string | null;
  }

  let { root, onselect, selectedPath = null }: Props = $props();

  // Expansion state keyed by node `path`. Held here (not derived from props) so
  // toggling persists across prop ticks within the same company session.
  // Default: collapsed (top-level folders start closed) for a calm deep tree.
  let expanded = $state(new Set<string>());

  // Flatten to display order on every render. `flattenTree` sorts internally
  // (folders-before-files, case-insensitive alphabetical), so we are robust to
  // unsorted input and independently correct from the Rust side.
  const rows = $derived(
    flattenTree(root?.children ?? [], (path) => expanded.has(path)),
  );

  function toggle(path: string): void {
    // Reassign a NEW Set so Svelte's reactivity picks up the change.
    const next = new Set(expanded);
    if (next.has(path)) {
      next.delete(path);
    } else {
      next.add(path);
    }
    expanded = next;
  }

  function onRowClick(node: FileNode): void {
    if (node.isDir) {
      toggle(node.path); // Folders toggle expansion only — no select fires.
    } else {
      onselect?.(node.path);
    }
  }
</script>

<div class="file-tree" role="tree" aria-label={`${root?.name ?? 'Company'} files`}>
  {#each rows as { node, depth } (node.path)}
    {#if node.isDir}
      <button
        type="button"
        class="ft-row ft-dir"
        style={`padding-left: ${8 + depth * 14}px`}
        aria-expanded={expanded.has(node.path)}
        onclick={() => onRowClick(node)}
      >
        <span class="ft-chevron" class:open={expanded.has(node.path)} aria-hidden="true">
          <svg viewBox="0 0 12 12" width="12" height="12">
            <path d="M4.5 2.5 L8 6 L4.5 9.5" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round" />
          </svg>
        </span>
        <span class="ft-label">{node.name}</span>
      </button>
    {:else}
      <button
        type="button"
        class="ft-row ft-file"
        class:selected={node.path === selectedPath}
        aria-current={node.path === selectedPath ? 'true' : undefined}
        style={`padding-left: ${8 + (depth + 1) * 14}px`}
        onclick={() => onRowClick(node)}
      >
        <span class="ft-label">{node.name}</span>
      </button>
    {/if}
  {/each}
</div>

<style>
  .file-tree {
    display: flex;
    flex-direction: column;
    gap: 1px;
    min-width: 0;
    font-family:
      'Inter Variable',
      Inter,
      -apple-system,
      'SF Pro Text',
      sans-serif;
  }

  .ft-row {
    display: flex;
    align-items: center;
    gap: 6px;
    width: 100%;
    height: 28px;
    padding: 0 8px;
    border: none;
    border-radius: 6px;
    background: transparent;
    color: var(--v4-text-2);
    font: inherit;
    font-size: var(--text-base);
    font-weight: 400;
    line-height: 1;
    text-align: left;
    cursor: pointer;
  }

  .ft-row:hover {
    background: var(--v4-control-faint);
    color: var(--v4-text-1);
  }

  /* Selected file row — neutral emphasis (no purple, hard Indigo policy). */
  .ft-row.selected {
    background: var(--v4-control-bg);
    color: var(--v4-text-1);
    font-weight: 600;
  }

  .ft-label {
    overflow: hidden;
    min-width: 0;
    flex: 1 1 auto;
    white-space: nowrap;
    text-overflow: ellipsis;
  }

  .ft-chevron {
    display: inline-flex;
    flex: 0 0 12px;
    align-items: center;
    justify-content: center;
    width: 12px;
    height: 12px;
    color: var(--v4-text-3);
    transition: transform 0.12s ease;
  }

  .ft-chevron.open {
    transform: rotate(90deg);
  }

  /* Files have no chevron; pad the label so it aligns past the chevron column. */
  .ft-file .ft-label {
    padding-left: 18px;
  }
</style>
