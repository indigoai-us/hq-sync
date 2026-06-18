<script lang="ts">
  /**
   * FilesModeSidebar — the file-explorer sidebar that REPLACES the 220px V4
   * primary sidebar when the app is in top-level Files mode (US-009).
   *
   * Obsidian vault-switcher layout: a compact connected-first mini company list
   * at the TOP (reusing the shared US-007 sort so the order matches the primary
   * sidebar's COMPANIES list exactly) and the selected company's file tree
   * BELOW. Picking a company reloads the tree; selecting a file fires
   * `onselectfile` so the shell can preview it in the main content area.
   *
   * The tree is fetched here (mirrors the old CompanyFilesPanel $effect) via the
   * already-noise-filtered `get_company_file_tree` command, keyed on the active
   * slug with a cancel flag. The presentational CompanyFileTree renders the
   * 28px fixed-height rows; this component imposes NO floating-card / box-shadow
   * wrapper (that was the prior overlap bug) — the tree sits flush and scrolls
   * inside its own region.
   *
   * No purple anywhere (hard Indigo policy); V4 tokens only, status as 6px dots.
   */
  import { invoke } from '@tauri-apps/api/core';
  import type { Workspace } from '../../lib/workspaces';
  import type { FileNode } from '../lib/file-tree';
  import CompanyFileTree from '../components/CompanyFileTree.svelte';
  import { sortV4CompaniesConnectedFirst } from './model';
  import './tokens.css';

  interface Props {
    /** `list_syncable_workspaces` workspaces — the mini company list source. */
    companies: Workspace[];
    /** The active company in Files mode (drives the tree below). */
    activeSlug: string | null;
    /** The currently-selected file's HQ-relative path; highlights the tree row. */
    selectedPath: string | null;
    onselectcompany?: (slug: string) => void;
    onselectfile?: (path: string) => void;
  }

  let { companies, activeSlug, selectedPath, onselectcompany, onselectfile }: Props = $props();

  // Connected-first mini list — same ordering as the primary sidebar (US-007).
  const companyRows = $derived(sortV4CompaniesConnectedFirst(companies, activeSlug));

  let tree = $state<FileNode | null>(null);
  let loading = $state(false);
  let error = $state<string | null>(null);

  // Fetch the active company's tree on every slug change. Cancel-flag guards
  // against an out-of-order completion when the user clicks through companies.
  $effect(() => {
    const slug = activeSlug;
    tree = null;
    error = null;

    if (!slug) {
      loading = false;
      return;
    }

    let cancelled = false;
    loading = true;

    void invoke<FileNode>('get_company_file_tree', { slug })
      .then((result) => {
        if (!cancelled) tree = result ?? null;
      })
      .catch((err) => {
        console.error('get_company_file_tree failed:', err);
        if (!cancelled) {
          error = String(err);
          tree = null;
        }
      })
      .finally(() => {
        if (!cancelled) loading = false;
      });

    return () => {
      cancelled = true;
    };
  });

  function handleSelectFile(path: string): void {
    onselectfile?.(path);
  }
</script>

