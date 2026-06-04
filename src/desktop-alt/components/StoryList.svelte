<script lang="ts">
  /**
   * StoryList — horizontal-row list view of a project's stories (US-006).
   *
   * Ported from hq-desktop's kanban-board.tsx StoryListRow, restyled to the
   * HQ Sync unified desktop token set (monochrome glass; no hardcoded hex).
   * Each row reads: state dot · ID · title + labels · state badge · priority ·
   * AC count. Presentational only — emits onselect(story); does not load data.
   */
  import {
    classifyStories,
    type Story,
    type StoryState,
  } from '../lib/projects-model';
  import LabelChip from './LabelChip.svelte';

  interface Props {
    /** The stories to render (US-004 Story shape). */
    stories: Story[];
    /** Fired when a row is activated by click or keyboard. */
    onselect?: (story: Story) => void;
  }

  let { stories, onselect }: Props = $props();

  // Reuse US-004's classifier so the list shares the board's state derivation.
  const classified = $derived(classifyStories(stories ?? []));

  const STATE_LABELS: Record<StoryState, string> = {
    pending: 'Pending',
    blocked: 'Blocked',
    'in-progress': 'In Progress',
    complete: 'Complete',
  };

  function priorityLabel(priority: number | undefined): string | null {
    return typeof priority === 'number' ? `P${priority}` : null;
  }

  function acCount(story: Story): { complete: number; total: number } {
    const total = story.acceptanceCriteria?.length ?? 0;
    return { complete: story.passes ? total : 0, total };
  }

  function activate(story: Story): void {
    onselect?.(story);
  }

  function handleKeydown(event: KeyboardEvent, story: Story): void {
    if (event.key === 'Enter' || event.key === ' ') {
      event.preventDefault();
      onselect?.(story);
    }
  }
</script>

<div class="story-list" aria-label="Stories">
  {#if classified.length === 0}
    <div class="empty-state">
      <p>No stories</p>
      <span>This project has no stories yet.</span>
    </div>
  {:else}
    {#each classified as item (item.story.id)}
      {@const labels = item.story.labels ?? []}
      {@const visibleLabels = labels.slice(0, 3)}
      {@const overflow = labels.length - visibleLabels.length}
      {@const ac = acCount(item.story)}
      {@const prio = priorityLabel(item.story.priority)}
      <button
        type="button"
        class="story-row"
        class:is-complete={item.story.passes}
        aria-label={`Story ${item.story.id}: ${item.story.title}`}
        onclick={() => activate(item.story)}
        onkeydown={(event) => handleKeydown(event, item.story)}
      >
        <span class="state-dot" data-state={item.state}></span>
        <span class="story-id">{item.story.id}</span>
        <div class="story-main">
          <span class="story-title" title={item.story.title}>{item.story.title}</span>
          {#if labels.length > 0}
            <div class="row-labels">
              {#each visibleLabels as label (label)}
                <LabelChip {label} />
              {/each}
              {#if overflow > 0}
                <span class="label-overflow" title={`${overflow} more`}>+{overflow}</span>
              {/if}
            </div>
          {/if}
        </div>
        <span class="state-badge" data-state={item.state}>{STATE_LABELS[item.state]}</span>
        {#if prio}
          <span class="priority-badge" data-priority={prio}>{prio}</span>
        {/if}
        {#if ac.total > 0}
          <span class="ac-count">{ac.complete}/{ac.total}</span>
        {/if}
      </button>
    {/each}
  {/if}
</div>

<style>
  .story-list {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    min-width: 0;
  }

  .story-row {
    display: flex;
    align-items: center;
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

  .story-row:hover {
    border-color: var(--border-strong);
    background: var(--row-hover);
  }

  .story-row:focus-visible {
    outline: 2px solid var(--blue);
    outline-offset: 2px;
  }

  .story-row.is-complete {
    opacity: 0.6;
  }

  .state-dot {
    flex: 0 0 auto;
    width: 8px;
    height: 8px;
    border-radius: 999px;
    background: var(--muted-3);
  }

  /* Status carried by neutral surface layering + the muted/emerald markers the
     desktop token set permits — no severity palette. */
  .state-dot[data-state='blocked'] {
    background: var(--amber);
  }

  .state-dot[data-state='in-progress'] {
    background: var(--emerald);
  }

  .state-dot[data-state='complete'] {
    background: var(--muted-2);
  }

  .story-id {
    flex: 0 0 auto;
    width: 56px;
    overflow: hidden;
    color: var(--muted);
    font-family:
      ui-monospace, SFMono-Regular, 'SF Mono', Menlo, Consolas, monospace;
    font-size: var(--text-base);
    font-weight: 600;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .story-main {
    display: flex;
    flex: 1 1 auto;
    align-items: center;
    gap: var(--space-2);
    min-width: 0;
  }

  .story-title {
    flex: 0 1 auto;
    overflow: hidden;
    color: var(--fg);
    font-size: var(--text-base);
    font-weight: 600;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .row-labels {
    display: flex;
    flex: 0 1 auto;
    gap: var(--space-1);
    min-width: 0;
    overflow: hidden;
  }

  .state-badge,
  .priority-badge,
  .label-overflow {
    display: inline-flex;
    flex: 0 0 auto;
    align-items: center;
    padding: 1px 7px;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--row-active);
    color: var(--muted-2);
    font-size: var(--text-base);
    font-weight: 650;
    line-height: 16px;
  }

  .state-badge[data-state='blocked'] {
    color: var(--amber);
  }

  .state-badge[data-state='in-progress'] {
    color: var(--emerald);
  }

  .state-badge[data-state='complete'] {
    color: var(--muted-3);
  }

  .priority-badge {
    font-variant-numeric: tabular-nums;
  }

  /* Color-coded priority (hq-desktop parity): P1 red · P2 amber · P3 blue. */
  .priority-badge[data-priority='P1'] {
    border-color: transparent;
    background: rgba(248, 113, 113, 0.15);
    color: var(--red);
  }
  .priority-badge[data-priority='P2'] {
    border-color: transparent;
    background: rgba(245, 158, 11, 0.15);
    color: var(--amber);
  }
  .priority-badge[data-priority='P3'] {
    border-color: transparent;
    background: rgba(96, 165, 250, 0.15);
    color: var(--blue);
  }

  .ac-count {
    flex: 0 0 auto;
    color: var(--muted-3);
    font-size: var(--text-base);
    font-variant-numeric: tabular-nums;
    font-weight: 600;
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
    font-size: var(--text-base);
  }

  @media (prefers-reduced-motion: reduce) {
    .story-row {
      transition: none;
    }
  }
</style>
