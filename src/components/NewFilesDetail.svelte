<script lang="ts">
  import { invoke } from '@tauri-apps/api/core';
  import { listen } from '@tauri-apps/api/event';

  interface NewFile {
    path: string;
    bytes: number;
    addedBy: string | null;
  }

  let files = $state<NewFile[]>([]);

  function formatBytes(n: number): string {
    if (n < 1024) return `${n} B`;
    if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
    if (n < 1024 * 1024 * 1024) return `${(n / (1024 * 1024)).toFixed(1)} MB`;
    return `${(n / (1024 * 1024 * 1024)).toFixed(2)} GB`;
  }

  $effect(() => {
    let unlisten: (() => void) | undefined;

    listen<NewFile[]>('new-files:list', (event) => {
      files = event.payload;
    }).then((fn) => {
      unlisten = fn;
      // Signal to Rust that the listener is registered — Rust will
      // emit the pending file list and show/focus the window. This
      // handshake avoids a race between webview load and data emit.
      invoke('detail_window_ready');
    });

    return () => {
      unlisten?.();
    };
  });
</script>

<div class="detail-window">
  <header class="detail-header">
    <h1>New Files</h1>
    <span class="detail-count">{files.length} file{files.length === 1 ? '' : 's'}</span>
  </header>

  {#if files.length === 0}
    <div class="detail-empty">
      <p>Waiting for file data...</p>
    </div>
  {:else}
    <div class="detail-list">
      <div class="detail-list-header">
        <span class="col-path">File</span>
        <span class="col-author">Added by</span>
        <span class="col-size">Size</span>
      </div>
      {#each files as file}
        <div class="detail-row">
          <span class="col-path detail-path" title={file.path}>
            {file.path}
          </span>
          <span class="col-author detail-author">
            {file.addedBy ?? 'Unknown'}
          </span>
          <span class="col-size detail-size">
            {formatBytes(file.bytes)}
          </span>
        </div>
      {/each}
    </div>
  {/if}
</div>

<style>
  .detail-window {
    display: flex;
    flex-direction: column;
    width: 100vw;
    height: 100vh;
    box-sizing: border-box;
    background: var(--popover-bg, rgba(18, 18, 20, 0.68));
    backdrop-filter: var(--popover-blur, blur(28px) saturate(1.45));
    -webkit-backdrop-filter: var(--popover-blur, blur(28px) saturate(1.45));
    color: var(--popover-text, #e0e0e0);
    font-family: system-ui, -apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif;
    overflow: hidden;
  }

  .detail-header {
    display: flex;
    align-items: baseline;
    gap: 0.5rem;
    padding: 1rem 1.25rem 0.75rem;
    border-bottom: 1px solid var(--popover-divider, rgba(255, 255, 255, 0.06));
    flex-shrink: 0;
  }

  .detail-header h1 {
    margin: 0;
    font-size: 1rem;
    font-weight: 600;
    color: var(--popover-text-heading, #ffffff);
  }

  .detail-count {
    font-size: 0.75rem;
    color: var(--popover-text-muted, #a0a0b0);
  }

  .detail-empty {
    flex: 1;
    display: flex;
    align-items: center;
    justify-content: center;
  }

  .detail-empty p {
    font-size: 0.8125rem;
    color: var(--popover-text-muted, #a0a0b0);
    margin: 0;
  }

  .detail-list {
    flex: 1;
    overflow-y: auto;
    padding: 0.25rem 0;
    scrollbar-width: thin;
    scrollbar-color: rgba(255, 255, 255, 0.15) transparent;
  }

  .detail-list::-webkit-scrollbar {
    width: 6px;
  }

  .detail-list::-webkit-scrollbar-track {
    background: transparent;
  }

  .detail-list::-webkit-scrollbar-thumb {
    background: rgba(255, 255, 255, 0.12);
    border-radius: 3px;
  }

  .detail-list:hover::-webkit-scrollbar-thumb {
    background: rgba(255, 255, 255, 0.22);
  }

  .detail-list-header {
    display: flex;
    align-items: center;
    padding: 0.375rem 1.25rem;
    font-size: 0.6875rem;
    font-weight: 600;
    color: var(--popover-text-muted, #a0a0b0);
    text-transform: uppercase;
    letter-spacing: 0.04em;
    border-bottom: 1px solid var(--popover-divider, rgba(255, 255, 255, 0.06));
  }

  .detail-row {
    display: flex;
    align-items: center;
    padding: 0.5rem 1.25rem;
    font-size: 0.8125rem;
    border-bottom: 1px solid var(--popover-divider, rgba(255, 255, 255, 0.06));
    transition: background-color 0.1s ease;
  }

  .detail-row:last-child {
    border-bottom: none;
  }

  .detail-row:hover {
    background: var(--popover-action-hover, rgba(255, 255, 255, 0.05));
  }

  .col-path {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .col-author {
    width: 140px;
    flex-shrink: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    text-align: left;
    padding-left: 0.75rem;
  }

  .col-size {
    width: 70px;
    flex-shrink: 0;
    text-align: right;
    padding-left: 0.75rem;
  }

  .detail-path {
    color: var(--popover-text, #e0e0e0);
  }

  .detail-author {
    color: var(--popover-text-muted, #a0a0b0);
    font-size: 0.75rem;
  }

  .detail-size {
    color: var(--popover-text-muted, #a0a0b0);
    font-size: 0.75rem;
    font-variant-numeric: tabular-nums;
  }
</style>
