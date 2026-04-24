<script lang="ts">
  import { onMount } from 'svelte';
  import { embeddingsStore, type EmbeddingsStatusValue } from '../stores/embeddings';

  // Local reactive mirror of the non-reactive class store. Svelte 5 runes
  // can't subscribe to an arbitrary subscriber-pattern singleton directly,
  // so we bridge via a $state tuple that we re-pull every time the store
  // fires `notify()`.
  let status = $state<EmbeddingsStatusValue>(embeddingsStore.status);
  let lastRunAt = $state<string | undefined>(embeddingsStore.lastRunAt);
  let errorMsg = $state<string | undefined>(embeddingsStore.errorMsg);
  let liveLine = $state<string | undefined>(embeddingsStore.liveLine);

  onMount(() => {
    const unsub = embeddingsStore.subscribe(() => {
      status = embeddingsStore.status;
      lastRunAt = embeddingsStore.lastRunAt;
      errorMsg = embeddingsStore.errorMsg;
      liveLine = embeddingsStore.liveLine;
    });
    // Seed from the Rust journal so a freshly-opened popover doesn't flash
    // "Pending" for runs that completed in a prior session.
    embeddingsStore.refresh();
    return unsub;
  });

  // Shared relative-time formatter. Mirrors SyncStats' timeAgo so the two
  // rows read consistently ("2 minutes ago" vs "2 min ago"). Kept inline —
  // the installer has a similar helper, but exporting across apps isn't
  // justified for ~15 lines.
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

  // Trim stdout lines that would overflow the popover. The PRD calls for
  // "~80 chars"; the exact number isn't load-bearing, it just needs to keep
  // the progress line on one row in a 320px-wide menubar window.
  function truncateLine(line: string, max = 80): string {
    if (line.length <= max) return line;
    return '…' + line.slice(-(max - 1));
  }

  async function handleRetry() {
    if (status === 'running') return; // button is disabled in this state, but double-guard
    try {
      await embeddingsStore.startNow('manual');
    } catch (err) {
      // If start_embeddings rejects (e.g. "already running"), surface it
      // inline as an error so the user gets feedback rather than a silent
      // no-op. Real error propagation will come from the subsequent
      // `embeddings:error` event for spawn-time failures.
      console.error('[embeddings] start_embeddings failed:', err);
    }
  }
</script>

