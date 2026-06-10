<script lang="ts">
  // Right-side thread panel (US-022). Opens within MessagesShell — an overlay on
  // narrow widths, a third column on the wide desktop-alt layout — NOT a new
  // window, so the master/detail state stays coherent in one surface.
  //
  // Layout:
  //
  //   ┌─────────────────────────────┐
  //   │ ‹ Back / Close   Thread     │  header
  //   ├─────────────────────────────┤
  //   │ pinned root message bubble  │  (always shown at top)
  //   ├─────────────────────────────┤
  //   │ reply list  (<Conversation/>) │
  //   │ + composer (posts rootEventId) │
  //   └─────────────────────────────┘
  //
  // The replies + composer reuse the shared <Conversation/> primitive. The panel
  // owns the fetch (fetch_thread), the send (send_thread_reply with rootEventId
  // set), and the live thread:new-reply append. It also registers the open thread
  // with the backend (set_active_thread) so the SINGLE DM poll path re-fetches it
  // on a "thread" wake and emits thread:new-reply.
  import { invoke } from '@tauri-apps/api/core';
  import { listen } from '@tauri-apps/api/event';
  import Conversation, { type ConversationMessage } from './Conversation.svelte';

  // A thread message (root or reply) as returned by fetch_thread / carried on a
  // thread:new-reply event. Mirrors the Rust `ThreadReply` (camelCase).
  interface ThreadReplyRow extends ConversationMessage {
    fromEmail?: string;
  }

  interface ThreadView {
    root: ThreadReplyRow;
    replies: ThreadReplyRow[];
    replyCount: number;
  }

  interface Props {
    // The root message being threaded. Identifies which thread to load + reply to.
    rootEventId: string;
    // "dm" | "channel" — selects the fetch query + the reply endpoint.
    scope: 'dm' | 'channel';
    // For a channel thread: the channel the root lives in.
    channelId?: string | null;
    // For a DM thread: the peer the root conversation is with (the reply recipient).
    withPersonUid?: string | null;
    // A title shown in the header (peer name or #channel). Cosmetic.
    title?: string;
    // Whether to render author names above incoming bubbles (channels: true).
    showAuthors?: boolean;
    // Close/back — returns to the main conversation.
    onclose: () => void;
    // Bubbled up so the parent can bump the root bubble's live reply-count in the
    // main conversation as replies land here.
    onreplycount?: (rootEventId: string, replyCount: number) => void;
  }

  let {
    rootEventId,
    scope,
    channelId = null,
    withPersonUid = null,
    title = 'Thread',
    showAuthors = false,
    onclose,
    onreplycount,
  }: Props = $props();

  let root = $state<ThreadReplyRow | null>(null);
  let replies = $state<ThreadReplyRow[]>([]);
  let replyCount = $state(0);
  let loading = $state(false);
  let loadError = $state<string | null>(null);

  let sending = $state(false);
  let sendError = $state<string | null>(null);

  // Dedupe set so an optimistic append + the live thread:new-reply (or a reload)
  // don't double-render the same reply.
  let seenIds = $state(new Set<string>());

  function appendReply(r: ThreadReplyRow): void {
    if (seenIds.has(r.eventId)) return;
    seenIds.add(r.eventId);
    replies = [...replies, r];
  }

  async function load(): Promise<void> {
    loading = true;
    loadError = null;
    try {
      const view = await invoke<ThreadView>('fetch_thread', {
        scope,
        rootEventId,
        channelId: scope === 'channel' ? channelId : null,
        withPersonUid: scope === 'dm' ? withPersonUid : null,
      });
      root = view.root ?? null;
      // Server returns replies newest-first; render chronologically.
      const ordered = [...(view.replies ?? [])].reverse();
      seenIds = new Set(ordered.map((r) => r.eventId));
      replies = ordered;
      replyCount = view.replyCount ?? ordered.length;
      onreplycount?.(rootEventId, replyCount);
      // Register the open thread (+ already-seen reply ids) so the SINGLE poll
      // path emits thread:new-reply only for genuinely new replies.
      void invoke('set_active_thread', {
        rootEventId,
        scope,
        channelId: scope === 'channel' ? channelId : null,
        withPersonUid: scope === 'dm' ? withPersonUid : null,
        seenReplyIds: [...seenIds],
      });
    } catch (err) {
      loadError = typeof err === 'string' ? err : 'Could not load this thread';
      console.error('thread-panel: fetch_thread failed', err);
    } finally {
      loading = false;
    }
  }

  async function sendReply(text: string): Promise<void> {
    if (!text || sending) return;
    sending = true;
    sendError = null;
    try {
      await invoke('send_thread_reply', {
        scope,
        rootEventId,
        body: text,
        channelId: scope === 'channel' ? channelId : null,
        toPersonUid: scope === 'dm' ? withPersonUid : null,
      });
      // Optimistic append — the durable copy lands server-side and reconciles on
      // the next thread:new-reply / reload.
      const optimistic: ThreadReplyRow = {
        eventId: `local-${rootEventId}-${replies.length}-${text.length}`,
        fromPersonUid: 'me',
        fromEmail: '',
        fromDisplayName: 'You',
        body: text,
        details: null,
        prompt: null,
        createdAt: new Date().toISOString(),
        direction: 'out',
      };
      appendReply(optimistic);
      replyCount += 1;
      onreplycount?.(rootEventId, replyCount);
    } catch (err) {
      sendError = typeof err === 'string' ? err : 'Failed to send reply';
      console.error('thread-panel: send_thread_reply failed', err);
    } finally {
      sending = false;
    }
  }

  $effect(() => {
    const unlisteners: Array<() => void> = [];

    // A new reply landed in THIS thread (emitted by the SINGLE DM poll path on a
    // "thread" wake). Append it and bump the live count; ignore replies for other
    // roots.
    listen<{ rootEventId: string; reply: ThreadReplyRow; replyCount?: number }>(
      'thread:new-reply',
      (e) => {
        if (e.payload.rootEventId !== rootEventId) return;
        appendReply(e.payload.reply);
        if (typeof e.payload.replyCount === 'number') {
          replyCount = e.payload.replyCount;
        } else {
          replyCount = replies.length;
        }
        onreplycount?.(rootEventId, replyCount);
      },
    ).then((fn) => unlisteners.push(fn));

    void load();

    return () => {
      for (const fn of unlisteners) fn();
      // Clear the active thread so the poll path stops re-fetching it.
      void invoke('set_active_thread', { rootEventId: null });
    };
  });
