<script lang="ts">
  export interface CatchUpItem {
    id: string;
    title: string;
    detail: string;
    rank?: number;
  }

  interface Props {
    items?: CatchUpItem[];
    onopen?: (item: CatchUpItem) => void;
  }

  let { items = [], onopen }: Props = $props();
</script>

<section class="catch-up" aria-label="Catch up" data-testid="v4-catch-up">
  <header>
    <h2>Catch up</h2>
    <span>{items.length} unread</span>
  </header>
  {#if items.length === 0}
    <p class="empty">Nothing new.</p>
  {:else}
    <div class="ranked-list">
      {#each items as item (item.id)}
        <button type="button" class="ranked-card" onclick={() => onopen?.(item)}>
          <span class="rank">{item.rank ?? ''}</span>
          <span class="text">
            <strong>{item.title}</strong>
            <small>{item.detail}</small>
          </span>
        </button>
      {/each}
    </div>
  {/if}
</section>

<style>
  .catch-up {
    display: grid;
    gap: 10px;
    width: min(460px, 100%);
    padding: 12px;
    border: 1px solid var(--border);
    border-radius: 8px;
    background: var(--surface-raise);
  }

  header,
  .ranked-card {
    display: flex;
    align-items: center;
    min-width: 0;
  }

  header {
    justify-content: space-between;
  }

  h2,
  .empty {
    margin: 0;
  }

  h2 {
    color: var(--fg);
    font-size: var(--text-base);
    font-weight: 600;
  }

  header span,
  .empty,
  small {
    color: var(--muted);
    font-size: var(--text-base);
  }

  .ranked-list {
    display: grid;
    gap: 6px;
  }

  .ranked-card {
    gap: 9px;
    width: 100%;
    padding: 9px;
    border: 1px solid var(--border);
    border-radius: 7px;
    background: transparent;
    color: inherit;
    font: inherit;
    text-align: left;
  }

  .rank {
    width: 22px;
    color: var(--fg-data);
    font-family: var(--font-mono);
    font-size: var(--text-xs);
  }

  .text {
    display: grid;
    gap: 2px;
    min-width: 0;
  }

  strong,
  small {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>
