<script lang="ts">
  /**
   * LibraryList — Foundry/"ops console" card grid of library items (workers +
   * skills), shared by the root Library page and the per-company panel.
   * Presentational only: applies the text filter and emits onselect(item).
   *
   * Visual language (Palantir-leaning): dark surface tiles, near-square corners,
   * hairline borders, a kind accent bar, monospace ALL-CAPS micro-labels, and a
   * pill hierarchy — scope (CORE / PERSONAL / company), worker type + team, or
   * skill pack + tool count.
   */
  import {
    filterLibraryItems,
    libraryItemKey,
    type LibraryItem,
  } from '../lib/library';

  interface Props {
    /** The unified worker+skill items to render. */
    items: LibraryItem[];
    /** Free-text filter (name / description / type / scope / pack). */
    query?: string;
    /** Fired when a card is activated by click or keyboard. */
    onselect?: (item: LibraryItem) => void;
  }

  let { items, query = '', onselect }: Props = $props();

  const visible = $derived(filterLibraryItems(items ?? [], query));

  /** Scope chip text + accent class. */
  function scope(item: LibraryItem): { label: string; cls: string } {
    if (item.kind === 'worker') {
      return item.worker.scope === 'company'
        ? { label: item.worker.company ?? 'company', cls: 'scope-company' }
        : { label: 'core', cls: 'scope-core' };
    }
    if (item.skill.scope === 'company') {
      return { label: item.skill.company ?? 'company', cls: 'scope-company' };
    }
    if (item.skill.scope === 'personal') return { label: 'personal', cls: 'scope-personal' };
    return { label: 'core', cls: 'scope-core' };
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
  <div class="library-grid" aria-label="Library items">
    {#each visible as item (libraryItemKey(item))}
      {@const sc = scope(item)}
      <button
        type="button"
        class="lib-card"
        class:is-worker={item.kind === 'worker'}
        class:is-skill={item.kind === 'skill'}
        aria-label={`${item.kind === 'worker' ? 'Worker' : 'Skill'} ${item.kind === 'worker' ? item.worker.name : item.skill.name}`}
        onclick={() => activate(item)}
        onkeydown={(event) => handleKeydown(event, item)}
      >
        <span class="accent" aria-hidden="true"></span>

        <div class="card-head">
          <span class="kind-tag">
            <span class="kind-dot" aria-hidden="true"></span>
            {item.kind === 'worker' ? 'Worker' : 'Skill'}
          </span>
          <span class="pill scope {sc.cls}">{sc.label}</span>
        </div>

        {#if item.kind === 'worker'}
          <h3 class="card-name" title={item.worker.name}>{item.worker.name}</h3>
          <div class="pill-row">
            {#if item.worker.type}
              <span class="pill meta">{item.worker.type}</span>
            {/if}
            {#if item.worker.team}
              <span class="pill meta ghost">{item.worker.team}</span>
            {/if}
          </div>
          {#if item.worker.description}
            <p class="card-desc">{item.worker.description}</p>
          {/if}
        {:else}
          <h3 class="card-name" title={item.skill.name}>{item.skill.name}</h3>
          <div class="pill-row">
            {#if item.skill.pack}
              <span class="pill meta pack">{item.skill.pack}</span>
            {/if}
            {#if item.skill.allowedTools.length > 0}
              <span class="pill meta ghost">{item.skill.allowedTools.length} tools</span>
            {/if}
          </div>
          {#if item.skill.description}
            <p class="card-desc">{item.skill.description}</p>
          {/if}
        {/if}
      </button>
    {/each}
  </div>
{/if}

<style>
  .library-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(272px, 1fr));
    align-items: start;
    gap: var(--space-2);
    min-width: 0;
  }

  .lib-card {
    position: relative;
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    min-width: 0;
    padding: var(--space-3) var(--space-3) var(--space-3) calc(var(--space-3) + 4px);
    overflow: hidden;
    /* Near-square corners + hairline border = Foundry tile. */
    border: 1px solid var(--border);
    border-radius: 4px;
    background: var(--row-active);
    text-align: left;
    cursor: pointer;
    transition:
      background 140ms ease,
      border-color 140ms ease,
      transform 140ms ease;
  }

  .lib-card:hover {
    border-color: var(--border-strong);
    background: var(--row-hover);
    transform: translateY(-1px);
  }

  .lib-card:focus-visible {
    outline: 2px solid var(--blue);
    outline-offset: 2px;
  }

  /* Left kind accent bar — emerald (worker) / blue (skill), brightens on hover. */
  .accent {
    position: absolute;
    inset-block: 0;
    inset-inline-start: 0;
    width: 3px;
    opacity: 0.55;
    transition: opacity 140ms ease;
  }
  .lib-card.is-worker .accent {
    background: var(--emerald);
  }
  .lib-card.is-skill .accent {
    background: var(--blue);
  }
  .lib-card:hover .accent {
    opacity: 1;
  }

  .card-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-2);
    min-width: 0;
  }

  /* Monospace ALL-CAPS micro-label — the signature ops-console tag. */
  .kind-tag {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    color: var(--muted-2);
    font-family: ui-monospace, SFMono-Regular, 'SF Mono', Menlo, Consolas, monospace;
    font-size: var(--text-micro);
    font-weight: 600;
    letter-spacing: 0.09em;
    text-transform: uppercase;
  }

  .kind-dot {
    width: 6px;
    height: 6px;
    border-radius: 999px;
    background: var(--muted-3);
  }
  .lib-card.is-worker .kind-dot {
    background: var(--emerald);
  }
  .lib-card.is-skill .kind-dot {
    background: var(--blue);
  }

  .card-name {
    margin: 0;
    overflow: hidden;
    color: var(--fg);
    font-size: var(--text-base);
    font-weight: 600;
    line-height: 18px;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .pill-row {
    display: flex;
    flex-wrap: wrap;
    gap: 5px;
    min-width: 0;
  }

  /* Base pill — mono, uppercase, hairline. */
  .pill {
    display: inline-flex;
    align-items: center;
    max-width: 100%;
    overflow: hidden;
    padding: 1px 7px;
    border: 1px solid var(--border);
    border-radius: 3px;
    background: var(--row-hover);
    color: var(--muted-2);
    font-family: ui-monospace, SFMono-Regular, 'SF Mono', Menlo, Consolas, monospace;
    font-size: var(--text-micro);
    font-weight: 600;
    letter-spacing: 0.05em;
    line-height: 15px;
    text-overflow: ellipsis;
    text-transform: uppercase;
    white-space: nowrap;
  }

  /* Scope pills carry the strongest hierarchy cue — tinted text + edge. */
  .pill.scope {
    flex: 0 0 auto;
  }
  .scope-core {
    border-color: color-mix(in srgb, var(--blue) 38%, transparent);
    color: var(--blue);
  }
  .scope-personal {
    border-color: color-mix(in srgb, var(--amber) 40%, transparent);
    color: var(--amber);
  }
  .scope-company {
    border-color: color-mix(in srgb, var(--emerald) 42%, transparent);
    color: var(--emerald);
  }

  /* Meta pills (type / pack / counts) stay neutral so scope dominates. */
  .pill.meta {
    color: var(--muted-2);
  }
  .pill.meta.ghost {
    background: transparent;
    color: var(--muted);
  }
  .pill.meta.pack {
    border-color: color-mix(in srgb, var(--blue) 26%, transparent);
    color: var(--muted-2);
  }

  .card-desc {
    margin: 2px 0 0;
    min-width: 0;
    overflow: hidden;
    color: var(--muted);
    font-size: var(--text-base);
    line-height: 16px;
    /* Clamp to two lines so tiles stay uniform. */
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
  }

  .empty-state {
    padding: var(--space-6);
    border: 1px dashed var(--border-strong);
    border-radius: 4px;
    background: var(--row-active);
    text-align: center;
  }

  .empty-state p {
    margin: 0 0 var(--space-1);
    color: var(--fg);
    font-weight: 600;
  }

  .empty-state span {
    color: var(--muted);
    font-size: var(--text-base);
  }

  @media (prefers-reduced-motion: reduce) {
    .lib-card,
    .accent {
      transition: none;
    }
    .lib-card:hover {
      transform: none;
    }
  }
</style>
