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
  .compose-backdrop {
    position: fixed;
    inset: 0;
    z-index: 50;
    display: flex;
    align-items: flex-start;
    justify-content: center;
    padding: 3.5rem 1.5rem 1.5rem;
    background: rgba(0, 0, 0, 0.42);
    backdrop-filter: blur(2px);
    -webkit-backdrop-filter: blur(2px);
  }

  .compose-sheet {
    width: 100%;
    max-width: 460px;
    display: flex;
    flex-direction: column;
    gap: 0.75rem;
    padding: 1.125rem 1.25rem 1.25rem;
    border-radius: 14px;
    border: 1px solid var(--popover-divider, rgba(255, 255, 255, 0.1));
    background: var(--popover-bg, #1a1a22);
    backdrop-filter: var(--popover-blur, blur(28px) saturate(1.45));
    -webkit-backdrop-filter: var(--popover-blur, blur(28px) saturate(1.45));
    box-shadow: 0 24px 64px rgba(0, 0, 0, 0.55);
    color: var(--popover-text, rgba(255, 255, 255, 0.86));
  }

  .compose-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }

  .compose-header h2 {
    margin: 0;
    font-size: 0.9375rem;
    font-weight: 600;
    color: var(--popover-text-heading, #ffffff);
  }

  .compose-close {
    border: none;
    background: transparent;
    color: var(--popover-text-muted, #a0a0b0);
    font-size: 1.25rem;
    line-height: 1;
    cursor: pointer;
    padding: 0 0.25rem;
    border-radius: 6px;
  }

  .compose-close:hover {
    background: rgba(255, 255, 255, 0.08);
    color: var(--popover-text, #e8e8ee);
  }

  .compose-field {
    display: flex;
    flex-direction: column;
    gap: 0.3125rem;
  }

  .compose-label {
    font-size: 0.625rem;
    font-weight: 600;
    letter-spacing: 0.04em;
    text-transform: uppercase;
    color: var(--popover-text-muted, #8a8a98);
  }

  .compose-note {
    margin: 0;
    font-size: 0.75rem;
    line-height: 1.5;
    color: var(--popover-text-muted, #b8b8c4);
    background: rgba(255, 176, 102, 0.12);
    border-left: 2px solid rgba(255, 176, 102, 0.45);
    padding: 0.5rem 0.625rem;
    border-radius: 0 6px 6px 0;
  }

  .compose-note-blocked {
    background: rgba(255, 120, 120, 0.12);
    border-left-color: rgba(255, 120, 120, 0.5);
    color: #ffbdbd;
  }

  .compose-body {
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
    line-height: 1.45;
  }

  .compose-body:focus {
    outline: none;
    border-color: rgba(255, 255, 255, 0.28);
    background: rgba(255, 255, 255, 0.06);
  }

  .compose-body:disabled {
    opacity: 0.55;
  }

  .compose-footer {
    display: flex;
    align-items: center;
    gap: 0.75rem;
  }

  .compose-hint {
    font-size: 0.6875rem;
    color: var(--popover-text-muted, #a0a0b0);
  }

  .compose-error {
    font-size: 0.75rem;
    color: #ff9b9b;
    word-break: break-word;
  }

  .btn {
    display: inline-flex;
    align-items: center;
    padding: 0.4375rem 0.875rem;
    border-radius: 7px;
    font-size: 0.75rem;
    font-weight: 600;
    cursor: pointer;
    border: none;
    font-family: inherit;
    transition: background-color 0.12s ease;
  }

  .btn-send {
    margin-left: auto;
    background: rgba(120, 170, 255, 0.26);
    color: #dce8ff;
  }

  .btn-send:hover:not(:disabled) {
    background: rgba(120, 170, 255, 0.38);
  }

  .btn-send:disabled {
    opacity: 0.45;
    cursor: default;
  }
</style>
