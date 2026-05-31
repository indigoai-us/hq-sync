<script lang="ts">
  import { invoke } from '@tauri-apps/api/core';
  import { untrack } from 'svelte';
  import CopyPromptButton from './CopyPromptButton.svelte';

  interface SyncStatus {
    lastSyncAt: string | null;
    pendingFiles: number;
    conflicts: number;
    daemonRunning: boolean;
    source: string;
  }

  interface Props {
    /** Open the Recent Changes (activity log) window. When provided, the
     *  "Last synced" row becomes a clickable history affordance — replaces
     *  the old footer "Recent Changes" action. Omitted → static row. */
    onhistory?: () => void;
  }

  let { onhistory }: Props = $props();

  let status = $state<SyncStatus | null>(null);
  let loading = $state(true);
  let error = $state('');

  function timeAgo(isoDate: string): string {
    const now = Date.now();
    const then = new Date(isoDate).getTime();
    if (isNaN(then)) return 'unknown';
    const seconds = Math.floor((now - then) / 1000);

    if (seconds < 0) return 'just now';
    if (seconds < 60) return 'just now';
    if (seconds < 3600) {
      const m = Math.floor(seconds / 60);
      return `${m} minute${m > 1 ? 's' : ''} ago`;
    }
    if (seconds < 86400) {
      const h = Math.floor(seconds / 3600);
      return `${h} hour${h > 1 ? 's' : ''} ago`;
    }
    const d = Math.floor(seconds / 86400);
    return `${d} day${d > 1 ? 's' : ''} ago`;
  }

  export async function refresh() {
    loading = true;
    error = '';
    try {
      status = await invoke<SyncStatus>('get_sync_status');
    } catch (e) {
      error = String(e);
    } finally {
      loading = false;
    }
  }

  $effect(() => {
    // Mount-only: untrack prevents re-fire when refresh() mutates $state
    untrack(() => refresh());
  });
</script>

