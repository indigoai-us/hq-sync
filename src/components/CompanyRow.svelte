<script lang="ts">
  // Per-company row rendered above SyncStats in the popover. Layout:
  //   name  · source-badge · last-synced · [Sync button]
  //
  // Source-aware click:
  //   aws | both → invoke('start_sync')      (existing flow)
  //   local      → invoke('promote_company') → auto start_sync on complete
  //
  // Promotion state + error recovery live in the `companiesState` store so
  // App.svelte's event listeners can drive the UI without prop-drilling.
  // Styling intentionally reuses SyncStats' `.stat-row` / `.stat-value`
  // tokens — no new badge component, no new button component.
  import { invoke } from '@tauri-apps/api/core';
  import type { CompanyInfo } from '../lib/stores';
  import { companiesState } from '../lib/stores';

  interface Props {
    company: CompanyInfo;
    /** ISO timestamp of last sync for this slug (from journal). Optional —
     *  missing value renders as 'never'. */
    lastSyncedAt?: string | null;
  }

  let { company, lastSyncedAt = null }: Props = $props();

  // Relative-time helper mirrors SyncStats.svelte's `timeAgo`. Kept local
  // (not extracted) to match the PRD's "don't over-componentize" note; the
  // sibling-project `EmbeddingsRow` didn't land here so there's nothing to
  // import. If a shared helper appears later, both sites should migrate.
  function timeAgo(iso: string | null): string {
    if (!iso) return 'never';
    const now = Date.now();
    const then = new Date(iso).getTime();
    if (isNaN(then)) return 'unknown';
    const seconds = Math.floor((now - then) / 1000);
    if (seconds < 0 || seconds < 60) return 'just now';
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

  // Reactive slice of the store for this slug — derived per row so we
  // don't re-render every row when an unrelated slug changes state.
  let isPromoting = $derived($companiesState.promoting.has(company.slug));
  let errorMsg = $derived($companiesState.lastError.get(company.slug));

  // Badge copy + class. `both` means we have both AWS + local (already
  // promoted); render as `Synced` (green/neutral).
  let badgeLabel = $derived(
    company.source === 'local' ? 'Local' : company.source === 'both' ? 'Synced' : 'AWS'
  );
  let badgeClass = $derived(company.source === 'local' ? 'badge-local' : 'badge-aws');

  async function handleSync() {
    if (isPromoting) return;

    if (company.source === 'local') {
      // Optimistic — flip the store before IPC. App.svelte's `promote:*`
      // listeners will reconcile (complete → start_sync, error → revert).
      companiesState.startPromote(company.slug);
      try {
        await invoke('promote_company', { slug: company.slug });
      } catch (e) {
        // Spawn-level failure (backend couldn't even start the runner).
        // Treat uniformly with stream-level promote:error per PRD caution.
        companiesState.setPromoteError(company.slug, String(e));
      }
    } else {
      // AWS-only or already promoted — straight to sync.
      try {
        await invoke('start_sync');
      } catch (e) {
        console.error('start_sync failed:', e);
      }
    }
  }

  function handleRetry() {
    companiesState.clearPromoteError(company.slug);
    handleSync();
  }
</script>

<div class="company-row-wrapper">
  <div class="stat-row">
    <span class="company-name">{company.name}</span>
    <span class="badge {badgeClass}">{badgeLabel}</span>
    <span class="stat-value synced-time">{timeAgo(lastSyncedAt)}</span>
    <button
      class="row-sync-button"
      class:promoting={isPromoting}
      disabled={isPromoting}
      onclick={handleSync}
      aria-label={isPromoting ? 'Promoting' : 'Sync'}
    >
      {#if isPromoting}
        <span class="row-spinner" data-testid="row-spinner" aria-hidden="true"></span>
        <span class="row-sync-label">Promoting</span>
      {:else}
        <span class="row-sync-label">Sync</span>
      {/if}
    </button>
  </div>
  {#if errorMsg}
    <div class="row-error">
      <span class="row-error-msg">{errorMsg}</span>
      <button class="row-retry-button" onclick={handleRetry}>Retry</button>
    </div>
  {/if}
</div>

<style>
  /* Reuse SyncStats' visual language: same container radius, same
     neutral row typography. Inline here (rather than @import) because
     Svelte scopes styles per-component and cross-component imports are
     verbose for a 60-line shared block. See SyncStats.svelte for the
     token source. */
  .company-row-wrapper {
    width: 100%;
    box-sizing: border-box;
    display: flex;
    flex-direction: column;
    gap: 0.25rem;
    padding: 0.5rem 0.75rem;
    border-radius: 8px;
    background: rgba(99, 102, 241, 0.06);
    border: 1px solid rgba(99, 102, 241, 0.12);
  }

  .stat-row {
    display: flex;
    align-items: center;
    gap: 0.4rem;
    font-size: 0.78rem;
  }

  .company-name {
    font-weight: 600;
    color: #e0e0e0;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    min-width: 0;
    flex-shrink: 1;
  }

  .badge {
    flex-shrink: 0;
    font-size: 0.625rem;
    font-weight: 600;
    line-height: 1;
    padding: 0.1875rem 0.4375rem;
    border-radius: 999px;
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }

  .badge-aws {
    background: rgba(34, 197, 94, 0.15);
    color: #4ade80;
    border: 1px solid rgba(34, 197, 94, 0.3);
  }

  .badge-local {
    background: rgba(245, 158, 11, 0.15);
    color: #f59e0b;
    border: 1px solid rgba(245, 158, 11, 0.3);
  }

  .synced-time {
    margin-left: auto;
    color: #a0a0b0;
    font-weight: 400;
    font-size: 0.7rem;
    white-space: nowrap;
    flex-shrink: 0;
  }

  .row-sync-button {
    flex-shrink: 0;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 0.375rem;
    /* Fixed width so the row doesn't reflow when the label flips
       Sync → Promoting (Promoting + spinner + gap ≈ 78px, padded to
       90px for breathing room on Retina/sub-pixel rounding). */
    min-width: 90px;
    padding: 0.25rem 0.625rem;
    font-size: 0.7rem;
    font-weight: 600;
    font-family: inherit;
    color: #ffffff;
    background-color: #6366f1;
    border: none;
    border-radius: 6px;
    cursor: pointer;
    transition: background-color 0.1s ease, opacity 0.1s ease;
  }

  .row-sync-button:hover:not(:disabled) {
    background-color: #4f46e5;
  }

  /* Promoting / disabled — match SyncStats' subdued treatment so the
     button reads "in progress, don't click" without looking broken. */
  .row-sync-button:disabled {
    opacity: 0.7;
    cursor: default;
  }

  .row-sync-button.promoting {
    background-color: #6366f1;
    /* No hover lift while promoting — the :hover:not(:disabled) rule
       already handles this, but state the intent for clarity. */
  }

  .row-sync-label {
    /* Keep label vertically centered next to the spinner and avoid
       baseline shift on flip. */
    line-height: 1;
  }

  /* Small 14px inline spinner — matches SyncStats' `.dot-spinner` sizing
     but inverted (white on purple) to sit on the button. */
  .row-spinner {
    display: inline-block;
    width: 14px;
    height: 14px;
    border: 2px solid rgba(255, 255, 255, 0.35);
    border-top-color: #ffffff;
    border-radius: 50%;
    animation: spin 0.7s linear infinite;
    flex-shrink: 0;
  }

  .row-error {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    font-size: 0.7rem;
    color: #ef4444;
  }

  .row-error-msg {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .row-retry-button {
    flex-shrink: 0;
    padding: 0.1875rem 0.5rem;
    font-size: 0.6875rem;
    font-weight: 600;
    font-family: inherit;
    color: #ffffff;
    background-color: #ef4444;
    border: none;
    border-radius: 5px;
    cursor: pointer;
  }

  .row-retry-button:hover {
    background-color: #dc2626;
  }

  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }

  @media (prefers-color-scheme: light) {
    .company-row-wrapper {
      background: rgba(99, 102, 241, 0.05);
      border-color: rgba(99, 102, 241, 0.15);
    }
    .company-name {
      color: #1a1a2e;
    }
    .synced-time {
      color: #6b7280;
    }
    .badge-aws {
      background: rgba(34, 197, 94, 0.12);
      color: #16a34a;
      border-color: rgba(34, 197, 94, 0.3);
    }
    .badge-local {
      background: rgba(245, 158, 11, 0.12);
      color: #d97706;
      border-color: rgba(245, 158, 11, 0.35);
    }
  }
</style>
