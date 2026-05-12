<script lang="ts">
  interface NewFile {
    path: string;
    bytes: number;
    addedBy: string | null;
  }

  interface Props {
    count: number;
    files: NewFile[];
    onclick?: () => void;
  }

  let { count, files, onclick }: Props = $props();
</script>

{#if count > 0}
  <button class="new-files-badge" onclick={onclick} title="View new files">
    <svg width="14" height="14" viewBox="0 0 16 16" fill="none" xmlns="http://www.w3.org/2000/svg" aria-hidden="true">
      <path d="M9 1.5H4a1 1 0 0 0-1 1v11a1 1 0 0 0 1 1h8a1 1 0 0 0 1-1V5.5L9 1.5z" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round" />
      <path d="M9 1.5V5.5h4" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round" />
      <path d="M8 8.5v3M6.5 10h3" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round" />
    </svg>
    <span class="new-files-label">
      {count} new file{count === 1 ? '' : 's'}
    </span>
  </button>
{/if}

<style>
  .new-files-badge {
    display: inline-flex;
    align-items: center;
    gap: 0.375rem;
    padding: 0.375rem 0.625rem;
    font-family: inherit;
    font-size: 0.75rem;
    font-weight: 500;
    color: var(--popover-text, #e0e0e0);
    background: var(--popover-surface, rgba(255, 255, 255, 0.08));
    border: 1px solid var(--popover-border, rgba(255, 255, 255, 0.18));
    border-radius: 8px;
    cursor: pointer;
    transition: background-color 0.15s ease, border-color 0.15s ease;
    box-shadow: inset 0 1px 0 var(--popover-highlight, rgba(255, 255, 255, 0.34));
  }

  .new-files-badge:hover {
    background: rgba(255, 255, 255, 0.12);
    border-color: rgba(255, 255, 255, 0.25);
  }

  .new-files-badge:active {
    background: rgba(255, 255, 255, 0.06);
  }

  .new-files-badge svg {
    color: var(--popover-success, #22c55e);
    flex-shrink: 0;
  }

  .new-files-label {
    white-space: nowrap;
  }
</style>
