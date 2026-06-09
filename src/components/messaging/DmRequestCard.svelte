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
  .request-card {
    display: flex;
    flex-direction: column;
    gap: 0.625rem;
    padding: 0.875rem;
    border: 1px solid var(--popover-divider, rgba(255, 255, 255, 0.12));
    border-radius: 12px;
    background: rgba(255, 255, 255, 0.035);
  }

  .request-head {
    display: flex;
    align-items: center;
    gap: 0.625rem;
    min-width: 0;
  }

  .request-avatar {
    flex-shrink: 0;
    width: 2rem;
    height: 2rem;
    border-radius: 50%;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    background: rgba(255, 176, 102, 0.22);
    color: #ffd9b0;
    font-size: 0.6875rem;
    font-weight: 600;
  }

  .request-id {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }

  .request-name {
    font-size: 0.875rem;
    font-weight: 600;
    color: var(--popover-text-heading, #ffffff);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .request-email {
    font-size: 0.6875rem;
    color: var(--popover-text-muted, #8a8a98);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .request-hint {
    margin: 0;
    font-size: 0.6875rem;
    color: var(--popover-text-muted, #a0a0b0);
  }

  .request-hint strong {
    color: var(--popover-text, #e8e8ee);
    font-weight: 600;
  }

  .request-message {
    margin: 0;
    padding: 0.5rem 0.75rem;
    border-left: 2px solid rgba(255, 255, 255, 0.18);
    border-radius: 0 6px 6px 0;
    background: rgba(255, 255, 255, 0.04);
    font-size: 0.8125rem;
    line-height: 1.45;
    color: var(--popover-text-muted, #b8b8c4);
    white-space: pre-wrap;
    word-break: break-word;
  }

  .request-no-message {
    margin: 0;
    font-size: 0.75rem;
    font-style: italic;
    color: var(--popover-text-muted, #8a8a98);
  }

  .request-error {
    margin: 0;
    font-size: 0.75rem;
    color: #ff9b9b;
  }

  .request-actions {
    display: flex;
    gap: 0.5rem;
  }

  .action {
    flex: 1;
    padding: 0.4375rem 0.5rem;
    border-radius: 8px;
    border: 1px solid transparent;
    font-family: inherit;
    font-size: 0.75rem;
    font-weight: 600;
    cursor: pointer;
    transition:
      background-color 0.12s ease,
      opacity 0.12s ease;
  }

  .action:disabled {
    cursor: default;
    opacity: 0.55;
  }

  .action-accept {
    background: rgba(120, 170, 255, 0.2);
    border-color: rgba(120, 170, 255, 0.4);
    color: #dce8ff;
  }

  .action-accept:hover:not(:disabled) {
    background: rgba(120, 170, 255, 0.32);
  }

  .action-decline {
    background: rgba(255, 255, 255, 0.06);
    border-color: rgba(255, 255, 255, 0.14);
    color: var(--popover-text, #e8e8ee);
  }

  .action-decline:hover:not(:disabled) {
    background: rgba(255, 255, 255, 0.12);
  }

  .action-block {
    background: rgba(255, 107, 107, 0.14);
    border-color: rgba(255, 107, 107, 0.34);
    color: #ffb0b0;
  }

  .action-block:hover:not(:disabled) {
    background: rgba(255, 107, 107, 0.24);
  }
</style>
