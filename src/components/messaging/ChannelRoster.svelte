<script lang="ts">
  // Channel roster (US-018): the member list for one channel, opened from the
  // ChannelView header member-count button. Each row shows the member's name +
  // role; the channel owner additionally sees a "Remove" affordance per other
  // member. An "Invite people" affordance reuses RecipientPicker scoped to the
  // channel (the picker enumerates the caller's contacts + company members; the
  // chosen person is invited via invite_to_channel).
  //
  // The roster owns its own list fetch + the invite/remove invokes; it tells the
  // parent (ChannelView) the new member count via `oncountchange` so the header
  // button stays in lockstep.
  import { invoke } from '@tauri-apps/api/core';
  import RecipientPicker from './RecipientPicker.svelte';
  import type { SelectedRecipient } from '../../lib/recipientPicker';
  import type { ChannelMember } from '../../lib/channels';

  interface Props {
    channelId: string;
    // The caller's own personUid — used both to resolve whether the caller is
    // the channel owner (their row's role === "owner") and to suppress a
    // "Remove" button on their own row.
    selfPersonUid?: string | null;
    onclose: () => void;
    oncountchange?: (count: number) => void;
  }

  let { channelId, selfPersonUid = null, onclose, oncountchange }: Props = $props();

  let members = $state<ChannelMember[]>([]);
  let loading = $state(false);
  let error = $state<string | null>(null);

  // The caller owns the channel when their own member row carries the "owner"
  // role. This is the single source of truth for the remove + invite
  // affordances (the Channel wire shape doesn't carry the caller's role). The
  // server also rejects a non-owner's mutation as defense-in-depth.
  const isOwner = $derived(
    selfPersonUid != null &&
      members.some((m) => m.personUid === selfPersonUid && m.role === 'owner'),
  );

  // Invite affordance state.
  let inviting = $state(false);
  let invitePick = $state<SelectedRecipient | null>(null);
  let inviteError = $state<string | null>(null);
  let inviteBusy = $state(false);

  // Which member is being removed (disables that row's button).
  let removing = $state<string | null>(null);

  interface MembersResponse {
    members: ChannelMember[];
  }

  async function loadMembers(): Promise<void> {
    loading = true;
    error = null;
    try {
      const resp = await invoke<MembersResponse>('list_channel_members', { channelId });
      members = resp.members ?? [];
      oncountchange?.(members.length);
    } catch (err) {
      error = typeof err === 'string' ? err : 'Could not load members';
      members = [];
      console.error('channel-roster: list_channel_members failed', err);
    } finally {
      loading = false;
    }
  }

  async function invite(): Promise<void> {
    const uid = invitePick?.personUid?.trim();
    if (!uid || inviteBusy) {
      if (!uid) inviteError = 'Pick a person with an HQ account to invite.';
      return;
    }
    inviteBusy = true;
    inviteError = null;
    try {
      const resp = await invoke<MembersResponse>('invite_to_channel', {
        channelId,
        personUids: [uid],
      });
      members = resp.members ?? members;
      oncountchange?.(members.length);
      invitePick = null;
      inviting = false;
    } catch (err) {
      inviteError = typeof err === 'string' ? err : 'Could not invite this person';
      console.error('channel-roster: invite_to_channel failed', err);
    } finally {
      inviteBusy = false;
    }
  }

  async function remove(personUid: string): Promise<void> {
    if (removing) return;
    removing = personUid;
    error = null;
    try {
      const resp = await invoke<MembersResponse>('remove_channel_member', {
        channelId,
        personUid,
      });
      // Prefer the server's returned list; fall back to a local prune.
      members = resp.members && resp.members.length > 0
        ? resp.members
        : members.filter((m) => m.personUid !== personUid);
      oncountchange?.(members.length);
    } catch (err) {
      error = typeof err === 'string' ? err : 'Could not remove this member';
      console.error('channel-roster: remove_channel_member failed', err);
    } finally {
      removing = null;
    }
  }

  function memberLabel(m: ChannelMember): string {
    return m.displayName?.trim() || m.email?.trim() || m.personUid;
  }

  function onBackdropKeydown(e: KeyboardEvent): void {
    if (e.key === 'Escape') onclose();
  }

  $effect(() => {
    void loadMembers();
  });
