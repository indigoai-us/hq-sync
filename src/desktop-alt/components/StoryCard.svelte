<script lang="ts">
  import type { Story } from '../lib/projects-model';
  import LabelChip from './LabelChip.svelte';

  /**
   * The US-004 Story type has no `model_hint` field, but hq-desktop's StoryCard
   * renders an optional model badge when one is present on the story. We accept a
   * defensively-typed augmentation so the badge appears only when upstream data
   * carries it, without forcing the canonical Story type to grow the field.
   */
  type StoryWithModelHint = Story & { model_hint?: string | null };

  interface Props {
    /** The story to render (US-004 Story shape). */
    story: StoryWithModelHint;
    /** Fired when the card is activated by click or keyboard. */
    onselect?: (story: Story) => void;
  }

  let { story, onselect }: Props = $props();

  const labels = $derived(story.labels ?? []);
  const visibleLabels = $derived(labels.slice(0, 2));
  const overflowCount = $derived(labels.length - visibleLabels.length);

  const acTotal = $derived(story.acceptanceCriteria?.length ?? 0);
  // AC progress: the Story type carries no per-AC done flags, only a story-level
  // `passes`. Mirroring hq-desktop's StoryCard, every AC shares the story state —
  // so completed stories read full (acTotal/acTotal) and everything else reads 0.
  const acComplete = $derived(story.passes ? acTotal : 0);
  const acPercent = $derived(acTotal > 0 ? (acComplete / acTotal) * 100 : 0);

  const priorityLabel = $derived(
    typeof story.priority === 'number' ? `P${story.priority}` : null,
  );
  const modelHint = $derived(story.model_hint ?? null);

  function activate(): void {
    onselect?.(story);
  }

  function handleKeydown(event: KeyboardEvent): void {
    if (event.key === 'Enter' || event.key === ' ') {
      event.preventDefault();
      onselect?.(story);
    }
  }
</script>

<button
  type="button"
  class="story-card"
  class:is-complete={story.passes}
  data-priority={priorityLabel}
  aria-label={`Story ${story.id}: ${story.title}`}
  onclick={activate}
  onkeydown={handleKeydown}
>
  <div class="card-top">
    <span class="story-id">{story.id}</span>
    <div class="badges">
      {#if modelHint}
        <span class="model-badge" title={`Model hint: ${modelHint}`}>{modelHint}</span>
      {/if}
      {#if priorityLabel}
        <span class="priority-badge" data-priority={priorityLabel}>{priorityLabel}</span>
      {/if}
    </div>
  </div>

  <h4 class="story-title" title={story.title}>{story.title}</h4>

  {#if labels.length > 0}
    <div class="labels">
      {#each visibleLabels as label (label)}
        <LabelChip {label} />
      {/each}
      {#if overflowCount > 0}
        <span class="label-overflow" title={`${overflowCount} more`}>+{overflowCount}</span>
      {/if}
    </div>
  {/if}

  {#if acTotal > 0}
    <div class="ac-progress">
      <div
        class="progress-track"
        role="progressbar"
        aria-valuemin={0}
        aria-valuemax={acTotal}
        aria-valuenow={acComplete}
        aria-label="Acceptance criteria complete"
      >
        <div
          class="progress-fill"
          style={`--progress-scale: ${Math.max(0, Math.min(1, acPercent / 100))}`}
        ></div>
      </div>
      <span class="ac-count">{acComplete}/{acTotal}</span>
    </div>
  {/if}
</button>

<style>
  .story-card {
    display: grid;
    gap: var(--space-2);
    width: 100%;
    min-width: 0;
    padding: var(--space-3);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--bg);
    box-shadow: 0 1px 2px rgba(0, 0, 0, 0.4);
    text-align: left;
    cursor: pointer;
    transition:
      background 150ms ease,
      border-color 150ms ease;
  }

  .story-card:hover {
    border-color: var(--border-strong);
    background: var(--row-hover);
  }

  .story-card:focus-visible {
    outline: 2px solid var(--blue);
    outline-offset: 2px;
  }

  .story-card.is-complete {
    opacity: 0.6;
  }

  .card-top {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-2);
    min-width: 0;
  }

  .story-id {
    overflow: hidden;
    color: var(--muted);
    font-family:
      ui-monospace, SFMono-Regular, 'SF Mono', Menlo, Consolas, monospace;
    font-size: var(--text-xs);
    font-weight: 600;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .badges {
    display: inline-flex;
    flex-shrink: 0;
    align-items: center;
    gap: var(--space-1);
  }

  .model-badge,
  .priority-badge {
    display: inline-flex;
    align-items: center;
    padding: 1px 6px;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--row-active);
    color: var(--muted-2);
    font-size: var(--text-xs);
    font-weight: 650;
    line-height: 14px;
    text-transform: uppercase;
  }

  .priority-badge {
    font-variant-numeric: tabular-nums;
    text-transform: none;
  }

  .story-title {
    display: -webkit-box;
    min-width: 0;
    margin: 0;
    overflow: hidden;
    color: var(--fg);
    font-size: var(--text-base);
    font-weight: 650;
    line-height: 18px;
    -webkit-box-orient: vertical;
    -webkit-line-clamp: 2;
    line-clamp: 2;
  }

  .labels {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-1);
    min-width: 0;
  }

  .label-overflow {
    display: inline-flex;
    align-items: center;
    padding: 1px 6px;
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--row-active);
    color: var(--muted-3);
    font-size: var(--text-xs);
    font-weight: 600;
    line-height: 16px;
  }

  .ac-progress {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    min-width: 0;
  }

  /* Mirrors the established SourcesList progress-bar visual language. */
  .progress-track {
    flex: 1;
    height: 5px;
    overflow: hidden;
    border-radius: 999px;
    background: var(--row-active);
  }

  .progress-fill {
    width: 100%;
    height: 100%;
    border-radius: inherit;
    background: var(--blue);
    transform: scaleX(var(--progress-scale, 0));
    transform-origin: left center;
    transition: transform 180ms cubic-bezier(0.2, 0.7, 0.2, 1);
  }

  .ac-count {
    flex-shrink: 0;
    color: var(--muted-3);
    font-size: var(--text-xs);
    font-variant-numeric: tabular-nums;
    font-weight: 600;
  }

  @media (prefers-reduced-motion: reduce) {
    .story-card,
    .progress-fill {
      transition: none;
    }
  }
</style>
