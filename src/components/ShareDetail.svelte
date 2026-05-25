<script lang="ts">
  import { invoke } from '@tauri-apps/api/core';
  import { listen } from '@tauri-apps/api/event';

  interface ShareEvent {
    eventId: string;
    issuerEmail: string;
    issuerDisplayName: string;
    paths: string[];
    note: string | null;
    permission: string;
    createdAt: string;
  }

  let events = $state<ShareEvent[]>([]);
  let copyFeedback = $state<string | null>(null);

  function basename(p: string): string {
    return p.split('/').filter(Boolean).pop() ?? p;
  }

  function formatDate(iso: string): string {
    try {
      return new Intl.DateTimeFormat(undefined, {
        dateStyle: 'medium',
        timeStyle: 'short',
      }).format(new Date(iso));
    } catch {
      return iso;
    }
  }

  function buildPrompt(evt: ShareEvent): string {
    const pathList = evt.paths.join(', ');
    const note = evt.note?.trim() || '(no note)';
    return `${evt.issuerDisplayName} shared these files with me: ${pathList}\n\nTheir note: ${note}.`;
  }

  async function copyPrompt(evt: ShareEvent): Promise<void> {
    try {
      await navigator.clipboard.writeText(buildPrompt(evt));
      copyFeedback = evt.eventId;
      setTimeout(() => {
        copyFeedback = null;
      }, 1800);
    } catch (err) {
      console.error('Clipboard write failed:', err);
    }
  }

  function openConsole(evt: ShareEvent): void {
    // URL-encode issuer email for query param.
    const url = `https://console.getindigo.ai/files/shared-with-me?from=${encodeURIComponent(evt.issuerEmail)}`;
    window.open(url, '_blank');
  }

  $effect(() => {
    let unlisten: (() => void) | undefined;

    listen<ShareEvent[]>('share:events-list', (event) => {
      events = event.payload;
    }).then((fn) => {
      unlisten = fn;
      // Signal to Rust that our listener is registered — Rust emits the
      // pending events + shows the window. Mirrors the new-files-detail
      // ready-handshake (races with WebviewWindowBuilder otherwise).
      invoke('share_detail_window_ready');
    });

    return () => {
      unlisten?.();
    };
  });
</script>

<div class="detail-window">
  <header class="detail-header">
    <h1>Shared with Me</h1>
    <span class="detail-count">{events.length} share{events.length === 1 ? '' : 's'}</span>
  </header>

  {#if events.length === 0}
    <div class="detail-empty">
      <p>Waiting for share data…</p>
    </div>
  {:else}
    <div class="events-list">
      {#each events as evt (evt.eventId)}
        <div class="event-card">
          <div class="event-header">
            <span class="event-issuer">{evt.issuerDisplayName}</span>
            <span class="event-email">{evt.issuerEmail}</span>
            <span class="event-date">{formatDate(evt.createdAt)}</span>
          </div>

          <ul class="paths-list">
            {#each evt.paths as p}
              <li class="path-item" title={p}>
                <span class="path-basename">{basename(p)}</span>
                <span class="path-full">{p}</span>
              </li>
            {/each}
          </ul>

          {#if evt.note}
            <p class="event-note">{evt.note}</p>
          {/if}

          <div class="event-actions">
            <button
              class="btn btn-copy"
              onclick={() => copyPrompt(evt)}
              aria-label="Copy prompt to clipboard"
            >
              {copyFeedback === evt.eventId ? 'Copied!' : 'Copy prompt'}
            </button>
            <button
              class="btn btn-console"
              onclick={() => openConsole(evt)}
              aria-label="Open in HQ Console"
            >
              Open in HQ Console ↗
            </button>
          </div>
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
    background: var(--popover-bg, rgba(18, 18, 20, 0.92));
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

  .events-list {
    flex: 1;
    overflow-y: auto;
    padding: 0.75rem 1rem;
    display: flex;
    flex-direction: column;
    gap: 0.75rem;
    scrollbar-width: thin;
    scrollbar-color: rgba(255, 255, 255, 0.15) transparent;
  }

  .events-list::-webkit-scrollbar {
    width: 6px;
  }

  .events-list::-webkit-scrollbar-thumb {
    background: rgba(255, 255, 255, 0.12);
    border-radius: 3px;
  }

  .event-card {
    background: rgba(255, 255, 255, 0.04);
    border: 1px solid rgba(255, 255, 255, 0.08);
    border-radius: 10px;
    padding: 0.875rem 1rem;
    display: flex;
    flex-direction: column;
    gap: 0.625rem;
  }

  .event-header {
    display: flex;
    align-items: baseline;
    gap: 0.5rem;
    flex-wrap: wrap;
  }

  .event-issuer {
    font-size: 0.875rem;
    font-weight: 600;
    color: var(--popover-text-heading, #ffffff);
  }

  .event-email {
    font-size: 0.75rem;
    color: var(--popover-text-muted, #a0a0b0);
  }

  .event-date {
    margin-left: auto;
    font-size: 0.6875rem;
    color: var(--popover-text-muted, #a0a0b0);
    white-space: nowrap;
  }

  .paths-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 0.25rem;
  }

  .path-item {
    display: flex;
    flex-direction: column;
    gap: 0.0625rem;
  }

  .path-basename {
    font-size: 0.8125rem;
    font-weight: 500;
    color: var(--popover-text, #e0e0e0);
  }

  .path-full {
    font-size: 0.6875rem;
    color: var(--popover-text-muted, #a0a0b0);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .event-note {
    margin: 0;
    font-size: 0.8125rem;
    color: var(--popover-text, #e0e0e0);
    background: rgba(255, 255, 255, 0.03);
    border-left: 2px solid rgba(255, 255, 255, 0.15);
    padding: 0.375rem 0.625rem;
    border-radius: 0 4px 4px 0;
    white-space: pre-wrap;
    word-break: break-word;
  }

  .event-actions {
    display: flex;
    gap: 0.5rem;
    flex-wrap: wrap;
  }

  .btn {
    display: inline-flex;
    align-items: center;
    padding: 0.375rem 0.75rem;
    border-radius: 6px;
    font-size: 0.75rem;
    font-weight: 500;
    cursor: pointer;
    border: none;
    transition: background-color 0.12s ease, color 0.12s ease;
    font-family: inherit;
  }

  .btn-copy {
    background: rgba(255, 255, 255, 0.1);
    color: var(--popover-text, #e0e0e0);
  }

  .btn-copy:hover {
    background: rgba(255, 255, 255, 0.16);
  }

  .btn-console {
    background: transparent;
    color: var(--popover-text-muted, #a0a0b0);
    border: 1px solid rgba(255, 255, 255, 0.1);
  }

  .btn-console:hover {
    background: rgba(255, 255, 255, 0.05);
    color: var(--popover-text, #e0e0e0);
  }
</style>
