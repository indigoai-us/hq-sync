<script lang="ts">
  import type { Workspace } from '../../lib/workspaces';
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
  }

  let { workspaces, syncState, progress, statsBySlug, cloudReachable }: Props = $props();

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
</script>

<section class="sources-panel" aria-labelledby="sources-title">
  <div class="panel-header">
    <h2 id="sources-title">Sources</h2>
    <span>{workspaces.length} source{workspaces.length === 1 ? '' : 's'}</span>
  </div>

  {#if rows.length === 0}
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
              <strong>{row.name}</strong>
              <span>{row.detail}</span>
            </div>
          </div>
          <div class="source-status" role="cell">
            <span>{stateLabel(row)}</span>
            {#if row.progressPct !== null}
              <div class="progress-track" aria-label={`${Math.round(row.progressPct)}% complete`}>
                <div class="progress-fill" style={`width: ${row.progressPct}%`}></div>
              </div>
            {/if}
          </div>
          <span class="source-muted" role="cell">{row.lastSyncLabel}</span>
          <span class="source-muted" role="cell">{row.transferredLabel}</span>
          <span class="action-pill {row.action.toLowerCase().replaceAll(' ', '-')}" role="cell">
            {row.action}
          </span>
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
    color: #18181b;
    font-size: 15px;
    font-weight: 680;
    line-height: 22px;
  }

  .panel-header span,
  .source-muted,
  .source-name span,
  .source-status span {
    color: #71717a;
    font-size: 12px;
    line-height: 18px;
  }

  .source-table {
    display: grid;
    gap: 6px;
  }

  .source-row {
    display: grid;
    grid-template-columns: minmax(190px, 1.8fr) minmax(150px, 1fr) minmax(84px, .7fr) minmax(92px, .8fr) auto;
    align-items: center;
    gap: 12px;
    min-height: 54px;
    padding: 9px 10px;
    border: 1px solid #e4e4e7;
    border-radius: 8px;
    background: #ffffff;
  }

  .source-head {
    min-height: 28px;
    padding: 0 10px;
    border: 0;
    background: transparent;
    color: #71717a;
    font-size: 11px;
    font-weight: 650;
    line-height: 16px;
    text-transform: uppercase;
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

  .source-name strong {
    display: block;
    min-width: 0;
    overflow: hidden;
    color: #18181b;
    font-size: 13px;
    font-weight: 650;
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
    background: #22c55e;
    flex: 0 0 auto;
  }

  .state-dot.syncing {
    background: #2563eb;
    animation: pulse 1.15s ease-in-out infinite;
  }

  .state-dot.warn {
    background: #e11d48;
  }

  .state-dot.paused {
    background: #a1a1aa;
  }

  .progress-track {
    width: 100%;
    height: 5px;
    margin-top: 5px;
    overflow: hidden;
    border-radius: 999px;
    background: #e4e4e7;
  }

  .progress-fill {
    height: 100%;
    border-radius: inherit;
    background: #2563eb;
    transition: width 160ms cubic-bezier(.2, .7, .2, 1);
  }

  .action-pill {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    min-width: 74px;
    height: 24px;
    padding: 0 9px;
    border-radius: 999px;
    background: #f4f4f5;
    color: #3f3f46;
    font-size: 12px;
    font-weight: 650;
    white-space: nowrap;
  }

  .action-pill.up-to-date {
    background: #ecfdf5;
    color: #047857;
  }

  .action-pill.syncing {
    background: #eff6ff;
    color: #1d4ed8;
  }

  .action-pill.reauth {
    background: #fff1f2;
    color: #be123c;
  }

  .action-pill.paused {
    background: #f4f4f5;
    color: #52525b;
  }

  .empty-state {
    padding: 28px;
    border: 1px dashed #d4d4d8;
    border-radius: 8px;
    background: #ffffff;
    text-align: center;
  }

  .empty-state p {
    margin: 0 0 4px;
    color: #18181b;
    font-weight: 650;
  }

  .empty-state span {
    color: #71717a;
    font-size: 12px;
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
