<script lang="ts">
  /**
   * CompanyFileTree — Obsidian-style collapsible folder tree (US-002, made LAZY
   * in US-010).
   *
   * The tree now LOADS CHILDREN ON FOLDER EXPAND instead of consuming a fully
   * pre-walked tree. This keeps the large HQ root (esp. `repos/`) fast: only
   * the visible level is fetched, and a folder's children are requested the
   * first time it is expanded.
   *
   * Data flow:
   *  - `rootPath` is the HQ-relative directory the tree is rooted at (`''` = HQ
   *    root; `companies/<slug>` when the company filter is active). Changing it
   *    resets the tree and reloads the top level.
   *  - `loadChildren(relPath)` is supplied by the parent and returns that
   *    directory's immediate children as `DirEntry[]` (the lazy `list_hq_dir`
   *    command). This component never calls `invoke()` directly — it stays
   *    presentational + cache-managing.
   *
   * Clicking a folder toggles expansion (and lazily fetches its children once);
   * clicking a file fires `onselect(node.path)` with the HQ-relative path.
   *
   * Styling mirrors V4SecondarySidebar's `.v4-row` (28px fixed height, 6px
   * radius, faint hover) via V4 tokens. No purple (hard Indigo policy).
   */
  import {
    dirEntryToLazyNode,
    flattenLazy,
    type DirEntry,
    type LazyNode,
  } from '../lib/file-tree';
  import '../v4/tokens.css';

  interface Props {
    /**
     * HQ-relative directory the tree is rooted at. `''` (or `'.'`) = HQ root;
     * `companies/<slug>` scopes to a company. The root row itself is not shown —
     * its immediate children are the top-level rows.
     */
    rootPath?: string;
    /** Lazy children loader (the `list_hq_dir` command), supplied by the parent. */
    loadChildren: (relPath: string) => Promise<DirEntry[]>;
    /** Fired when a FILE row is activated, with its HQ-folder-relative path. */
    onselect?: (path: string) => void;
    /** The currently-selected file path; the matching row is highlighted. */
    selectedPath?: string | null;
  }

  let { rootPath = '', loadChildren, onselect, selectedPath = null }: Props = $props();

  // The lazily-built top-level node list (children of `rootPath`).
  let roots = $state<LazyNode[]>([]);
  // Expansion state keyed by node `path`. Reassigned (new Set) on every change
  // so Svelte's reactivity fires.
  let expanded = $state(new Set<string>());
  // Per-directory load state keyed by path so a spinner / re-fetch guard works.
  let loadingPaths = $state(new Set<string>());
  let rootLoading = $state(false);
  let rootError = $state<string | null>(null);

  // (Re)load the top level whenever the root path changes. A cancel flag guards
  // against an out-of-order completion when the user switches the filter fast.
  $effect(() => {
    const base = rootPath;
    roots = [];
    expanded = new Set();
    loadingPaths = new Set();
    rootError = null;
    rootLoading = true;

    let cancelled = false;
    void loadChildren(base)
      .then((entries) => {
        if (!cancelled) roots = entries.map(dirEntryToLazyNode);
      })
      .catch((err) => {
        console.error('list_hq_dir failed:', err);
        if (!cancelled) {
          rootError = String(err);
          roots = [];
        }
      })
      .finally(() => {
        if (!cancelled) rootLoading = false;
      });

    return () => {
      cancelled = true;
    };
  });

  const rows = $derived(flattenLazy(roots, (path) => expanded.has(path)));

  /** Recursively replace the node at `path` with its loaded children. Pure-ish:
   *  returns a NEW array so Svelte sees the change. */
  function withLoadedChildren(
    nodes: LazyNode[],
    path: string,
    children: LazyNode[],
  ): LazyNode[] {
    return nodes.map((node) => {
      if (node.path === path) {
        return { ...node, loaded: true, children };
      }
      if (node.isDir && node.children && path.startsWith(`${node.path}/`)) {
        return { ...node, children: withLoadedChildren(node.children, path, children) };
      }
      return node;
    });
  }

  async function ensureLoaded(node: LazyNode): Promise<void> {
    if (node.loaded || loadingPaths.has(node.path)) return;
    const next = new Set(loadingPaths);
    next.add(node.path);
    loadingPaths = next;
    try {
      const entries = await loadChildren(node.path);
      roots = withLoadedChildren(roots, node.path, entries.map(dirEntryToLazyNode));
    } catch (err) {
      console.error('list_hq_dir failed:', err);
      // Mark loaded with empty children so we don't spin forever; the empty
      // folder simply shows nothing under it.
      roots = withLoadedChildren(roots, node.path, []);
    } finally {
      const done = new Set(loadingPaths);
      done.delete(node.path);
      loadingPaths = done;
    }
  }

  function toggle(node: LazyNode): void {
    const next = new Set(expanded);
    if (next.has(node.path)) {
      next.delete(node.path);
    } else {
      next.add(node.path);
      // Lazily fetch this folder's children the first time it opens.
      if (!node.loaded) void ensureLoaded(node);
    }
    expanded = next;
  }

  function onRowClick(node: LazyNode): void {
    if (node.isDir) {
      toggle(node); // Folders toggle expansion only — no select fires.
    } else {
      onselect?.(node.path);
    }
  }
</script>

<div class="file-tree" role="tree" aria-label="Files">
  {#if rootLoading}
    <div class="ft-status" aria-label="Loading files">Loading…</div>
  {:else if rootError}
    <div class="ft-status" role="alert">Files unavailable</div>
  {:else if rows.length === 0}
    <div class="ft-status">No files</div>
  {:else}
    {#each rows as { node, depth } (node.path)}
      {#if node.isDir}
        <button
          type="button"
          class="ft-row ft-dir"
          style={`padding-left: ${8 + depth * 14}px`}
          aria-expanded={expanded.has(node.path)}
          onclick={() => onRowClick(node)}
        >
          <span
            class="ft-chevron"
            class:open={expanded.has(node.path)}
            class:hidden={!node.hasChildren}
            aria-hidden="true"
          >
            <svg viewBox="0 0 12 12" width="12" height="12">
              <path d="M4.5 2.5 L8 6 L4.5 9.5" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round" />
            </svg>
          </span>
          <span class="ft-label">{node.name}</span>
          {#if loadingPaths.has(node.path)}
            <span class="ft-spinner" aria-hidden="true"></span>
          {/if}
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
  {/if}
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

  /* Empty dirs keep the chevron column for alignment but hide the glyph. */
  .ft-chevron.hidden {
    visibility: hidden;
  }

  /* Files have no chevron; pad the label so it aligns past the chevron column. */
  .ft-file .ft-label {
    padding-left: 18px;
  }

  .ft-spinner {
    flex: 0 0 10px;
    width: 10px;
    height: 10px;
    border: 1.5px solid var(--v4-hairline);
    border-top-color: var(--v4-text-3);
    border-radius: 50%;
    animation: ft-spin 0.7s linear infinite;
  }

  @keyframes ft-spin {
    to {
      transform: rotate(360deg);
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .ft-spinner {
      animation: none;
    }
  }

  .ft-status {
    padding: 12px;
    color: var(--v4-text-3);
    font-size: var(--text-base);
    text-align: center;
  }
</style>
