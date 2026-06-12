<script lang="ts">
  import type { SyncState } from '../lib/sync-model';
  import { getV4TitleBarModel } from './model';
  import './tokens.css';

  /**
   * V4 title bar — SPEC section 4 + chrome-master.png: 40px tall, traffic
   * lights overlay the top-left (TitleBarStyle::Overlay → 78px left inset),
   * live sync status (6px dot + 13px sentence + text-3 meta), and exactly one
   * contextual primary text action on the right (Sync Now / Cancel / Retry).
   */
  interface Props {
    syncState: SyncState;
    /** Connected workspaces being watched (companies + personal). */
    watchedCount: number;
    /** Human relative last-sync label ("just now", "5m ago"). */
    lastSyncLabel?: string | null;
    /** Company currently transferring, while syncing. */
    syncingCompany?: string | null;
    fanoutDone?: number;
    fanoutTotal?: number;
    /** Plain-language error summary, for error states. */
    errorSummary?: string | null;
    onsync?: () => void;
    oncancel?: () => void;
    onretry?: () => void;
  }

  let {
    syncState,
    watchedCount,
    lastSyncLabel = null,
    syncingCompany = null,
    fanoutDone = 0,
    fanoutTotal = 0,
    errorSummary = null,
    onsync,
    oncancel,
    onretry,
  }: Props = $props();

  const model = $derived(
    getV4TitleBarModel({
      syncState,
      watchedCount,
      lastSyncLabel,
      syncingCompany,
      fanoutDone,
      fanoutTotal,
      errorSummary,
    }),
  );

  function handleAction() {
    if (model.action.id === 'cancel') oncancel?.();
    else if (model.action.id === 'retry') onretry?.();
    else onsync?.();
  }
</script>

<header class="v4-titlebar" data-tauri-drag-region aria-label="Sync status">
  <div class="v4-status">
    <span
      class={`v4-dot ${model.tone}`}
      class:pulsing={syncState === 'syncing'}
      aria-hidden="true"
    ></span>
    <span class="v4-sentence">{model.sentence}</span>
    {#if model.meta}
      <span class="v4-meta">{model.meta}</span>
    {/if}
  </div>
  <button type="button" class="v4-action" onclick={handleAction}>
    {model.action.label}
  </button>
</header>

<style>
  .v4-titlebar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    flex: 0 0 40px;
    height: 40px;
    /* 78px left inset clears the overlay traffic lights. */
    padding: 0 14px 0 78px;
    border-bottom: 1px solid var(--v4-hairline);
    background: var(--v4-ground);
    font-family:
      'Inter Variable',
      Inter,
      -apple-system,
      'SF Pro Text',
      sans-serif;
  }

  .v4-status {
    display: flex;
    align-items: center;
    gap: 8px;
    overflow: hidden;
    min-width: 0;
    /* Children of a drag region must not intercept the drag mousedown. */
    pointer-events: none;
  }

  .v4-dot {
    flex: 0 0 6px;
    width: 6px;
    height: 6px;
    border-radius: 50%;
  }

  .v4-dot.ok {
    background: var(--v4-ok);
  }

  .v4-dot.warn {
    background: var(--v4-warn);
  }

  .v4-dot.error {
    background: var(--v4-error);
  }

  .v4-dot.idle {
    background: var(--v4-idle);
  }

  .v4-dot.pulsing {
    animation: v4-dot-pulse 1.4s ease-in-out infinite;
  }

  @keyframes v4-dot-pulse {
    0%,
    100% {
      opacity: 1;
    }

    50% {
      opacity: 0.35;
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .v4-dot.pulsing {
      animation: none;
    }
  }

  .v4-sentence {
    color: var(--v4-text-1);
    font-size: 13px;
    font-weight: 400;
    line-height: 1;
    white-space: nowrap;
  }

  .v4-meta {
    overflow: hidden;
    min-width: 0;
    color: var(--v4-text-3);
    font-size: 13px;
    font-weight: 400;
    line-height: 1;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .v4-action {
    flex: 0 0 auto;
    padding: 4px 8px;
    border: none;
    border-radius: 6px;
    background: transparent;
    color: var(--v4-text-1);
    font: inherit;
    font-size: 13px;
    font-weight: 400;
    line-height: 1;
    cursor: pointer;
  }

  .v4-action:hover {
    background: var(--v4-control-faint);
  }
</style>
