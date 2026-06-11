<script lang="ts">
  import { invoke } from '@tauri-apps/api/core';
  import type { Workspace } from '../../lib/workspaces';
  import SyncModeToggle from '../../components/SyncModeToggle.svelte';
  import {
    buildSourceRows,
    type SourceViewModel,
    type SyncProgress,
    type SyncState,
    type WorkspaceSyncStats,
  } from '../lib/sync-model';

  interface Props {
    workspaces: Workspace[];
    syncState: SyncState;
    progress: SyncProgress | null;
    statsBySlug: Record<string, WorkspaceSyncStats>;
    cloudReachable: boolean;
    /** True during the first real-state fetch → skeleton rows, not empty state. */
    loading?: boolean;
    /** Re-fetch workspaces after a successful in-place Connect. */
    onrefresh?: () => void;
  }

  let {
    workspaces,
    syncState,
    progress,
    statsBySlug,
    cloudReachable,
    loading = false,
    onrefresh,
  }: Props = $props();

  const rows = $derived(
    buildSourceRows({
      workspaces,
      syncState,
      progress,
      statsBySlug,
      cloudReachable,
    }),
  );

  function stateLabel(row: SourceViewModel): string {
    switch (row.liveState) {
      case 'syncing':
        return 'Syncing';
      case 'warn':
        return 'Needs attention';
      case 'paused':
        return 'Paused';
      case 'ok':
      default:
        return 'OK';
    }
  }

  // Per-row Connect state: `true` while in flight, an error string after a
  // failure (the row flips to a Retry affordance), absent otherwise. Mirrors
  // the menubar WorkspaceList Connect flow — the same `connect_workspace_to_cloud`
  // command provisions a local-only folder or reconciles a broken manifest row.
  let connectState = $state<Record<string, true | string>>({});

  async function handleConnect(slug: string) {
    if (connectState[slug] === true) return; // block double-clicks while in flight
    connectState = { ...connectState, [slug]: true };
    try {
      await invoke('connect_workspace_to_cloud', { slug });
      const { [slug]: _done, ...rest } = connectState;
      connectState = rest;
      onrefresh?.();
    } catch (err) {
      const msg = String(err);
      console.error('connect_workspace_to_cloud failed:', msg);
      connectState = { ...connectState, [slug]: msg };
    }
  }
</script>

