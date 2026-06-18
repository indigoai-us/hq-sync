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
   * Selecting a file is a placeholder no-op here: it only records the selected
   * path in local `$state`. The preview pane / open-in-editor actions are US-004,
   * so this panel intentionally imports no preview or open component.
   */
  import { invoke } from '@tauri-apps/api/core';
  import type { FileNode } from '../lib/file-tree';
  import CompanyFileTree from '../components/CompanyFileTree.svelte';

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
  // Placeholder selection only — US-004 builds the preview from this.
  let selectedPath = $state<string | null>(null);

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
    // Placeholder selection state only. US-004 wires the preview pane.
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
      <div class="files-tree-wrap">
        <CompanyFileTree root={tree} onselect={handleSelect} />
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

  .files-tree-wrap {
    padding: 8px;
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
