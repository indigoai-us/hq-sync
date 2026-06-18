<script lang="ts">
  /**
   * CompanyFilesPanel — the company "Files" secondary-sidebar surface (US-003).
   *
   * Loads the active company's LOCAL file tree via the `get_company_file_tree`
   * Tauri command (US-001) and renders it with the presentational
   * `CompanyFileTree` (US-002). Files are local-on-disk, so this panel does NOT
   * gate on `cloudBacked` — the prop is accepted to match the sibling panels'
   * shape but the tree exists regardless of cloud state.
   *
   * Selecting a file fetches + previews it (US-004): the tree and a
   * `FilePreviewPane` lay out side-by-side, the preview self-fetches the file
   * content, and the open actions (Open in Claude Code / Reveal in Finder) live
   * in the preview header. `hqFolderPath` is loaded once here (the data owner)
   * via `get_config` and passed down so the preview can build the claude://
   * session folder and the absolute reveal path.
   */
  import { invoke } from '@tauri-apps/api/core';
  import type { FileNode } from '../lib/file-tree';
  import CompanyFileTree from '../components/CompanyFileTree.svelte';
  import FilePreviewPane from '../components/FilePreviewPane.svelte';

  interface Props {
    slug: string;
    cloudBacked?: boolean;
  }

  // `cloudBacked` is accepted for prop-shape parity with the other panels but is
  // intentionally not consumed — local files exist whether or not the company is
  // cloud-backed.
  let { slug }: Props = $props();

  let tree = $state<FileNode | null>(null);
  let loading = $state(false);
  let error = $state<string | null>(null);
  let reloadToken = $state(0);
  // The currently-selected file's HQ-relative path; drives the preview pane.
  let selectedPath = $state<string | null>(null);
  // Absolute HQ root for the open actions (claude:// session folder + the
  // absolute reveal path). Loaded once via get_config; empty until resolved.
  let hqFolderPath = $state('');

  // Load the HQ root once (mirrors SecretsPanel / ActivityPanel). Errors fall
  // back to '' — the preview's open actions self-suppress when the root is
  // empty, so a config-load failure degrades gracefully rather than throwing.
  $effect(() => {
    let cancelled = false;
    void invoke<{ hqFolderPath?: string }>('get_config')
      .then((config) => {
        if (!cancelled) hqFolderPath = config.hqFolderPath ?? '';
      })
      .catch((err) => {
        console.error('get_config failed:', err);
        if (!cancelled) hqFolderPath = '';
      });
    return () => {
      cancelled = true;
    };
  });

  $effect(() => {
    reloadToken;
    tree = null;
    error = null;
    selectedPath = null;

    if (!slug) {
      loading = false;
      return;
    }

    let cancelled = false;
    loading = true;

    void invoke<FileNode>('get_company_file_tree', { slug })
      .then((result) => {
        if (!cancelled) {
          tree = result ?? null;
        }
      })
      .catch((err) => {
        console.error('get_company_file_tree failed:', err);
        if (!cancelled) {
          error = String(err);
          tree = null;
        }
      })
      .finally(() => {
        if (!cancelled) {
          loading = false;
        }
      });

    return () => {
      cancelled = true;
    };
  });

  function retry() {
    reloadToken += 1;
  }

  function handleSelect(path: string): void {
    selectedPath = path;
  }
</script>