</script>

<aside class="thread-panel" aria-label="Thread">
  <header class="thread-header" data-tauri-drag-region>
    <button class="thread-close" type="button" onclick={onclose} aria-label="Close thread">
      ‹ Back
    </button>
    <h2 class="thread-title">{title}</h2>
  </header>

  <div class="thread-root">
    {#if root}
      {#if showAuthors}
        <span class="thread-root-author">{root.fromDisplayName}</span>
      {/if}
      <div class="thread-root-bubble">
        <p class="thread-root-body">{root.body}</p>
        {#if root.details}
          <div class="thread-root-details">{root.details}</div>
        {/if}
      </div>
      <span class="thread-root-label">
        {replyCount} {replyCount === 1 ? 'reply' : 'replies'}
      </span>
    {:else if loading}
      <p class="thread-root-status">Loading thread…</p>
    {:else if loadError}
      <p class="thread-root-status thread-root-error" role="alert">{loadError}</p>
    {/if}
  </div>

  <div class="thread-body">
    <Conversation
      messages={replies}
      {showAuthors}
      loading={loading && replies.length === 0}
      error={loadError && replies.length === 0 ? loadError : null}
      {sending}
      {sendError}
      placeholder="Reply in thread…"
      onsend={sendReply}
    />
  </div>
</aside>

<style>
  .thread-panel {
    display: flex;
    flex-direction: column;
    min-height: 0;
    min-width: 0;
    height: 100%;
    background: var(--popover-bg, #14141a);
    border-left: 1px solid var(--popover-divider, rgba(255, 255, 255, 0.08));
  }

  .thread-header {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    padding: 0.875rem 1rem 0.75rem;
    border-bottom: 1px solid var(--popover-divider, rgba(255, 255, 255, 0.06));
    flex-shrink: 0;
  }

  .thread-close {
    border: none;
    background: rgba(255, 255, 255, 0.06);
    color: var(--popover-text, #d8d8e0);
    font-family: inherit;
    font-size: 0.75rem;
    font-weight: 600;
    padding: 0.25rem 0.625rem;
    border-radius: 7px;
    cursor: pointer;
    transition: background-color 0.12s ease;
  }

  .thread-close:hover {
    background: rgba(255, 255, 255, 0.12);
  }

  .thread-title {
    margin: 0;
    font-size: 0.875rem;
    font-weight: 600;
    color: var(--popover-text-heading, #ffffff);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  /* Pinned root message at the top of the panel. */
  .thread-root {
    flex-shrink: 0;
    display: flex;
    flex-direction: column;
    gap: 0.25rem;
    padding: 0.875rem 1rem;
    border-bottom: 1px solid var(--popover-divider, rgba(255, 255, 255, 0.06));
    background: rgba(120, 170, 255, 0.06);
  }

  .thread-root-author {
    font-size: 0.6875rem;
    font-weight: 600;
    color: var(--popover-text-muted, #a0a0b0);
  }

  .thread-root-bubble {
    padding: 0.5rem 0.75rem;
    border-radius: 12px;
    border-bottom-left-radius: 4px;
    background: rgba(255, 255, 255, 0.07);
  }

  .thread-root-body {
    margin: 0;
    font-size: 0.875rem;
    line-height: 1.45;
    color: var(--popover-text, #e8e8ee);
    white-space: pre-wrap;
    word-break: break-word;
  }

  .thread-root-details {
    margin-top: 0.5rem;
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

  .thread-root-label {
    font-size: 0.625rem;
    font-weight: 600;
    letter-spacing: 0.02em;
    color: var(--popover-text-muted, #8a8a98);
    text-transform: uppercase;
  }

  .thread-root-status {
    margin: 0;
    font-size: 0.75rem;
    color: var(--popover-text-muted, #a0a0b0);
  }

  .thread-root-error {
    color: #ff9b9b;
  }

  /* The reply list + composer (shared <Conversation/>) flexes to fill. */
  .thread-body {
    flex: 1;
    display: flex;
    flex-direction: column;
    min-height: 0;
    min-width: 0;
  }
</style>
