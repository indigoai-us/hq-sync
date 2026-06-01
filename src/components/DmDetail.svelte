<script lang="ts">
  // Same stylesheet App.svelte uses so this window gets the canonical Liquid
  // Glass palette (and an opaque near-black behind the translucent surface).
  import '../styles/popover.css';
  import { invoke } from '@tauri-apps/api/core';
  import { listen } from '@tauri-apps/api/event';
  import { tick } from 'svelte';

  // The DM that opened the window (the reply target). Also the most recent
  // inbound message — it anchors who the conversation is with.
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

  // One rendered message in the thread. `direction` is relative to the signed-in
  // user: "out" = I sent it, "in" = the other person sent it.
  interface ThreadMessage {
    eventId: string;
    fromPersonUid: string;
    fromEmail: string;
    fromDisplayName: string;
    body: string;
    details?: string | null;
    prompt?: string | null;
    createdAt: string;
    direction: 'in' | 'out';
  }

  interface ThreadResponse {
    messages: ThreadMessage[];
    nextCursor?: string | null;
  }

  let event = $state<DmEvent | null>(null);
  let messages = $state<ThreadMessage[]>([]);
  let loadingThread = $state(false);
  let threadError = $state<string | null>(null);
  let copiedId = $state<string | null>(null);

  let replyText = $state('');
  let sending = $state(false);
  let sendError = $state<string | null>(null);

  let scrollEl = $state<HTMLDivElement | null>(null);

  async function scrollToBottom(): Promise<void> {
    await tick();
    if (scrollEl) scrollEl.scrollTop = scrollEl.scrollHeight;
  }

  /**
   * Merge the server thread (newest-first) into chronological order and ensure
   * the live DM that opened the window is present — the conversation mirror is
   * written best-effort server-side, so the just-arrived DM may not be in the
   * thread response yet. Dedupe by eventId.
   */
  function buildThread(serverMsgs: ThreadMessage[], live: DmEvent | null): ThreadMessage[] {
    const chrono = [...serverMsgs].reverse();
    if (live && !chrono.some((m) => m.eventId === live.eventId)) {
      chrono.push({
        eventId: live.eventId,
        fromPersonUid: live.fromPersonUid,
        fromEmail: live.fromEmail,
        fromDisplayName: live.fromDisplayName,
        body: live.body,
        details: live.details ?? null,
        prompt: live.prompt ?? null,
        createdAt: live.createdAt,
        direction: 'in',
      });
    }
    return chrono;
  }

  async function loadThread(forEvent: DmEvent): Promise<void> {
    loadingThread = true;
    threadError = null;
    try {
      const resp = await invoke<ThreadResponse>('fetch_dm_thread', {
        withPersonUid: forEvent.fromPersonUid,
      });
      messages = buildThread(resp.messages ?? [], forEvent);
    } catch (err) {
      // Non-fatal: still show the single live message + composer.
      threadError = typeof err === 'string' ? err : 'Could not load earlier messages';
      messages = buildThread([], forEvent);
      console.error('dm-detail: fetch_dm_thread failed', err);
    } finally {
      loadingThread = false;
      void scrollToBottom();
    }
  }

  async function sendReply(): Promise<void> {
    const text = replyText.trim();
    if (!text || sending || !event) return;
    sending = true;
    sendError = null;
    try {
      await invoke('send_dm', { toPersonUid: event.fromPersonUid, body: text });
      // Optimistically append the sent message so the thread updates instantly;
      // the durable copy lands in the mirror and shows on the next open.
      messages = [
        ...messages,
        {
          eventId: `local-${messages.length}-${text.length}`,
          fromPersonUid: 'me',
          fromEmail: '',
          fromDisplayName: 'You',
          body: text,
          details: null,
          prompt: null,
          createdAt: new Date().toISOString(),
          direction: 'out',
        },
      ];
      replyText = '';
      void scrollToBottom();
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

  function formatTime(iso: string): string {
    try {
      return new Intl.DateTimeFormat(undefined, {
        hour: 'numeric',
        minute: '2-digit',
      }).format(new Date(iso));
    } catch {
      return '';
    }
  }

  async function copyPrompt(id: string, prompt: string | null | undefined): Promise<void> {
    const p = prompt?.trim();
    if (!p) return;
    try {
      await navigator.clipboard.writeText(p);
      copiedId = id;
      setTimeout(() => {
        if (copiedId === id) copiedId = null;
      }, 1800);
    } catch (err) {
      console.error('dm-detail: clipboard write failed', err);
    }
  }

  $effect(() => {
    let unlisten: (() => void) | undefined;

    listen<DmEvent>('dm:detail-event', (e) => {
      event = e.payload;
      void loadThread(e.payload);
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
    <h1>{event ? event.fromDisplayName : 'Direct Message'}</h1>
    {#if event?.fromEmail}
      <span class="detail-count">{event.fromEmail}</span>
    {/if}
  </header>

  {#if !event}
    <div class="detail-empty">
      <p>Waiting for message…</p>
    </div>
  {:else}
    <div class="dm-thread" bind:this={scrollEl}>
      {#if loadingThread}
        <p class="dm-thread-status">Loading conversation…</p>
      {/if}
      {#if threadError}
        <p class="dm-thread-status dm-thread-error" role="alert">{threadError}</p>
      {/if}

      {#each messages as msg (msg.eventId)}
        <div class="dm-msg dm-msg-{msg.direction}">
          <div class="dm-bubble">
            <p class="dm-bubble-body">{msg.body}</p>
            {#if msg.details}
              <div class="dm-bubble-details">{msg.details}</div>
            {/if}
            {#if msg.prompt}
              <button
                class="btn btn-copy"
                onclick={() => copyPrompt(msg.eventId, msg.prompt)}
                aria-label="Copy agent prompt to clipboard"
              >
                {copiedId === msg.eventId ? 'Copied!' : 'Copy prompt'}
              </button>
            {/if}
          </div>
          <span class="dm-msg-time">{formatTime(msg.createdAt)}</span>
        </div>
      {/each}
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
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
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

  /* ── Thread (scrollable conversation) ─────────────────────────────────── */

  .dm-thread {
    flex: 1;
    overflow-y: auto;
    padding: 1rem 1.25rem;
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
    scrollbar-width: thin;
    scrollbar-color: rgba(255, 255, 255, 0.15) transparent;
  }

  .dm-thread::-webkit-scrollbar {
    width: 6px;
  }

  .dm-thread::-webkit-scrollbar-thumb {
    background: rgba(255, 255, 255, 0.12);
    border-radius: 3px;
  }

  .dm-thread-status {
    margin: 0 auto;
    font-size: 0.6875rem;
    color: var(--popover-text-muted, #a0a0b0);
  }

  .dm-thread-error {
    color: #ff9b9b;
  }

  .dm-msg {
    display: flex;
    flex-direction: column;
    max-width: 80%;
  }

  .dm-msg-in {
    align-self: flex-start;
    align-items: flex-start;
  }

  .dm-msg-out {
    align-self: flex-end;
    align-items: flex-end;
  }

  .dm-bubble {
    padding: 0.5rem 0.75rem;
    border-radius: 12px;
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
  }

  .dm-msg-in .dm-bubble {
    background: rgba(255, 255, 255, 0.07);
    border-bottom-left-radius: 4px;
  }

  .dm-msg-out .dm-bubble {
    background: rgba(120, 170, 255, 0.22);
    border-bottom-right-radius: 4px;
  }

  .dm-bubble-body {
    margin: 0;
    font-size: 0.875rem;
    line-height: 1.45;
    color: var(--popover-text, #e8e8ee);
    white-space: pre-wrap;
    word-break: break-word;
  }

  .dm-bubble-details {
    font-size: 0.8125rem;
    line-height: 1.5;
    color: var(--popover-text, #e0e0e0);
    background: rgba(0, 0, 0, 0.18);
    border-left: 2px solid rgba(255, 255, 255, 0.15);
    padding: 0.5rem 0.625rem;
    border-radius: 0 6px 6px 0;
    white-space: pre-wrap;
    word-break: break-word;
  }

  .dm-msg-time {
    font-size: 0.625rem;
    color: var(--popover-text-muted, #8a8a98);
    margin: 0.125rem 0.25rem 0;
  }

  .btn {
    display: inline-flex;
    align-items: center;
    align-self: flex-start;
    padding: 0.3125rem 0.625rem;
    border-radius: 6px;
    font-size: 0.6875rem;
    font-weight: 500;
    cursor: pointer;
    border: none;
    transition: background-color 0.12s ease, color 0.12s ease;
    font-family: inherit;
  }

  .btn-copy {
    background: rgba(255, 255, 255, 0.12);
    color: var(--popover-text, #e0e0e0);
  }

  .btn-copy:hover {
    background: rgba(255, 255, 255, 0.2);
  }

  /* ── Reply composer ───────────────────────────────────────────────────── */

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
