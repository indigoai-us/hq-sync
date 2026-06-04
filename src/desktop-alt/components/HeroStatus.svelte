<script lang="ts">
  import type { DaemonStatus, SyncProgress, SyncState, SyncStatus } from '../lib/sync-model';
  import {
    currentSyncLabel,
    formatBytes,
    formatUptime,
    latestFullSync,
    timeAgo,
    type SyncCompanyRef,
  } from '../lib/sync-model';
  import type { Workspace } from '../../lib/workspaces';

  interface Props {
    syncState: SyncState;
    progress: SyncProgress | null;
    companies: SyncCompanyRef[];
    workspaces: Workspace[];
    status: SyncStatus | null;
    daemon: DaemonStatus | null;
    indexedFiles: number;
    observedVaultBytes: number;
    /** During the first real-state fetch, skeleton the metric values. */
    loading?: boolean;
    onsync: () => void;
    onsettings: () => void;
    onaddsource: () => void;
    actionMessage?: string;
    actionError?: string;
  }

  let {
    syncState,
    progress,
    companies,
    workspaces,
    status,
    daemon,
    indexedFiles,
    observedVaultBytes,
    loading = false,
    onsync,
    onsettings,
    onaddsource,
    actionMessage = '',
    actionError = '',
  }: Props = $props();

  const lastFullSyncLabel = $derived(timeAgo(latestFullSync(workspaces, status)));
  const syncNowLabel = $derived(currentSyncLabel(progress, workspaces, companies));
  const uptimeLabel = $derived(formatUptime(daemon));
  const vaultSizeLabel = $derived(observedVaultBytes > 0 ? `${formatBytes(observedVaultBytes)} observed` : 'No size data');
</script>

