<script lang="ts">
  // Shared conversation primitive: a scrollable thread of in/out bubbles plus a
  // reply composer. Extracted from DmDetail.svelte (US-008) so DMs, channels,
  // and threads can all render the same thread + composer surface. Pure
  // presentation — the parent owns the message list, the send call, and the
  // optimistic append; this component just renders `messages` and invokes the
  // `onsend` callback. Visuals (bubble + composer CSS) live here so they travel
  // with the component.
  import { tick } from 'svelte';
  import ReactionBar from './ReactionBar.svelte';
  import { type ReactionMap } from '../../lib/reactions';
  import { copyableText, type CopyKind } from '../../lib/conversation-copy';

  // One rendered message in the thread. `direction` is relative to the signed-in
  // user: "out" = I sent it, "in" = the other person sent it. Extra fields
  // beyond these are tolerated (channels/threads carry more) — only these are
  // read here.
  export interface ConversationMessage {
    eventId: string;
    fromPersonUid: string;
    fromDisplayName: string;
    body: string;
    details?: string | null;
    prompt?: string | null;
    createdAt: string;
    direction: 'in' | 'out';
    // Optimistic-send states (US-010). `pending` marks an outbound message that
    // was held behind a connection request — rendered with a "Pending" chip
    // until `dm:request-update` flips it (US-011). `pendingLabel` is the chip
    // text (e.g. "Pending — waiting for Ada to accept").
    pending?: boolean;
    pendingLabel?: string | null;
    // Threads (US-022). A root message carries its own eventId as `rootEventId`
    // and a `replyCount`; when `replyCount > 0` a tap-visible "{n} replies · last
    // {time}" affordance renders under the bubble and opens the thread via
    // `onopenthread`. `lastReplyAt` (ISO) drives the "last {time}" stamp.
    rootEventId?: string | null;
    replyCount?: number | null;
    lastReplyAt?: string | null;
  }

  interface Props {
    messages: ConversationMessage[];
    // When true, render the author's display name above incoming bubbles
    // (channels/threads). DMs pass false — there's only one peer.
    showAuthors?: boolean;
    loading?: boolean;
    error?: string | null;
    // Composer state, owned by the parent so "Sending…"/disabled/error stays in
    // lockstep with the actual send call.
    sending?: boolean;
    sendError?: string | null;
    placeholder?: string;
    // Called with the trimmed composer text when the user sends. The parent
    // performs the send + optimistic append; on success it should leave
    // `sendError` null, which clears the composer.
    onsend: (text: string) => void | Promise<void>;
    // Reserved for later stories (reactions). No-op by default.
    onreact?: (eventId: string) => void;
    // Threads (US-022). Called with a root message's `rootEventId` when the user
    // taps its reply-count affordance — the parent opens the ThreadPanel.
    onopenthread?: (rootEventId: string) => void;
    // When set, the root bubble whose `rootEventId` matches gets an "active
    // thread" highlight (the ThreadPanel for it is open).
    activeRootEventId?: string | null;
    // Reactions (US-025). The host owns the reaction map (messageId → sorted
    // aggregates) and the toggle. When `ontogglereaction` is set, a ReactionBar
    // renders under every bubble (pills + a tap-visible add-reaction trigger).
    // Hosts that don't support reactions (e.g. an invited-not-joined channel
    // preview) simply omit the callback and no bar renders.
    reactions?: ReactionMap;
    ontogglereaction?: (messageId: string, emoji: string) => void;
    // When true, the reply composer is hidden and a static note renders in its
    // place. Used for read-only history or preview panes that have no writable
    // recipient yet.
    readonly?: boolean;
  }

  // `onreact` is part of the public API for a later story (reactions) but unused
  // here, so it's intentionally left out of the destructure to avoid
  // unused-binding noise — it still type-checks as an accepted prop.
  let {
    messages,
    showAuthors = false,
    loading = false,
    error = null,
    sending = false,
    sendError = null,
    placeholder = 'Reply…',
    onsend,
    onopenthread,
    activeRootEventId = null,
    reactions = {},
    ontogglereaction,
    readonly = false,
  }: Props = $props();

  let replyText = $state('');
  // Tracks the last successful copy so the "Copied!" feedback stays scoped to
  // the exact affordance the user clicked — a bubble can offer both a
  // copy-message and a copy-prompt action.
  let copied = $state<{ id: string; kind: CopyKind } | null>(null);
  const isCopied = (id: string, kind: CopyKind) =>
    copied?.id === id && copied?.kind === kind;
  let scrollEl = $state<HTMLDivElement | null>(null);

  async function scrollToBottom(): Promise<void> {
    await tick();
    if (scrollEl) scrollEl.scrollTop = scrollEl.scrollHeight;
  }

  // Auto-scroll to the newest message whenever the thread changes (initial load,
  // optimistic append, or new inbound). Mirrors DmDetail's prior scroll calls.
  $effect(() => {
    // Touch length so the effect re-runs on every append.
    void messages.length;
    void scrollToBottom();
  });

  async function send(): Promise<void> {
    const text = replyText.trim();
    if (!text || sending) return;
    await onsend(text);
    // Clear the composer only on a clean send. The parent sets `sendError`
    // inside `onsend` (synchronously, in its catch) when the send fails, so a
    // null `sendError` here means success — matching DmDetail's prior behavior
    // of clearing `replyText` only in the try path.
    if (!sendError) replyText = '';
  }

  function onReplyKeydown(e: KeyboardEvent): void {
    if ((e.metaKey || e.ctrlKey) && e.key === 'Enter') {
      e.preventDefault();
      void send();
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

  function dayKey(iso: string): string {
    const date = new Date(iso);
    if (Number.isNaN(date.getTime())) return '';
    return date.toDateString();
  }

  function formatDateSeparator(iso: string): string {
    try {
      return new Intl.DateTimeFormat(undefined, {
        month: 'short',
        day: 'numeric',
        year: 'numeric',
      }).format(new Date(iso));
    } catch {
      return '';
    }
  }

  function startsNewDay(index: number): boolean {
    if (index === 0) return true;
    return dayKey(messages[index - 1]?.createdAt ?? '') !== dayKey(messages[index]?.createdAt ?? '');
  }

  // Short relative-time stamp for the "last {time}" reply affordance (US-022).
  // Falls back to the absolute clock time for anything older than a day or
  // unparseable.
  function formatRelative(iso: string | null | undefined): string {
    if (!iso) return '';
    const then = new Date(iso).getTime();
    if (Number.isNaN(then)) return '';
    const diffMs = Date.now() - then;
    if (diffMs < 0) return 'now';
    const sec = Math.floor(diffMs / 1000);
    if (sec < 60) return 'just now';
    const min = Math.floor(sec / 60);
    if (min < 60) return `${min}m ago`;
    const hr = Math.floor(min / 60);
    if (hr < 24) return `${hr}h ago`;
    return formatTime(iso);
  }

  // True when this message is a thread root that should show the reply-count
  // affordance (it carries a rootEventId and at least one reply).
  function hasReplies(msg: ConversationMessage): boolean {
    return !!msg.rootEventId && (msg.replyCount ?? 0) > 0;
  }

  function openThread(rootEventId: string | null | undefined): void {
    const id = rootEventId?.trim();
    if (id) onopenthread?.(id);
  }

  // Copy either a message's body or its attached agent prompt to the clipboard.
  // The text selection (trim + empty→null) lives in the pure `copyableText`
  // helper so it stays unit-tested; here we just perform the write and flash the
  // scoped "Copied!" feedback.
  async function copyText(id: string, kind: CopyKind, msg: ConversationMessage): Promise<void> {
    const text = copyableText(msg, kind);
    if (!text) return;
    try {
      await navigator.clipboard.writeText(text);
      copied = { id, kind };
      setTimeout(() => {
        if (copied?.id === id && copied?.kind === kind) copied = null;
      }, 1800);
    } catch (err) {
      console.error('conversation: clipboard write failed', err);
    }
  }
</script>

<div class="dm-thread" bind:this={scrollEl}>
  {#if loading}
    <p class="dm-thread-status">Loading conversation…</p>
  {/if}
  {#if error}
    <p class="dm-thread-status dm-thread-error" role="alert">{error}</p>
  {/if}

  {#each messages as msg, index (msg.eventId)}
    {#if startsNewDay(index)}
      <div class="date-separator" aria-label={formatDateSeparator(msg.createdAt)}>
        <span>{formatDateSeparator(msg.createdAt)}</span>
      </div>
    {/if}
    <div class="dm-msg dm-msg-{msg.direction}">
      {#if showAuthors && msg.direction === 'in'}
        <span class="dm-msg-author">{msg.fromDisplayName}</span>
      {/if}
      <div
        class="dm-bubble"
        class:dm-bubble-thread-active={!!activeRootEventId && msg.rootEventId === activeRootEventId}
      >
        <!-- Copy the whole message. Hover/focus-revealed on every bubble so it
             stays out of the way until wanted; copying the agent prompt is a
             separate, always-visible labelled action below. -->
        <div class="dm-bubble-actions">
          <button
            type="button"
            class="dm-action"
            class:dm-action-done={isCopied(msg.eventId, 'body')}
            onclick={() => copyText(msg.eventId, 'body', msg)}
            aria-label={isCopied(msg.eventId, 'body') ? 'Message copied' : 'Copy message'}
            title={isCopied(msg.eventId, 'body') ? 'Copied!' : 'Copy message'}
          >
            {#if isCopied(msg.eventId, 'body')}
              <svg width="13" height="13" viewBox="0 0 16 16" fill="none" xmlns="http://www.w3.org/2000/svg" aria-hidden="true">
                <path d="M3 8.5L6.5 12L13 4.5" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" />
              </svg>
            {:else}
              <svg width="13" height="13" viewBox="0 0 16 16" fill="none" xmlns="http://www.w3.org/2000/svg" aria-hidden="true">
                <rect x="5.5" y="5.5" width="8" height="8" rx="1.5" stroke="currentColor" stroke-width="1.3" />
                <path d="M3.5 10.5H3A1.5 1.5 0 0 1 1.5 9V3A1.5 1.5 0 0 1 3 1.5h6A1.5 1.5 0 0 1 10.5 3v.5" stroke="currentColor" stroke-width="1.3" stroke-linecap="round" stroke-linejoin="round" />
              </svg>
            {/if}
          </button>
        </div>
        <p class="dm-bubble-body">{msg.body}</p>
        {#if msg.details}
          <div class="dm-bubble-details">{msg.details}</div>
        {/if}
        {#if msg.prompt}
          <button
            class="btn btn-copy"
            onclick={() => copyText(msg.eventId, 'prompt', msg)}
            aria-label="Copy agent prompt to clipboard"
          >
            {isCopied(msg.eventId, 'prompt') ? 'Copied!' : 'Copy prompt'}
          </button>
        {/if}
      </div>
      {#if hasReplies(msg)}
        <button
          class="thread-affordance"
          type="button"
          onclick={() => openThread(msg.rootEventId)}
          aria-label={`Open thread — ${msg.replyCount} ${(msg.replyCount ?? 0) === 1 ? 'reply' : 'replies'}`}
        >
          <span class="thread-affordance-count">
            {msg.replyCount}
            {(msg.replyCount ?? 0) === 1 ? 'reply' : 'replies'}
          </span>
          {#if msg.lastReplyAt}
            <span class="thread-affordance-time">· last {formatRelative(msg.lastReplyAt)}</span>
          {/if}
        </button>
      {/if}
      {#if ontogglereaction && !msg.pending}
        <ReactionBar
          messageId={msg.eventId}
          reactions={reactions[msg.eventId]}
          ontoggle={ontogglereaction}
        />
      {/if}
      {#if msg.pending}
        <span class="dm-msg-pending">{msg.pendingLabel || 'Pending'}</span>
      {:else}
        <span class="dm-msg-time">{formatTime(msg.createdAt)}</span>
        {#if msg.direction === 'out'}
          <span class="dm-msg-time">Delivered</span>
        {/if}
      {/if}
    </div>
  {/each}
</div>

{#if readonly}
  <div class="dm-reply dm-reply-readonly">
    <span class="dm-reply-hint">Replies aren’t available in this preview.</span>
  </div>
{:else}
  <div class="dm-reply">
    <textarea
      class="dm-reply-input"
      bind:value={replyText}
      onkeydown={onReplyKeydown}
      {placeholder}
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
      <button class="btn btn-send" onclick={send} disabled={sending || replyText.trim().length === 0}>
        {sending ? 'Sending…' : 'Send'}
      </button>
    </div>
  </div>
{/if}

<style>
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
    max-width: min(80%, 420px);
  }

  .dm-msg-in {
    align-self: flex-start;
    align-items: flex-start;
  }

  .dm-msg-out {
    align-self: flex-end;
    align-items: flex-end;
  }

  .dm-msg-author {
    font-size: 0.6875rem;
    font-weight: 600;
    color: var(--popover-text-muted, #a0a0b0);
    margin: 0 0.25rem 0.125rem;
  }

  .dm-bubble {
    position: relative;
    padding: 0.5rem 0.75rem;
    border-radius: 16px;
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
  }

  /* Hover/focus copy-message toolbar, pinned to the bubble's top-right. Hidden
     until the bubble is hovered or something inside it is focused (keyboard
     users reach the button by Tab), so it never clutters the thread. */
  .dm-bubble-actions {
    position: absolute;
    /* Float just above the bubble's top-right corner so the control never sits
       over the message text on hover (Slack/Discord pattern). */
    top: -0.625rem;
    right: 0.375rem;
    z-index: 2;
    display: flex;
    gap: 0.125rem;
    opacity: 0;
    transition: opacity 0.12s ease;
  }

  .dm-bubble:hover .dm-bubble-actions,
  .dm-bubble:focus-within .dm-bubble-actions {
    opacity: 1;
  }

  .dm-action {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 1.375rem;
    height: 1.375rem;
    padding: 0;
    border: none;
    border-radius: 6px;
    cursor: pointer;
    background: rgba(0, 0, 0, 0.32);
    color: var(--popover-text, #e0e0e0);
    transition: background-color 0.12s ease, color 0.12s ease;
  }

  .dm-action:hover {
    background: rgba(0, 0, 0, 0.48);
  }

  .dm-action:focus-visible {
    outline: 2px solid rgba(255, 255, 255, 0.4);
    outline-offset: 1px;
  }

  .dm-action-done {
    color: #7ee0a8;
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

  .dm-msg-pending {
    font-size: 0.625rem;
    font-weight: 600;
    color: #ffd9b0;
    background: rgba(255, 176, 102, 0.18);
    padding: 0.0625rem 0.4375rem;
    border-radius: 999px;
    margin: 0.1875rem 0.25rem 0;
  }

  .date-separator {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    margin: 0.5rem 0;
    color: var(--popover-text-muted, #8a8a98);
    font-size: 0.6875rem;
  }

  .date-separator::before,
  .date-separator::after {
    content: '';
    height: 1px;
    flex: 1;
    background: var(--popover-divider, rgba(255, 255, 255, 0.08));
  }

  /* ── Thread reply-count affordance (US-022) ───────────────────────────── */
  /* Tap-visible (NOT hover-gated) — the standalone window is frameless and has
     no reliable hover, so the affordance is always rendered under a root bubble
     that has replies. */

  .thread-affordance {
    display: inline-flex;
    align-items: center;
    gap: 0.25rem;
    align-self: inherit;
    margin: 0.25rem 0.125rem 0;
    padding: 0.1875rem 0.5rem;
    border: 1px solid rgba(120, 170, 255, 0.28);
    border-radius: 999px;
    background: rgba(120, 170, 255, 0.12);
    color: #bcd4ff;
    font-family: inherit;
    font-size: 0.6875rem;
    font-weight: 600;
    line-height: 1;
    cursor: pointer;
    transition: background-color 0.12s ease, border-color 0.12s ease;
  }

  .thread-affordance:hover,
  .thread-affordance:focus-visible {
    background: rgba(120, 170, 255, 0.22);
    border-color: rgba(120, 170, 255, 0.45);
    outline: none;
  }

  .thread-affordance-count {
    font-weight: 600;
  }

  .thread-affordance-time {
    font-weight: 500;
    color: rgba(188, 212, 255, 0.72);
  }

  /* The root bubble of the thread currently open in the ThreadPanel. */
  .dm-bubble-thread-active {
    box-shadow:
      0 0 0 1px rgba(120, 170, 255, 0.55),
      0 0 0 4px rgba(120, 170, 255, 0.16);
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

  /* ──────────────────────────────────────────────────────────────────────
   * Messages-window override layer (desktop "Company OS" language).
   *
   * Conversation is shared: the classic popover DmDetail window renders it as
   * the monochrome light-glass surface defined ABOVE (those rules are the
   * default and stay untouched), while the standalone Messages window adopts
   * the desktop token system. These overrides are gated under
   * `[data-window='messages']` so DmDetail's appearance is unaffected.
   *
   * Outbound vs inbound is distinguished by SURFACE LAYERING + alignment, not
   * a saturated blue fill: inbound left on a subtle raise surface, outbound
   * right on a restrained --accent-soft "self/primary" tint. Tokens resolve
   * from the shared desktop alias layer (desktop-alt.css).
   * ────────────────────────────────────────────────────────────────────── */

  :global([data-window='messages']) .dm-thread {
    padding: var(--space-4) var(--space-5);
    gap: var(--space-2);
    scrollbar-color: var(--scrollbar-thumb) transparent;
  }

  :global([data-window='messages']) .dm-thread-status {
    font-size: var(--text-base);
    color: var(--muted);
  }

  :global([data-window='messages']) .dm-thread-error {
    color: var(--red);
  }

  :global([data-window='messages']) .dm-msg-author {
    font-family: var(--font-mono);
    font-size: var(--text-micro);
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--muted);
  }

  :global([data-window='messages']) .dm-bubble {
    padding: var(--space-2) var(--space-3);
    border-radius: 16px;
    border: 1px solid var(--border);
  }

  :global([data-window='messages']) .dm-msg-in .dm-bubble {
    background: var(--surface-raise);
    border-bottom-left-radius: var(--radius-sm);
  }

  :global([data-window='messages']) .dm-msg-out .dm-bubble {
    background: var(--accent-soft);
    border-color: var(--border-strong);
    border-bottom-right-radius: var(--radius-sm);
  }

  :global([data-window='messages']) .dm-bubble-body {
    font-size: var(--text-base);
    line-height: 1.5;
    color: var(--fg);
  }

  :global([data-window='messages']) .dm-bubble-details {
    font-family: var(--font-mono);
    font-size: var(--text-xs);
    line-height: 1.5;
    color: var(--fg-data);
    background: var(--surface-panel);
    border-left: 1px solid var(--border-strong);
    border-radius: var(--radius-sm);
    padding: var(--space-2) var(--space-3);
  }

  :global([data-window='messages']) .dm-msg-time {
    font-family: var(--font-mono);
    font-size: var(--text-micro);
    color: var(--muted-3);
    margin: var(--space-1) var(--space-1) 0;
  }

  :global([data-window='messages']) .dm-msg-pending {
    font-family: var(--font-mono);
    font-size: var(--text-micro);
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--muted-2);
    background: var(--surface-raise);
    border-radius: var(--radius-sm);
    padding: 2px var(--space-2);
    margin: var(--space-1) var(--space-1) 0;
  }

  :global([data-window='messages']) .btn-copy {
    background: var(--surface-raise);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    color: var(--fg);
    font-family: var(--font-sans);
    font-size: var(--text-sm);
  }

  :global([data-window='messages']) .btn-copy:hover {
    background: var(--row-hover);
  }

  :global([data-window='messages']) .dm-action {
    background: var(--surface-panel);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    color: var(--muted-2);
  }

  :global([data-window='messages']) .dm-action:hover {
    background: var(--row-hover);
    color: var(--fg);
  }

  :global([data-window='messages']) .dm-action-done {
    color: var(--emerald, #7ee0a8);
  }

  :global([data-window='messages']) .dm-reply {
    padding: var(--space-3) var(--space-5) var(--space-4);
    border-top: 1px solid var(--border);
    background: var(--surface-panel);
  }

  :global([data-window='messages']) .dm-reply-input {
    border-radius: var(--radius-sm);
    border: 1px solid var(--border);
    background: var(--surface-raise);
    color: var(--fg);
    font-family: var(--font-sans);
    font-size: var(--text-base);
    line-height: 1.45;
  }

  :global([data-window='messages']) .dm-reply-input:focus {
    border-color: var(--accent);
    outline: 1px solid var(--accent);
    outline-offset: -1px;
    background: var(--surface-raise);
  }

  :global([data-window='messages']) .dm-reply-hint {
    font-family: var(--font-mono);
    font-size: var(--text-micro);
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--muted);
  }

  :global([data-window='messages']) .dm-reply-error {
    font-size: var(--text-sm);
    color: var(--red);
  }

  :global([data-window='messages']) .btn-send {
    background: var(--accent);
    color: #fff;
    border-radius: var(--radius-sm);
    font-family: var(--font-sans);
    font-weight: 600;
  }

  :global([data-window='messages']) .btn-send:hover:not(:disabled) {
    background: var(--accent);
    filter: brightness(1.1);
  }
</style>
