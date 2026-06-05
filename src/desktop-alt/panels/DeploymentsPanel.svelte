<script lang="ts">
  import { invoke } from '@tauri-apps/api/core';
  import { companyStore } from '../lib/company-store.svelte';
  import DeploymentRow, {
    type DeploymentEntry,
    type DeploymentState,
  } from '../components/DeploymentRow.svelte';

  interface Props {
    slug: string;
  }

  let { slug }: Props = $props();

  let deployments = $state<DeploymentEntry[]>([]);
  let loading = $state(false);
  let error = $state<string | null>(null);
  let reloadToken = $state(0);

  const activeCount = $derived(countByState('active'));
  const deployingCount = $derived(countByState('deploying'));
  const pausedCount = $derived(countByState('paused'));

  $effect(() => {
    reloadToken;
    deployments = [];
    error = null;

    if (!slug) {
      loading = false;
      return;
    }

    let cancelled = false;

    const warm = companyStore.deployments(slug);
    deployments = warm ? warm.map(normalizeDeployment) : [];
    loading = warm === null;

    void invoke<Partial<DeploymentEntry>[]>('get_company_deployments', { slug })
      .then((result) => {
        if (!cancelled) {
          deployments = Array.isArray(result) ? result.map(normalizeDeployment) : [];
          companyStore.setDeployments(slug, Array.isArray(result) ? result : []);
        }
      })
      .catch((err) => {
        console.error('get_company_deployments failed:', err);
        if (!cancelled) {
          error = String(err);
          deployments = [];
        }
      })
      .finally(() => {
        if (!cancelled) {
          loading = false;
        }
      });

    return () => {
      cancelled = true;
    };
  });

  function normalizeDeployment(entry: Partial<DeploymentEntry>): DeploymentEntry {
    return {
      sub: stringOrFallback(entry.sub, 'Untitled'),
      url: stringOrFallback(entry.url, ''),
      state: normalizeState(entry.state),
      lastDeploy: stringOrFallback(entry.lastDeploy, '-'),
      size: stringOrFallback(entry.size, '-'),
      ver: stringOrFallback(entry.ver, '-'),
      pwd: entry.pwd === true,
    };
  }

  function stringOrFallback(value: unknown, fallback: string): string {
    return typeof value === 'string' && value.trim() ? value : fallback;
  }

  function normalizeState(value: unknown): DeploymentState {
    return value === 'deploying' || value === 'paused' ? value : 'active';
  }

  function countByState(state: DeploymentState): number {
    return deployments.filter((deployment) => deployment.state === state).length;
  }

  function retry() {
    reloadToken += 1;
  }
</script>

