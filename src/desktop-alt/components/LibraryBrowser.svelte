<script lang="ts">
  /**
   * LibraryBrowser — the shared body of the Library surface, used by both the
   * root LibraryPage and the per-company CompanyLibraryPanel. Owns the
   * Workers|Skills segmented toggle, the text filter, the master list, and the
   * detail slide-over. Data (items/loading/error) is loaded by the caller and
   * passed in, so this component stays presentational + reusable.
   */
  import {
    toLibraryItems,
    type LibraryItem,
    type LibraryItems,
  } from '../lib/library';
  import LibraryList from './LibraryList.svelte';
  import LibraryDetailPanel from './LibraryDetailPanel.svelte';

  interface Props {
    /** The loaded library payload (workers + skills) for this scope. */
    items: LibraryItems;
    /** Whether the data is still loading. */
    loading?: boolean;
    /** Error string if the load failed. */
    error?: string | null;
  }

  let { items, loading = false, error = null }: Props = $props();

  type Filter = 'all' | 'workers' | 'skills';
  let filter = $state<Filter>('all');
  let query = $state('');
  let selected = $state<LibraryItem | null>(null);

  const allItems = $derived(toLibraryItems(items));
  const workerCount = $derived(items.workers?.length ?? 0);
  const skillCount = $derived(items.skills?.length ?? 0);

  const scopedItems = $derived(
    filter === 'workers'
      ? allItems.filter((item) => item.kind === 'worker')
      : filter === 'skills'
        ? allItems.filter((item) => item.kind === 'skill')
        : allItems,
  );

  const tabs: { id: Filter; label: string }[] = [
    { id: 'all', label: 'All' },
    { id: 'workers', label: 'Workers' },
    { id: 'skills', label: 'Skills' },
  ];

  function selectItem(item: LibraryItem): void {
    selected = item;
  }

  function closeDetail(): void {
    selected = null;
  }
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

    <input
      class="search"
      type="search"
      placeholder="Search…"
      aria-label="Search library"
      bind:value={query}
    />
  </div>

  {#if error}
    <div class="browser-error" role="alert">{error}</div>
  {/if}

  {#if loading}
    <div class="browser-loading" aria-busy="true">
      {#each [0, 1, 2] as row (row)}
        <div class="row-skeleton"></div>
      {/each}
    </div>
  {:else}
    <LibraryList items={scopedItems} {query} onselect={selectItem} />
  {/if}

  <LibraryDetailPanel item={selected} onclose={closeDetail} />
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

  .segmented {
    display: inline-flex;
    gap: var(--space-1);
    padding: var(--space-1);
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
    background: var(--row-active);
  }

  .segmented button {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1);
    padding: var(--space-1) var(--space-3);
    border: 0;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--muted);
    font: inherit;
    font-size: var(--text-sm);
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
    font-size: 11px;
    font-weight: 650;
    line-height: 16px;
    text-align: center;
  }

  .search {
    flex: 1 1 180px;
    max-width: 280px;
    min-width: 0;
    height: 32px;
    padding: 0 var(--space-3);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--bg);
    color: var(--fg);
    font: inherit;
    font-size: var(--text-sm);
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
    border-radius: var(--radius-sm);
    background: var(--row-active);
    color: var(--amber);
    font-size: var(--text-sm);
  }

  .browser-loading {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }

  .row-skeleton {
    height: 52px;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
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
    .segmented button {
      transition: none;
    }

    .row-skeleton {
      animation: none;
    }
  }
</style>