<section class="sources-panel" aria-labelledby="sources-title">
  <div class="panel-header">
    <h2 id="sources-title">Sources</h2>
    <span>{loading && rows.length === 0 ? '' : `${workspaces.length} source${workspaces.length === 1 ? '' : 's'}`}</span>
  </div>

  {#if loading && rows.length === 0}
    <div class="source-table" aria-busy="true" aria-label="Loading sources">
      {#each [0, 1, 2, 3] as row (row)}
        <div class="source-row source-skeleton-row">
          <div class="source-name">
            <span class="skeleton-dot"></span>
            <div style="flex: 1; min-width: 0;">
              <span class="skeleton-line" style="width: 46%"></span>
              <span class="skeleton-line" style="width: 64%"></span>
            </div>
          </div>
          <span class="skeleton-line" style="width: 60%"></span>
          <span class="skeleton-line" style="width: 70%"></span>
          <span class="skeleton-line" style="width: 50%"></span>
          <span class="skeleton-pill"></span>
        </div>
      {/each}
    </div>
  {:else if rows.length === 0}
    <div class="empty-state">
      <p>No syncable workspaces found.</p>
      <span>Connect a workspace from Settings, then sync will show it here.</span>
    </div>
  {:else}
    <div class="source-table" role="table" aria-label="Sync sources">
      <div class="source-row source-head" role="row">
        <span>Name</span>
        <span>Status</span>
        <span>Last sync</span>
        <span>Transferred</span>
        <span>Action</span>
      </div>
      {#each rows as row (row.key)}
        <div class="source-row" role="row" title={row.warning ?? ''}>
          <div class="source-name" role="cell">
            <span class="state-dot {row.liveState}" aria-label={stateLabel(row)}></span>
            <div>
              <div class="source-name-row">
                <strong>{row.name}</strong>
                {#if row.isPersonal}
                  <span class="personal-tag">Personal</span>
                {/if}
              </div>
              <span>{row.detail}</span>
            </div>
          </div>
          <div class="source-status" role="cell">
            <span>{stateLabel(row)}</span>
            {#if row.progressPct !== null}
              <div class="progress-track" aria-label={`${Math.round(row.progressPct)}% complete`}>
                <div
                  class="progress-fill"
                  style={`--progress-scale: ${Math.max(0, Math.min(1, row.progressPct / 100))}`}
                ></div>
              </div>
            {/if}
          </div>
          <span class="source-muted" role="cell">{row.lastSyncLabel}</span>
          <span class="source-muted" role="cell">{row.transferredLabel}</span>
          <div class="source-action" class:has-sync-mode={row.showSyncMode} role="cell">
            {#if row.connectable}
              {@const err = typeof connectState[row.slug] === 'string' ? (connectState[row.slug] as string) : null}
              {@const connecting = connectState[row.slug] === true}
              <button
                type="button"
                class="connect-btn"
                class:connecting
                class:errored={err !== null}
                disabled={connecting || !cloudReachable}
                title={err ?? 'Connect this workspace to the cloud'}
                data-testid="source-connect"
                onclick={() => handleConnect(row.slug)}
              >
                {connecting ? 'Connecting…' : err ? 'Retry' : 'Connect'}
              </button>
            {:else}
              <span class="action-pill {row.action.toLowerCase().replaceAll(' ', '-')}">
                {row.action}
              </span>
              {#if row.showSyncMode}
                <div class="sync-mode-slot">
                  <SyncModeToggle slug={row.slug} {cloudReachable} />
                </div>
              {/if}
            {/if}
          </div>
        </div>
      {/each}
    </div>
  {/if}
</section>

<style>
  .sources-panel {
    min-width: 0;
  }

  .panel-header {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 12px;
    margin-bottom: 10px;
  }

  .panel-header h2 {
    margin: 0;
    color: var(--fg);
    font-size: var(--text-base);
    font-weight: 600;
    line-height: 22px;
  }

  .panel-header span,
  .source-muted,
  .source-name span,
  .source-status span {
    color: var(--muted);
    font-size: var(--text-base);
    line-height: 18px;
  }

  .source-table {
    display: grid;
    gap: 6px;
    min-width: 0;
  }

  .source-row {
    display: grid;
    grid-template-columns: minmax(0, 1.8fr) minmax(0, 1fr) minmax(0, .7fr) minmax(0, .8fr) minmax(0, auto);
    align-items: center;
    gap: 12px;
    min-height: 54px;
    padding: 9px 10px;
    border: 1px solid var(--border);
    border-radius: 8px;
    background: var(--bg);
    transition: transform 140ms cubic-bezier(.2, .7, .2, 1);
  }

  .source-head {
    min-height: 28px;
    padding: 0 10px;
    border: 0;
    background: transparent;
    color: var(--muted);
    font-size: var(--text-micro);
    font-weight: 600;
    line-height: 16px;
    text-transform: uppercase;
    transition: none;
  }

  .source-row:not(.source-head):hover {
    border-color: var(--border-strong);
    box-shadow: 0 1px 2px rgba(0, 0, 0, 0.4);
    transform: translateY(-1px);
  }

  .source-name {
    display: flex;
    align-items: center;
    gap: 9px;
    min-width: 0;
  }

  .source-name div,
  .source-status {
    min-width: 0;
  }

  /* Grid cells must be allowed to shrink below their content width so the
     row's minmax(0, …) tracks never force the table wider than its column.
     Text cells clip with an ellipsis instead of pushing the table out. */
  .source-muted {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .source-head span {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .source-name strong {
    display: block;
    min-width: 0;
    overflow: hidden;
    color: var(--fg);
    font-size: var(--text-base);
    font-weight: 600;
    line-height: 18px;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .source-name span {
    display: block;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .state-dot {
    width: 9px;
    height: 9px;
    border-radius: 999px;
    background: var(--emerald);
    flex: 0 0 auto;
    box-shadow: 0 0 0 3px rgba(52, 211, 153, 0.16);
  }

  .state-dot.syncing {
    background: var(--blue);
    box-shadow: 0 0 0 3px rgba(96, 165, 250, 0.18);
    animation: pulse 1.15s ease-in-out infinite;
  }

  .state-dot.warn {
    background: var(--red);
    box-shadow: 0 0 0 3px rgba(248, 113, 113, 0.16);
  }

  .state-dot.paused {
    background: var(--muted-2);
    box-shadow: 0 0 0 3px rgba(161, 161, 170, 0.16);
  }

  .progress-track {
    width: 100%;
    height: 5px;
    margin-top: 5px;
    overflow: hidden;
    border-radius: 999px;
    background: rgba(255, 255, 255, 0.08);
  }

  .progress-fill {
    width: 100%;
    height: 100%;
    border-radius: inherit;
    background: var(--blue);
    transform: scaleX(var(--progress-scale, 0));
    transform-origin: left center;
    transition: transform 180ms cubic-bezier(.2, .7, .2, 1);
  }

  .action-pill {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    min-width: 74px;
    height: 24px;
    padding: 0 9px;
    border-radius: 999px;
    background: var(--row-active);
    color: var(--muted-2);
    font-size: var(--text-base);
    font-weight: 600;
    white-space: nowrap;
  }

  .action-pill.up-to-date {
    background: rgba(52, 211, 153, 0.12);
    color: var(--emerald);
  }

  .action-pill.syncing {
    background: rgba(96, 165, 250, 0.12);
    color: var(--blue);
  }

  .action-pill.reauth {
    background: rgba(248, 113, 113, 0.12);
    color: var(--red);
  }

  .action-pill.needs-attention {
    background: rgba(248, 113, 113, 0.12);
    color: var(--red);
  }

  .action-pill.paused {
    background: var(--row-active);
    color: var(--muted-3);
  }

  .source-name-row {
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
  }

  /* Higher specificity than the broad `.source-name span` rule above so the
     tag stays an inline pill (not a clipped block) with its own color/size. */
  .source-name .personal-tag {
    display: inline-flex;
    align-items: center;
    flex: 0 0 auto;
    height: 16px;
    padding: 0 6px;
    border: 1px solid rgba(99, 102, 241, 0.32);
    border-radius: 999px;
    background: rgba(99, 102, 241, 0.14);
    color: #a5a8ff;
    font-size: var(--text-base);
    font-weight: 600;
    line-height: 1;
    letter-spacing: 0.02em;
    overflow: visible;
    white-space: nowrap;
  }

  .source-action {
    position: relative;
    display: inline-flex;
    align-items: center;
    justify-content: flex-end;
    min-width: 74px;
  }

  /* Inline Connect for local-only / broken rows. An actionable button (not a
     status pill) so the Sources tab can reconcile a workspace to the cloud
     without bouncing the user back to the menubar. */
  .connect-btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    min-width: 74px;
    height: 24px;
    padding: 0 11px;
    border: 1px solid rgba(96, 165, 250, 0.45);
    border-radius: 999px;
    background: rgba(96, 165, 250, 0.12);
    color: var(--blue);
    font-size: var(--text-base);
    font-weight: 600;
    white-space: nowrap;
    cursor: pointer;
    transition: background-color 0.12s ease, border-color 0.12s ease, opacity 0.12s ease;
  }

  .connect-btn:hover:not(:disabled) {
    background: rgba(96, 165, 250, 0.2);
    border-color: rgba(96, 165, 250, 0.65);
  }

  .connect-btn:disabled {
    opacity: 0.55;
    cursor: default;
  }

  .connect-btn.connecting {
    color: var(--muted-2);
  }

  .connect-btn.errored {
    border-color: rgba(248, 113, 113, 0.5);
    background: rgba(248, 113, 113, 0.12);
    color: var(--red);
  }

  .action-pill {
    transition: opacity 0.12s ease;
  }

  .sync-mode-slot {
    position: absolute;
    top: 50%;
    right: 0;
    transform: translateY(-50%);
    opacity: 0;
    pointer-events: none;
    transition: opacity 0.12s ease;
  }

  .source-row:hover .source-action.has-sync-mode .action-pill,
  .source-row:focus-within .source-action.has-sync-mode .action-pill {
    opacity: 0;
  }

  .source-row:hover .source-action.has-sync-mode .sync-mode-slot,
  .source-row:focus-within .source-action.has-sync-mode .sync-mode-slot {
    opacity: 1;
    pointer-events: auto;
  }

  /* ---- loading skeletons ------------------------------------------------ */
  .source-skeleton-row:hover {
    border-color: var(--border);
    box-shadow: none;
    transform: none;
  }

  .source-skeleton-row .skeleton-dot {
    width: 9px;
    height: 9px;
    border-radius: 999px;
    background: var(--row-active);
    flex: 0 0 auto;
    animation: sources-skeleton-pulse 1.2s ease-in-out infinite;
  }

  .source-skeleton-row .skeleton-line {
    display: block;
    height: 10px;
    margin: 3px 0;
    border-radius: 999px;
    background: var(--row-active);
    animation: sources-skeleton-pulse 1.2s ease-in-out infinite;
  }

  .source-skeleton-row .skeleton-pill {
    display: block;
    width: 74px;
    height: 24px;
    border-radius: 999px;
    background: var(--row-active);
    animation: sources-skeleton-pulse 1.2s ease-in-out infinite;
  }

  @keyframes sources-skeleton-pulse {
    0%,
    100% {
      opacity: 0.5;
    }
    50% {
      opacity: 1;
    }
  }

  .empty-state {
    padding: 28px;
    border: 1px dashed var(--border-strong);
    border-radius: 8px;
    background: var(--bg);
    text-align: center;
  }

  .empty-state p {
    margin: 0 0 4px;
    color: var(--fg);
    font-weight: 600;
  }

  .empty-state span {
    color: var(--muted);
    font-size: var(--text-base);
  }

  @keyframes pulse {
    0%,
    100% {
      opacity: 0.45;
    }
    50% {
      opacity: 1;
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .source-row,
    .progress-fill {
      transition: none;
    }

    .source-row:not(.source-head):hover {
      transform: none;
    }

    .state-dot.syncing {
      animation: none;
    }

    .source-skeleton-row .skeleton-dot,
    .source-skeleton-row .skeleton-line,
    .source-skeleton-row .skeleton-pill {
      animation: none;
    }
  }

  @media (max-width: 900px) {
    .source-head {
      display: none;
    }

    .source-row {
      grid-template-columns: minmax(0, 1fr) auto;
    }

    .source-status,
    .source-muted {
      display: none;
    }
  }
</style>
