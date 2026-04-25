<script lang="ts">
  import type { ConflictFile } from '../stores/conflicts';
  import ConflictRow from './ConflictRow.svelte';

  interface Props {
    conflicts: ConflictFile[];
    onresolve: (path: string, strategy: 'keep-local' | 'keep-remote') => void;
    onopen: (path: string) => void;
    ondismiss: () => void;
  }

  let { conflicts, onresolve, onopen, ondismiss }: Props = $props();

  let pendingCount = $derived(
    conflicts.filter((c) => c.status === 'pending').length
  );
  let hasPending = $derived(pendingCount > 0);
  let resolvedCount = $derived(
    conflicts.filter((c) => c.status === 'resolved').length
  );
  let isResolving = $derived(
    conflicts.some((c) => c.status === 'resolving')
  );
</script>

<!-- svelte-ignore a11y_interactive_supports_focus -->
<div
  class="conflict-modal"
  role="dialog"
  aria-label="Resolve sync conflicts"
  onkeydown={(e) => { if (e.key === 'Escape') ondismiss(); }}
>
  <!-- Header -->
  <div class="modal-header">
    <div class="header-left">
      <svg class="warning-icon" width="18" height="18" viewBox="0 0 16 16" fill="none" xmlns="http://www.w3.org/2000/svg">
        <path d="M8 1.5L1 13.5h14L8 1.5Z" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" />
        <path d="M8 6v3" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" />
        <circle cx="8" cy="11.5" r="0.75" fill="currentColor" />
      </svg>
      <h2 class="modal-title">Resolve Conflicts</h2>
      <span class="count-badge">{conflicts.length}</span>
    </div>
    <button class="dismiss-btn" onclick={ondismiss} title="Dismiss" aria-label="Dismiss conflict modal">
      <svg width="14" height="14" viewBox="0 0 16 16" fill="none" xmlns="http://www.w3.org/2000/svg">
        <path d="M12 4L4 12M4 4l8 8" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" />
      </svg>
    </button>
  </div>

  {#if resolvedCount > 0 && resolvedCount < conflicts.length}
    <div class="progress-summary">
      {resolvedCount} of {conflicts.length} resolved
    </div>
  {/if}

  <!-- Scrollable conflict list -->
  <div class="conflict-list">
    {#each conflicts as conflict (conflict.path)}
      <ConflictRow {conflict} {onresolve} {onopen} />
    {/each}
  </div>

  <!-- Footer with bulk actions -->
  <div class="modal-footer">
    <button
      class="bulk-btn bulk-local"
      disabled={!hasPending || isResolving}
      onclick={async () => {
        for (const c of conflicts) {
          if (c.status === 'pending') await onresolve(c.path, 'keep-local');
        }
      }}
    >
      All → Keep Local
    </button>
    <button
      class="bulk-btn bulk-remote"
      disabled={!hasPending || isResolving}
      onclick={async () => {
        for (const c of conflicts) {
          if (c.status === 'pending') await onresolve(c.path, 'keep-remote');
        }
      }}
    >
      All → Keep Remote
    </button>
  </div>
</div>

<style>
  .conflict-modal {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }

  /* Header */
  .modal-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 0 0 0.5rem 0;
    flex-shrink: 0;
  }

  .header-left {
    display: flex;
    align-items: center;
    gap: 0.4rem;
  }

  .warning-icon {
    color: var(--popover-warning, #f59e0b);
    flex-shrink: 0;
  }

  .modal-title {
    font-size: 0.875rem;
    font-weight: 600;
    color: var(--popover-text-heading, #ffffff);
    margin: 0;
    line-height: 1.2;
  }

  .count-badge {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    min-width: 1.25rem;
    height: 1.25rem;
    padding: 0 0.35rem;
    font-size: 0.6875rem;
    font-weight: 600;
    border-radius: 10px;
    background: rgba(245, 158, 11, 0.15);
    color: var(--popover-warning, #f59e0b);
  }

  .dismiss-btn {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 24px;
    height: 24px;
    padding: 0;
    background: none;
    border: none;
    border-radius: 6px;
    color: var(--popover-text-muted, #a0a0b0);
    cursor: pointer;
    transition: background-color 0.1s ease, color 0.1s ease;
  }

  .dismiss-btn:hover {
    background: var(--popover-action-hover, rgba(255, 255, 255, 0.05));
    color: var(--popover-text, #e0e0e0);
  }

  .progress-summary {
    font-size: 0.6875rem;
    color: #22c55e;
    padding-bottom: 0.375rem;
    flex-shrink: 0;
  }

  /* Scrollable list */
  .conflict-list {
    display: flex;
    flex-direction: column;
    gap: 0.375rem;
    overflow-y: auto;
    max-height: 250px;
    min-height: 0;
    flex: 1;

    /* Thin scrollbar */
    scrollbar-width: thin;
    scrollbar-color: var(--popover-border, rgba(255, 255, 255, 0.18)) transparent;
  }

  .conflict-list::-webkit-scrollbar {
    width: 4px;
  }

  .conflict-list::-webkit-scrollbar-track {
    background: transparent;
  }

  .conflict-list::-webkit-scrollbar-thumb {
    background: var(--popover-border, rgba(255, 255, 255, 0.18));
    border-radius: 2px;
  }

  /* Footer */
  .modal-footer {
    display: flex;
    gap: 0.375rem;
    padding-top: 0.5rem;
    flex-shrink: 0;
  }

  .bulk-btn {
    flex: 1;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    padding: 0.4375rem 0.5rem;
    font-size: 0.75rem;
    font-weight: 600;
    font-family: inherit;
    border: none;
    border-radius: 9px;
    cursor: pointer;
    transition: background-color 0.1s ease, opacity 0.1s ease;
  }

  .bulk-btn:disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }

  .bulk-local {
    color: var(--popover-primary-text, #111113);
    background: var(--popover-primary, #ffffff);
  }

  .bulk-local:hover:not(:disabled) {
    background: var(--popover-primary-hover, rgba(255, 255, 255, 0.9));
  }

  .bulk-remote {
    color: var(--popover-text, #e0e0e0);
    background: rgba(245, 158, 11, 0.15);
  }

  .bulk-remote:hover:not(:disabled) {
    background: rgba(245, 158, 11, 0.25);
    color: var(--popover-warning, #f59e0b);
  }

  @media (prefers-color-scheme: light) {
    .progress-summary {
      color: #16a34a;
    }

    .bulk-remote {
      color: var(--popover-text, #374151);
      background: rgba(245, 158, 11, 0.1);
    }

    .bulk-remote:hover:not(:disabled) {
      background: rgba(245, 158, 11, 0.2);
      color: var(--popover-warning, #d97706);
    }
  }
</style>