</script>

<!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
<div class="roster-backdrop" onclick={onclose} onkeydown={onBackdropKeydown} role="presentation">
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div
    class="roster-sheet"
    role="dialog"
    aria-modal="true"
    aria-label="Channel members"
    tabindex="-1"
    onclick={(e) => e.stopPropagation()}
    onkeydown={onBackdropKeydown}
  >
    <header class="roster-header">
      <h2>Members{#if members.length > 0} ({members.length}){/if}</h2>
      <button class="roster-close" type="button" onclick={onclose} aria-label="Close">×</button>
    </header>

    {#if isOwner}
      {#if inviting}
        <div class="invite-row">
          <RecipientPicker
            bind:selected={invitePick}
            onselect={(r) => {
              invitePick = r;
              inviteError = null;
            }}
            placeholder="Invite someone…"
          />
          <div class="invite-actions">
            {#if inviteError}
              <span class="invite-error" role="alert">{inviteError}</span>
            {/if}
            <button class="btn btn-ghost" type="button" onclick={() => (inviting = false)}>
              Cancel
            </button>
            <button
              class="btn btn-primary"
              type="button"
              onclick={invite}
              disabled={inviteBusy || !invitePick}
            >
              {inviteBusy ? 'Inviting…' : 'Invite'}
            </button>
          </div>
        </div>
      {:else}
        <button class="invite-open" type="button" onclick={() => (inviting = true)}>
          + Invite people
        </button>
      {/if}
    {/if}

    {#if loading}
      <p class="roster-status">Loading members…</p>
    {:else if error}
      <p class="roster-status roster-error" role="alert">{error}</p>
    {:else if members.length === 0}
      <p class="roster-status">No members yet.</p>
    {:else}
      <ul class="member-list">
        {#each members as m (m.personUid)}
          <li class="member-row">
            <span class="member-meta">
              <span class="member-name">{memberLabel(m)}</span>
              <span class="member-role" class:owner={m.role === 'owner'}>{m.role}</span>
            </span>
            {#if isOwner && m.role !== 'owner' && m.personUid !== selfPersonUid}
              <button
                class="member-remove"
                type="button"
                onclick={() => remove(m.personUid)}
                disabled={removing !== null}
              >
                {removing === m.personUid ? 'Removing…' : 'Remove'}
              </button>
            {/if}
          </li>
        {/each}
      </ul>
    {/if}
  </div>
</div>

<style>
  .roster-backdrop {
    position: fixed;
    inset: 0;
    z-index: 60;
    display: flex;
    align-items: flex-start;
    justify-content: center;
    padding: 3.5rem 1.5rem 1.5rem;
    background: rgba(0, 0, 0, 0.42);
    backdrop-filter: blur(2px);
    -webkit-backdrop-filter: blur(2px);
  }

  .roster-sheet {
    width: 100%;
    max-width: 420px;
    max-height: 70vh;
    display: flex;
    flex-direction: column;
    gap: 0.625rem;
    padding: 1.125rem 1.25rem 1.25rem;
    border-radius: 14px;
    border: 1px solid var(--popover-divider, rgba(255, 255, 255, 0.1));
    background: var(--popover-bg, #1a1a22);
    backdrop-filter: var(--popover-blur, blur(28px) saturate(1.45));
    -webkit-backdrop-filter: var(--popover-blur, blur(28px) saturate(1.45));
    box-shadow: 0 24px 64px rgba(0, 0, 0, 0.55);
    color: var(--popover-text, rgba(255, 255, 255, 0.86));
    overflow: hidden;
  }

  .roster-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }

  .roster-header h2 {
    margin: 0;
    font-size: var(--text-base);
    font-weight: 600;
    color: var(--popover-text-heading, #ffffff);
  }

  .roster-close {
    border: none;
    background: transparent;
    color: var(--popover-text-muted, #a0a0b0);
    font-size: var(--text-lg);
    line-height: 1;
    cursor: pointer;
    padding: 0 0.25rem;
    border-radius: 6px;
  }

  .roster-close:hover {
    background: rgba(255, 255, 255, 0.08);
    color: var(--popover-text, #e8e8ee);
  }

  .invite-open {
    align-self: flex-start;
    border: 1px solid rgba(120, 170, 255, 0.32);
    background: rgba(120, 170, 255, 0.16);
    color: #dce8ff;
    font-family: inherit;
    font-size: var(--text-base);
    font-weight: 600;
    padding: 0.3125rem 0.625rem;
    border-radius: 7px;
    cursor: pointer;
  }

  .invite-open:hover {
    background: rgba(120, 170, 255, 0.28);
  }

  .invite-row {
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
    padding: 0.625rem;
    border: 1px solid var(--popover-divider, rgba(255, 255, 255, 0.1));
    border-radius: 10px;
    background: rgba(255, 255, 255, 0.03);
  }

  .invite-actions {
    display: flex;
    align-items: center;
    gap: 0.5rem;
  }

  .invite-error {
    font-size: var(--text-base);
    color: #ff9b9b;
    margin-right: auto;
  }

  .roster-status {
    margin: 0.5rem 0;
    font-size: var(--text-base);
    color: var(--popover-text-muted, #a0a0b0);
  }

  .roster-error {
    color: #ff9b9b;
  }

  .member-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 0.125rem;
    overflow-y: auto;
    scrollbar-width: thin;
    scrollbar-color: rgba(255, 255, 255, 0.15) transparent;
  }

  .member-row {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    padding: 0.4375rem 0.5rem;
    border-radius: 8px;
  }

  .member-row:hover {
    background: rgba(255, 255, 255, 0.04);
  }

  .member-meta {
    display: flex;
    align-items: baseline;
    gap: 0.5rem;
    min-width: 0;
  }

  .member-name {
    font-size: var(--text-base);
    font-weight: 500;
    color: var(--popover-text, #e8e8ee);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .member-role {
    font-size: var(--text-base);
    font-weight: 600;
    letter-spacing: 0.03em;
    text-transform: uppercase;
    color: var(--popover-text-muted, #8a8a98);
  }

  .member-role.owner {
    color: #ffd9b0;
  }

  .member-remove {
    margin-left: auto;
    flex-shrink: 0;
    border: 1px solid rgba(255, 107, 107, 0.34);
    background: rgba(255, 107, 107, 0.12);
    color: #ffb0b0;
    font-family: inherit;
    font-size: var(--text-base);
    font-weight: 600;
    padding: 0.1875rem 0.5rem;
    border-radius: 6px;
    cursor: pointer;
  }

  .member-remove:hover:not(:disabled) {
    background: rgba(255, 107, 107, 0.22);
  }

  .member-remove:disabled {
    opacity: 0.5;
    cursor: default;
  }

  .btn {
    display: inline-flex;
    align-items: center;
    padding: 0.3125rem 0.75rem;
    border-radius: 7px;
    font-size: var(--text-base);
    font-weight: 600;
    cursor: pointer;
    border: none;
    font-family: inherit;
    transition: background-color 0.12s ease;
  }

  .btn-ghost {
    background: rgba(255, 255, 255, 0.06);
    color: var(--popover-text, #e8e8ee);
  }

  .btn-ghost:hover {
    background: rgba(255, 255, 255, 0.12);
  }

  .btn-primary {
    background: rgba(120, 170, 255, 0.26);
    color: #dce8ff;
  }

  .btn-primary:hover:not(:disabled) {
    background: rgba(120, 170, 255, 0.38);
  }

  .btn-primary:disabled {
    opacity: 0.45;
    cursor: default;
  }
</style>
