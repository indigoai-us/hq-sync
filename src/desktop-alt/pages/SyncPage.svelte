<script lang="ts">
  import type { Workspace } from '../../lib/workspaces';
  import AttentionPanel from '../components/AttentionPanel.svelte';
  import HeroStatus from '../components/HeroStatus.svelte';
  import SourcesList from '../components/SourcesList.svelte';
  import {
    buildAttentionItems,
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
    progress,
    companies,
    status,
    daemon,
    indexedFiles,
    observedVaultBytes,
    statsBySlug,
    cloudReachable,
    cloudError,
    manifestError,
    activity,
    syncErrorMessage,
    onsync,
    onsettings,
    onaddsource,
    actionMessage = '',
    actionError = '',
  }: Props = $props();

  const attentionItems = $derived(
    buildAttentionItems({
      workspaces,
      syncState,
      syncErrorMessage,
      cloudReachable,
      cloudError,
      manifestError,
    }),
  );

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
    />

    <aside class="side-column">
      <AttentionPanel items={attentionItems} onaction={onsettings} />

      <section class="activity-panel" aria-labelledby="activity-title">
        <div class="panel-header">
          <h2 id="activity-title">Recent activity</h2>
          <span>{recentActivity.length}</span>
        </div>

        {#if recentActivity.length === 0}
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
    gap: 22px;
  }

  .sync-grid {
    display: grid;
    grid-template-columns: minmax(0, 1fr) minmax(270px, 320px);
    align-items: start;
    gap: 22px;
  }

  .side-column {
    display: grid;
    gap: 18px;
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

  .panel-header span {
    color: #71717a;
    font-size: 12px;
  }

  .activity-empty,
  .activity-list {
    border: 1px solid #e4e4e7;
    border-radius: 8px;
    background: #ffffff;
  }

  .activity-empty {
    display: grid;
    gap: 3px;
    min-height: 74px;
    align-content: center;
    padding: 14px;
  }

  .activity-empty strong {
    color: #18181b;
    font-size: 13px;
    font-weight: 650;
  }

  .activity-empty span {
    color: #71717a;
    font-size: 12px;
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
    background: #fafafa;
    transform: translateX(2px);
  }

  .activity-dot {
    width: 8px;
    height: 8px;
    margin-top: 5px;
    border-radius: 999px;
    background: #2563eb;
    box-shadow: 0 0 0 3px rgb(37 99 235 / 0.12);
  }

  .activity-dot.up {
    background: #16a34a;
    box-shadow: 0 0 0 3px rgb(22 163 74 / 0.12);
  }

  .activity-dot.deleted {
    background: #dc2626;
    box-shadow: 0 0 0 3px rgb(220 38 38 / 0.12);
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
    color: #18181b;
    font-size: 12px;
    font-weight: 650;
    line-height: 17px;
  }

  .activity-copy span {
    color: #71717a;
    font-size: 11px;
    line-height: 16px;
  }

  @media (prefers-reduced-motion: reduce) {
    .activity-list li {
      transition: none;
    }

    .activity-list li:hover {
      transform: none;
    }
  }

  @media (max-width: 980px) {
    .sync-grid {
      grid-template-columns: minmax(0, 1fr);
    }
  }
</style>
