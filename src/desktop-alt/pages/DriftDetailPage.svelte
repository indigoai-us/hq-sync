<script lang="ts">
  import { invoke } from '@tauri-apps/api/core';
  import '../v4/tokens.css';

  interface DriftEntry {
    path: string;
    size: number;
    gitShaLocal: string | null;
    gitShaUpstream: string | null;
  }

  interface DriftReport {
    count: number;
    modified: DriftEntry[];
    missing: DriftEntry[];
    added: DriftEntry[];
    hqVersion: string;
    targetRepo: string;
    targetRef: string;
  }

  interface CoreState {
    targetVersion: string;
    driftReport: DriftReport;
  }

  let coreState = $state<CoreState | null>(null);
  let loading = $state(true);
  let error = $state<string | null>(null);
  let restoring = $state<string | null>(null);
  let kept = $state<Set<string>>(new Set());

  const report = $derived(coreState?.driftReport ?? null);

  $effect(() => {
    void loadCoreState();
  });

  async function loadCoreState() {
    loading = true;
    error = null;
    try {
      coreState = await invoke<CoreState | null>('check_core_state');
    } catch (err) {
      error = String(err);
      coreState = null;
    } finally {
      loading = false;
    }
  }

  async function restore(entry: DriftEntry) {
    if (!report || restoring) return;
    restoring = entry.path;
    error = null;
    try {
      await invoke('restore_from_upstream', {
        path: entry.path,
        expectedUpstreamSha: entry.gitShaUpstream,
        targetRepo: report.targetRepo,
        targetRef: report.targetRef,
      });
      await loadCoreState();
    } catch (err) {
      error = String(err);
    } finally {
      restoring = null;
    }
  }

  function keep(entry: DriftEntry) {
    kept = new Set([...kept, entry.path]);
  }

  function formatBytes(size: number): string {
    if (size < 1024) return `${size} B`;
    if (size < 1024 * 1024) return `${(size / 1024).toFixed(1)} KB`;
    return `${(size / (1024 * 1024)).toFixed(1)} MB`;
  }
</script>

<section class="drift-page" aria-labelledby="drift-title" aria-busy={loading}>
  <header class="page-header">
    <div>
      <p>{report ? `Compared with ${report.targetRepo}@${report.targetRef}` : 'Checking core state'}</p>
      <h1 id="drift-title">Core drift</h1>
    </div>
    <button type="button" onclick={loadCoreState}>Recheck</button>
  </header>

  {#if error}
    <p class="error" role="alert">{error}</p>
  {/if}

  {#if loading}
    <div class="empty-state">Checking locked core files…</div>
  {:else if report}
    <div class="summary-row">
      <strong>{report.count}</strong>
      <span>user-edited files drifted from v{coreState?.targetVersion || report.hqVersion}</span>
    </div>

    <section class="drift-section" aria-label="Modified files">
      <h2>MODIFIED</h2>
      {#each report.modified as entry (entry.path)}
        <div class="drift-row">
          <span class="kind">MODIFIED</span>
          <span class="path" title={entry.path}>{entry.path}</span>
          <span>{formatBytes(entry.size)}</span>
          <button type="button" disabled={restoring === entry.path} onclick={() => restore(entry)}>
            Restore
          </button>
          <button type="button" class:selected={kept.has(entry.path)} onclick={() => keep(entry)}>
            Keep edit
          </button>
        </div>
      {/each}
    </section>

    <section class="drift-section" aria-label="Missing files">
      <h2>MISSING</h2>
      {#each report.missing as entry (entry.path)}
        <div class="drift-row">
          <span class="kind">MISSING</span>
          <span class="path" title={entry.path}>{entry.path}</span>
          <span>{formatBytes(entry.size)}</span>
          <button type="button" disabled={restoring === entry.path} onclick={() => restore(entry)}>
            Restore
          </button>
          <button type="button" class:selected={kept.has(entry.path)} onclick={() => keep(entry)}>
            Keep missing
          </button>
        </div>
      {/each}
    </section>

    <section class="drift-section" aria-label="Added files">
      <h2>ADDED</h2>
      {#each report.added as entry (entry.path)}
        <div class="drift-row">
          <span class="kind">ADDED</span>
          <span class="path" title={entry.path}>{entry.path}</span>
          <span>{formatBytes(entry.size)}</span>
          <button type="button" disabled title="Added files are local only">Restore</button>
          <button type="button" class:selected={kept.has(entry.path)} onclick={() => keep(entry)}>
            Keep file
          </button>
        </div>
      {/each}
    </section>
  {:else}
    <div class="empty-state">Core state is unavailable.</div>
  {/if}
</section>

<style>
  .drift-page {
    display: flex;
    flex-direction: column;
    gap: 14px;
    min-width: 0;
    color: var(--v4-text-1);
  }

  .page-header,
  .summary-row,
  .drift-row {
    display: flex;
    align-items: center;
    gap: 12px;
    min-width: 0;
  }

  .page-header {
    justify-content: space-between;
  }

  h1,
  h2,
  p {
    margin: 0;
  }

  h1 {
    font-size: 18px;
    font-weight: 500;
  }

  h2,
  .page-header p,
  .summary-row span,
  .empty-state {
    color: var(--v4-text-3);
    font-size: 12px;
    font-weight: 500;
  }

  .summary-row,
  .empty-state {
    padding: 12px;
    border: 1px solid var(--v4-hairline);
    border-radius: 8px;
    background: var(--v4-inset);
  }

  .summary-row strong {
    color: var(--v4-text-1);
    font-size: 18px;
    font-weight: 500;
  }

  .drift-section {
    display: grid;
    gap: 6px;
  }

  .drift-row {
    display: grid;
    grid-template-columns: 82px minmax(0, 1fr) 80px auto auto;
    padding: 10px 12px;
    border: 1px solid var(--v4-rowline);
    border-radius: 7px;
    background: var(--v4-surface);
  }

  .kind,
  .drift-row span {
    overflow: hidden;
    color: var(--v4-text-2);
    font-size: 12px;
    line-height: 1.3;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .kind {
    color: var(--v4-text-3);
    font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
  }

  .path {
    color: var(--v4-text-1);
    font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
  }

  button {
    height: 28px;
    padding: 0 10px;
    border: 1px solid var(--v4-hairline);
    border-radius: 6px;
    background: transparent;
    color: var(--v4-text-2);
    font: inherit;
    font-size: 12px;
  }

  button.selected {
    background: var(--v4-control-bg);
    color: var(--v4-text-1);
  }

  button:disabled {
    opacity: 0.5;
  }

  .error {
    color: var(--v4-error);
    font-size: 12px;
  }
</style>
