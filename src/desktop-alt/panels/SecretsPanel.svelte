<script lang="ts">
  import { invoke } from '@tauri-apps/api/core';
  import { buildClaudeCodeUrl } from '../../lib/claude-code-link';
  import { companyStore } from '../lib/company-store.svelte';
  import SecretEnvRow, { type SecretEnv, type SecretItem } from '../components/SecretEnvRow.svelte';

  interface Props {
    slug: string;
    cloudBacked?: boolean;
  }

  let { slug, cloudBacked = true }: Props = $props();

  let secrets = $state<SecretEnv[]>([]);
  let loading = $state(false);
  let error = $state<string | null>(null);
  let reloadToken = $state(0);
  let actionBusy = $state<'export' | 'new' | null>(null);
  let actionMessage = $state<string | null>(null);

  const totalCount = $derived(secrets.reduce((total, secretEnv) => total + secretEnv.count, 0));

  $effect(() => {
    reloadToken;
    secrets = [];
    error = null;

    if (!slug || !cloudBacked) {
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

  async function openSecretsPrompt(mode: 'export' | 'new'): Promise<void> {
    if (actionBusy) return;
    actionBusy = mode;
    actionMessage = null;

    const prompt =
      mode === 'export'
        ? [
            `/hq-secrets ${slug}`,
            '',
            `Help me export an environment file safely for company ${slug}.`,
            'Use the HQ secrets workflow. Do not print secret values into chat; write the requested local artifact only after confirming the target environment and path.',
          ].join('\n')
        : [
            `/hq-secrets ${slug}`,
            '',
            `Help me create or update a secret for company ${slug}.`,
            'Ask for the key, environment, and value handling path, then use the HQ secrets workflow without echoing the value back.',
          ].join('\n');

    try {
      const config = await invoke<{ hqFolderPath?: string }>('get_config').catch(() => ({
        hqFolderPath: '',
      }));
      const url = buildClaudeCodeUrl({ folder: config.hqFolderPath ?? '', prompt });
      await invoke('open_claude_code_link', { url });
      actionMessage = mode === 'export' ? 'Opened export workflow.' : 'Opened new-key workflow.';
    } catch (err) {
      console.error('open_claude_code_link for secrets failed:', err);
      try {
        await navigator.clipboard.writeText(prompt);
        actionMessage = 'Prompt copied. Paste it into Claude Code to continue.';
      } catch {
        actionMessage = 'Could not open Claude Code.';
      }
    } finally {
      actionBusy = null;
    }
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
      onclick={() => void openSecretsPrompt('export')}
      disabled={actionBusy !== null || !cloudBacked}
      title="Export via HQ secrets workflow"
    >
      {actionBusy === 'export' ? 'Opening…' : 'Export .env'}
    </button>
    <button
      class="toolbar-button"
      type="button"
      onclick={() => void openSecretsPrompt('new')}
      disabled={actionBusy !== null || !cloudBacked}
      title="Create via HQ secrets workflow"
    >
      {actionBusy === 'new' ? 'Opening…' : 'New key'}
    </button>
  </div>

  {#if actionMessage}
    <p class="action-status" role="status">{actionMessage}</p>
  {/if}

  {#if !cloudBacked}
    <div class="secrets-error secrets-note" role="status">
      <div>
        <strong>Connect this company to manage secrets</strong>
        <span>Secret metadata is available after the local company is cloud-backed.</span>
      </div>
    </div>
  {/if}

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
    font-size: var(--text-base);
    font-weight: 600;
    line-height: 22px;
  }

  .secrets-title span,
  .card-header span,
  .empty-state,
  .doc-note {
    color: var(--muted);
    font-size: var(--text-base);
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
    font-size: var(--text-base);
    font-weight: 600;
    white-space: nowrap;
    cursor: pointer;
  }

  .toolbar-button:disabled {
    color: var(--muted-3);
    background: var(--row-hover);
    cursor: default;
  }

  .toolbar-button:hover:not(:disabled) {
    border-color: var(--border-strong);
    background: var(--row-hover);
  }

  .action-status {
    margin: -4px 0 0;
    color: var(--muted-2);
    font-size: var(--text-base);
    line-height: 16px;
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

  .secrets-note {
    border-color: var(--border);
    background: var(--bg-raised);
    color: var(--muted);
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
    font-size: var(--text-base);
    line-height: 18px;
  }

  .secrets-error span {
    font-size: var(--text-base);
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
    font-size: var(--text-base);
    font-weight: 600;
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
      var(--v4-control-faint),
      var(--v4-hairline),
      var(--v4-control-faint)
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