<aside class="files-sidebar" aria-label="Files explorer">
  <div class="fs-companies-area">
    <div class="fs-section-label" id="fs-companies-label">Companies</div>
    <nav class="fs-company-list" aria-labelledby="fs-companies-label">
      {#each companyRows as row (row.slug)}
        <button
          type="button"
          class="fs-company-row"
          class:active={row.slug === activeSlug}
          aria-current={row.slug === activeSlug ? 'page' : undefined}
          onclick={() => onselectcompany?.(row.slug)}
        >
          <span class={`fs-dot ${row.tone}`} aria-hidden="true"></span>
          <span class="fs-company-name">{row.label}</span>
        </button>
      {/each}
    </nav>
  </div>

  <div class="fs-divider" aria-hidden="true"></div>

  <div class="fs-tree-area" aria-label="Company files" aria-busy={loading}>
    {#if !activeSlug}
      <div class="fs-empty">Pick a company to browse its files</div>
    {:else if loading}
      <div class="fs-skeleton" aria-label="Loading files">
        {#each Array(6) as _, index (index)}
          <span style={`width: ${88 - index * 8}%`}></span>
        {/each}
      </div>
    {:else if error}
      <div class="fs-empty" role="alert">Files unavailable</div>
    {:else if tree && tree.children.length > 0}
      <CompanyFileTree root={tree} onselect={handleSelectFile} {selectedPath} />
    {:else}
      <div class="fs-empty">No files yet</div>
    {/if}
  </div>
</aside>

<style>
  .files-sidebar {
    display: flex;
    flex-direction: column;
    flex: 0 0 220px;
    width: 220px;
    min-height: 0;
    height: 100%;
    /* Clip at the sidebar boundary so the only scrollers are the mini company
       list and the tree region (each scrolls inside its own flex track). */
    overflow: hidden;
    padding: 14px 10px 0;
    border-right: 1px solid var(--v4-hairline);
    background: var(--v4-raised);
    font-family:
      'Inter Variable',
      Inter,
      -apple-system,
      'SF Pro Text',
      sans-serif;
  }

  .fs-section-label {
    flex: 0 0 auto;
    margin: 0 0 6px;
    padding: 0 8px;
    color: var(--v4-text-3);
    font-size: var(--text-base);
    font-weight: 400;
    letter-spacing: 0.06em;
    text-transform: uppercase;
  }

  /* Mini company list: caps at ~40% of the sidebar and scrolls on overflow so
     the tree below always gets its own region. */
  .fs-companies-area {
    display: flex;
    flex: 0 1 auto;
    flex-direction: column;
    min-height: 0;
    max-height: 40%;
  }

  .fs-company-list {
    display: flex;
    flex: 1 1 auto;
    flex-direction: column;
    gap: var(--v4-row-gap);
    min-height: 0;
    overflow-y: auto;
    padding-right: 2px;
    scrollbar-color: var(--v4-hairline) transparent;
    scrollbar-width: thin;
  }

  .fs-company-list::-webkit-scrollbar {
    width: 6px;
  }

  .fs-company-list::-webkit-scrollbar-thumb {
    border-radius: 999px;
    background: var(--v4-hairline);
  }

  .fs-company-row {
    display: flex;
    align-items: center;
    gap: 8px;
    box-sizing: border-box;
    width: 100%;
    height: var(--v4-row-h);
    /* Lock the row to exactly --v4-row-h (same pattern as V4Sidebar) so a tall
       glyph or sub-pixel font metrics can never grow/shrink it; flex-shrink:0
       stops the scroll container compressing rows when the list overflows. */
    min-height: var(--v4-row-h);
    max-height: var(--v4-row-h);
    flex: 0 0 auto;
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

  .fs-company-row:hover {
    background: var(--v4-control-faint);
    color: var(--v4-text-1);
  }

  .fs-company-row.active {
    background: var(--v4-active-row);
    color: var(--v4-text-1);
    font-weight: 500;
  }

  .fs-dot {
    flex: 0 0 6px;
    align-self: center;
    width: 6px;
    height: 6px;
    border-radius: 50%;
  }

  .fs-dot.ok {
    background: var(--v4-ok);
  }

  .fs-dot.warn {
    background: var(--v4-warn);
  }

  .fs-dot.error {
    background: var(--v4-error);
  }

  .fs-dot.idle {
    background: var(--v4-idle);
  }

  .fs-company-name {
    flex: 1 1 auto;
    overflow: hidden;
    min-width: 0;
    white-space: nowrap;
    line-height: var(--v4-row-h);
    -webkit-mask-image: linear-gradient(to right, #000 calc(100% - 24px), transparent 100%);
    mask-image: linear-gradient(to right, #000 calc(100% - 24px), transparent 100%);
  }

  .fs-divider {
    flex: 0 0 auto;
    margin: var(--v4-space-3) -10px var(--v4-space-2);
    border-top: 1px solid var(--v4-hairline);
  }

  /* The tree region takes the remaining height and is the tree's scroller. No
     card/box-shadow wrapper (the prior overlap bug); small flush padding only. */
  .fs-tree-area {
    flex: 1 1 auto;
    min-height: 0;
    overflow-y: auto;
    padding: 0 4px 8px;
    scrollbar-color: var(--v4-hairline) transparent;
    scrollbar-width: thin;
  }

  .fs-tree-area::-webkit-scrollbar {
    width: 6px;
  }

  .fs-tree-area::-webkit-scrollbar-thumb {
    border-radius: 999px;
    background: var(--v4-hairline);
  }

  .fs-empty {
    padding: 20px 12px;
    color: var(--v4-text-3);
    font-size: var(--text-base);
    line-height: 1.4;
    text-align: center;
  }

  .fs-skeleton {
    display: grid;
    gap: 10px;
    padding: 8px;
  }

  .fs-skeleton span {
    height: 16px;
    border-radius: 5px;
    background: linear-gradient(
      90deg,
      var(--v4-control-faint),
      var(--v4-hairline),
      var(--v4-control-faint)
    );
    background-size: 200% 100%;
    animation: fs-skeleton 1.2s ease-in-out infinite;
  }

  @keyframes fs-skeleton {
    from {
      background-position: 0 0;
    }
    to {
      background-position: -200% 0;
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .fs-skeleton span {
      animation: none;
    }
  }
</style>