<section class="hero-status" aria-labelledby="sync-hero-title">
  <div class="hero-main">
    {#if loading}
      <span class="hero-kicker-skeleton" aria-hidden="true"></span>
    {:else}
      <p class="hero-kicker">Last full sync · {lastFullSyncLabel}</p>
    {/if}
    <h1 id="sync-hero-title">Sync</h1>
    {#if syncState === 'syncing'}
      <p class="hero-current">Syncing now {syncNowLabel}</p>
    {:else if syncState === 'auth-error'}
      <p class="hero-current attention">Sign in again to resume syncing.</p>
    {:else if syncState === 'conflict'}
      <p class="hero-current attention">Sync conflict needs review.</p>
    {:else if syncState === 'error'}
      <p class="hero-current attention">Sync needs attention.</p>
    {:else}
      <p class="hero-current">All sources are read from current workspace state.</p>
    {/if}
  </div>

  <div class="hero-actions" aria-label="Quick actions">
    <button class="action-chip primary" type="button" onclick={onsync} disabled={syncState === 'syncing'}>
      Sync all
    </button>
    <button class="action-chip" type="button" onclick={onsettings}>Settings</button>
    <button class="action-chip" type="button" onclick={onaddsource} title="Coming soon">Add source</button>
  </div>

  {#if actionError}
    <p class="hero-feedback error" role="status">{actionError}</p>
  {:else if actionMessage}
    <p class="hero-feedback" role="status">{actionMessage}</p>
  {/if}

  <div class="hero-metrics" aria-label="Sync metrics" aria-busy={loading}>
    {#each [{ label: 'Sources', value: workspaces.length.toLocaleString() }, { label: 'Indexed files', value: indexedFiles.toLocaleString() }, { label: 'Vault size', value: vaultSizeLabel }, { label: 'Uptime', value: uptimeLabel }] as m (m.label)}
      <div class="metric">
        <span class="metric-label">{m.label}</span>
        {#if loading}
          <span class="metric-skeleton" aria-hidden="true"></span>
        {:else}
          <strong>{m.value}</strong>
        {/if}
      </div>
    {/each}
  </div>
</section>

<style>
  .hero-status {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto;
    gap: 16px 24px;
    padding-bottom: 22px;
    border-bottom: 1px solid var(--border);
  }

  .hero-main {
    min-width: 0;
  }

  .hero-kicker,
  .hero-current,
  .hero-feedback {
    margin: 0;
    color: var(--muted);
    font-size: var(--text-base);
    line-height: 18px;
  }

  .hero-status h1 {
    margin: 2px 0 4px;
    color: var(--fg);
    font-size: var(--text-base);
    font-weight: 680;
    letter-spacing: 0;
    line-height: 34px;
  }

  .hero-current {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .hero-current.attention,
  .hero-feedback.error {
    color: var(--red);
  }

  .hero-actions {
    display: flex;
    align-items: flex-start;
    gap: 8px;
  }

  .action-chip {
    height: 30px;
    padding: 0 12px;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: transparent;
    color: var(--fg);
    font: inherit;
    font-weight: 600;
    cursor: default;
    transition:
      opacity 140ms cubic-bezier(.2, .7, .2, 1),
      transform 140ms cubic-bezier(.2, .7, .2, 1);
  }

  .action-chip:hover:not(:disabled) {
    background: var(--row-hover);
    color: var(--fg);
    transform: translateY(-1px);
  }

  .action-chip:focus-visible {
    outline: 2px solid var(--blue);
    outline-offset: 2px;
  }

  .action-chip.primary {
    border-color: var(--fg);
    background: var(--fg);
    color: var(--bg);
  }

  .action-chip:disabled {
    opacity: 0.56;
  }

  .hero-feedback {
    grid-column: 1 / -1;
    margin-top: -8px;
    animation: feedbackIn 160ms cubic-bezier(.2, .7, .2, 1);
  }

  .hero-metrics {
    grid-column: 1 / -1;
    display: grid;
    grid-template-columns: repeat(4, minmax(0, 1fr));
    gap: 10px;
  }

  .metric {
    min-width: 0;
    padding: 12px;
    border: 1px solid var(--border);
    border-radius: 8px;
    background: var(--bg);
    transition: transform 140ms cubic-bezier(.2, .7, .2, 1);
  }

  .metric:hover {
    border-color: var(--border-strong);
    box-shadow: 0 1px 2px rgba(0, 0, 0, 0.4);
    transform: translateY(-1px);
  }

  .metric-label {
    display: block;
    color: var(--muted);
    font-size: var(--text-base);
    font-weight: 650;
    line-height: 16px;
    text-transform: uppercase;
  }

  .metric strong {
    display: block;
    min-width: 0;
    overflow-wrap: anywhere;
    color: var(--fg);
    font-size: var(--text-base);
    font-weight: 680;
    line-height: 24px;
  }

  /* ---- loading skeletons ------------------------------------------------ */
  .metric-skeleton {
    display: block;
    width: 60%;
    height: 18px;
    margin-top: 3px;
    border-radius: 999px;
    background: var(--row-active);
    animation: hero-skeleton-pulse 1.2s ease-in-out infinite;
  }

  .hero-kicker-skeleton {
    display: block;
    width: 140px;
    height: 12px;
    border-radius: 999px;
    background: var(--row-active);
    animation: hero-skeleton-pulse 1.2s ease-in-out infinite;
  }

  @keyframes hero-skeleton-pulse {
    0%,
    100% {
      opacity: 0.5;
    }
    50% {
      opacity: 1;
    }
  }

  @keyframes feedbackIn {
    from {
      opacity: 0;
      transform: translateY(-2px);
    }
    to {
      opacity: 1;
      transform: translateY(0);
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .action-chip,
    .metric {
      transition: none;
    }

    .action-chip:hover:not(:disabled),
    .metric:hover {
      transform: none;
    }

    .hero-feedback,
    .metric-skeleton,
    .hero-kicker-skeleton {
      animation: none;
    }
  }

  @media (max-width: 860px) {
    .hero-status {
      grid-template-columns: minmax(0, 1fr);
    }

    .hero-actions {
      flex-wrap: wrap;
    }

    .hero-metrics {
      grid-template-columns: repeat(2, minmax(0, 1fr));
    }
  }
</style>
