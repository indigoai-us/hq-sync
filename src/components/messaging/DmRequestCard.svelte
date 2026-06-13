<script lang="ts" module>
  // Re-export the shared request types so existing imports
  // (`import DmRequestCard, { type DmRequest } from './DmRequestCard.svelte'`)
  // keep working. The wire shape + pure helpers live in lib/dmRequests.ts so
  // they're unit-tested without a DOM (mirrors lib/recipientPicker.ts).
  export type { DmRequest, RequestAction } from '../../lib/dmRequests';
</script>

<script lang="ts">
  import {
    type DmRequest,
    type RequestAction,
    requestDisplayName,
    requestInitials,
  } from '../../lib/dmRequests';
  // A pending connection request, rendered as a bordered CARD (deliberately NOT
  // a chat bubble) so an incoming request reads as something to act on, not a
  // message that silently landed in a thread. Shows the requester's name +
  // email, an optional trust hint, the held first message quoted/muted, and
  // Accept (primary) / Decline / Block (destructive) actions.
  //
  // The card owns the respond_dm_request invoke + busy/error state; on success
  // it calls back to the parent (`onresolved`) so the parent can decrement the
  // Requests count and, on accept, swap the card for the standard <Conversation>
  // (the held message becomes a thread).
  import { invoke } from '@tauri-apps/api/core';

  interface Props {
    request: DmRequest;
    // Called after a successful respond_dm_request. The parent prunes the
    // request from the Requests list, decrements the segment count, and (on
    // 'accept') opens the standard conversation thread with the requester.
    onresolved: (request: DmRequest, action: RequestAction) => void;
  }

  let { request, onresolved }: Props = $props();

  // Which action is in flight (disables all buttons + shows a label). null = idle.
  let busy = $state<RequestAction | null>(null);
  let error = $state<string | null>(null);

  const name = $derived(requestDisplayName(request));
  const avatar = $derived(requestInitials(request));

  async function respond(action: RequestAction): Promise<void> {
    if (busy) return;
    busy = action;
    error = null;
    try {
      await invoke('respond_dm_request', {
        pairKey: request.pairKey,
        action,
      });
      // Success — let the parent prune + (on accept) swap in the thread.
      onresolved(request, action);
    } catch (err) {
      error = typeof err === 'string' ? err : `Could not ${action} this request`;
      console.error(`dm-request: respond_dm_request ${action} failed`, err);
    } finally {
      busy = null;
    }
  }
</script>

<article class="request-card" aria-label={`Connection request from ${name}`}>
  <header class="request-head">
    <span class="request-avatar" aria-hidden="true">{avatar}</span>
    <span class="request-id">
      <span class="request-name">{name}</span>
      {#if request.fromEmail}
        <span class="request-email">{request.fromEmail}</span>
      {/if}
    </span>
  </header>

  {#if request.sharedCompany?.trim()}
    <p class="request-hint" title="Why you're seeing this request">
      Also in <strong>{request.sharedCompany.trim()}</strong>
    </p>
  {/if}

  {#if request.message?.trim()}
    <blockquote class="request-message">{request.message.trim()}</blockquote>
  {:else}
    <p class="request-no-message">No message included.</p>
  {/if}

  {#if error}
    <p class="request-error" role="alert">{error}</p>
  {/if}

  <div class="request-actions">
    <button
      class="action action-accept"
      type="button"
      disabled={busy !== null}
      onclick={() => respond('accept')}
    >
      {busy === 'accept' ? 'Accepting…' : 'Accept'}
    </button>
    <button
      class="action action-decline"
      type="button"
      disabled={busy !== null}
      onclick={() => respond('decline')}
    >
      {busy === 'decline' ? 'Declining…' : 'Decline'}
    </button>
    <button
      class="action action-block"
      type="button"
      disabled={busy !== null}
      onclick={() => respond('block')}
    >
      {busy === 'block' ? 'Blocking…' : 'Block'}
    </button>
  </div>
</article>

<style>
  /* Desktop "Company OS" language: a hairline-bordered card (deliberately NOT a
     chat bubble) over a low-fill surface, one 13px body size with monospace caps
     for the avatar + email, accent reserved for the primary Accept CTA + focus
     ring. Decline is neutral; Block carries --red only to MARK the destructive
     action, never decoration. No side-stripe borders. Tokens come from the
     shared desktop alias layer (desktop-alt.css). */

  .request-card {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    padding: var(--space-3);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--surface-raise);
    font-family: var(--font-sans);
    letter-spacing: -0.006em;
  }

  .request-head {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    min-width: 0;
  }

  .request-avatar {
    flex-shrink: 0;
    width: 28px;
    height: 28px;
    border-radius: 7px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    background: var(--surface-panel);
    border: 1px solid var(--border);
    color: var(--muted-2);
    font-family: var(--font-mono);
    font-size: var(--text-micro);
    font-weight: 600;
  }

  .request-id {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }

  .request-name {
    font-size: var(--text-base);
    font-weight: 600;
    color: var(--fg);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .request-email {
    font-family: var(--font-mono);
    font-size: var(--text-micro);
    color: var(--muted);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .request-hint {
    margin: 0;
    font-size: var(--text-base);
    color: var(--muted);
  }

  .request-hint strong {
    color: var(--fg);
    font-weight: 600;
  }

  .request-message {
    margin: 0;
    padding: var(--space-2) var(--space-3);
    border: 1px solid var(--border);
    border-radius: var(--radius-sm);
    background: var(--surface-panel);
    font-size: var(--text-base);
    line-height: 1.45;
    color: var(--muted-2);
    white-space: pre-wrap;
    word-break: break-word;
  }

  .request-no-message {
    margin: 0;
    font-size: var(--text-base);
    font-style: italic;
    color: var(--muted);
  }

  .request-error {
    margin: 0;
    font-size: var(--text-sm);
    color: var(--red);
  }

  .request-actions {
    display: flex;
    gap: var(--space-2);
  }

  .action {
    flex: 1;
    padding: var(--space-2);
    border-radius: var(--radius-sm);
    border: 1px solid transparent;
    font-family: var(--font-sans);
    font-size: var(--text-base);
    font-weight: 600;
    cursor: pointer;
    transition:
      background-color 0.12s ease,
      filter 0.12s ease,
      opacity 0.12s ease;
  }

  .action:disabled {
    cursor: default;
    opacity: 0.55;
  }

  .action:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 1px;
  }

  .action-accept {
    background: var(--accent);
    border-color: var(--accent);
    color: #fff;
  }

  .action-accept:hover:not(:disabled) {
    filter: brightness(1.1);
  }

  .action-decline {
    background: var(--surface-raise);
    border-color: var(--border-strong);
    color: var(--fg);
  }

  .action-decline:hover:not(:disabled) {
    background: var(--row-hover);
  }

  .action-block {
    background: transparent;
    border-color: var(--border-strong);
    color: var(--red);
  }

  .action-block:hover:not(:disabled) {
    background: var(--row-hover);
  }
</style>