<section class="deployments-panel" aria-labelledby="deployments-panel-title">
  <header class="deployments-toolbar">
    <div class="deployments-title">
      <h2 id="deployments-panel-title">Deployments</h2>
      <span>{loading ? 'Loading deployments' : `${deployments.length} subdomains`}</span>
    </div>

    <div class="deployments-controls" aria-label="Deployment controls">
      <div class="counts" aria-label="Deployment state counts">
        <span><strong>{activeCount}</strong> active</span>
        <span><strong>{deployingCount}</strong> deploying</span>
        <span><strong>{pausedCount}</strong> paused</span>
      </div>
      <button class="toolbar-button" type="button" disabled title="Find deployments">
        Find
      </button>
      <button
        class="toolbar-button"
        type="button"
        disabled
        title="Deploy from terminal: /deploy"
        aria-label="Deploy from terminal: /deploy"
      >
        Deploy
      </button>
    </div>
  </header>

  {#if error}
    <div class="deployments-error" role="alert">
      <div>
        <strong>Deployments unavailable</strong>
        <span>{error}</span>
      </div>
      <button type="button" onclick={retry}>Retry</button>
    </div>
  {/if}

  <section class="deployments-card" aria-labelledby="deployments-list-title" aria-busy={loading}>
    <header class="card-header">
      <h3 id="deployments-list-title">Subdomains</h3>
      <span>{loading ? 'Loading' : `${deployments.length} total`}</span>
    </header>

    <div class="deployment-table">
      <div class="table-head" aria-hidden="true">
        <span></span>
        <span>Subdomain</span>
        <span>Last deploy</span>
        <span>Size</span>
        <span>Version</span>
        <span></span>
      </div>

      {#if loading}
        <div class="deployment-skeleton" aria-label="Loading deployments">
          {#each Array(4) as _, index (index)}
            <span style={`width: ${92 - index * 9}%`}></span>
          {/each}
        </div>
      {:else if deployments.length > 0}
        <div class="deployment-list">
          {#each deployments as deployment, index (`${deployment.url}:${index}`)}
            <DeploymentRow {deployment} />
          {/each}
        </div>
      {:else}
        <div class="empty-state">No provisioned subdomains for this company.</div>
      {/if}
    </div>
  </section>
</section>

<style>
  .deployments-panel {
    display: grid;
    gap: 14px;
    min-width: 0;
  }

  .deployments-toolbar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 16px;
    min-width: 0;
  }

  .deployments-title {
    min-width: 0;
  }

  .deployments-title h2 {
    margin: 0;
    color: var(--fg);
    font-size: var(--text-base);
    font-weight: 600;
    line-height: 22px;
  }

  .deployments-title span,
  .card-header span,
  .empty-state,
  .counts {
    color: var(--muted);
    font-size: var(--text-base);
    line-height: 16px;
  }

  .deployments-title span {
    display: block;
    margin-top: 2px;
  }

  .deployments-controls {
    display: flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
  }

  .counts {
    display: flex;
    align-items: center;
    gap: 9px;
    min-width: 0;
    padding-right: 4px;
    white-space: nowrap;
  }

  .counts strong {
    color: var(--fg);
    font-weight: 600;
  }

  .toolbar-button,
  .deployments-error button {
    height: 30px;
    min-width: 0;
    padding: 0 11px;
    border: 1px solid var(--border);
    border-radius: 5px;
    background: transparent;
    color: var(--fg);
    font: inherit;
    font-size: var(--text-base);
    font-weight: 600;
    white-space: nowrap;
    cursor: default;
  }

  .toolbar-button:disabled {
    color: var(--muted-3);
    background: var(--row-hover);
  }

  .deployments-error {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 14px;
    min-width: 0;
    padding: 12px;
    border: 1px solid rgba(245, 158, 11, 0.3);
    border-radius: 8px;
    background: rgba(245, 158, 11, 0.1);
    color: var(--amber);
  }

  .deployments-error div {
    display: grid;
    gap: 3px;
    min-width: 0;
  }

  .deployments-error strong,
  .deployments-error span {
    min-width: 0;
    overflow-wrap: anywhere;
  }

  .deployments-error strong {
    font-size: var(--text-base);
    line-height: 18px;
  }

  .deployments-error span {
    font-size: var(--text-base);
    line-height: 16px;
  }

  .deployments-card {
    min-width: 0;
    border: 1px solid var(--border);
    border-radius: 8px;
    background: var(--bg);
    box-shadow: 0 1px 2px rgba(0, 0, 0, 0.4);
  }

  .card-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
    min-width: 0;
    padding: 11px 13px;
    border-bottom: 1px solid var(--border);
  }

  .card-header h3 {
    min-width: 0;
    margin: 0;
    overflow: hidden;
    color: var(--muted-2);
    font-size: var(--text-base);
    font-weight: 600;
    line-height: 18px;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .deployment-table {
    min-width: 0;
  }

  .table-head {
    display: grid;
    grid-template-columns: 14px 1.4fr 1fr auto auto auto;
    align-items: center;
    gap: 12px;
    min-width: 0;
    padding: 8px 13px;
    border-bottom: 1px solid var(--border);
    color: var(--muted);
    font-size: var(--text-micro);
    font-weight: 600;
    line-height: 15px;
    text-transform: uppercase;
  }

  .deployment-list {
    display: grid;
  }

  .deployment-skeleton {
    display: grid;
    gap: 10px;
    padding: 14px 13px;
  }

  .deployment-skeleton span {
    height: 18px;
    border-radius: 5px;
    background: linear-gradient(
      90deg,
      rgba(255, 255, 255, 0.05),
      rgba(255, 255, 255, 0.1),
      rgba(255, 255, 255, 0.05)
    );
    background-size: 200% 100%;
    animation: skeleton 1.2s ease-in-out infinite;
  }

  .empty-state {
    padding: 26px 13px;
    text-align: center;
  }

  @keyframes skeleton {
    from {
      background-position: 0 0;
    }

    to {
      background-position: -200% 0;
    }
  }

  @media (max-width: 760px) {
    .deployments-toolbar {
      align-items: stretch;
      flex-direction: column;
    }

    .deployments-controls {
      flex-wrap: wrap;
    }

    .counts {
      width: 100%;
    }

    .table-head {
      grid-template-columns: 14px minmax(0, 1fr) auto;
    }

    .table-head span:nth-child(3),
    .table-head span:nth-child(4),
    .table-head span:nth-child(5) {
      display: none;
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .deployment-skeleton span {
      animation: none;
    }
  }
</style>
