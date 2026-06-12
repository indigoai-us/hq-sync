<script lang="ts">
  import { invoke } from '@tauri-apps/api/core';
  import '../v4/tokens.css';

  export interface ConflictResolutionItem {
    path: string;
    localOwner: string;
    localWhen: string;
    remoteOwner: string;
    remoteWhen: string;
    localPreview: string;
    remotePreview: string;
    changedRegion?: string;
  }

  interface Props {
    conflicts?: ConflictResolutionItem[];
    oncomplete?: () => void;
  }

  let { conflicts = [], oncomplete }: Props = $props();

  let selectedIndex = $state(0);
  let resolvingPath = $state<string | null>(null);
  let error = $state<string | null>(null);
  let resolved = $state<Set<string>>(new Set());

  const pendingConflicts = $derived(conflicts.filter((conflict) => !resolved.has(conflict.path)));
  const currentConflict = $derived(
    pendingConflicts[Math.min(selectedIndex, Math.max(0, pendingConflicts.length - 1))] ?? null,
  );
  const progressLabel = $derived(
    conflicts.length === 0
      ? '0 of 0'
      : `${Math.min(resolved.size + 1, conflicts.length)} of ${conflicts.length}`,
  );

  async function resolveCurrent(strategy: 'keep-local' | 'keep-remote') {
    if (!currentConflict || resolvingPath) return;
    resolvingPath = currentConflict.path;
    error = null;
    try {
      await invoke('resolve_conflict', { path: currentConflict.path, strategy });
      resolved = new Set([...resolved, currentConflict.path]);
      selectedIndex = Math.min(selectedIndex, Math.max(0, pendingConflicts.length - 2));
      if (resolved.size + 1 >= conflicts.length) oncomplete?.();
    } catch (err) {
      error = String(err);
    } finally {
      resolvingPath = null;
    }
  }

  function openCurrentInEditor() {
    if (!currentConflict) return;
    void invoke('open_in_editor', { path: currentConflict.path }).catch((err) => {
      error = String(err);
    });
  }

  function decideLater() {
    if (pendingConflicts.length <= 1) return;
    selectedIndex = (selectedIndex + 1) % pendingConflicts.length;
  }
</script>

<section class="conflict-page" aria-labelledby="conflict-title">
  <header class="page-header">
    <div>
      <p>{progressLabel}</p>
      <h1 id="conflict-title">Resolve conflict</h1>
    </div>
    <p class="retention-note">Both versions are retained until you choose one.</p>
  </header>

  {#if error}
    <p class="error" role="alert">{error}</p>
  {/if}

  {#if currentConflict}
    <div class="path-row">
      <span>{currentConflict.path}</span>
    </div>

    <div class="compare-grid" aria-label="Conflict comparison">
      <article class="version-pane selected" aria-label="Your version">
        <header>
          <span>YOURS</span>
          <strong>{currentConflict.localOwner}</strong>
          <time>{currentConflict.localWhen}</time>
        </header>
        <pre class="changed-region">{currentConflict.localPreview}</pre>
      </article>

      <article class="version-pane" aria-label="Cloud version">
        <header>
          <span>CLOUD</span>
          <strong>{currentConflict.remoteOwner}</strong>
          <time>{currentConflict.remoteWhen}</time>
        </header>
        <pre class="changed-region">{currentConflict.remotePreview}</pre>
      </article>
    </div>

    <footer class="actions">
      <button
        type="button"
        class="primary"
        disabled={resolvingPath === currentConflict.path}
        onclick={() => resolveCurrent('keep-local')}
      >
        Keep yours
      </button>
      <button
        type="button"
        disabled={resolvingPath === currentConflict.path}
        onclick={() => resolveCurrent('keep-remote')}
      >
        Use this one
      </button>
      <button type="button" onclick={openCurrentInEditor}>Open in editor</button>
      <button type="button" class="text" onclick={decideLater}>Decide later</button>
    </footer>
  {:else}
    <div class="empty-state">All conflicts are resolved.</div>
  {/if}
</section>

<style>
  .conflict-page {
    display: flex;
    flex-direction: column;
    gap: 16px;
    min-width: 0;
    color: var(--v4-text-1);
  }

  .page-header,
  .actions,
  .path-row {
    display: flex;
    align-items: center;
    gap: 12px;
    min-width: 0;
  }

  .page-header {
    justify-content: space-between;
  }

  h1,
  p {
    margin: 0;
  }

  h1 {
    font-size: 18px;
    font-weight: 500;
    line-height: 1.25;
  }

  .page-header p,
  .retention-note,
  .path-row,
  .empty-state,
  time {
    color: var(--v4-text-3);
    font-size: 12px;
    line-height: 1.35;
  }

  .path-row {
    justify-content: space-between;
    padding: 10px 12px;
    border: 1px solid var(--v4-hairline);
    border-radius: 8px;
    background: var(--v4-inset);
  }

  .path-row span {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .compare-grid {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: 12px;
    min-width: 0;
  }

  .version-pane {
    display: flex;
    min-width: 0;
    flex-direction: column;
    overflow: hidden;
    border: 1px solid var(--v4-hairline);
    border-radius: 8px;
    background: var(--v4-surface);
  }

  .version-pane.selected {
    border-color: var(--v4-control-border);
    box-shadow: 0 0 0 1px var(--v4-control-border);
  }

  .version-pane header {
    display: grid;
    grid-template-columns: auto minmax(0, 1fr) auto;
    gap: 8px;
    padding: 10px 12px;
    border-bottom: 1px solid var(--v4-rowline);
  }

  .version-pane header span,
  .version-pane header strong {
    overflow: hidden;
    font-size: 12px;
    line-height: 1.35;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .version-pane header span {
    color: var(--v4-text-3);
    font-weight: 500;
  }

  .version-pane header strong {
    color: var(--v4-text-1);
    font-weight: 500;
  }

  .changed-region {
    min-height: 240px;
    margin: 0;
    padding: 14px;
    overflow: auto;
    background: var(--v4-inset);
    color: var(--v4-text-2);
    font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
    font-size: 12px;
    line-height: 1.45;
    white-space: pre-wrap;
  }

  .actions {
    flex-wrap: wrap;
  }

  button {
    height: 32px;
    padding: 0 12px;
    border: 1px solid var(--v4-hairline);
    border-radius: 6px;
    background: transparent;
    color: var(--v4-text-2);
    font: inherit;
    font-size: 12px;
    font-weight: 500;
  }

  button.primary {
    border-color: transparent;
    background: var(--v4-control-bg);
    color: var(--v4-text-1);
  }

  button.text {
    border-color: transparent;
    color: var(--v4-text-3);
  }

  button:disabled {
    opacity: 0.5;
  }

  .error {
    color: var(--v4-error);
    font-size: 12px;
  }

  .empty-state {
    padding: 24px;
    border: 1px dashed var(--v4-hairline);
    border-radius: 8px;
    text-align: center;
  }
</style>