<div class="sync-stats">
  {#if loading}
    <div class="stats-loading">
      <span class="dot-spinner"></span>
    </div>
  {:else if error}
    <p class="stats-error">{error}</p>
  {:else if status}
    {#if onhistory}
      <!-- Clickable: opens Recent Changes (activity log). The clock icon +
           trailing chevron signal that the synced state is browsable. -->
      <button
        type="button"
        class="stat-row stat-row-action"
        onclick={onhistory}
        title="View recent changes"
        aria-label="View recent changes"
      >
        <svg class="stat-icon" width="14" height="14" viewBox="0 0 16 16" fill="none" xmlns="http://www.w3.org/2000/svg">
          <circle cx="8" cy="8" r="6.5" stroke="currentColor" stroke-width="1.5" />
          <path d="M8 4.5V8l2.5 2" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" />
        </svg>
        <span class="stat-label">Last synced</span>
        <span class="stat-value">
          {status.lastSyncAt ? timeAgo(status.lastSyncAt) : 'never'}
        </span>
        <svg class="stat-chevron" width="12" height="12" viewBox="0 0 16 16" fill="none" xmlns="http://www.w3.org/2000/svg" aria-hidden="true">
          <path d="M6 4l4 4-4 4" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" />
        </svg>
      </button>
    {:else}
      <div class="stat-row">
        <svg class="stat-icon" width="14" height="14" viewBox="0 0 16 16" fill="none" xmlns="http://www.w3.org/2000/svg">
          <circle cx="8" cy="8" r="6.5" stroke="currentColor" stroke-width="1.5" />
          <path d="M8 4.5V8l2.5 2" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" />
        </svg>
        <span class="stat-label">Last synced</span>
        <span class="stat-value">
          {status.lastSyncAt ? timeAgo(status.lastSyncAt) : 'never'}
        </span>
      </div>
    {/if}

    {#if status.conflicts > 0}
      <div class="stat-row conflict">
        <svg class="stat-icon" width="14" height="14" viewBox="0 0 16 16" fill="none" xmlns="http://www.w3.org/2000/svg">
          <path d="M8 1.5L1 13.5h14L8 1.5Z" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" />
          <path d="M8 6v3" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" />
          <circle cx="8" cy="11.5" r="0.75" fill="currentColor" />
        </svg>
        <span class="stat-label">Conflicts</span>
        <span class="stat-value">{status.conflicts}</span>
        <CopyPromptButton
          variant="compact"
          label="Copy resolve-conflicts prompt"
          issue={{ kind: 'sync-conflict', payload: { count: status.conflicts } }}
        />
      </div>
    {/if}
  {/if}
</div>

<style>
  .sync-stats {
    /* Fill the popover-body content width. Removed max-width:280px —
       it left the card left-aligned in a 286px space with 6px of dead
       space on the right, making "never" / "0 files" look pushed off
       the edge relative to the centered sync button beneath it.
       box-sizing:border-box so the 1px border + padding are counted
       inside width:100% (prevents a 2px horizontal overflow). */
    width: 100%;
    box-sizing: border-box;
    display: flex;
    flex-direction: column;
    gap: 0.4rem;
    /* margin-top removed — parent popover-body already has
       gap:0.75rem between children, stacking margin-top on top of
       that creates doubled space above the card. */
    padding: 0.6rem 0.75rem;
    border-radius: 12px;
    background: var(--popover-surface, rgba(255, 255, 255, 0.08));
    border: 1px solid var(--popover-border, rgba(255, 255, 255, 0.18));
    box-shadow: inset 0 1px 0 var(--popover-highlight, rgba(255, 255, 255, 0.34));
  }

  .stats-loading {
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 0.5rem 0;
  }

  .dot-spinner {
    display: inline-block;
    width: 14px;
    height: 14px;
    border: 2px solid var(--popover-progress-track, rgba(255, 255, 255, 0.14));
    border-top-color: var(--popover-progress-fill, #ffffff);
    border-radius: 50%;
    animation: spin 0.7s linear infinite;
  }

  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }

  .stats-error {
    /* Snapped to canonical 11px (v0.1.85) — was 0.7rem (11.2px). */
    font-size: 0.6875rem;
    color: #a0a0b0;
    margin: 0;
    text-align: center;
  }

  .stat-row {
    display: flex;
    align-items: center;
    gap: 0.4rem;
    /* Snapped to canonical 12px (v0.1.85) — was 0.78rem (12.48px). */
    font-size: 0.75rem;
  }

  .stat-icon {
    flex-shrink: 0;
    color: #a0a0b0;
  }

  .stat-label {
    color: #a0a0b0;
  }

  .stat-value {
    margin-left: auto;
    color: #e0e0e0;
    font-weight: 500;
  }

  /* "Last synced" rendered as a history button. Negative margins let the
     hover background bleed to the card edges while the row stays visually
     aligned with a static stat-row. */
  .stat-row-action {
    width: calc(100% + 1.5rem);
    margin: -0.35rem -0.75rem;
    padding: 0.35rem 0.75rem;
    background: none;
    border: none;
    font-family: inherit;
    color: inherit;
    cursor: pointer;
    border-radius: var(--radius-md);
    transition: background-color 0.1s ease;
  }

  .stat-row-action:hover {
    background: var(--popover-action-hover, rgba(255, 255, 255, 0.1));
  }

  /* Trailing chevron — sits flush right after the value, nudges on hover to
     reinforce the navigability. */
  .stat-chevron {
    flex-shrink: 0;
    margin-left: 0.25rem;
    color: #a0a0b0;
    transition: transform 0.12s ease, color 0.12s ease;
  }

  .stat-row-action:hover .stat-chevron {
    color: #e0e0e0;
    transform: translateX(2px);
  }

  /* Conflict row: same calm grey as a normal stat row. The Copy-prompt button
     to the right of the value carries the affordance; severity colour was
     removed (per design: errors/warnings of any kind are a grey friendly
     notice, never yellow or red). */
  .stat-row.conflict .stat-value {
    /* No margin-left:auto here — the CopyPromptButton takes the right edge
       slot, and the value sits to its left at the natural flex position.
       Reapply auto-margin so the count + button hug the right side together. */
    margin-left: auto;
    font-weight: 600;
  }

  @media (prefers-color-scheme: light) {
    .sync-stats {
      background: var(--popover-surface, rgba(255, 255, 255, 0.5));
      border-color: var(--popover-border, rgba(0, 0, 0, 0.12));
    }

    .stat-icon {
      color: #6b7280;
    }

    .stat-label {
      color: #6b7280;
    }

    .stat-value {
      color: #111113;
    }

    .stat-chevron {
      color: #6b7280;
    }

    .stat-row-action:hover .stat-chevron {
      color: #111113;
    }

    .stats-error {
      color: #6b7280;
    }
  }
</style>
