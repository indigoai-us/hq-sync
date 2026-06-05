<script lang="ts">
  // Channel conversation pane (US-018). Renders one channel's thread + composer
  // by REUSING the shared <Conversation showAuthors={true}/> (channels are
  // multi-party, so author names show above incoming messages). The header
  // shows the channel #name, a scope chip (personal glyph vs company name), and
  // a member-count button that opens <ChannelRoster/>.
  //
  // If the caller is invited-but-not-joined, the composer is replaced by a join
  // CTA: joining (join_channel) flips membership to "joined" and the composer
  // appears. The pane owns its own message fetch, send, mark-read, and the
  // live `channel:new-message` refresh for the channel it's showing.
  import { invoke } from '@tauri-apps/api/core';
  import { listen } from '@tauri-apps/api/event';
  import Conversation, { type ConversationMessage } from './Conversation.svelte';
  import ChannelRoster from './ChannelRoster.svelte';
  import {
    type Channel,
    channelDisplayName,
    scopeChipLabel,
    isInvitedNotJoined,
  } from '../../lib/channels';

  interface Props {
    channel: Channel;
    // The caller's own personUid — passed through to the roster so it can
    // suppress a self-remove button.
    selfPersonUid?: string | null;
    // Bubbled up so the parent (MessagesShell) can clear the rail unread + the
    // channel's metadata when membership/member-count changes here.
    onchannelchange?: (channel: Channel) => void;
    onread?: (channelId: string) => void;
    // Threads (US-022). Forwarded to <Conversation/> so a root message's
    // reply-count affordance opens the ThreadPanel in MessagesShell. Called with
    // the root message's eventId; the parent supplies this channel's id as the
    // thread scope. `activeRootEventId` highlights the open thread's root bubble.
    onopenthread?: (rootEventId: string) => void;
    activeRootEventId?: string | null;
  }

  let {
    channel,
    selfPersonUid = null,
    onchannelchange,
    onread,
    onopenthread,
    activeRootEventId = null,
  }: Props = $props();

  interface ChannelMessageRow extends ConversationMessage {
    fromEmail?: string;
  }

  interface ChannelDetail {
    channel: Channel;
    messages: ChannelMessageRow[];
    nextCursor?: string | null;
  }

  // Local mutable copy of the channel metadata (membership/member-count can
  // change in place via join/roster actions).
  let current = $state<Channel>(channel);
  let messages = $state<ChannelMessageRow[]>([]);
  let loading = $state(false);
  let threadError = $state<string | null>(null);

  let sending = $state(false);
  let sendError = $state<string | null>(null);

  let joining = $state(false);
  let joinError = $state<string | null>(null);

  let rosterOpen = $state(false);
  let memberCount = $state<number | null>(channel.memberCount ?? null);

  const title = $derived(channelDisplayName(current));
  const chip = $derived(scopeChipLabel(current));
  const isPersonal = $derived(current.scope === 'personal');
  const invited = $derived(isInvitedNotJoined(current));

  // Owner determination: the creator is the channel owner. The Channel wire
  // shape doesn't carry the caller's role, so the roster (which lists per-member
  // roles) is the source of truth — it resolves the caller's own role against
  // `selfPersonUid` and only shows the owner-only remove/invite affordances when
  // the caller IS the owner. ChannelView simply hands the roster `selfPersonUid`
  // and lets it decide; the server also rejects a non-owner's remove/invite POST
  // as defense-in-depth.

  async function load(): Promise<void> {
    loading = true;
    threadError = null;
    sendError = null;
    try {
      const detail = await invoke<ChannelDetail>('fetch_channel', {
        channelId: current.channelId,
      });
      // Server returns newest-first; render oldest → newest.
      messages = [...(detail.messages ?? [])].reverse();
      if (detail.channel) {
        current = { ...current, ...detail.channel };
        memberCount = current.memberCount ?? memberCount;
        onchannelchange?.(current);
      }
      // Opening a joined channel marks it read.
      if (!invited) void markRead();
    } catch (err) {
      threadError = typeof err === 'string' ? err : 'Could not load this channel';
      messages = [];
      console.error('channel-view: fetch_channel failed', err);
    } finally {
      loading = false;
    }
  }

  async function markRead(): Promise<void> {
    try {
      await invoke('mark_channel_read', { channelId: current.channelId });
      onread?.(current.channelId);
    } catch (err) {
      // Non-fatal — the unread will reconcile on the next poll.
      console.error('channel-view: mark_channel_read failed', err);
    }
  }

  async function send(text: string): Promise<void> {
    if (!text || sending) return;
    sending = true;
    sendError = null;
    try {
      await invoke('send_channel_message', { channelId: current.channelId, body: text });
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
      sendError = typeof err === 'string' ? err : 'Failed to send message';
      console.error('channel-view: send_channel_message failed', err);
    } finally {
      sending = false;
    }
  }

  async function join(): Promise<void> {
    if (joining) return;
    joining = true;
    joinError = null;
    try {
      const updated = await invoke<Channel>('join_channel', { channelId: current.channelId });
      current = { ...current, ...updated, membership: updated.membership ?? 'joined' };
      onchannelchange?.(current);
      // Now a member — load the thread + mark read.
      await load();
    } catch (err) {
      joinError = typeof err === 'string' ? err : 'Could not join this channel';
      console.error('channel-view: join_channel failed', err);
    } finally {
      joining = false;
    }
  }

  function handleRosterCount(count: number): void {
    memberCount = count;
    current = { ...current, memberCount: count };
    onchannelchange?.(current);
  }

  // Reload when the selected channel changes (parent swaps `channel`).
  $effect(() => {
    // Touch channelId so the effect re-runs on selection change.
    const id = channel.channelId;
    current = channel;
    memberCount = channel.memberCount ?? null;
    void id;
    void load();
  });

  // Live refresh: a `channel:new-message` for THIS channel reloads the thread
  // (and re-marks read since the user is looking at it). Other channels are
  // handled by the parent list. `channel:updated` for this channel patches the
  // local metadata.
  $effect(() => {
    const unlisteners: Array<() => void> = [];
    listen<{ channelId: string; unread?: number }>('channel:new-message', (e) => {
      if (e.payload.channelId === current.channelId) {
        void load();
      }
    }).then((fn) => unlisteners.push(fn));
    listen<Channel>('channel:updated', (e) => {
      if (e.payload.channelId === current.channelId) {
        current = { ...current, ...e.payload };
        memberCount = current.memberCount ?? memberCount;
        onchannelchange?.(current);
      }
    }).then((fn) => unlisteners.push(fn));
    return () => {
      for (const fn of unlisteners) fn();
    };
  });
