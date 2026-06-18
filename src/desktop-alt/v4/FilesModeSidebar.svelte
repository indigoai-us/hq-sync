<script lang="ts">
  /**
   * FilesModeSidebar — the file-explorer sidebar that REPLACES the 220px V4
   * primary sidebar when the app is in top-level Files mode (US-009, reworked in
   * US-010).
   *
   * US-010 changes the model in three ways, all driven by live-beta feedback:
   *  1. EXIT CONTROL — a clear back/exit button in the header returns the app to
   *     the previous view (default Home) and restores the normal sidebar.
   *     Leaving Files mode is reversible without restarting.
   *  2. ROOT-BY-DEFAULT — with no company selected the tree shows the HQ ROOT
   *     (top-level `companies/`, `repos/`, `core/`, `personal/`, `workspace/`…),
   *     noise-filtered. The mini company list is an OPTIONAL FILTER, not a
   *     prerequisite.
   *  3. COMPANY-AS-FILTER — selecting a company scopes the tree to
   *     `companies/<slug>/`; an active-filter chip shows the scope and clears
   *     it back to the full root. The filter simply sets the tree's root path.
   *
   * The tree LAZY-LOADS children per folder (US-010) via the `list_hq_dir`
   * command so the large HQ root (esp. `repos/`) is never eagerly walked.
   * Selecting a file fires `onselectfile` so the shell previews it in the main
   * content area.
   *
   * No purple anywhere (hard Indigo policy); V4 tokens only, status as 6px dots.
   */
  import { invoke } from '@tauri-apps/api/core';
  import type { Workspace } from '../../lib/workspaces';
  import type { DirEntry } from '../lib/file-tree';
  import CompanyFileTree from '../components/CompanyFileTree.svelte';
  import { sortV4CompaniesConnectedFirst } from './model';
  import './tokens.css';

  interface Props {
    /** `list_syncable_workspaces` workspaces — the mini company list source. */
    companies: Workspace[];
    /** The active company FILTER (null = show the full HQ root). */
    activeSlug: string | null;
    /** The currently-selected file's HQ-relative path; highlights the tree row. */
    selectedPath: string | null;
    /** Toggle the company filter (slug to scope, null to clear back to root). */
    onselectcompany?: (slug: string | null) => void;
    onselectfile?: (path: string) => void;
    /** Leave Files mode and restore the previous view (default Home). */
    onexit?: () => void;
  }

  let {
    companies,
    activeSlug,
    selectedPath,
    onselectcompany,
    onselectfile,
    onexit,
  }: Props = $props();

  // Connected-first mini list — same ordering as the primary sidebar (US-007).
  const companyRows = $derived(sortV4CompaniesConnectedFirst(companies, activeSlug));

  // The active company's display label (for the filter chip).
  const activeLabel = $derived(
    activeSlug ? (companyRows.find((row) => row.slug === activeSlug)?.label ?? activeSlug) : null,
  );

  // The tree's root path: HQ root by default, scoped to the company subtree
  // when the filter is active. This is the ONLY thing the filter changes.
  const treeRootPath = $derived(activeSlug ? `companies/${activeSlug}` : '');

  // Lazy children loader — the per-directory `list_hq_dir` command. Returns one
  // directory's immediate children (noise-filtered, path-guarded in Rust).
  function loadChildren(relPath: string): Promise<DirEntry[]> {
    return invoke<DirEntry[]>('list_hq_dir', { relPath });
  }

  function handleSelectFile(path: string): void {
    onselectfile?.(path);
  }
</script>

