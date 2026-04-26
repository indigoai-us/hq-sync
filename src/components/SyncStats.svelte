<script lang="ts">
  import { invoke } from '@tauri-apps/api/core';
  import { untrack } from 'svelte';

  interface SyncStatus {
    lastSyncAt: string | null;
    pendingFiles: number;
    conflicts: number;
    daemonRunning: boolean;
    source: string;
  }

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

    <div class="stat-row">
      <svg class="stat-icon" width="14" height="14" viewBox="0 0 16 16" fill="none" xmlns="http://www.w3.org/2000/svg">
        <path d="M9 1.5H3.5a1 1 0 0 0-1 1v11a1 1 0 0 0 1 1h9a1 1 0 0 0 1-1V6L9 1.5Z" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" />
        <path d="M9 1.5V6h4.5" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" />
      </svg>
      <span class="stat-label">Pending</span>
      <span class="stat-value">
        {status.pendingFiles} file{status.pendingFiles !== 1 ? 's' : ''}
      </span>
    </div>

    {#if status.conflicts > 0}
      <div class="stat-row conflict">
        <svg class="stat-icon" width="14" height="14" viewBox="0 0 16 16" fill="none" xmlns="http://www.w3.org/2000/svg">
          <path d="M8 1.5L1 13.5h14L8 1.5Z" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" />
          <path d="M8 6v3" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" />
          <circle cx="8" cy="11.5" r="0.75" fill="currentColor" />
        </svg>
        <span class="stat-label">Conflicts</span>
        <span class="stat-value">{status.conflicts}</span>
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
    font-size: 0.7rem;
    color: #a0a0b0;
    margin: 0;
    text-align: center;
  }

  .stat-row {
    display: flex;
    align-items: center;
    gap: 0.4rem;
    font-size: 0.78rem;
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

  .stat-row.conflict .stat-icon,
  .stat-row.conflict .stat-label {
    color: #f59e0b;
  }

  .stat-row.conflict .stat-value {
    color: #f59e0b;
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

    .stats-error {
      color: #6b7280;
    }

    .stat-row.conflict .stat-icon,
    .stat-row.conflict .stat-label,
    .stat-row.conflict .stat-value {
      color: #d97706;
    }
  }
</style>
