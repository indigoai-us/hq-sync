<script lang="ts">
  // Same stylesheet App.svelte uses so this window gets the canonical Liquid
  // Glass palette (and an opaque near-black behind the translucent surface).
  import '../styles/popover.css';
  import { invoke } from '@tauri-apps/api/core';
  import { listen } from '@tauri-apps/api/event';
  import Conversation, { type ConversationMessage } from './messaging/Conversation.svelte';
  import { type ReactionEvent, dmScope } from '../lib/reactions';
  import { ReactionController } from '../lib/reactionController.svelte';

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
  interface ThreadMessage extends ConversationMessage {
    fromEmail: string;
  }

  interface ThreadResponse {
    messages: ThreadMessage[];
    nextCursor?: string | null;
  }

  let event = $state<DmEvent | null>(null);
  let messages = $state<ThreadMessage[]>([]);
  let loadingThread = $state(false);
  let threadError = $state<string | null>(null);

  let sending = $state(false);
  let sendError = $state<string | null>(null);

  // Reactions (US-025) for this DM conversation. Created when the DM event
  // arrives (its peer is the scope), kept in step with the visible messages.
  let reactionsCtl = $state<ReactionController | null>(null);

  $effect(() => {
    const peer = event?.fromPersonUid;
    if (!peer) {
      reactionsCtl?.dispose();
      reactionsCtl = null;
      return;
    }
    const controller = new ReactionController(dmScope(peer));
    reactionsCtl = controller;
    return () => controller.dispose();
  });

  $effect(() => {
    const controller = reactionsCtl;
    if (!controller) return;
    const ids = messages
      .filter((m) => !m.eventId.startsWith('local-'))
      .map((m) => m.eventId);
    void controller.setMessages(ids);
  });

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
    }
  }

  async function sendReply(text: string): Promise<void> {
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
    } catch (err) {
      sendError = typeof err === 'string' ? err : 'Failed to send reply';
      console.error('dm-detail: send_dm failed', err);
    } finally {
      sending = false;
    }
  }

  $effect(() => {
    let unlisten: (() => void) | undefined;

    const unlisteners: Array<() => void> = [];

    listen<DmEvent>('dm:detail-event', (e) => {
      event = e.payload;
      void loadThread(e.payload);
    }).then((fn) => {
      unlisten = fn;
      unlisteners.push(fn);
      // Ready-handshake: tell Rust the listener is mounted so it emits the
      // pending event + shows the window (mirrors ShareDetail).
      invoke('dm_detail_window_ready');
    });

    // Live reaction reconcile for this DM (US-025).
    listen<ReactionEvent>('message:reaction', (e) => {
      reactionsCtl?.applyEvent(e.payload);
    }).then((fn) => unlisteners.push(fn));

    return () => {
      unlisten?.();
      for (const fn of unlisteners) fn();
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
    <Conversation
      {messages}
      showAuthors={false}
      loading={loadingThread}
      error={threadError}
      {sending}
      {sendError}
      placeholder={`Reply to ${event.fromDisplayName}…`}
      onsend={sendReply}
      reactions={reactionsCtl?.map ?? {}}
      ontogglereaction={reactionsCtl ? reactionsCtl.toggle : undefined}
    />
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
    font-size: var(--text-lg);
    font-weight: 600;
    color: var(--popover-text-heading, #ffffff);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .detail-count {
    margin-left: auto;
    font-size: var(--text-base);
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
    font-size: var(--text-base);
    color: var(--popover-text-muted, #a0a0b0);
    margin: 0;
  }
</style>
