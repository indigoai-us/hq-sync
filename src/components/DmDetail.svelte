<script lang="ts">
  // Same stylesheet App.svelte uses so this window gets the canonical Liquid
  // Glass palette (and an opaque near-black behind the translucent surface).
  import '../styles/popover.css';
  import { invoke } from '@tauri-apps/api/core';
  import { listen } from '@tauri-apps/api/event';

  interface DmEvent {
    eventId: string;
    fromPersonUid: string;
    fromEmail: string;
    fromDisplayName: string;
    body: string;
    details?: string | null;
    prompt?: string | null;
    createdAt: string;
  }

  let event = $state<DmEvent | null>(null);
  let copied = $state(false);

  let replyText = $state('');
  let sending = $state(false);
  let sent = $state(false);
  let sendError = $state<string | null>(null);

  async function sendReply(): Promise<void> {
    const text = replyText.trim();
    if (!text || sending || !event) return;
    sending = true;
    sendError = null;
    try {
      await invoke('send_dm', { toPersonUid: event.fromPersonUid, body: text });
      replyText = '';
      sent = true;
      setTimeout(() => {
        sent = false;
      }, 1800);
    } catch (err) {
      sendError = typeof err === 'string' ? err : 'Failed to send reply';
      console.error('dm-detail: send_dm failed', err);
    } finally {
      sending = false;
    }
  }

  function onReplyKeydown(e: KeyboardEvent): void {
    if ((e.metaKey || e.ctrlKey) && e.key === 'Enter') {
      e.preventDefault();
      void sendReply();
    }
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

  async function copyPrompt(): Promise<void> {
    const p = event?.prompt?.trim();
    if (!p) return;
    try {
      await navigator.clipboard.writeText(p);
      copied = true;
      setTimeout(() => {
        copied = false;
      }, 1800);
    } catch (err) {
      console.error('dm-detail: clipboard write failed', err);
    }
  }

  $effect(() => {
    let unlisten: (() => void) | undefined;

    listen<DmEvent>('dm:detail-event', (e) => {
      event = e.payload;
    }).then((fn) => {
      unlisten = fn;
      // Ready-handshake: tell Rust the listener is mounted so it emits the
      // pending event + shows the window (mirrors ShareDetail).
      invoke('dm_detail_window_ready');
    });

    return () => {
      unlisten?.();
    };
  });
</script>

<div class="detail-window">
  <header class="detail-header">
    <h1>Direct Message</h1>
    {#if event}
      <span class="detail-count">{formatDate(event.createdAt)}</span>
    {/if}
  </header>

  {#if !event}
    <div class="detail-empty">
      <p>Waiting for message…</p>
    </div>
  {:else}
    <div class="dm-body-wrap">
      <div class="dm-from">
        <span class="dm-from-name">{event.fromDisplayName}</span>
        {#if event.fromEmail}
          <span class="dm-from-email">{event.fromEmail}</span>
        {/if}
      </div>

      <p class="dm-message">{event.body}</p>

      {#if event.details}
        <div class="dm-details">{event.details}</div>
      {/if}

      {#if event.prompt}
        <div class="dm-actions">
          <button class="btn btn-copy" onclick={copyPrompt} aria-label="Copy agent prompt to clipboard">
            {copied ? 'Copied!' : 'Copy prompt'}
          </button>
          <span class="dm-actions-hint">Paste into your agent session</span>
        </div>
      {/if}
    </div>

    <div class="dm-reply">
      <textarea
        class="dm-reply-input"
        bind:value={replyText}
        onkeydown={onReplyKeydown}
        placeholder="Reply to {event.fromDisplayName}…"
        rows="3"
        disabled={sending}
        aria-label="Reply message"
      ></textarea>
      <div class="dm-reply-footer">
        {#if sendError}
          <span class="dm-reply-error" role="alert">{sendError}</span>
        {:else if sent}
          <span class="dm-reply-sent">Sent ✓</span>
        {:else}
          <span class="dm-reply-hint">⌘↵ to send</span>
        {/if}
        <button
          class="btn btn-send"
          onclick={sendReply}
          disabled={sending || replyText.trim().length === 0}
        >
          {sending ? 'Sending…' : 'Send'}
        </button>
      </div>
    </div>
  {/if}
</div>

<style>
  :global([data-window="dm-detail"] html),
  :global([data-window="dm-detail"] body) {
    margin: 0;
    padding: 0;
    background: #0d0d10;
    color-scheme: dark;
  }

  .detail-window {
    display: flex;
    flex-direction: column;
    width: 100vw;
    height: 100vh;
    box-sizing: border-box;
    background: var(--popover-bg, #14141a);
    backdrop-filter: var(--popover-blur, blur(28px) saturate(1.45));
    -webkit-backdrop-filter: var(--popover-blur, blur(28px) saturate(1.45));
    color: var(--popover-text, rgba(255, 255, 255, 0.86));
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
    margin-left: auto;
    font-size: 0.6875rem;
    color: var(--popover-text-muted, #a0a0b0);
    white-space: nowrap;
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

  .dm-body-wrap {
    flex: 1;
    overflow-y: auto;
    padding: 1rem 1.25rem;
    display: flex;
    flex-direction: column;
    gap: 0.875rem;
    scrollbar-width: thin;
    scrollbar-color: rgba(255, 255, 255, 0.15) transparent;
  }

  .dm-body-wrap::-webkit-scrollbar {
    width: 6px;
  }

  .dm-body-wrap::-webkit-scrollbar-thumb {
    background: rgba(255, 255, 255, 0.12);
    border-radius: 3px;
  }

  .dm-from {
    display: flex;
    align-items: baseline;
    gap: 0.5rem;
    flex-wrap: wrap;
  }

  .dm-from-name {
    font-size: 0.9375rem;
    font-weight: 600;
    color: var(--popover-text-heading, #ffffff);
  }

  .dm-from-email {
    font-size: 0.75rem;
    color: var(--popover-text-muted, #a0a0b0);
  }

  .dm-message {
    margin: 0;
    font-size: 0.875rem;
    line-height: 1.45;
    color: var(--popover-text, #e0e0e0);
    white-space: pre-wrap;
    word-break: break-word;
  }

  .dm-details {
    font-size: 0.8125rem;
    line-height: 1.5;
    color: var(--popover-text, #e0e0e0);
    background: rgba(255, 255, 255, 0.03);
    border-left: 2px solid rgba(255, 255, 255, 0.15);
    padding: 0.625rem 0.75rem;
    border-radius: 0 6px 6px 0;
    white-space: pre-wrap;
    word-break: break-word;
  }

  .dm-actions {
    display: flex;
    align-items: center;
    gap: 0.75rem;
    flex-wrap: wrap;
    margin-top: 0.25rem;
  }

  .dm-actions-hint {
    font-size: 0.6875rem;
    color: var(--popover-text-muted, #a0a0b0);
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

  .dm-reply {
    flex-shrink: 0;
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
    padding: 0.875rem 1.25rem 1rem;
    border-top: 1px solid var(--popover-divider, rgba(255, 255, 255, 0.06));
  }

  .dm-reply-input {
    width: 100%;
    box-sizing: border-box;
    resize: none;
    padding: 0.5rem 0.625rem;
    border-radius: 8px;
    border: 1px solid rgba(255, 255, 255, 0.1);
    background: rgba(255, 255, 255, 0.04);
    color: var(--popover-text, #e0e0e0);
    font-family: inherit;
    font-size: 0.8125rem;
    line-height: 1.4;
  }

  .dm-reply-input:focus {
    outline: none;
    border-color: rgba(255, 255, 255, 0.28);
    background: rgba(255, 255, 255, 0.06);
  }

  .dm-reply-input:disabled {
    opacity: 0.6;
  }

  .dm-reply-footer {
    display: flex;
    align-items: center;
    gap: 0.75rem;
  }

  .dm-reply-hint {
    font-size: 0.6875rem;
    color: var(--popover-text-muted, #a0a0b0);
  }

  .dm-reply-sent {
    font-size: 0.75rem;
    color: #7ee2a8;
  }

  .dm-reply-error {
    font-size: 0.75rem;
    color: #ff9b9b;
    word-break: break-word;
  }

  .btn-send {
    margin-left: auto;
    background: rgba(120, 170, 255, 0.22);
    color: #dce8ff;
  }

  .btn-send:hover:not(:disabled) {
    background: rgba(120, 170, 255, 0.32);
  }

  .btn-send:disabled {
    opacity: 0.45;
    cursor: default;
  }
</style>
