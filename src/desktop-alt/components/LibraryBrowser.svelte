<script lang="ts">
  /**
   * LibraryBrowser — the shared body of the Library surface, used by both the
   * root (all-scopes) LibraryPage and the per-company CompanyLibraryPanel. Owns
   * the Workers|Skills toggle, a multi-select **scope filter** (Core / Personal /
   * per-company), the text filter, the card grid, and the detail slide-over.
   * Data (items/loading/error) is loaded by the caller and passed in.
   */
  import { onMount } from 'svelte';
  import {
    facetLabel,
    libraryFacets,
    libraryItemFacet,
    toLibraryItems,
    type LibraryItem,
    type LibraryItems,
  } from '../lib/library';
  import LibraryList from './LibraryList.svelte';
  import LibraryDetailPanel from './LibraryDetailPanel.svelte';
  import MarketplacePanel from '../panels/MarketplacePanel.svelte';

  interface Props {
    /** The loaded library payload (workers + skills) for this scope. */
    items: LibraryItems;
    /** Whether the data is still loading. */
    loading?: boolean;
    /** Error string if the load failed. */
    error?: string | null;
  }

  let { items, loading = false, error = null }: Props = $props();

  type Filter = 'all' | 'workers' | 'skills' | 'marketplace';
  let filter = $state<Filter>('all');
  // The Marketplace tab is a self-contained surface (its own fetch + search +
  // detail slide-over via MarketplacePanel), so the library toolbar's scope
  // filter and text search don't apply while it's active.
  const isMarketplace = $derived(filter === 'marketplace');
  let query = $state('');
  let selected = $state<LibraryItem | null>(null);

  const allItems = $derived(toLibraryItems(items));
  const facets = $derived(libraryFacets(allItems));

  // Selected scope facets. New facets default ON (visible); a user's explicit
  // deselection persists across reloads. Tracked via a "known" set so a freshly
  // appearing company doesn't silently reset the user's choices.
  let selectedFacets = $state<Set<string>>(new Set());
  let knownFacets = $state<Set<string>>(new Set());
  $effect(() => {
    let changed = false;
    for (const f of facets) {
      if (!knownFacets.has(f)) {
        knownFacets.add(f);
        selectedFacets.add(f);
        changed = true;
      }
    }
    if (changed) selectedFacets = new Set(selectedFacets);
  });

  const showScopeFilter = $derived(facets.length > 1);
  let scopeMenuOpen = $state(false);

  const facetFiltered = $derived(
    showScopeFilter
      ? allItems.filter((item) => selectedFacets.has(libraryItemFacet(item)))
      : allItems,
  );

  const workerCount = $derived(facetFiltered.filter((i) => i.kind === 'worker').length);
  const skillCount = $derived(facetFiltered.filter((i) => i.kind === 'skill').length);

  const scopedItems = $derived(
    filter === 'workers'
      ? facetFiltered.filter((item) => item.kind === 'worker')
      : filter === 'skills'
        ? facetFiltered.filter((item) => item.kind === 'skill')
        : facetFiltered,
  );

  const allFacetsSelected = $derived(
    facets.length > 0 && facets.every((f) => selectedFacets.has(f)),
  );
  const scopeButtonLabel = $derived(
    allFacetsSelected
      ? 'All scopes'
      : `${facets.filter((f) => selectedFacets.has(f)).length} of ${facets.length}`,
  );

  const tabs: { id: Filter; label: string }[] = [
    { id: 'all', label: 'All' },
    { id: 'workers', label: 'Workers' },
    { id: 'skills', label: 'Skills' },
    { id: 'marketplace', label: 'Marketplace' },
  ];

  function toggleFacet(facet: string): void {
    if (selectedFacets.has(facet)) selectedFacets.delete(facet);
    else selectedFacets.add(facet);
    selectedFacets = new Set(selectedFacets);
  }

  function selectAllFacets(): void {
    selectedFacets = new Set(facets);
  }
  function clearFacets(): void {
    selectedFacets = new Set();
  }

  function selectItem(item: LibraryItem): void {
    selected = item;
  }
  function closeDetail(): void {
    selected = null;
  }

  function onDocMousedown(event: MouseEvent): void {
    const target = event.target as HTMLElement | null;
    if (target && !target.closest('[data-scope-filter]')) {
      scopeMenuOpen = false;
    }
  }
  onMount(() => {
    document.addEventListener('mousedown', onDocMousedown);
    return () => document.removeEventListener('mousedown', onDocMousedown);
  });
</script>