<div class="embeddings-row" data-embeddings-status={status}>
  <div class="stat-row">
    <!-- Sparkle / index icon — distinct from SyncStats' clock/file icons -->
    <svg
      class="stat-icon"
      width="14"
      height="14"
      viewBox="0 0 16 16"
      fill="none"
      xmlns="http://www.w3.org/2000/svg"
      aria-hidden="true"
    >
      <path
        d="M8 2v2M8 12v2M2 8h2M12 8h2M4.22 4.22l1.42 1.42M10.36 10.36l1.42 1.42M4.22 11.78l1.42-1.42M10.36 5.64l1.42-1.42"
        stroke="currentColor"
        stroke-width="1.5"
        stroke-linecap="round"
      />
      <circle cx="8" cy="8" r="1.5" fill="currentColor" />
    </svg>
    <span class="stat-label">Embeddings</span>

    {#if status === 'running'}
      <span class="stat-value running" data-status="running">
        <span class="dot-spinner" aria-hidden="true"></span>
        Running…
      </span>
    {:else if status === 'ok'}
      <span class="stat-value ok" data-status="ok">
        {lastRunAt ? timeAgo(lastRunAt) : 'Up to date'}
      </span>
    {:else if status === 'error'}
      <span class="stat-value error" data-status="error">Failed</span>
    {:else if status === 'pending'}
      <span class="stat-value pending" data-status="pending">Pending…</span>
    {:else}
      <span class="stat-value" data-status="idle">
        {lastRunAt ? timeAgo(lastRunAt) : 'never'}
      </span>
    {/if}
  </div>

  {#if status === 'running' && liveLine}
    <!-- Live progress tail — reuses the same `.live-progress` aesthetic as
         the sync section of Popover.svelte. Kept inline (not in the parent's
         scope) so this row is self-contained and the test can read the text
         from one component. -->
    <div class="live-progress" data-embeddings-live>
      <p class="live-line" title={liveLine}>{truncateLine(liveLine)}</p>
    </div>
  {/if}

  {#if status === 'error'}
    <div class="embeddings-error-footer">
      {#if errorMsg}
        <p class="error-message" title={errorMsg}>{errorMsg}</p>
      {/if}
      <button
        type="button"
        class="retry-button"
        onclick={handleRetry}
        disabled={false}
      >
        Retry
      </button>
    </div>
  {/if}
</div>

<style>
  /* Card styling matches SyncStats so the two stat panels read as a pair —
     same tint, same border, same padding. Embeddings intentionally gets its
     own card (not a shared wrapper) so expanding states (live progress,
     Retry row) don't force SyncStats to grow alongside it. */
  .embeddings-row {
    width: 100%;
    box-sizing: border-box;
    display: flex;
    flex-direction: column;
    gap: 0.4rem;
    padding: 0.6rem 0.75rem;
    border-radius: 8px;
    background: rgba(99, 102, 241, 0.06);
    border: 1px solid rgba(99, 102, 241, 0.12);
  }

  @media (prefers-color-scheme: light) {
    .embeddings-row {
      background: rgba(99, 102, 241, 0.05);
      border-color: rgba(99, 102, 241, 0.15);
    }
  }

  /* Row layout matches SyncStats' .stat-row. We don't import the SyncStats
     styles because Svelte scopes CSS per-component; instead the shared look
     comes from both components following the same visual grammar (14px
     icon + muted label + right-aligned value). */
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
    display: inline-flex;
    align-items: center;
    gap: 0.375rem;
  }

  .stat-value.running {
    color: #6366f1;
  }
  .stat-value.error {
    color: #ef4444;
    font-weight: 600;
  }
  .stat-value.ok {
    color: #10b981;
  }
  .stat-value.pending {
    color: #a0a0b0;
    font-style: italic;
  }

  .dot-spinner {
    display: inline-block;
    width: 10px;
    height: 10px;
    border: 1.5px solid rgba(99, 102, 241, 0.25);
    border-top-color: #6366f1;
    border-radius: 50%;
    animation: spin 0.7s linear infinite;
  }

  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }

  /* Live progress — echoes Popover's `.live-progress` visual style. */
  .live-progress {
    display: flex;
    flex-direction: column;
    gap: 0.25rem;
    padding: 0.375rem 0.5rem;
    border-radius: 6px;
    background: rgba(99, 102, 241, 0.06);
  }

  .live-line {
    margin: 0;
    font-family: ui-monospace, SFMono-Regular, Menlo, Monaco, monospace;
    font-size: 0.6875rem;
    line-height: 1.35;
    color: #a0a0b0;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .embeddings-error-footer {
    display: flex;
    align-items: center;
    gap: 0.5rem;
  }

  .error-message {
    margin: 0;
    font-size: 0.6875rem;
    color: #ef4444;
    line-height: 1.3;
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .retry-button {
    font-size: 0.6875rem;
    font-family: inherit;
    padding: 0.25rem 0.625rem;
    background: rgba(99, 102, 241, 0.15);
    color: #e0e0e0;
    border: 1px solid rgba(99, 102, 241, 0.3);
    border-radius: 5px;
    cursor: pointer;
    flex-shrink: 0;
  }

  .retry-button:hover:not(:disabled) {
    background: rgba(99, 102, 241, 0.25);
  }

  .retry-button:disabled {
    opacity: 0.5;
    cursor: default;
  }

  @media (prefers-color-scheme: light) {
    .stat-icon,
    .stat-label {
      color: #6b7280;
    }
    .stat-value {
      color: #1a1a2e;
    }
    .stat-value.error {
      color: #b91c1c;
    }
    .stat-value.ok {
      color: #047857;
    }
    .retry-button {
      background: rgba(99, 102, 241, 0.1);
      color: #1a1a2e;
      border-color: rgba(99, 102, 241, 0.35);
    }
  }
</style>
