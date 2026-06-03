<script lang="ts">
  /**
   * LibraryList — row list of library items (workers + skills), shared by the
   * root Library page and the per-company Library panel. Presentational only:
   * applies the text filter and emits onselect(item); does not load data.
   *
   * Each row reads: kind glyph · name · kind badge · scope/type chips · ellipsed
   * description. Mirrors StoryList's row language (monochrome glass tokens).
   */
  import {
    filterLibraryItems,
    libraryItemKey,
    type LibraryItem,
  } from '../lib/library';
  import LabelChip from './LabelChip.svelte';

  interface Props {
    /** The unified worker+skill items to render. */
    items: LibraryItem[];
    /** Free-text filter (name / description / type / scope). */
    query?: string;
    /** Fired when a row is activated by click or keyboard. */
    onselect?: (item: LibraryItem) => void;
  }

  let { items, query = '', onselect }: Props = $props();

  const visible = $derived(filterLibraryItems(items ?? [], query));

  function scopeLabel(item: LibraryItem): string {
    if (item.kind === 'worker') {
      return item.worker.scope === 'company'
        ? (item.worker.company ?? 'company')
        : 'shared';
    }
    if (item.skill.scope === 'company') return item.skill.company ?? 'company';
    return item.skill.scope; // 'root' | 'personal'
  }

  function activate(item: LibraryItem): void {
    onselect?.(item);
  }

  function handleKeydown(event: KeyboardEvent, item: LibraryItem): void {
    if (event.key === 'Enter' || event.key === ' ') {
      event.preventDefault();
      onselect?.(item);
    }
  }
</script>

<div class="library-list" aria-label="Library items">
  {#if visible.length === 0}
    <div class="empty-state">
      <p>No matches</p>
      <span>
        {#if (items?.length ?? 0) === 0}
          Nothing here yet.
        {:else}
          Try a different search.
        {/if}
      </span>
    </div>
  {:else}
    {#each visible as item (libraryItemKey(item))}
      {#if item.kind === 'worker'}
        <button
          type="button"
          class="library-row"
          aria-label={`Worker ${item.worker.name}`}
          onclick={() => activate(item)}
          onkeydown={(event) => handleKeydown(event, item)}
        >
          <span class="kind-dot kind-worker" aria-hidden="true"></span>
          <div class="row-main">
            <div class="row-head">
              <span class="row-name" title={item.worker.name}>{item.worker.name}</span>
              <span class="kind-badge">Worker</span>
              {#if item.worker.type}
                <LabelChip label={item.worker.type} />
              {/if}
              <span class="scope-chip">{scopeLabel(item)}</span>
            </div>
            {#if item.worker.description}
              <span class="row-desc">{item.worker.description}</span>
            {/if}
          </div>
        </button>
      {:else}
        <button
          type="button"
          class="library-row"
          aria-label={`Skill ${item.skill.name}`}
          onclick={() => activate(item)}
          onkeydown={(event) => handleKeydown(event, item)}
        >
          <span class="kind-dot kind-skill" aria-hidden="true"></span>
          <div class="row-main">
            <div class="row-head">
              <span class="row-name" title={item.skill.name}>{item.skill.name}</span>
              <span class="kind-badge">Skill</span>
              <span class="scope-chip">{scopeLabel(item)}</span>
            </div>
            {#if item.skill.description}
              <span class="row-desc">{item.skill.description}</span>
            {/if}
          </div>
        </button>
      {/if}
    {/each}
  {/if}
</div>

<style>
  .library-list {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    min-width: 0;
  }

  .library-row {
    display: flex;
    align-items: flex-start;
    gap: var(--space-3);
    width: 100%;
    min-width: 0;
    padding: var(--space-2) var(--space-3);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--bg);
    text-align: left;
    cursor: pointer;
    transition:
      background 150ms ease,
      border-color 150ms ease;
  }

  .library-row:hover {
    border-color: var(--border-strong);
    background: var(--row-hover);
  }

  .library-row:focus-visible {
    outline: 2px solid var(--blue);
    outline-offset: 2px;
  }

  .kind-dot {
    flex: 0 0 auto;
    width: 8px;
    height: 8px;
    margin-top: 5px;
    border-radius: 999px;
    background: var(--muted-3);
  }

  .kind-dot.kind-worker {
    background: var(--emerald);
  }

  .kind-dot.kind-skill {
    background: var(--blue);
  }

  .row-main {
    display: flex;
    flex: 1 1 auto;
    flex-direction: column;
    gap: 3px;
    min-width: 0;
  }

  .row-head {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2);
    min-width: 0;
  }

  .row-name {
    flex: 0 1 auto;
    overflow: hidden;
    color: var(--fg);
    font-size: var(--text-base);
    font-weight: 600;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .kind-badge,
  .scope-chip {
    display: inline-flex;
    flex: 0 0 auto;
    align-items: center;
    padding: 1px 7px;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--row-active);
    color: var(--muted-2);
    font-size: var(--text-xs);
    font-weight: 650;
    line-height: 16px;
  }

  .scope-chip {
    color: var(--muted-3);
    text-transform: lowercase;
  }

  .row-desc {
    min-width: 0;
    overflow: hidden;
    color: var(--muted);
    font-size: var(--text-xs);
    line-height: 16px;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .empty-state {
    padding: var(--space-6);
    border: 1px dashed var(--border-strong);
    border-radius: var(--radius-sm);
    background: var(--bg);
    text-align: center;
  }

  .empty-state p {
    margin: 0 0 var(--space-1);
    color: var(--fg);
    font-weight: 650;
  }

  .empty-state span {
    color: var(--muted);
    font-size: var(--text-xs);
  }

  @media (prefers-reduced-motion: reduce) {
    .library-row {
      transition: none;
    }
  }
</style>