<div class="library-browser">
  <div class="toolbar">
    <div class="segmented" role="tablist" aria-label="Filter library">
      {#each tabs as tab (tab.id)}
        <button
          type="button"
          role="tab"
          aria-selected={filter === tab.id}
          class:active={filter === tab.id}
          onclick={() => (filter = tab.id)}
        >
          {tab.label}
          {#if tab.id === 'workers'}
            <span class="seg-count">{workerCount}</span>
          {:else if tab.id === 'skills'}
            <span class="seg-count">{skillCount}</span>
          {/if}
        </button>
      {/each}
    </div>

    <div class="toolbar-right">
      {#if showScopeFilter && !isMarketplace}
        <div class="scope-filter" data-scope-filter>
          <button
            type="button"
            class="scope-trigger"
            class:active={!allFacetsSelected}
            aria-haspopup="listbox"
            aria-expanded={scopeMenuOpen}
            onclick={() => (scopeMenuOpen = !scopeMenuOpen)}
          >
            <span class="scope-label">Scope</span>
            <span class="scope-value">{scopeButtonLabel}</span>
            <span class="scope-caret" aria-hidden="true">⌄</span>
          </button>
          {#if scopeMenuOpen}
            <div class="scope-menu" role="listbox" aria-label="Visible scopes">
              <div class="scope-menu-actions">
                <button type="button" onclick={selectAllFacets}>All</button>
                <span aria-hidden="true">·</span>
                <button type="button" onclick={clearFacets}>None</button>
              </div>
              {#each facets as facet (facet)}
                <button
                  type="button"
                  class="scope-option"
                  role="option"
                  aria-selected={selectedFacets.has(facet)}
                  onclick={() => toggleFacet(facet)}
                >
                  <span class="checkbox" class:checked={selectedFacets.has(facet)} aria-hidden="true">
                    {selectedFacets.has(facet) ? '✓' : ''}
                  </span>
                  <span class="scope-option-label" data-facet={facet}>{facetLabel(facet)}</span>
                </button>
              {/each}
            </div>
          {/if}
        </div>
      {/if}

      {#if !isMarketplace}
        <input
          class="search"
          type="search"
          placeholder="Search…"
          aria-label="Search library"
          bind:value={query}
        />
      {/if}
    </div>
  </div>

  {#if isMarketplace}
    <MarketplacePanel />
  {:else}
    {#if error}
      <div class="browser-error" role="alert">{error}</div>
    {/if}

    {#if loading}
      <div class="browser-loading" aria-busy="true">
        {#each [0, 1, 2, 3, 4, 5] as cell (cell)}
          <div class="card-skeleton"></div>
        {/each}
      </div>
    {:else}
      <LibraryList items={scopedItems} {query} onselect={selectItem} />
    {/if}

    <LibraryDetailPanel item={selected} onclose={closeDetail} />
  {/if}
</div>

<style>
  .library-browser {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
    min-width: 0;
  }

  .toolbar {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3);
    min-width: 0;
  }

  .toolbar-right {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    min-width: 0;
  }

  .segmented {
    display: inline-flex;
    gap: var(--space-1);
    padding: var(--space-1);
    border: 1px solid var(--border);
    border-radius: 4px;
    background: var(--row-active);
  }

  .segmented button {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1);
    padding: var(--space-1) var(--space-3);
    border: 0;
    border-radius: 3px;
    background: transparent;
    color: var(--muted);
    font: inherit;
    font-size: var(--text-base);
    font-weight: 600;
    cursor: pointer;
    transition:
      background 140ms ease,
      color 140ms ease;
  }

  .segmented button:hover {
    color: var(--fg);
  }

  .segmented button.active {
    background: var(--bg);
    color: var(--fg);
  }

  .segmented button:focus-visible {
    outline: 2px solid var(--blue);
    outline-offset: 2px;
  }

  .seg-count {
    min-width: 18px;
    padding: 0 5px;
    border-radius: 999px;
    background: var(--row-hover);
    color: var(--muted-3);
    font-family: ui-monospace, SFMono-Regular, 'SF Mono', Menlo, Consolas, monospace;
    font-size: var(--text-base);
    font-weight: 650;
    font-variant-numeric: tabular-nums;
    line-height: 16px;
    text-align: center;
  }

  /* ---- scope filter (multi-select) -------------------------------------- */
  .scope-filter {
    position: relative;
    flex: 0 0 auto;
  }

  .scope-trigger {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    height: 32px;
    padding: 0 var(--space-2);
    border: 1px solid var(--border);
    border-radius: 4px;
    background: var(--row-active);
    color: var(--fg);
    font: inherit;
    font-size: var(--text-base);
    cursor: pointer;
    transition:
      background 140ms ease,
      border-color 140ms ease;
  }

  .scope-trigger:hover {
    border-color: var(--border-strong);
    background: var(--row-hover);
  }

  .scope-trigger.active {
    border-color: color-mix(in srgb, var(--blue) 45%, transparent);
  }

  .scope-trigger:focus-visible {
    outline: 2px solid var(--blue);
    outline-offset: 2px;
  }

  .scope-label {
    color: var(--muted);
    font-family: ui-monospace, SFMono-Regular, 'SF Mono', Menlo, Consolas, monospace;
    font-size: var(--text-micro);
    font-weight: 600;
    letter-spacing: 0.08em;
    text-transform: uppercase;
  }

  .scope-value {
    color: var(--fg);
    font-size: var(--text-base);
    font-weight: 600;
  }

  .scope-caret {
    color: var(--muted-3);
    font-size: var(--text-base);
    line-height: 1;
  }

  .scope-menu {
    position: absolute;
    top: calc(100% + var(--space-1));
    right: 0;
    z-index: 50;
    min-width: 184px;
    max-height: 320px;
    overflow-y: auto;
    padding: var(--space-1);
    border: 1px solid var(--border-strong);
    border-radius: 4px;
    background: var(--bg);
    box-shadow: 0 12px 32px rgba(0, 0, 0, 0.4);
  }

  .scope-menu-actions {
    display: flex;
    align-items: center;
    gap: var(--space-1);
    padding: var(--space-1) var(--space-2) var(--space-2);
    border-bottom: 1px solid var(--border);
    margin-bottom: var(--space-1);
    color: var(--muted-3);
    font-size: var(--text-base);
  }

  .scope-menu-actions button {
    border: 0;
    background: transparent;
    color: var(--blue);
    font: inherit;
    font-size: var(--text-base);
    font-weight: 600;
    cursor: pointer;
  }

  .scope-menu-actions button:hover {
    text-decoration: underline;
  }

  .scope-option {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    width: 100%;
    padding: var(--space-1) var(--space-2);
    border: 0;
    border-radius: 3px;
    background: transparent;
    color: var(--muted-2);
    font: inherit;
    font-size: var(--text-base);
    text-align: left;
    cursor: pointer;
  }

  .scope-option:hover {
    background: var(--row-hover);
    color: var(--fg);
  }

  .checkbox {
    display: inline-flex;
    flex: 0 0 auto;
    align-items: center;
    justify-content: center;
    width: 15px;
    height: 15px;
    border: 1px solid var(--border-strong);
    border-radius: 3px;
    background: var(--row-active);
    color: var(--bg);
    font-size: var(--text-base);
    font-weight: 800;
    line-height: 1;
  }

  .checkbox.checked {
    border-color: var(--blue);
    background: var(--blue);
  }

  /* Scope option label carries the same per-facet tint as the card pills. */
  .scope-option-label {
    font-weight: 600;
  }
  .scope-option-label[data-facet='core'] {
    color: var(--blue);
  }
  .scope-option-label[data-facet='personal'] {
    color: var(--amber);
  }
  .scope-option-label:not([data-facet='core']):not([data-facet='personal']) {
    color: var(--emerald);
  }

  .search {
    flex: 1 1 160px;
    max-width: 240px;
    min-width: 0;
    height: 32px;
    padding: 0 var(--space-3);
    border: 1px solid var(--border);
    border-radius: 4px;
    background: var(--bg);
    color: var(--fg);
    font: inherit;
    font-size: var(--text-base);
  }

  .search::placeholder {
    color: var(--muted-3);
  }

  .search:focus-visible {
    outline: 2px solid var(--blue);
    outline-offset: 1px;
  }

  .browser-error {
    padding: var(--space-3);
    border: 1px solid var(--border);
    border-radius: 4px;
    background: var(--row-active);
    color: var(--amber);
    font-size: var(--text-base);
  }

  .browser-loading {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(296px, 1fr));
    gap: var(--space-2);
  }

  .card-skeleton {
    height: 104px;
    border: 1px solid var(--border);
    border-radius: 4px;
    background: var(--row-active);
    animation: lib-skeleton-pulse 1.3s ease-in-out infinite;
  }

  @keyframes lib-skeleton-pulse {
    0%,
    100% {
      opacity: 0.5;
    }
    50% {
      opacity: 1;
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .segmented button,
    .scope-trigger {
      transition: none;
    }
    .card-skeleton {
      animation: none;
    }
  }
</style>
