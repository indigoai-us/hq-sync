<script lang="ts">
  /**
   * StoryKanban — read-only 4-column story board with a Board/List toggle (US-006).
   *
   * Ported from hq-desktop's kanban-board.tsx (KanbanColumn + board grid + list
   * view + collapse behavior), restyled to the HQ Sync unified desktop token set
   * (monochrome glass; no hardcoded hex). This component owns the segmented
   * Board/List toggle and the column container so the board reads as one cohesive
   * unit; the rows themselves come from StoryList and the cards from StoryCard.
   *
   * Presentational only: takes `stories` (+ optional `loading` / `onselect`).
   * Data loading lives in a parent (US-003's get_local_project_prd), never here.
   */
  import {
    classifyStories,
    groupByState,
    STORY_STATES,
    type Story,
    type StoryState,
  } from '../lib/projects-model';
  import StoryCard from './StoryCard.svelte';
  import StoryList from './StoryList.svelte';

  interface Props {
    /** The stories to render (US-004 Story shape). */
    stories: Story[];
    /** When true, render the loading skeleton instead of content. */
    loading?: boolean;
    /** Fired when a story card or row is activated. */
    onselect?: (story: Story) => void;
  }

  let { stories, loading = false, onselect }: Props = $props();

  type ViewMode = 'board' | 'list';
  let viewMode = $state<ViewMode>('board');

  // Per-column collapse state, keyed by StoryState. Collapsed hides the body.
  let collapsed = $state<Record<StoryState, boolean>>({
    pending: false,
    blocked: false,
    'in-progress': false,
    complete: false,
  });

  const COLUMN_LABELS: Record<StoryState, string> = {
    pending: 'Pending',
    blocked: 'Blocked',
    'in-progress': 'In Progress',
    complete: 'Complete',
  };

  const classified = $derived(classifyStories(stories ?? []));
  const grouped = $derived(groupByState(classified));

  function toggleColumn(state: StoryState): void {
    collapsed[state] = !collapsed[state];
  }
</script>

