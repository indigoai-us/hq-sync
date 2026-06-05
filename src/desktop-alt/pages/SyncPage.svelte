<script lang="ts">
  import type { Workspace } from '../../lib/workspaces';
  import SourcesList from '../components/SourcesList.svelte';
  import {
    formatBytes,
    friendlySyncError,
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

  // The sync verdict + Sync Now / Settings actions now live in the global title
  // bar and sidebar, so this page is just the workspace ledger + activity feed.
  // Remaining hero-only props (status, daemon, indexedFiles, onsettings, …) are
  // still accepted by the parent but intentionally not consumed here.
  let {
    workspaces,
    syncState,
    ready = true,
    progress,
    statsBySlug,
    cloudReachable,
    activity,
    syncErrorMessage = '',
  }: Props = $props();

  const recentActivity = $derived(
    [...activity].sort((a, b) => b.at - a.at).slice(0, 6),
  );

  // Plain-language headline + optional technical detail for the error banner.
  const friendlyError = $derived(
    syncErrorMessage ? friendlySyncError(syncErrorMessage) : null,
  );

  function activityVerb(entry: ActivityEntry): string {
    if (entry.direction === 'up') return 'Uploaded';
    if (entry.direction === 'deleted') return 'Deleted';
    return entry.isNew ? 'Added' : 'Downloaded';
  }

  // Show the trailing two path segments in mono — enough to recognize the file
  // without overflowing the narrow feed column.
  function shortPath(path: string): string {
    const parts = path.split('/').filter(Boolean);
    return parts.length <= 2 ? path : `…/${parts.slice(-2).join('/')}`;
  }
</script>

<section class="sync-page" aria-label="Sync">
  {#if friendlyError}
    <div class="sync-error">
      <p class="sync-error-summary" role="alert">{friendlyError.summary}</p>
      {#if friendlyError.detail}
        <details class="sync-error-details">
          <summary>Technical details</summary>
          <p class="sync-error-detail-text">{friendlyError.detail}</p>
        </details>
      {/if}
    </div>
  {/if}

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
                  <span class="activity-line">
                    <span class="activity-verb">{activityVerb(item)}</span>
                    <span class="activity-path">{shortPath(item.path)}</span>
                  </span>
                  <span class="activity-meta">
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
    font-family: var(--font-display);
    font-size: var(--text-lg);
    font-weight: 600;
    line-height: 20px;
    letter-spacing: -0.01em;
  }

  .panel-header span {
    color: var(--muted-3);
    font-family: var(--font-mono);
    font-size: var(--text-xs);
  }

  .sync-error {
    margin: 0;
    padding: 9px 12px;
    border: 1px solid var(--border);
    border-radius: 8px;
    background: var(--surface-raise);
  }

  .sync-error-summary {
    margin: 0;
    color: var(--amber);
    font-size: var(--text-sm);
    line-height: 17px;
  }

  .sync-error-details {
    margin-top: 6px;
  }

  .sync-error-details summary {
    color: var(--muted);
    font-size: var(--text-xs);
    cursor: pointer;
    user-select: none;
  }

  .sync-error-details summary:hover {
    color: var(--muted-2);
  }

  .sync-error-detail-text {
    margin: 6px 0 0;
    color: var(--muted-2);
    font-family: var(--font-mono);
    font-size: var(--text-xs);
    line-height: 16px;
    overflow-wrap: anywhere;
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
    font-weight: 600;
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
    padding: 7px 12px;
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

  .activity-line,
  .activity-meta {
    display: flex;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .activity-line {
    gap: 5px;
    align-items: baseline;
    line-height: 17px;
  }

  .activity-verb {
    flex: 0 0 auto;
    color: var(--fg);
    font-size: var(--text-sm);
    font-weight: 500;
  }

  .activity-path {
    min-width: 0;
    overflow: hidden;
    color: var(--fg-data);
    font-family: var(--font-mono);
    font-size: var(--text-xs);
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .activity-meta {
    display: block;
    color: var(--muted);
    font-size: var(--text-xs);
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