<section class="files-panel" aria-labelledby="files-panel-title">
  <header class="files-header">
    <div class="files-title">
      <h2 id="files-panel-title">Files</h2>
      <span>{loading ? 'Loading files' : 'Local company files'}</span>
    </div>
  </header>

  {#if error}
    <div class="files-error" role="alert">
      <div>
        <strong>Files unavailable</strong>
        <span>{error}</span>
      </div>
      <button type="button" onclick={retry}>Retry</button>
    </div>
  {/if}

  <section class="files-card" aria-labelledby="files-tree-title" aria-busy={loading}>
    <header class="card-header">
      <h3 id="files-tree-title">{tree?.name ?? slug}</h3>
    </header>

    {#if loading}
      <div class="files-skeleton" aria-label="Loading files">
        {#each Array(5) as _, index (index)}
          <span style={`width: ${88 - index * 8}%`}></span>
        {/each}
      </div>
    {:else if tree && tree.children.length > 0}
      <div class="files-split">
        <div class="files-tree-col">
          <CompanyFileTree root={tree} onselect={handleSelect} {selectedPath} />
        </div>
        <div class="files-preview-col">
          {#if selectedPath}
            <FilePreviewPane path={selectedPath} {hqFolderPath} />
          {:else}
            <div class="preview-empty">Select a file to preview it</div>
          {/if}
        </div>
      </div>
    {:else if !error}
      <div class="empty-state">No files yet</div>
    {/if}
  </section>
</section>

<style>
  .files-panel {
    display: grid;
    gap: 12px;
    min-width: 0;
  }

  .files-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 16px;
    min-width: 0;
  }

  .files-title {
    min-width: 0;
  }

  .files-title h2 {
    margin: 0;
    color: var(--fg);
    font-size: var(--text-base);
    font-weight: 600;
    line-height: 22px;
  }

  .files-title span,
  .card-header h3,
  .empty-state {
    color: var(--muted);
    font-size: var(--text-base);
    line-height: 16px;
  }

  .files-title span {
    display: block;
    margin-top: 2px;
  }

  .files-error {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 14px;
    min-width: 0;
    padding: 12px;
    border: 1px solid rgba(245, 158, 11, 0.3);
    border-radius: 8px;
    background: rgba(245, 158, 11, 0.1);
    color: var(--amber);
  }

  .files-error div {
    display: grid;
    gap: 3px;
    min-width: 0;
  }

  .files-error strong,
  .files-error span {
    min-width: 0;
    overflow-wrap: anywhere;
  }

  .files-error strong {
    font-size: var(--text-base);
    line-height: 18px;
  }

  .files-error span {
    font-size: var(--text-base);
    line-height: 16px;
  }

  .files-error button {
    height: 30px;
    min-width: 0;
    padding: 0 11px;
    border: 1px solid var(--border);
    border-radius: 5px;
    background: transparent;
    color: var(--fg);
    font: inherit;
    font-size: var(--text-base);
    font-weight: 600;
    white-space: nowrap;
    cursor: pointer;
  }

  .files-error button:hover {
    border-color: var(--border-strong);
    background: var(--row-hover);
  }

  .files-card {
    min-width: 0;
    border: 1px solid var(--border);
    border-radius: 8px;
    background: var(--bg);
    box-shadow: 0 1px 2px rgba(0, 0, 0, 0.4);
  }

  .card-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
    min-width: 0;
    padding: 11px 13px;
    border-bottom: 1px solid var(--border);
  }

  .card-header h3 {
    min-width: 0;
    margin: 0;
    overflow: hidden;
    color: var(--muted-2);
    font-weight: 600;
    line-height: 18px;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  /* Tree + preview side-by-side. Tree column is width-limited; preview flexes.
     Both get min-width:0 and their own scroll so long content scrolls inside
     the card rather than pushing the window wide. A bounded max-height keeps the
     split from growing past the viewport. */
  .files-split {
    display: grid;
    grid-template-columns: minmax(180px, 280px) minmax(0, 1fr);
    align-items: stretch;
    min-width: 0;
    max-height: min(640px, calc(100vh - 220px));
  }

  .files-tree-col {
    min-width: 0;
    padding: 8px;
    overflow: auto;
    border-right: 1px solid var(--border);
  }

  .files-preview-col {
    display: flex;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
  }

  .preview-empty {
    display: grid;
    flex: 1 1 auto;
    place-items: center;
    min-height: 160px;
    padding: 26px 13px;
    color: var(--muted);
    font-size: var(--text-base);
    text-align: center;
  }

  @media (max-width: 720px) {
    .files-split {
      grid-template-columns: 1fr;
      max-height: none;
    }

    .files-tree-col {
      border-right: 0;
      border-bottom: 1px solid var(--border);
    }
  }

  .files-skeleton {
    display: grid;
    gap: 10px;
    padding: 14px 13px;
  }

  .files-skeleton span {
    height: 18px;
    border-radius: 5px;
    background: linear-gradient(
      90deg,
      var(--v4-control-faint),
      var(--v4-hairline),
      var(--v4-control-faint)
    );
    background-size: 200% 100%;
    animation: skeleton 1.2s ease-in-out infinite;
  }

  .empty-state {
    padding: 26px 13px;
    text-align: center;
  }

  @keyframes skeleton {
    from {
      background-position: 0 0;
    }

    to {
      background-position: -200% 0;
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .files-skeleton span {
      animation: none;
    }
  }
</style>