<section class="story-kanban" aria-label="Story board">
  <div class="board-toolbar">
    <div class="view-toggle" role="group" aria-label="Board view mode">
      <button
        type="button"
        class="toggle-segment"
        class:is-active={viewMode === 'board'}
        aria-pressed={viewMode === 'board'}
        data-testid="view-toggle-board"
        onclick={() => (viewMode = 'board')}
      >
        Board
      </button>
      <button
        type="button"
        class="toggle-segment"
        class:is-active={viewMode === 'list'}
        aria-pressed={viewMode === 'list'}
        data-testid="view-toggle-list"
        onclick={() => (viewMode = 'list')}
      >
        List
      </button>
    </div>
  </div>

  {#if loading}
    <div class="board-loading" aria-busy="true" aria-label="Loading stories">
      {#each STORY_STATES as state (state)}
        <div class="skeleton-column">
          <div class="skeleton-header"></div>
          <div class="skeleton-card"></div>
          <div class="skeleton-card"></div>
        </div>
      {/each}
    </div>
  {:else if viewMode === 'board'}
    <div class="board-scroll">
      <div class="board-grid">
        {#each STORY_STATES as state (state)}
          {@const columnStories = grouped[state]}
          <div class="kanban-column">
            <button
              type="button"
              class="column-header"
              aria-expanded={!collapsed[state]}
              onclick={() => toggleColumn(state)}
            >
              <span class="status-dot" data-state={state}></span>
              <span class="column-label">{COLUMN_LABELS[state]}</span>
              <span class="count-badge">{columnStories.length}</span>
              <span class="chevron" class:is-open={!collapsed[state]} aria-hidden="true">›</span>
            </button>

            {#if !collapsed[state]}
              <div class="column-body">
                {#if columnStories.length === 0}
                  <div class="column-empty">
                    <span>No stories</span>
                  </div>
                {:else}
                  {#each columnStories as item (item.story.id)}
                    <StoryCard story={item.story} {onselect} />
                  {/each}
                {/if}
              </div>
            {/if}
          </div>
        {/each}
      </div>
    </div>
  {:else}
    <div class="list-scroll">
      <StoryList {stories} {onselect} />
    </div>
  {/if}
</section>

<style>
  .story-kanban {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    min-width: 0;
    height: 100%;
  }

  .board-toolbar {
    display: flex;
    flex-shrink: 0;
    justify-content: flex-end;
  }

  /* Segmented control in the app's primary language: the active segment carries
     the monochrome `--popover-primary` fill, matching the popover's toggle. */
  .view-toggle {
    display: inline-flex;
    gap: 2px;
    padding: 2px;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--row-active);
  }

  .toggle-segment {
    display: inline-flex;
    align-items: center;
    padding: var(--space-1) var(--space-3);
    border: 0;
    border-radius: calc(var(--radius-sm) - 2px);
    background: transparent;
    color: var(--muted);
    font-size: var(--text-base);
    font-weight: 600;
    cursor: pointer;
    transition:
      background 140ms ease,
      color 140ms ease;
  }

  .toggle-segment:hover {
    color: var(--fg);
  }

  .toggle-segment.is-active {
    background: var(--popover-primary);
    color: var(--popover-primary-text);
  }

  .toggle-segment:focus-visible {
    outline: 2px solid var(--blue);
    outline-offset: 2px;
  }

  .board-scroll {
    flex: 1 1 auto;
    min-height: 0;
    overflow-x: auto;
    overflow-y: hidden;
  }

  .board-grid {
    display: grid;
    grid-template-columns: repeat(4, minmax(240px, 1fr));
    gap: var(--space-4);
    min-width: 960px;
    height: 100%;
  }

  .kanban-column {
    display: flex;
    flex-direction: column;
    min-height: 0;
  }

  .column-header {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    width: 100%;
    padding: var(--space-2) var(--space-3);
    border: 0;
    border-radius: var(--radius-sm);
    background: transparent;
    text-align: left;
    cursor: pointer;
    transition: background 140ms ease;
  }

  .column-header:hover {
    background: var(--row-hover);
  }

  .column-header:focus-visible {
    outline: 2px solid var(--blue);
    outline-offset: 2px;
  }

  .status-dot {
    flex: 0 0 auto;
    width: 8px;
    height: 8px;
    border-radius: 999px;
    background: var(--muted-3);
  }

  .status-dot[data-state='blocked'] {
    background: var(--amber);
  }

  .status-dot[data-state='in-progress'] {
    background: var(--emerald);
  }

  .status-dot[data-state='complete'] {
    background: var(--muted-2);
  }

  .column-label {
    color: var(--muted);
    font-size: var(--text-base);
    font-weight: 600;
  }

  .count-badge {
    display: inline-flex;
    align-items: center;
    padding: 0 6px;
    border-radius: var(--radius-sm);
    background: var(--row-active);
    color: var(--muted-3);
    font-size: var(--text-base);
    font-variant-numeric: tabular-nums;
    font-weight: 600;
    line-height: 16px;
  }

  .chevron {
    margin-left: auto;
    color: var(--muted-3);
    font-size: var(--text-base);
    line-height: 1;
    transition: transform 150ms ease;
  }

  .chevron.is-open {
    transform: rotate(90deg);
  }

  .column-body {
    display: flex;
    flex: 1 1 auto;
    flex-direction: column;
    gap: var(--space-2);
    min-height: 0;
    margin-top: var(--space-2);
    overflow-y: auto;
    padding-right: var(--space-1);
  }

  .column-empty {
    display: flex;
    align-items: center;
    justify-content: center;
    padding: var(--space-5);
    border: 1px dashed var(--border);
    border-radius: var(--radius-sm);
  }

  .column-empty span {
    color: var(--muted-3);
    font-size: var(--text-base);
  }

  .list-scroll {
    flex: 1 1 auto;
    min-height: 0;
    overflow-y: auto;
  }

  /* Loading skeleton — neutral shimmer over the row surface. */
  .board-loading {
    display: grid;
    grid-template-columns: repeat(4, minmax(240px, 1fr));
    gap: var(--space-4);
    min-width: 960px;
  }

  .skeleton-column {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  .skeleton-header {
    height: 28px;
    border-radius: var(--radius-sm);
    background: var(--row-active);
  }

  .skeleton-card {
    height: 84px;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--row-active);
  }

  .skeleton-header,
  .skeleton-card {
    animation: skeleton-pulse 1.3s ease-in-out infinite;
  }

  @keyframes skeleton-pulse {
    0%,
    100% {
      opacity: 0.5;
    }
    50% {
      opacity: 1;
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .toggle-segment,
    .column-header,
    .chevron {
      transition: none;
    }

    .skeleton-header,
    .skeleton-card {
      animation: none;
    }
  }
</style>
