<script lang="ts">
  // Create-channel sheet (US-018). An OVERLAY inside MessagesShell (mirrors
  // ComposeMessage): a dimmed backdrop + centered sheet capturing:
  //   • name      — required, rendered with a leading # affordance
  //   • scope     — Personal, or one of the caller's companies (the company
  //                 picker reuses the same source as RecipientPicker:
  //                 meetings_list_memberships)
  //   • invites   — optional initial members, added one at a time via the same
  //                 RecipientPicker the roster uses
  //
  // On success the parent is handed the created Channel so it can drop it into
  // the rail (under Personal or the right company header) and open it.
  import { invoke } from '@tauri-apps/api/core';
  import RecipientPicker from './RecipientPicker.svelte';
  import type { SelectedRecipient } from '../../lib/recipientPicker';
  import type { Channel } from '../../lib/channels';

  interface Props {
    onclose: () => void;
    oncreated: (channel: Channel) => void;
    // Optional preset scope (e.g. the user clicked "+ New channel" under a
    // company header). `companyUid` null → personal.
    presetCompanyUid?: string | null;
  }

  let { onclose, oncreated, presetCompanyUid = null }: Props = $props();

  interface MembershipRow {
    companyUid: string;
    companyName: string | null;
    role: string | null;
    status: string;
  }

  // "personal" or a companyUid.
  let scopeValue = $state<string>(presetCompanyUid ?? 'personal');
  let name = $state('');
  let companies = $state<{ companyUid: string; companyName: string | null }[]>([]);

  // Initial invites — accumulated as the user picks them.
  let invitePick = $state<SelectedRecipient | null>(null);
  let invites = $state<SelectedRecipient[]>([]);

  let creating = $state(false);
  let createError = $state<string | null>(null);

  const isPersonal = $derived(scopeValue === 'personal');

  const canCreate = $derived(name.trim().length > 0 && !creating);

  async function loadCompanies(): Promise<void> {
    try {
      const list = await invoke<MembershipRow[]>('meetings_list_memberships');
      companies = (list ?? [])
        .filter((m) => m.status === 'active')
        .map((m) => ({ companyUid: m.companyUid, companyName: m.companyName }));
    } catch (err) {
      console.error('create-channel: meetings_list_memberships failed', err);
      companies = [];
    }
  }

  function addInvite(r: SelectedRecipient | null): void {
    if (!r) return;
    const uid = r.personUid?.trim();
    if (!uid) return; // Only HQ-account people can be channel members.
    if (invites.some((i) => i.personUid === uid)) {
      invitePick = null;
      return;
    }
    invites = [...invites, r];
    invitePick = null;
  }

  function removeInvite(uid: string | undefined): void {
    invites = invites.filter((i) => i.personUid !== uid);
  }

  function inviteLabel(r: SelectedRecipient): string {
    return r.displayName?.trim() || r.email;
  }

  async function create(): Promise<void> {
    const trimmed = name.trim();
    if (!trimmed || creating) return;
    creating = true;
    createError = null;
    try {
      const channel = await invoke<Channel>('create_channel', {
        name: trimmed,
        scope: isPersonal ? 'personal' : 'company',
        companyUid: isPersonal ? null : scopeValue,
        invite: invites.map((i) => i.personUid).filter((u): u is string => !!u),
      });
      oncreated(channel);
    } catch (err) {
      createError = typeof err === 'string' ? err : 'Could not create the channel';
      console.error('create-channel: create_channel failed', err);
    } finally {
      creating = false;
    }
  }

  function onNameKeydown(e: KeyboardEvent): void {
    if ((e.metaKey || e.ctrlKey) && e.key === 'Enter') {
      e.preventDefault();
      if (canCreate) void create();
    }
  }

  function onBackdropKeydown(e: KeyboardEvent): void {
    if (e.key === 'Escape') onclose();
  }

  $effect(() => {
    void loadCompanies();
  });
</script>

