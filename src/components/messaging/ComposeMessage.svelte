<script lang="ts">
  // New Message compose sheet (US-010).
  //
  // An OVERLAY inside MessagesShell (not a new OS window): a dimmed backdrop with
  // a centered sheet containing a RecipientPicker, a body textarea (composer
  // styling borrowed from Conversation.svelte), and a send button. The button
  // adapts to the selected recipient's connectionState:
  //
  //   active            → "Send"
  //   pending|none      → "Send request" + a "you're not connected" note
  //   blocked|declined  → disabled + "{name} isn't accepting messages."
  //
  // ⌘↵ sends. On success the parent is told the outcome (delivered vs
  // connection_requested) so it can open the thread or render a Pending bubble.
  import { invoke } from '@tauri-apps/api/core';
  import RecipientPicker from './RecipientPicker.svelte';
  import type { SelectedRecipient } from '../../lib/recipientPicker';

  // Discriminant returned by the Rust `send_dm_to_email` command. `tag = "state"`
  // + camelCase serde → "delivered" | "connectionRequested".
  type SendOutcome = { state: 'delivered' | 'connectionRequested' };

  /** What the parent receives when a send succeeds. */
  export interface ComposeSendResult {
    recipient: SelectedRecipient;
    body: string;
    /** True when the backend held the message and sent a connect request (202). */
    pending: boolean;
  }

  interface Props {
    onclose: () => void;
    onsent: (result: ComposeSendResult) => void;
  }

  let { onclose, onsent }: Props = $props();

  let recipient = $state<SelectedRecipient | null>(null);
  let body = $state('');
  let sending = $state(false);
  let sendError = $state<string | null>(null);

  function nameOf(r: SelectedRecipient): string {
    return r.displayName?.trim() || r.email;
  }

  // blocked or declined recipients can't be messaged at all.
  const isBlocked = $derived(
    recipient != null && (recipient.connectionState === 'blocked' || (recipient.connectionState as string) === 'declined'),
  );
  const isActive = $derived(recipient != null && recipient.connectionState === 'active');
  // "Send request" path: a known-but-not-active recipient (pending/none).
  const needsRequest = $derived(recipient != null && !isActive && !isBlocked);

  const canSend = $derived(
    recipient != null && !isBlocked && body.trim().length > 0 && !sending,
  );

  const sendLabel = $derived(sending ? 'Sending…' : needsRequest ? 'Send request' : 'Send');

  async function send(): Promise<void> {
    if (!recipient || isBlocked) return;
    const text = body.trim();
    if (!text || sending) return;
    sending = true;
    sendError = null;
    try {
      const outcome = await invoke<SendOutcome>('send_dm_to_email', {
        toEmail: recipient.email || null,
        toPersonUid: recipient.personUid ?? null,
        body: text,
      });
      const pending = outcome?.state === 'connectionRequested';
      onsent({ recipient, body: text, pending });
    } catch (err) {
      sendError = typeof err === 'string' ? err : 'Failed to send message';
      console.error('compose: send_dm_to_email failed', err);
    } finally {
      sending = false;
    }
  }

  function onBodyKeydown(e: KeyboardEvent): void {
    if ((e.metaKey || e.ctrlKey) && e.key === 'Enter') {
      e.preventDefault();
      if (canSend) void send();
    }
  }

  function onBackdropKeydown(e: KeyboardEvent): void {
    if (e.key === 'Escape') onclose();
  }
</script>

<!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
<div
  class="compose-backdrop"
  onclick={onclose}
  onkeydown={onBackdropKeydown}
  role="presentation"
