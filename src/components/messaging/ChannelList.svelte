<script lang="ts">
  // Channels segment rail (US-018). Renders the caller's channels grouped under
  // a "Personal" header and one header per company. Each row shows the channel
  // #name + a scope chip (a personal glyph vs the company name) and an unread
  // badge. A "+ New channel" affordance opens the create flow. Selection + the
  // create affordance are bubbled to the parent (MessagesShell); the grouping
  // logic lives in lib/channels.ts (unit-tested).
  import {
    type Channel,
    type CompanyLabel,
    channelDisplayName,
    scopeChipLabel,
    groupChannels,
    isInvitedNotJoined,
  } from '../../lib/channels';

  interface Props {
    channels: Channel[];
    companies?: CompanyLabel[];
    loading?: boolean;
    error?: string | null;
    selectedId?: string | null;
    onselect: (channel: Channel) => void;
    // Open the create flow. `companyUid` is the group the "+ New channel" was
    // clicked under (null = top-level / Personal).
    oncreate: (companyUid: string | null) => void;
  }

  let {
    channels,
    companies = [],
    loading = false,
    error = null,
    selectedId = null,
    onselect,
    oncreate,
  }: Props = $props();

  const groups = $derived(groupChannels(channels, companies));
</script>

<div class="channel-rail">
  <div class="channel-rail-top">
    <button class="new-channel-btn" type="button" onclick={() => oncreate(null)}>
      + New channel
    </button>
  </div>

  {#if loading}
    <p class="rail-status">Loading channels…</p>
  {:else if error}
    <p class="rail-status rail-error" role="alert">{error}</p>
  {:else if groups.length === 0}
    <div class="segment-empty">
      <p class="segment-empty-title">No channels yet</p>
      <p class="segment-empty-sub">
        Create a personal or company channel to start a group conversation.
      </p>
    </div>
  {:else}
    {#each groups as group (group.key)}
      <div class="channel-group">
        <div class="group-header">
          <span class="group-label">
            {#if group.scope === 'personal'}
              <span class="group-glyph" aria-hidden="true">◐</span>
            {/if}
            {group.label}
          </span>
          <button
            class="group-add"
            type="button"
            title={`New channel in ${group.label}`}
            aria-label={`New channel in ${group.label}`}
            onclick={() => oncreate(group.companyUid ?? null)}
          >+</button>
        </div>
        <ul class="channel-list">
          {#each group.channels as ch (ch.channelId)}
            <li>
              <button
                class="channel-row"
                class:active={selectedId === ch.channelId}
                type="button"
                onclick={() => onselect(ch)}
              >
                <span class="channel-hash" aria-hidden="true">#</span>
                <span class="channel-name">{channelDisplayName(ch)}</span>
                {#if isInvitedNotJoined(ch)}
                  <span class="invited-chip" title="You're invited">invited</span>
                {:else}
                  <span class="scope-chip" class:personal={ch.scope === 'personal'}>
                    {scopeChipLabel(ch)}
                  </span>
                {/if}
                {#if (ch.unread ?? 0) > 0}
                  <span class="unread-badge">{(ch.unread ?? 0) > 9 ? '9+' : ch.unread}</span>
                {/if}
              </button>
            </li>
          {/each}
        </ul>
      </div>
    {/each}
  {/if}
</div>

<style>
  .channel-rail {
    display: flex;
    flex-direction: column;
    gap: 0.25rem;
  }

  .channel-rail-top {
    padding: 0.125rem 0.375rem 0.5rem;
  }

  .new-channel-btn {
    width: 100%;
    border: 1px solid rgba(120, 170, 255, 0.32);
    background: rgba(120, 170, 255, 0.16);
    color: #dce8ff;
    font-family: inherit;
    font-size: 0.75rem;
    font-weight: 600;
    padding: 0.375rem 0.5rem;
    border-radius: 7px;
    cursor: pointer;
    transition: background-color 0.12s ease;
  }

  .new-channel-btn:hover {
    background: rgba(120, 170, 255, 0.28);
  }

  .rail-status {
    margin: 0.5rem 0.625rem;
    font-size: 0.75rem;
    color: var(--popover-text-muted, #a0a0b0);
  }

  .rail-error {
    color: #ff9b9b;
  }

  .segment-empty {
    padding: 1.25rem 0.875rem;
    text-align: center;
  }

  .segment-empty-title {
    margin: 0 0 0.375rem;
    font-size: 0.8125rem;
    font-weight: 600;
    color: var(--popover-text, #e8e8ee);
  }

  .segment-empty-sub {
    margin: 0;
    font-size: 0.75rem;
    line-height: 1.45;
    color: var(--popover-text-muted, #8a8a98);
  }

  .channel-group {
    display: flex;
    flex-direction: column;
    gap: 0.125rem;
    margin-bottom: 0.375rem;
  }

  .group-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 0.3125rem 0.625rem 0.1875rem;
  }

  .group-label {
    display: inline-flex;
    align-items: center;
    gap: 0.25rem;
    font-size: 0.625rem;
    font-weight: 600;
    letter-spacing: 0.04em;
    text-transform: uppercase;
    color: var(--popover-text-muted, #8a8a98);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .group-glyph {
    font-size: 0.6875rem;
    line-height: 1;
    color: #c0a8ff;
  }

  .group-add {
    flex-shrink: 0;
    border: none;
    background: transparent;
    color: var(--popover-text-muted, #8a8a98);
    font-size: 0.9375rem;
    line-height: 1;
    cursor: pointer;
    padding: 0 0.25rem;
    border-radius: 5px;
  }

  .group-add:hover {
    background: rgba(255, 255, 255, 0.08);
    color: var(--popover-text, #e8e8ee);
  }

  .channel-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 0.0625rem;
  }

  .channel-row {
    display: flex;
    align-items: center;
    gap: 0.3125rem;
    width: 100%;
    text-align: left;
    padding: 0.375rem 0.5rem;
    border: none;
    border-radius: 8px;
    background: transparent;
    color: inherit;
    font-family: inherit;
    cursor: pointer;
    transition: background-color 0.12s ease;
  }

  .channel-row:hover {
    background: rgba(255, 255, 255, 0.05);
  }

  .channel-row.active {
    background: rgba(120, 170, 255, 0.16);
  }

  .channel-hash {
    flex-shrink: 0;
    font-size: 0.8125rem;
    font-weight: 600;
    color: var(--popover-text-muted, #8a8a98);
  }

  .channel-name {
    font-size: 0.8125rem;
    font-weight: 500;
    color: var(--popover-text, #e8e8ee);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-width: 0;
  }

  .scope-chip {
    flex-shrink: 0;
    margin-left: auto;
    font-size: 0.5625rem;
    font-weight: 600;
    letter-spacing: 0.02em;
    padding: 0.0625rem 0.375rem;
    border-radius: 999px;
    background: rgba(120, 170, 255, 0.16);
    color: #cfe0ff;
    max-width: 6.5rem;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .scope-chip.personal {
    background: rgba(180, 140, 255, 0.18);
    color: #e0d0ff;
  }

  .invited-chip {
    flex-shrink: 0;
    margin-left: auto;
    font-size: 0.5625rem;
    font-weight: 600;
    letter-spacing: 0.02em;
    text-transform: uppercase;
    padding: 0.0625rem 0.375rem;
    border-radius: 999px;
    background: rgba(255, 176, 102, 0.22);
    color: #ffd9b0;
  }

  .unread-badge {
    flex-shrink: 0;
    min-width: 1.0625rem;
    height: 1.0625rem;
    padding: 0 0.25rem;
    box-sizing: border-box;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    border-radius: 999px;
    font-size: 0.5625rem;
    font-weight: 700;
    line-height: 1;
    background: rgba(120, 170, 255, 0.32);
    color: #eaf2ff;
  }
</style>