<!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
<div class="create-backdrop" onclick={onclose} onkeydown={onBackdropKeydown} role="presentation">
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div
    class="create-sheet"
    role="dialog"
    aria-modal="true"
    aria-label="New channel"
    tabindex="-1"
    onclick={(e) => e.stopPropagation()}
    onkeydown={onBackdropKeydown}
  >
    <header class="create-header">
      <h2>New channel</h2>
      <button class="create-close" type="button" onclick={onclose} aria-label="Close">×</button>
    </header>

    <div class="create-field">
      <span class="create-label" id="channel-name-label">Name</span>
      <div class="name-input-wrap">
        <span class="name-hash" aria-hidden="true">#</span>
        <input
          class="name-input"
          type="text"
          bind:value={name}
          onkeydown={onNameKeydown}
          placeholder="general"
          aria-labelledby="channel-name-label"
          autocomplete="off"
          spellcheck="false"
        />
      </div>
    </div>

    <div class="create-field">
      <span class="create-label" id="channel-scope-label">Scope</span>
      <select class="scope-select" bind:value={scopeValue} aria-labelledby="channel-scope-label">
        <option value="personal">Personal</option>
        {#each companies as co (co.companyUid)}
          <option value={co.companyUid}>{co.companyName?.trim() || co.companyUid}</option>
        {/each}
      </select>
      <p class="scope-hint">
        {isPersonal
          ? 'A personal channel — only people you invite can see it.'
          : 'A company channel — discoverable by your teammates.'}
      </p>
    </div>

    <div class="create-field">
      <span class="create-label">Invite people <span class="optional">(optional)</span></span>
      <RecipientPicker
        bind:selected={invitePick}
        onselect={(r) => addInvite(r)}
        placeholder="Add someone…"
      />
      {#if invites.length > 0}
        <ul class="invite-chips">
          {#each invites as i (i.personUid)}
            <li class="invite-chip">
              <span class="invite-chip-name">{inviteLabel(i)}</span>
              <button
                class="invite-chip-remove"
                type="button"
                onclick={() => removeInvite(i.personUid)}
                aria-label={`Remove ${inviteLabel(i)}`}
              >×</button>
            </li>
          {/each}
        </ul>
      {/if}
    </div>

    <div class="create-footer">
      {#if createError}
        <span class="create-error" role="alert">{createError}</span>
      {:else}
        <span class="create-hint">⌘↵ to create</span>
      {/if}
      <button class="btn btn-send" type="button" onclick={create} disabled={!canCreate}>
        {creating ? 'Creating…' : 'Create channel'}
      </button>
    </div>
  </div>
</div>

<style>
  .create-backdrop {
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

  .create-sheet {
    width: 100%;
    max-width: 460px;
    display: flex;
    flex-direction: column;
    gap: 0.875rem;
    padding: 1.125rem 1.25rem 1.25rem;
    border-radius: 14px;
    border: 1px solid var(--popover-divider, rgba(255, 255, 255, 0.1));
    background: var(--popover-bg, #1a1a22);
    backdrop-filter: var(--popover-blur, blur(28px) saturate(1.45));
    -webkit-backdrop-filter: var(--popover-blur, blur(28px) saturate(1.45));
    box-shadow: 0 24px 64px rgba(0, 0, 0, 0.55);
    color: var(--popover-text, rgba(255, 255, 255, 0.86));
  }

  .create-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }

  .create-header h2 {
    margin: 0;
    font-size: 0.9375rem;
    font-weight: 600;
    color: var(--popover-text-heading, #ffffff);
  }

  .create-close {
    border: none;
    background: transparent;
    color: var(--popover-text-muted, #a0a0b0);
    font-size: 1.25rem;
    line-height: 1;
    cursor: pointer;
    padding: 0 0.25rem;
    border-radius: 6px;
  }

  .create-close:hover {
    background: rgba(255, 255, 255, 0.08);
    color: var(--popover-text, #e8e8ee);
  }

  .create-field {
    display: flex;
    flex-direction: column;
    gap: 0.3125rem;
  }

  .create-label {
    font-size: 0.625rem;
    font-weight: 600;
    letter-spacing: 0.04em;
    text-transform: uppercase;
    color: var(--popover-text-muted, #8a8a98);
  }

  .optional {
    font-weight: 500;
    text-transform: none;
    letter-spacing: 0;
    color: var(--popover-text-muted, #6f6f7c);
  }

  .name-input-wrap {
    display: flex;
    align-items: center;
    gap: 0.25rem;
    border-radius: 8px;
    border: 1px solid rgba(255, 255, 255, 0.1);
    background: rgba(255, 255, 255, 0.04);
    padding: 0 0.625rem;
  }

  .name-input-wrap:focus-within {
    border-color: rgba(255, 255, 255, 0.28);
    background: rgba(255, 255, 255, 0.06);
  }

  .name-hash {
    color: var(--popover-text-muted, #8a8a98);
    font-size: 0.875rem;
    font-weight: 600;
  }

  .name-input {
    flex: 1;
    border: none;
    background: transparent;
    color: var(--popover-text, #e0e0e0);
    font-family: inherit;
    font-size: 0.8125rem;
    line-height: 1.4;
    padding: 0.5rem 0;
  }

  .name-input:focus {
    outline: none;
  }

  .scope-select {
    width: 100%;
    box-sizing: border-box;
    padding: 0.5rem 0.625rem;
    border-radius: 8px;
    border: 1px solid rgba(255, 255, 255, 0.1);
    background: rgba(255, 255, 255, 0.04);
    color: var(--popover-text, #e0e0e0);
    font-family: inherit;
    font-size: 0.8125rem;
  }

  .scope-select:focus {
    outline: none;
    border-color: rgba(255, 255, 255, 0.28);
  }

  .scope-hint {
    margin: 0;
    font-size: 0.6875rem;
    color: var(--popover-text-muted, #8a8a98);
  }

  .invite-chips {
    list-style: none;
    margin: 0.125rem 0 0;
    padding: 0;
    display: flex;
    flex-wrap: wrap;
    gap: 0.375rem;
  }

  .invite-chip {
    display: inline-flex;
    align-items: center;
    gap: 0.25rem;
    padding: 0.1875rem 0.375rem 0.1875rem 0.5rem;
    border-radius: 999px;
    background: rgba(120, 170, 255, 0.16);
    border: 1px solid rgba(120, 170, 255, 0.28);
  }

  .invite-chip-name {
    font-size: 0.6875rem;
    color: #dce8ff;
  }

  .invite-chip-remove {
    border: none;
    background: transparent;
    color: #dce8ff;
    font-size: 0.875rem;
    line-height: 1;
    cursor: pointer;
    padding: 0 0.125rem;
  }

  .invite-chip-remove:hover {
    color: #ffffff;
  }

  .create-footer {
    display: flex;
    align-items: center;
    gap: 0.75rem;
    margin-top: 0.25rem;
  }

  .create-hint {
    font-size: 0.6875rem;
    color: var(--popover-text-muted, #a0a0b0);
  }

  .create-error {
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
