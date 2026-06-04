<script lang="ts">
  import type { CompanyBoardCard } from '../lib/company-board.svelte';

  interface Props {
    card: CompanyBoardCard;
  }

  let { card }: Props = $props();

  const title = $derived(card.title?.trim() || 'Untitled card');
  const assigneeInitials = $derived(
    card.assigneeInitials?.trim() ||
      title
        .split(/\s+/)
        .filter(Boolean)
        .slice(0, 2)
        .map((part) => part[0]?.toUpperCase())
        .join('') ||
      '--',
  );
  const tag = $derived(card.tag?.trim() || card.labels?.find((label) => label.trim()) || 'Untagged');
  const age = $derived(card.age?.trim() || card.subtitle?.trim() || 'New');
</script>

<article class="board-card" aria-label={title}>
  <h3 title={title}>{title}</h3>
  <div class="card-meta">
    <span class="assignee" title="Assignee initials">{assigneeInitials}</span>
    <span class="tag" title={tag}>{tag}</span>
    <span class="age" title={age}>{age}</span>
  </div>
</article>

<style>
  .board-card {
    display: grid;
    gap: 10px;
    min-width: 0;
    min-height: 92px;
    padding: 12px;
    border: 1px solid var(--border);
    border-radius: 8px;
    background: var(--bg);
    box-shadow: 0 1px 2px rgba(0, 0, 0, 0.4);
  }

  .board-card h3 {
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

  .card-meta {
    display: grid;
    grid-template-columns: 28px minmax(0, 1fr) auto;
    align-items: center;
    gap: 7px;
    min-width: 0;
  }

  .assignee {
    width: 28px;
    height: 28px;
    overflow: hidden;
    border-radius: 999px;
    background: var(--row-active);
    color: var(--fg);
    font-size: var(--text-micro);
    font-weight: 750;
    line-height: 28px;
    text-align: center;
    text-transform: uppercase;
    white-space: nowrap;
  }

  .tag,
  .age {
    min-width: 0;
    overflow: hidden;
    color: var(--muted-3);
    font-size: var(--text-base);
    font-weight: 600;
    line-height: 18px;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .tag {
    padding: 0 7px;
    border: 1px solid var(--border);
    border-radius: 999px;
    background: var(--row-hover);
  }

  .age {
    color: var(--muted);
  }
</style>