<aside class="files-sidebar" aria-label="Files explorer">
  <div class="fs-header">
    <button type="button" class="fs-exit" onclick={() => onexit?.()}>
      <span class="fs-exit-icon" aria-hidden="true">
        <svg viewBox="0 0 16 16" width="14" height="14">
          <path d="M10 3.5 L5.5 8 L10 12.5" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" />
        </svg>
      </span>
      <span class="fs-exit-label">Back</span>
    </button>
    <span class="fs-title">Files</span>
  </div>

  <div class="fs-companies-area">
    <div class="fs-section-label" id="fs-companies-label">Filter by company</div>
    <nav class="fs-company-list" aria-labelledby="fs-companies-label">
      {#each companyRows as row (row.slug)}
        <button
          type="button"
          class="fs-company-row"
          class:active={row.slug === activeSlug}
          aria-pressed={row.slug === activeSlug}
          onclick={() =>
            onselectcompany?.(row.slug === activeSlug ? null : row.slug)}
        >
          <span class={`fs-dot ${row.tone}`} aria-hidden="true"></span>
          <span class="fs-company-name">{row.label}</span>
        </button>
      {/each}
    </nav>
  </div>

  <div class="fs-divider" aria-hidden="true"></div>

  <div class="fs-scope">
    {#if activeSlug}
      <span class="fs-scope-chip">
        <span class="fs-scope-label">companies/{activeSlug}</span>
        <button
          type="button"
          class="fs-scope-clear"
          aria-label={`Clear ${activeLabel} filter`}
          title="Clear filter"
          onclick={() => onselectcompany?.(null)}
        >
          <svg viewBox="0 0 12 12" width="11" height="11" aria-hidden="true">
            <path d="M3 3 L9 9 M9 3 L3 9" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" />
          </svg>
        </button>
      </span>
    {:else}
      <span class="fs-scope-root">HQ root</span>
    {/if}
  </div>

  <div class="fs-tree-area" aria-label="File tree">
    {#key treeRootPath}
      <CompanyFileTree
        rootPath={treeRootPath}
        {loadChildren}
        onselect={handleSelectFile}
        {selectedPath}
      />
    {/key}
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

  /* Header: a prominent Back/exit control + the mode title. */
  .fs-header {
    display: flex;
    flex: 0 0 auto;
    align-items: center;
    gap: 8px;
    margin: 0 0 var(--v4-space-3);
    padding: 0 2px;
  }

  .fs-exit {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    height: 26px;
    padding: 0 10px 0 6px;
    border: 1px solid var(--v4-hairline);
    border-radius: 6px;
    background: var(--v4-control-faint);
    color: var(--v4-text-1);
    font: inherit;
    font-size: var(--text-base);
    font-weight: 500;
    cursor: pointer;
  }

  .fs-exit:hover {
    background: var(--v4-control-bg);
    border-color: var(--v4-text-3);
  }

  .fs-exit-icon {
    display: inline-flex;
    align-items: center;
    justify-content: center;
  }

  .fs-title {
    color: var(--v4-text-3);
    font-size: var(--text-base);
    font-weight: 600;
    letter-spacing: 0.04em;
    text-transform: uppercase;
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

  /* Mini company list: caps at ~36% of the sidebar and scrolls on overflow so
     the tree below always gets its own region. */
  .fs-companies-area {
    display: flex;
    flex: 0 1 auto;
    flex-direction: column;
    min-height: 0;
    max-height: 36%;
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

  /* Active-scope indicator: shows whether the tree is the full HQ root or a
     company filter, and offers a clear affordance for the latter. */
  .fs-scope {
    display: flex;
    flex: 0 0 auto;
    align-items: center;
    min-height: 22px;
    margin: 0 0 var(--v4-space-2);
    padding: 0 6px;
  }

  .fs-scope-root {
    color: var(--v4-text-3);
    font-size: var(--text-base);
    font-weight: 500;
  }

  .fs-scope-chip {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    max-width: 100%;
    padding: 2px 4px 2px 8px;
    border: 1px solid var(--v4-hairline);
    border-radius: 999px;
    background: var(--v4-control-faint);
    color: var(--v4-text-1);
    font-size: var(--text-base);
    font-weight: 500;
  }

  .fs-scope-label {
    overflow: hidden;
    min-width: 0;
    white-space: nowrap;
    text-overflow: ellipsis;
  }

  .fs-scope-clear {
    display: inline-flex;
    flex: 0 0 auto;
    align-items: center;
    justify-content: center;
    width: 16px;
    height: 16px;
    padding: 0;
    border: none;
    border-radius: 50%;
    background: transparent;
    color: var(--v4-text-3);
    cursor: pointer;
  }

  .fs-scope-clear:hover {
    background: var(--v4-control-bg);
    color: var(--v4-text-1);
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
</style>
