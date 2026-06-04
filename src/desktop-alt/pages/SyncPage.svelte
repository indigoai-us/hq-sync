<script lang="ts">
  import type { Workspace } from '../../lib/workspaces';
  import HeroStatus from '../components/HeroStatus.svelte';
  import SourcesList from '../components/SourcesList.svelte';
  import {
    formatBytes,
    timeAgo,
    type ActivityEntry,
    type DaemonStatus,
    type SyncCompanyRef,
    type SyncProgress,
    type SyncState,
    type SyncStatus,
    type WorkspaceSyncStats,
  } from '../lib/sync-model';

  interface Props {
    workspaces: Workspace[];
    syncState: SyncState;
    /** False during the first real-state fetch → show skeletons, not 0/empty. */
    ready?: boolean;
    progress: SyncProgress | null;
    companies: SyncCompanyRef[];
    status: SyncStatus | null;
    daemon: DaemonStatus | null;
    indexedFiles: number;
    observedVaultBytes: number;
    statsBySlug: Record<string, WorkspaceSyncStats>;
    cloudReachable: boolean;
    cloudError: string | null;
    manifestError: string | null;
    activity: ActivityEntry[];
    syncErrorMessage: string;
    onsync: () => void;
    onsettings: () => void;
    onaddsource: () => void;
    actionMessage?: string;
    actionError?: string;
  }

  let {
    workspaces,
    syncState,
    ready = true,
    progress,
    companies,
    status,
    daemon,
    indexedFiles,
    observedVaultBytes,
    statsBySlug,
    cloudReachable,
    activity,
    onsync,
    onsettings,
    onaddsource,
    actionMessage = '',
    actionError = '',
  }: Props = $props();

  const recentActivity = $derived(
    [...activity].sort((a, b) => b.at - a.at).slice(0, 6),
  );

  function activityVerb(entry: ActivityEntry): string {
    if (entry.direction === 'up') return 'Uploaded';
    if (entry.direction === 'deleted') return 'Deleted';
    return entry.isNew ? 'Added' : 'Downloaded';
  }
</script>

<section class="sync-page" aria-label="Sync">
  <HeroStatus
    {syncState}
    {progress}
    {companies}
    {workspaces}
    {status}
    {daemon}
    {indexedFiles}
    {observedVaultBytes}
    loading={!ready}
    {onsync}
    {onsettings}
    {onaddsource}
    {actionMessage}
    {actionError}
  />

  <div class="sync-grid">
    <SourcesList
      {workspaces}
      {syncState}
      {progress}
      {statsBySlug}
      {cloudReachable}
      loading={!ready}
    />

    <aside class="side-column">
      <section class="activity-panel" aria-labelledby="activity-title">
        <div class="panel-header">
          <h2 id="activity-title">Recent activity</h2>
          <span>{ready ? recentActivity.length : ''}</span>
        </div>

        {#if !ready && recentActivity.length === 0}
          <ol class="activity-list" aria-busy="true">
            {#each [0, 1, 2, 3] as row (row)}
              <li class="activity-skeleton-row">
                <span class="skeleton-dot"></span>
                <div class="activity-copy">
                  <span class="skeleton-line" style="width: 78%"></span>
                  <span class="skeleton-line" style="width: 52%"></span>
                </div>
              </li>
            {/each}
          </ol>
        {:else if recentActivity.length === 0}
          <div class="activity-empty">
            <strong>No sync events yet</strong>
            <span>Activity appears after files upload, download, or delete.</span>
          </div>
        {:else}
          <ol class="activity-list">
            {#each recentActivity as item (`${item.company}:${item.path}:${item.at}`)}
              <li>
                <span class="activity-dot {item.direction}"></span>
                <div class="activity-copy">
                  <strong>{activityVerb(item)} {item.path}</strong>
                  <span>
                    {item.author ? `${item.author} · ` : ''}{item.company} · {formatBytes(item.bytes)} · {timeAgo(item.at)}
                  </span>
                </div>
              </li>
            {/each}
          </ol>
        {/if}
      </section>
    </aside>
  </div>
</section>

<style>
  .sync-page {
    display: grid;
    gap: 16px;
  }

  .sync-grid {
    display: grid;
    grid-template-columns: minmax(0, 1fr) minmax(270px, 320px);
    align-items: start;
    gap: 16px;
  }

  .side-column {
    display: grid;
    gap: 12px;
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
    font-weight: 680;
    line-height: 22px;
  }

  .panel-header span {
    color: var(--muted);
    font-size: var(--text-base);
  }

  .activity-empty,
  .activity-list {
    border: 1px solid var(--border);
    border-radius: 8px;
    background: var(--row-active);
  }

  .activity-empty {
    display: grid;
    gap: 3px;
    min-height: 74px;
    align-content: center;
    padding: 14px;
  }

  .activity-empty strong {
    color: var(--fg);
    font-size: var(--text-base);
    font-weight: 650;
  }

  .activity-empty span {
    color: var(--muted);
    font-size: var(--text-base);
    line-height: 17px;
  }

  .activity-list {
    display: grid;
    gap: 0;
    margin: 0;
    padding: 6px 0;
    list-style: none;
  }

  .activity-list li {
    display: grid;
    grid-template-columns: 12px minmax(0, 1fr);
    gap: 8px;
    padding: 8px 12px;
    transition: transform 140ms cubic-bezier(.2, .7, .2, 1);
  }

  .activity-list li:hover {
    background: var(--row-hover);
    transform: translateX(2px);
  }

  .activity-dot {
    width: 8px;
    height: 8px;
    margin-top: 5px;
    border-radius: 999px;
    background: var(--blue);
    box-shadow: 0 0 0 3px rgba(96, 165, 250, 0.18);
  }

  .activity-dot.up {
    background: var(--emerald);
    box-shadow: 0 0 0 3px rgba(52, 211, 153, 0.16);
  }

  .activity-dot.deleted {
    background: var(--red);
    box-shadow: 0 0 0 3px rgba(248, 113, 113, 0.16);
  }

  .activity-copy {
    min-width: 0;
  }

  .activity-copy strong,
  .activity-copy span {
    display: block;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .activity-copy strong {
    color: var(--fg);
    font-size: var(--text-base);
    font-weight: 650;
    line-height: 17px;
  }

  .activity-copy span {
    color: var(--muted);
    font-size: var(--text-base);
    line-height: 16px;
  }

  /* ---- loading skeletons ------------------------------------------------ */
  .activity-skeleton-row {
    grid-template-columns: 12px minmax(0, 1fr);
  }

  .activity-skeleton-row:hover {
    background: transparent;
    transform: none;
  }

  .skeleton-dot {
    width: 8px;
    height: 8px;
    margin-top: 5px;
    border-radius: 999px;
    background: var(--row-active);
    animation: sync-skeleton-pulse 1.2s ease-in-out infinite;
  }

  .skeleton-line {
    display: block;
    height: 10px;
    margin-bottom: 5px;
    border-radius: 999px;
    background: var(--row-active);
    animation: sync-skeleton-pulse 1.2s ease-in-out infinite;
  }

  @keyframes sync-skeleton-pulse {
    0%,
    100% {
      opacity: 0.5;
    }
    50% {
      opacity: 1;
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .activity-list li {
      transition: none;
    }

    .activity-list li:hover {
      transform: none;
    }

    .skeleton-dot,
    .skeleton-line {
      animation: none;
    }
  }

  @media (max-width: 980px) {
    .sync-grid {
      grid-template-columns: minmax(0, 1fr);
    }
  }
</style>
