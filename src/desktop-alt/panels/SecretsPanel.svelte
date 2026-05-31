<script lang="ts">
  import { invoke } from '@tauri-apps/api/core';
  import { companyStore } from '../lib/company-store.svelte';
  import SecretEnvRow, { type SecretEnv, type SecretItem } from '../components/SecretEnvRow.svelte';

  interface Props {
    slug: string;
  }

  let { slug }: Props = $props();

  let secrets = $state<SecretEnv[]>([]);
  let loading = $state(false);
  let error = $state<string | null>(null);
  let reloadToken = $state(0);

  const totalCount = $derived(secrets.reduce((total, secretEnv) => total + secretEnv.count, 0));

  $effect(() => {
    reloadToken;
    secrets = [];
    error = null;

    if (!slug) {
      loading = false;
      return;
    }

    let cancelled = false;

    const warm = companyStore.secrets(slug);
    secrets = warm ? warm.map(normalizeSecretEnv) : [];
    loading = warm === null;

    void invoke<Partial<SecretEnv>[]>('get_company_secrets', { slug })
      .then((result) => {
        if (!cancelled) {
          secrets = Array.isArray(result) ? result.map(normalizeSecretEnv) : [];
          companyStore.setSecrets(slug, Array.isArray(result) ? result : []);
        }
      })
      .catch((err) => {
        console.error('get_company_secrets failed:', err);
        if (!cancelled) {
          error = String(err);
          secrets = [];
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

  function normalizeSecretEnv(entry: Partial<SecretEnv>): SecretEnv {
    const items = Array.isArray(entry.items) ? entry.items.map(normalizeSecretItem) : [];
    return {
      env: stringOrFallback(entry.env, 'unknown'),
      count: numberOrFallback(entry.count, items.length),
      items,
    };
  }

  function normalizeSecretItem(item: Partial<SecretItem>): SecretItem {
    return {
      key: stringOrFallback(item.key, 'UNTITLED_KEY'),
      upd: stringOrFallback(item.upd, '-'),
      rot: stringOrFallback(item.rot, '-'),
    };
  }

  function stringOrFallback(value: unknown, fallback: string): string {
    return typeof value === 'string' && value.trim() ? value : fallback;
  }

  function numberOrFallback(value: unknown, fallback: number): number {
    return typeof value === 'number' && Number.isFinite(value) ? value : fallback;
  }

  function retry() {
    reloadToken += 1;
  }
</script>

<section class="secrets-panel" aria-labelledby="secrets-panel-title">
  <header class="secrets-header">
    <div class="secrets-title">
      <h2 id="secrets-panel-title">Secrets</h2>
      <span>{loading ? 'Loading secrets' : `${totalCount} keys`}</span>
    </div>
  </header>

  <p class="doc-note">
    Read-only metadata. Values are never sent to the client — use /hq-secrets to fetch a value.
  </p>

  <div class="secrets-toolbar" aria-label="Secrets controls">
    <button
      class="toolbar-button"
      type="button"
      disabled
      title="Export not available — use /hq-secrets exec"
      aria-label="Export not available — use /hq-secrets exec"
    >
      Export .env
    </button>
    <button
      class="toolbar-button"
      type="button"
      disabled
      title="Create from CLI: hq secrets set"
      aria-label="Create from CLI: hq secrets set"
    >
      New key
    </button>
  </div>

  {#if error}
    <div class="secrets-error" role="alert">
      <div>
        <strong>Secrets unavailable</strong>
        <span>{error}</span>
      </div>
      <button type="button" onclick={retry}>Retry</button>
    </div>
  {/if}

  <section class="secrets-card" aria-labelledby="secrets-list-title" aria-busy={loading}>
    <header class="card-header">
      <h3 id="secrets-list-title">Environments</h3>
      <span>{loading ? 'Loading' : `${secrets.length} total`}</span>
    </header>

    {#if loading}
      <div class="secrets-skeleton" aria-label="Loading secrets">
        {#each Array(3) as _, index (index)}
          <span style={`width: ${88 - index * 10}%`}></span>
        {/each}
      </div>
    {:else if secrets.length > 0}
      <div class="secrets-list">
        {#each secrets as secretEnv, index (`${secretEnv.env}:${index}`)}
          <SecretEnvRow {secretEnv} />
        {/each}
      </div>
    {:else}
      <div class="empty-state">No secrets yet</div>
    {/if}
  </section>
</section>

<style>
  .secrets-panel {
    display: grid;
    gap: 12px;
    min-width: 0;
  }

  .secrets-header,
  .secrets-toolbar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 16px;
    min-width: 0;
  }

  .secrets-title {
    min-width: 0;
  }

  .secrets-title h2 {
    margin: 0;
    color: var(--fg);
    font-size: 16px;
    font-weight: 680;
    line-height: 22px;
  }

  .secrets-title span,
  .card-header span,
  .empty-state,
  .doc-note {
    color: var(--muted);
    font-size: 12px;
    line-height: 16px;
  }

  .secrets-title span {
    display: block;
    margin-top: 2px;
  }

  .doc-note {
    margin: -2px 0 0;
    overflow-wrap: anywhere;
  }

  .secrets-toolbar {
    justify-content: flex-start;
    gap: 8px;
  }

  .toolbar-button,
  .secrets-error button {
    height: 30px;
    min-width: 0;
    padding: 0 11px;
    border: 1px solid var(--border);
    border-radius: 5px;
    background: transparent;
    color: var(--fg);
    font: inherit;
    font-size: 12px;
    font-weight: 650;
    white-space: nowrap;
    cursor: default;
  }

  .toolbar-button:disabled {
    color: var(--muted-3);
    background: var(--row-hover);
  }

  .secrets-error {
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

  .secrets-error div {
    display: grid;
    gap: 3px;
    min-width: 0;
  }

  .secrets-error strong,
  .secrets-error span {
    min-width: 0;
    overflow-wrap: anywhere;
  }

  .secrets-error strong {
    font-size: 13px;
    line-height: 18px;
  }

  .secrets-error span {
    font-size: 12px;
    line-height: 16px;
  }

  .secrets-card {
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
    font-size: 13px;
    font-weight: 700;
    line-height: 18px;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .secrets-list {
    display: grid;
  }

  .secrets-skeleton {
    display: grid;
    gap: 10px;
    padding: 14px 13px;
  }

  .secrets-skeleton span {
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
    .secrets-toolbar {
      flex-wrap: wrap;
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .secrets-skeleton span {
      animation: none;
    }
  }
</style>