>
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div
    class="compose-sheet"
    role="dialog"
    aria-modal="true"
    aria-label="New message"
    tabindex="-1"
    onclick={(e) => e.stopPropagation()}
    onkeydown={onBackdropKeydown}
  >
    <header class="compose-header">
      <h2>New message</h2>
      <button class="compose-close" type="button" onclick={onclose} aria-label="Close">×</button>
    </header>

    <div class="compose-field">
      <span class="compose-label">To</span>
      <RecipientPicker
        bind:selected={recipient}
        onselect={(r) => {
          recipient = r;
          sendError = null;
        }}
      />
    </div>

    {#if recipient}
      {#if isBlocked}
        <p class="compose-note compose-note-blocked" role="alert">
          {nameOf(recipient)} isn't accepting messages.
        </p>
      {:else if needsRequest}
        <p class="compose-note">
          You're not connected to {nameOf(recipient)} yet. We'll send them a
          request to connect along with your message.
        </p>
      {/if}
    {/if}

    <textarea
      class="compose-body"
      bind:value={body}
      onkeydown={onBodyKeydown}
      placeholder={recipient ? `Message ${nameOf(recipient)}…` : 'Write your message…'}
      rows="5"
      disabled={sending || isBlocked}
      aria-label="Message body"
    ></textarea>

    <div class="compose-footer">
      {#if sendError}
        <span class="compose-error" role="alert">{sendError}</span>
      {:else}
        <span class="compose-hint">⌘↵ to send</span>
      {/if}
      <button class="btn btn-send" type="button" onclick={send} disabled={!canSend}>
        {sendLabel}
      </button>
    </div>
  </div>
</div>

<style>
  /* Desktop "Company OS" language: monochrome glass sheet, hairline borders,
     one 13px size + 11px monospace caps for the field label, accent reserved
     for the primary Send CTA + focus ring. The "not connected" / "blocked"
     notes mark state via copy + a neutral (or restrained --red) surface — no
     side-stripe border, no amber/orange decoration. Tokens come from the
     shared desktop alias layer (desktop-alt.css). */

  .compose-backdrop {
    position: fixed;
    inset: 0;
    z-index: 50;
    display: flex;
    align-items: flex-start;
    justify-content: center;
    padding: 3.5rem var(--space-5) var(--space-5);
    background: rgba(0, 0, 0, 0.5);
  }

  .compose-sheet {
    width: 100%;
    max-width: 460px;
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    padding: var(--space-4) var(--space-5) var(--space-5);
    border-radius: var(--radius-md);
    border: 1px solid var(--border-strong);
    background: var(--bg);
    box-shadow: 0 24px 64px rgba(0, 0, 0, 0.55);
    color: var(--fg);
    font-family: var(--font-sans);
    letter-spacing: -0.006em;
  }

  .compose-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }

  .compose-header h2 {
    margin: 0;
    font-family: var(--font-display);
    font-size: var(--text-base);
    font-weight: 600;
    letter-spacing: -0.01em;
    color: var(--fg);
  }

  .compose-close {
    border: none;
    background: transparent;
    color: var(--muted);
    font-size: 1.25rem;
    line-height: 1;
    cursor: pointer;
    padding: 0 var(--space-1);
    border-radius: var(--radius-sm);
  }

  .compose-close:hover {
    background: var(--row-hover);
    color: var(--fg);
  }

  .compose-field {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }

  .compose-label {
    font-family: var(--font-mono);
    font-size: var(--text-micro);
    font-weight: 600;
    letter-spacing: 0.04em;
    text-transform: uppercase;
    color: var(--muted);
  }

  .compose-note {
    margin: 0;
    font-size: var(--text-base);
    line-height: 1.5;
    color: var(--muted-2);
    background: var(--surface-raise);
    border: 1px solid var(--border);
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-sm);
  }

  .compose-note-blocked {
    background: var(--surface-raise);
    border-color: var(--border-strong);
    color: var(--red);
  }

  .compose-body {
    width: 100%;
    box-sizing: border-box;
    resize: none;
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-sm);
    border: 1px solid var(--border);
    background: var(--surface-raise);
    color: var(--fg);
    font-family: var(--font-sans);
    font-size: var(--text-base);
    line-height: 1.45;
  }

  .compose-body:focus {
    outline: none;
    border-color: var(--accent);
    box-shadow: 0 0 0 1px var(--accent);
  }

  .compose-body:disabled {
    opacity: 0.55;
  }

  .compose-footer {
    display: flex;
    align-items: center;
    gap: var(--space-3);
  }

  .compose-hint {
    font-family: var(--font-mono);
    font-size: var(--text-micro);
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--muted);
  }

  .compose-error {
    font-size: var(--text-sm);
    color: var(--red);
    word-break: break-word;
  }

  .btn {
    display: inline-flex;
    align-items: center;
    padding: var(--space-2) var(--space-4);
    border-radius: var(--radius-sm);
    font-family: var(--font-sans);
    font-size: var(--text-base);
    font-weight: 600;
    cursor: pointer;
    border: none;
    transition: background-color 0.12s ease, filter 0.12s ease;
  }

  .btn:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 1px;
  }

  .btn-send {
    margin-left: auto;
    background: var(--accent);
    color: #fff;
  }

  .btn-send:hover:not(:disabled) {
    filter: brightness(1.1);
  }

  .btn-send:disabled {
    opacity: 0.45;
    cursor: default;
  }
</style>
