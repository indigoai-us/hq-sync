<script lang="ts">
  import { invoke } from '@tauri-apps/api/core';
  import './tokens.css';

  interface Props {
    currentVersion: string;
    targetVersion: string;
    available?: boolean;
  }

  let { currentVersion, targetVersion, available = true }: Props = $props();

  let updateState = $state<'available' | 'in-progress' | 'failed'>('available');
  let logTail = $state<string[]>([]);

  async function installUpdate() {
    if (!available || updateState === 'in-progress') return;
    updateState = 'in-progress';
    logTail = [`Preparing update from ${currentVersion} to ${targetVersion}`];
    try {
      const result = await invoke<string>('install_hq_core_update');
      logTail = [...logTail, result || 'Update finished'];
      updateState = 'available';
    } catch (err) {
      logTail = [...logTail, String(err)].slice(-4);
      updateState = 'failed';
    }
  }
</script>

<article class="core-update-card" data-state={updateState} aria-label="Core update">
  <header>
    <span>{updateState === 'in-progress' ? 'IN PROGRESS' : updateState === 'failed' ? 'FAILED' : 'AVAILABLE'}</span>
    <strong>HQ core update</strong>
  </header>

  <p>Installed v{currentVersion} · target v{targetVersion}</p>

  {#if logTail.length > 0}
    <pre aria-label="Update log tail">{logTail.join('\n')}</pre>
  {/if}

  <footer>
    <button type="button" disabled={!available || updateState === 'in-progress'} onclick={installUpdate}>
      {updateState === 'in-progress' ? 'Updating…' : updateState === 'failed' ? 'Try again' : 'Install update'}
    </button>
  </footer>
</article>

<style>
  .core-update-card {
    display: grid;
    gap: 10px;
    min-width: 0;
    padding: 14px;
    border: 1px solid var(--v4-hairline);
    border-radius: 8px;
    background: var(--v4-surface);
    color: var(--v4-text-1);
  }

  header,
  footer {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
    min-width: 0;
  }

  header span {
    color: var(--v4-text-3);
    font-size: 11px;
    font-weight: 500;
  }

  header strong {
    overflow: hidden;
    font-size: 14px;
    font-weight: 500;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  p,
  pre {
    margin: 0;
    color: var(--v4-text-2);
    font-size: 12px;
    line-height: 1.4;
  }

  pre {
    max-height: 96px;
    overflow: auto;
    padding: 10px;
    border-radius: 6px;
    background: var(--v4-inset);
    font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
    white-space: pre-wrap;
  }

  button {
    height: 30px;
    padding: 0 11px;
    border: 1px solid var(--v4-hairline);
    border-radius: 6px;
    background: var(--v4-control-bg);
    color: var(--v4-text-1);
    font: inherit;
    font-size: 12px;
  }

  button:disabled {
    opacity: 0.5;
  }

  .core-update-card[data-state='failed'] {
    border-color: color-mix(in srgb, var(--v4-error) 35%, var(--v4-hairline));
  }
</style>