</script>

<header class="channel-header" data-tauri-drag-region>
  <div class="channel-title">
    <span class="channel-hash" aria-hidden="true">#</span>
    <h2>{title}</h2>
    <span class="scope-chip" class:personal={isPersonal} title={`Scope: ${chip}`}>
      {#if isPersonal}
        <span class="scope-glyph" aria-hidden="true">◐</span>
      {/if}
      {chip}
    </span>
  </div>
  <button
    class="member-count-btn"
    type="button"
    onclick={() => (rosterOpen = true)}
    title="View members"
    aria-label="View members"
  >
    {#if memberCount != null}
      {memberCount} {memberCount === 1 ? 'member' : 'members'}
    {:else}
      Members
    {/if}
  </button>
</header>

{#if invited}
  <Conversation
    {messages}
    showAuthors={true}
    {loading}
    error={threadError}
    sending={false}
    sendError={null}
    placeholder=""
    onsend={() => {}}
  />
  <div class="join-cta">
    <p class="join-text">
      You've been invited to <strong>#{title}</strong>. Join to read the full
      conversation and post.
    </p>
    {#if joinError}
      <p class="join-error" role="alert">{joinError}</p>
    {/if}
    <button class="btn btn-join" type="button" onclick={join} disabled={joining}>
      {joining ? 'Joining…' : `Join #${title}`}
    </button>
  </div>
{:else}
  <Conversation
    {messages}
    showAuthors={true}
    {loading}
    error={threadError}
    {sending}
    {sendError}
    placeholder={`Message #${title}…`}
    onsend={send}
    {onopenthread}
    {activeRootEventId}
  />
{/if}

{#if rosterOpen}
  <ChannelRoster
    channelId={current.channelId}
    {selfPersonUid}
    onclose={() => (rosterOpen = false)}
    oncountchange={handleRosterCount}
  />
{/if}

<style>
  .channel-header {
    display: flex;
    align-items: center;
    gap: 0.75rem;
    padding: 1rem 1.25rem 0.75rem;
    border-bottom: 1px solid var(--popover-divider, rgba(255, 255, 255, 0.06));
    flex-shrink: 0;
  }

  .channel-title {
    display: flex;
    align-items: center;
    gap: 0.4375rem;
    min-width: 0;
  }

  .channel-hash {
    font-size: 0.9375rem;
    font-weight: 600;
    color: var(--popover-text-muted, #8a8a98);
  }

  .channel-title h2 {
    margin: 0;
    font-size: 0.9375rem;
    font-weight: 600;
    color: var(--popover-text-heading, #ffffff);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .scope-chip {
    display: inline-flex;
    align-items: center;
    gap: 0.1875rem;
    flex-shrink: 0;
    font-size: 0.625rem;
    font-weight: 600;
    letter-spacing: 0.02em;
    padding: 0.125rem 0.4375rem;
    border-radius: 999px;
    background: rgba(120, 170, 255, 0.16);
    color: #cfe0ff;
  }

  .scope-chip.personal {
    background: rgba(180, 140, 255, 0.18);
    color: #e0d0ff;
  }

  .scope-glyph {
    font-size: 0.6875rem;
    line-height: 1;
  }

  .member-count-btn {
    margin-left: auto;
    flex-shrink: 0;
    border: 1px solid rgba(255, 255, 255, 0.12);
    background: rgba(255, 255, 255, 0.05);
    color: var(--popover-text, #d8d8e0);
    font-family: inherit;
    font-size: 0.6875rem;
    font-weight: 500;
    padding: 0.25rem 0.5rem;
    border-radius: 7px;
    cursor: pointer;
    transition: background-color 0.12s ease;
  }

  .member-count-btn:hover {
    background: rgba(255, 255, 255, 0.1);
  }

  .join-cta {
    flex-shrink: 0;
    display: flex;
    flex-direction: column;
    gap: 0.625rem;
    padding: 1rem 1.25rem 1.25rem;
    border-top: 1px solid var(--popover-divider, rgba(255, 255, 255, 0.06));
  }

  .join-text {
    margin: 0;
    font-size: 0.8125rem;
    line-height: 1.5;
    color: var(--popover-text-muted, #b8b8c4);
  }

  .join-text strong {
    color: var(--popover-text, #e8e8ee);
    font-weight: 600;
  }

  .join-error {
    margin: 0;
    font-size: 0.75rem;
    color: #ff9b9b;
  }

  .btn {
    display: inline-flex;
    align-items: center;
    align-self: flex-start;
    padding: 0.4375rem 0.875rem;
    border-radius: 7px;
    font-size: 0.75rem;
    font-weight: 600;
    cursor: pointer;
    border: none;
    font-family: inherit;
    transition: background-color 0.12s ease;
  }

  .btn-join {
    background: rgba(120, 170, 255, 0.26);
    color: #dce8ff;
  }

  .btn-join:hover:not(:disabled) {
    background: rgba(120, 170, 255, 0.38);
  }

  .btn-join:disabled {
    opacity: 0.45;
    cursor: default;
  }
</style>
