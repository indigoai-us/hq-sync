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
      <!-- Naked small grey text — a quiet timestamp, not a card. Still opens
           Recent Changes on click (no icon/chevron chrome; hover lifts the
           colour to hint the affordance). -->
      <button
        type="button"
        class="last-synced"
        onclick={onhistory}
        title="View recent changes"
        aria-label="View recent changes"
      >
        Last synced {status.lastSyncAt ? timeAgo(status.lastSyncAt) : 'never'}
      </button>
    {:else}
      <p class="last-synced last-synced-static">
        Last synced {status.lastSyncAt ? timeAgo(status.lastSyncAt) : 'never'}
      </p>
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
    /* Naked — no card. The last-synced line is just quiet text in the body;
       the conflict row (when present) sits directly beneath it. */
    width: 100%;
    box-sizing: border-box;
    display: flex;
    flex-direction: column;
    gap: 0.4rem;
  }

  /* "Last synced X ago" — small, grey, no chrome. As a button it stays a
     Recent-Changes affordance; hover lifts the colour. Left-aligned, hugs
     its text so it reads as a caption, not a full-width row. */
  .last-synced {
    align-self: flex-start;
    margin: 0;
    padding: 0;
    border: none;
    background: none;
    font-family: inherit;
    font-size: var(--text-xs, 0.6875rem);
    line-height: 1.3;
    color: var(--popover-text-muted, rgba(255, 255, 255, 0.52));
    text-align: left;
    cursor: pointer;
    transition: color 0.1s ease;
  }

  .last-synced-static {
    cursor: default;
  }

  .last-synced:hover {
    color: var(--popover-text, rgba(255, 255, 255, 0.86));
  }

  .last-synced-static:hover {
    color: var(--popover-text-muted, rgba(255, 255, 255, 0.52));
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
  }
</style>
