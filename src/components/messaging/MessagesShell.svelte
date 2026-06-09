<script lang="ts">
  // Dedicated Messages window (US-009). A resizable master/detail shell:
  //
  //   ┌──────────────┬─────────────────────────────┐
  //   │ segmented    │                             │
  //   │ rail         │   conversation pane         │
  //   │ (DMs /       │   (<Conversation/>)         │
  //   │  Requests /  │                             │
  //   │  Channels)   │                             │
  //   └──────────────┴─────────────────────────────┘
  //
  // This story scaffolds all three segments. Direct Messages is functional:
  // it lists the caller's contacts (derived from list_contacts — connections +
  // company teammates) and, on click, loads that peer's thread into the shared
  // <Conversation> component. Requests and Channels are present but scaffolded
  // (empty/placeholder) — compose, request handling, and channels are later
  // stories.
  //
  // Visuals adopt the desktop "Company OS" design language: the standalone
  // Messages window consumes the SAME token layer as the desktop window via
  // `desktop-alt.css` (which `@import`s the canonical `popover.css` primitives
  // and adds the desktop alias layer + 13px type ramp, scoped to
  // `html[data-window='messages']` alongside `desktop-alt`). The Geist/Inter
  // faces are bundled offline here so the window renders the real type rather
  // than a system fallback. See DESIGN.md → "Big-window type & chrome".
  import '@fontsource-variable/inter/wght.css';
  import '@fontsource-variable/inter-tight/wght.css';
  import '@fontsource-variable/geist-mono/wght.css';
  import '../../desktop-alt/styles/desktop-alt.css';
  import { invoke } from '@tauri-apps/api/core';
  import { listen } from '@tauri-apps/api/event';
  import Conversation, { type ConversationMessage } from './Conversation.svelte';
  import ComposeMessage, { type ComposeSendResult } from './ComposeMessage.svelte';
  import DmRequestCard from './DmRequestCard.svelte';
  import {
    type DmRequest,
    type RequestAction,
    addRequest,
    removeRequest,
  } from '../../lib/dmRequests';

  type Segment = 'dms' | 'requests' | 'channels';

  // A person the caller can DM (connection or company teammate). Mirrors the
  // Rust `Contact` wire shape (camelCase).
  interface Contact {
    personUid: string;
    email: string;
    displayName: string;
    companyUid?: string | null;
    source?: string | null;
  }

  interface ContactsResponse {
    contacts: Contact[];
  }

  interface ThreadMessage extends ConversationMessage {
    fromEmail: string;
  }

  interface ThreadResponse {
    messages: ThreadMessage[];
    nextCursor?: string | null;
  }

  interface UnreadSummary {
    unreadDms: number;
    pendingRequests: number;
  }

  interface RequestsResponse {
    requests: DmRequest[];
    nextCursor?: string | null;
  }

  let segment = $state<Segment>('dms');

  let contacts = $state<Contact[]>([]);
  let loadingContacts = $state(false);
  let contactsError = $state<string | null>(null);

  // Pending incoming connection requests (US-011). The count drives the Requests
  // segment badge; the list renders one DmRequestCard each. `list_dm_requests`
  // is the source of truth; `dm:request-new` / `dm:request-update` keep it live.
  let requests = $state<DmRequest[]>([]);
  let loadingRequests = $state(false);
  let requestsError = $state<string | null>(null);
  // Derived count — the segment badge stays in lockstep with the rendered list.
  let pendingRequests = $derived(requests.length);

  // Selected peer + its loaded thread.
  let selected = $state<Contact | null>(null);
  let messages = $state<ThreadMessage[]>([]);
  let loadingThread = $state(false);
  let threadError = $state<string | null>(null);

  let sending = $state(false);
  let sendError = $state<string | null>(null);

  // New Message compose overlay (US-010).
  let composing = $state(false);

  function openCompose(): void {
    composing = true;
  }

  // Handle a successful compose send. On a connection-requested (202) result the
  // message is rendered optimistically as a Pending bubble and the right pane
  // switches to that pending conversation; on a delivered (200) result we open
  // the normal thread for the recipient. The `dm:request-update` event that
  // flips Pending→active is consumed in US-011 — here we only render the Pending
  // state from the send response.
  function handleComposeSent(result: ComposeSendResult): void {
    composing = false;
    const r = result.recipient;
    const peer: Contact = {
      personUid: r.personUid ?? `email:${r.email}`,
      email: r.email,
      displayName: r.displayName ?? r.email,
      companyUid: null,
      source: null,
    };
    segment = 'dms';
    selected = peer;
    threadError = null;
    sendError = null;

    if (result.pending) {
      // 202 — held behind a connection request. Render the just-sent message as
      // a Pending bubble; do NOT load a thread (there isn't one yet).
      loadingThread = false;
      messages = [
        {
          eventId: `pending-${Date.now()}`,
          fromPersonUid: 'me',
          fromEmail: '',
          fromDisplayName: 'You',
          body: result.body,
          details: null,
          prompt: null,
          createdAt: new Date().toISOString(),
          direction: 'out',
          pending: true,
          pendingLabel: `Pending — waiting for ${displayLabel(peer)} to accept`,
        },
      ];
    } else {
      // 200 — delivered to an active connection. Open the normal thread (if the
      // recipient resolved to a real personUid); otherwise show the optimistic
      // message until the next poll.
      if (r.personUid) {
        void selectContact(peer);
      } else {
        loadingThread = false;
        messages = [
          {
            eventId: `local-${Date.now()}`,
            fromPersonUid: 'me',
            fromEmail: '',
            fromDisplayName: 'You',
            body: result.body,
            details: null,
            prompt: null,
            createdAt: new Date().toISOString(),
            direction: 'out',
          },
        ];
      }
    }
    // A brand-new conversation may now exist server-side; refresh the rail.
    void loadContacts();
  }

  function displayLabel(c: Contact): string {
    return c.displayName?.trim() || c.email?.trim() || c.personUid;
  }

  function initials(c: Contact): string {
    const name = displayLabel(c);
    const parts = name.split(/\s+/).filter(Boolean);
    if (parts.length >= 2) return (parts[0][0] + parts[1][0]).toUpperCase();
    return name.slice(0, 2).toUpperCase();
  }

  async function loadContacts(): Promise<void> {
    loadingContacts = true;
    contactsError = null;
    try {
      const resp = await invoke<ContactsResponse>('list_contacts');
      contacts = resp.contacts ?? [];
    } catch (err) {
      contactsError = typeof err === 'string' ? err : 'Could not load conversations';
      contacts = [];
      console.error('messages: list_contacts failed', err);
    } finally {
      loadingContacts = false;
    }
  }

  async function loadUnreadSummary(): Promise<void> {
    try {
      // Kept for parity with the popover summary; the authoritative request
      // count now comes from `loadRequests` (the rendered list). We still read
      // the summary so any future unread surface stays wired.
      await invoke<UnreadSummary>('get_unread_summary');
    } catch (err) {
      // Non-fatal — the rail still renders.
      console.error('messages: get_unread_summary failed', err);
    }
  }

  async function loadRequests(): Promise<void> {
    loadingRequests = true;
    requestsError = null;
    try {
      const resp = await invoke<RequestsResponse>('list_dm_requests');
      requests = resp.requests ?? [];
    } catch (err) {
      requestsError =
        typeof err === 'string' ? err : 'Could not load connection requests';
      requests = [];
      console.error('messages: list_dm_requests failed', err);
    } finally {
      loadingRequests = false;
    }
  }

  // A request card resolved (Accept / Decline / Block succeeded). Prune it from
  // the list (the count badge follows via the derived `pendingRequests`). On
  // Accept, the held first message becomes a live thread — swap to the DMs
  // segment and open the standard <Conversation> with the requester so the card
  // is replaced by the thread, satisfying "the held message becomes a thread".
  function handleRequestResolved(req: DmRequest, action: RequestAction): void {
    requests = removeRequest(requests, req.pairKey);
    if (action === 'accept') {
      const peer: Contact = {
        personUid: req.fromPersonUid,
        email: req.fromEmail,
        displayName: req.fromDisplayName,
        companyUid: null,
        source: 'request',
      };
      segment = 'dms';
      void selectContact(peer);
      // The new connection now appears as a contact — refresh the rail.
      void loadContacts();
    }
  }

  async function selectContact(c: Contact): Promise<void> {
    selected = c;
    messages = [];
    threadError = null;
    sendError = null;
    loadingThread = true;
    try {
      const resp = await invoke<ThreadResponse>('fetch_dm_thread', {
        withPersonUid: c.personUid,
      });
      // Server returns newest-first; render chronologically (oldest → newest).
      messages = [...(resp.messages ?? [])].reverse();
    } catch (err) {
      threadError = typeof err === 'string' ? err : 'Could not load this conversation';
      messages = [];
      console.error('messages: fetch_dm_thread failed', err);
    } finally {
      loadingThread = false;
    }
  }

  async function sendReply(text: string): Promise<void> {
    if (!text || sending || !selected) return;
    sending = true;
    sendError = null;
    try {
      await invoke('send_dm', { toPersonUid: selected.personUid, body: text });
      // Optimistic append — the durable copy lands in the mirror and shows on
      // the next thread load.
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
      console.error('messages: send_dm failed', err);
    } finally {
      sending = false;
    }
  }

  $effect(() => {
    const unlisteners: Array<() => void> = [];

    // A new DM may arrive while this window is open — refresh the contact list
    // (so a brand-new conversation appears) and the request count. The badge
    // reset is handled in Rust on messages_window_ready.
    listen('dm:new-events', () => {
      void loadContacts();
      void loadUnreadSummary();
    }).then((fn) => unlisteners.push(fn));

    // A brand-new incoming connection request landed (US-011) — append it to the
    // Requests list (the segment badge follows via the derived count). Dedupe by
    // pairKey so a re-emit doesn't double-add.
    listen<DmRequest>('dm:request-new', (e) => {
      requests = addRequest(requests, e.payload);
    }).then((fn) => unlisteners.push(fn));

    // A pending request resolved elsewhere (accepted/declined/blocked, or pruned
    // by the poll diff). Drop it from the Requests list; the count badge follows.
    listen<{ pairKey: string; state?: string }>('dm:request-update', (e) => {
      requests = removeRequest(requests, e.payload.pairKey);
    }).then((fn) => unlisteners.push(fn));

    // Ready-handshake: tell Rust the listeners are mounted so it shows + focuses
    // the window and resets the unread badge (mirrors DmDetail).
    void loadContacts();
    void loadRequests();
    void loadUnreadSummary();
    invoke('messages_window_ready');

    return () => {
      for (const fn of unlisteners) fn();
    };
  });
</script>

<div class="messages-window">
  <aside class="rail">
    <header class="rail-header" data-tauri-drag-region>
      <h1>Messages</h1>
      <button
        class="new-message-btn"
        type="button"
        onclick={openCompose}
        title="New message"
        aria-label="New message"
      >
        + New message
      </button>
    </header>

    <nav class="segments" aria-label="Message segments">
      <button
        class="segment"
        class:active={segment === 'dms'}
        type="button"
        onclick={() => (segment = 'dms')}
      >
        Direct Messages
      </button>
      <button
        class="segment"
        class:active={segment === 'requests'}
        type="button"
        onclick={() => (segment = 'requests')}
      >
        Requests
        {#if pendingRequests > 0}
          <span class="segment-badge">{pendingRequests}</span>
        {/if}
      </button>
      <button
        class="segment"
        class:active={segment === 'channels'}
        type="button"
        onclick={() => (segment = 'channels')}
      >
        Channels
      </button>
    </nav>

    <div class="rail-body">
      {#if segment === 'dms'}
        {#if loadingContacts}
          <p class="rail-status">Loading conversations…</p>
        {:else if contactsError}
          <p class="rail-status rail-error" role="alert">{contactsError}</p>
        {:else if contacts.length === 0}
          <p class="rail-status">No conversations yet.</p>
        {:else}
          <ul class="contact-list">
            {#each contacts as c (c.personUid)}
              <li>
                <button
                  class="contact-row"
                  class:active={selected?.personUid === c.personUid}
                  type="button"
                  onclick={() => selectContact(c)}
                >
                  <span class="contact-avatar" aria-hidden="true">{initials(c)}</span>
                  <span class="contact-meta">
                    <span class="contact-name">{displayLabel(c)}</span>
                    {#if c.email}
                      <span class="contact-sub">{c.email}</span>
                    {/if}
                  </span>
                </button>
              </li>
            {/each}
          </ul>
        {/if}
      {:else if segment === 'requests'}
        {#if loadingRequests}
          <p class="rail-status">Loading requests…</p>
        {:else if requestsError}
          <p class="rail-status rail-error" role="alert">{requestsError}</p>
        {:else if requests.length === 0}
          <div class="segment-empty">
            <p class="segment-empty-title">No pending requests</p>
            <p class="segment-empty-sub">
              Connection requests will appear here once someone outside your team
              asks to message you.
            </p>
          </div>
        {:else}
          <ul class="request-list">
            {#each requests as req (req.pairKey)}
              <li>
                <DmRequestCard request={req} onresolved={handleRequestResolved} />
              </li>
            {/each}
          </ul>
        {/if}
      {:else}
        <!-- Scaffold: channels are a later story. -->
        <div class="segment-empty">
          <p class="segment-empty-title">Channels</p>
          <p class="segment-empty-sub">Coming soon.</p>
        </div>
      {/if}
    </div>
  </aside>

  <section class="pane">
    {#if segment !== 'dms'}
      <div class="pane-empty">
        <p>
          {segment === 'requests'
            ? 'Review connection requests on the left — accept, decline, or block each one.'
            : 'Channels are coming soon.'}
        </p>
      </div>
    {:else if !selected}
      <div class="pane-empty">
        <p>Select a conversation to start messaging.</p>
      </div>
    {:else}
      <header class="pane-header" data-tauri-drag-region>
        <h2>{displayLabel(selected)}</h2>
        {#if selected.email}
          <span class="pane-sub">{selected.email}</span>
        {/if}
      </header>
      <Conversation
        {messages}
        showAuthors={false}
        loading={loadingThread}
        error={threadError}
        {sending}
        {sendError}
        placeholder={`Message ${displayLabel(selected)}…`}
        onsend={sendReply}
      />
    {/if}
  </section>

  {#if composing}
    <ComposeMessage onclose={() => (composing = false)} onsent={handleComposeSent} />
  {/if}
</div>

<style>
  /* The Messages window adopts the desktop "Company OS" language: monochrome
     glass surfaces layered by alpha, ONE 13px type size with 11px monospace
     ALL-CAPS micro-labels, hierarchy by weight + the grey/white split, hairline
     borders, square-ish corners, and the Indigo accent reserved for the
     active/selected row + focus ring only. Tokens come from the shared desktop
     alias layer (desktop-alt.css, scoped to data-window='messages'). */

  .messages-window {
    display: flex;
    width: 100vw;
    height: 100vh;
    box-sizing: border-box;
    background: var(--bg-gradient);
    color: var(--fg);
    font-family: var(--font-sans);
    font-size: var(--text-base);
    letter-spacing: -0.006em;
    overflow: hidden;
  }

  /* ── Left rail ──────────────────────────────────────────────────────── */

  .rail {
    width: 240px;
    flex-shrink: 0;
    display: flex;
    flex-direction: column;
    border-right: 1px solid var(--border);
    background: var(--surface-rail);
    min-height: 0;
  }

  .rail-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-2);
    padding: var(--space-4) var(--space-4) var(--space-3);
    flex-shrink: 0;
  }

  .rail-header h1 {
    margin: 0;
    font-family: var(--font-display);
    font-size: var(--text-base);
    font-weight: 600;
    letter-spacing: -0.01em;
    color: var(--fg);
  }

  .new-message-btn {
    flex-shrink: 0;
    border: 1px solid var(--border-strong);
    background: var(--surface-raise);
    color: var(--fg);
    font-family: var(--font-sans);
    font-size: var(--text-sm);
    font-weight: 500;
    padding: var(--space-1) var(--space-2);
    border-radius: var(--radius-sm);
    cursor: pointer;
    transition: background-color 0.12s ease, border-color 0.12s ease;
  }

  .new-message-btn:hover {
    background: var(--row-hover);
    border-color: var(--accent);
  }

  .new-message-btn:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 1px;
  }

  .segments {
    display: flex;
    flex-direction: column;
    gap: 1px;
    padding: 0 var(--space-3) var(--space-3);
    flex-shrink: 0;
  }

  .segment {
    position: relative;
    display: flex;
    align-items: center;
    gap: var(--space-2);
    width: 100%;
    height: 30px;
    text-align: left;
    padding: 0 var(--space-2) 0 calc(var(--space-3) + 6px);
    border: none;
    border-radius: 6px;
    background: transparent;
    color: var(--muted);
    font-family: var(--font-sans);
    font-size: var(--text-base);
    font-weight: 400;
    cursor: pointer;
    transition:
      background-color 0.12s cubic-bezier(0.2, 0.7, 0.2, 1),
      color 0.12s cubic-bezier(0.2, 0.7, 0.2, 1);
  }

  .segment:hover {
    background: var(--row-hover);
    color: var(--fg);
  }

  /* Active nav cue mirrors the desktop sidebar: a restrained row-active surface
     plus a 4px Indigo dot in the left gutter — not a filled accent pill. */
  .segment.active {
    background: var(--row-active);
    color: var(--fg);
    font-weight: 500;
  }

  .segment.active::before {
    content: '';
    position: absolute;
    left: var(--space-2);
    top: 50%;
    width: 4px;
    height: 4px;
    margin-top: -2px;
    border-radius: 999px;
    background: var(--accent);
  }

  .segment:focus-visible {
    outline: 2px solid var(--blue);
    outline-offset: -2px;
  }

  /* Request count: a subtle neutral pill that marks state (count), not
     decoration. Monospace tabular numerals, no stoplight color. */
  .segment-badge {
    margin-left: auto;
    min-width: 18px;
    height: 16px;
    padding: 0 var(--space-1);
    box-sizing: border-box;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    border-radius: var(--radius-sm);
    font-family: var(--font-mono);
    font-size: var(--text-micro);
    font-weight: 600;
    line-height: 1;
    font-variant-numeric: tabular-nums;
    background: var(--surface-raise);
    color: var(--muted-2);
  }

  .rail-body {
    flex: 1;
    overflow-y: auto;
    min-height: 0;
    padding: var(--space-1) var(--space-2) var(--space-3);
  }

  .rail-status {
    margin: var(--space-2) var(--space-3);
    font-size: var(--text-base);
    color: var(--muted);
  }

  .rail-error {
    color: var(--red);
  }

  .contact-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 1px;
  }

  .contact-row {
    position: relative;
    display: flex;
    align-items: center;
    gap: var(--space-2);
    width: 100%;
    text-align: left;
    padding: var(--space-2) var(--space-2) var(--space-2) calc(var(--space-2) + 2px);
    border: none;
    border-radius: 6px;
    background: transparent;
    color: inherit;
    font-family: var(--font-sans);
    cursor: pointer;
    transition: background-color 0.12s cubic-bezier(0.2, 0.7, 0.2, 1);
  }

  .contact-row:hover {
    background: var(--row-hover);
  }

  /* Selected conversation: restrained row-active surface + a 2px Indigo edge —
     the desktop "active row" treatment, accent kept to a hairline. */
  .contact-row.active {
    background: var(--row-active);
  }

  .contact-row.active::before {
    content: '';
    position: absolute;
    left: 0;
    top: 7px;
    bottom: 7px;
    width: 2px;
    border-radius: 999px;
    background: var(--accent);
  }

  .contact-row:focus-visible {
    outline: 2px solid var(--blue);
    outline-offset: -2px;
  }

  .contact-avatar {
    flex-shrink: 0;
    width: 26px;
    height: 26px;
    border-radius: 7px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    background: var(--surface-raise);
    border: 1px solid var(--border);
    color: var(--muted-2);
    font-family: var(--font-mono);
    font-size: var(--text-micro);
    font-weight: 600;
    letter-spacing: 0.02em;
  }

  .contact-meta {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }

  .contact-name {
    font-size: var(--text-base);
    font-weight: 600;
    color: var(--fg);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .contact-sub {
    font-family: var(--font-mono);
    font-size: var(--text-micro);
    color: var(--muted);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .request-list {
    list-style: none;
    margin: 0;
    padding: var(--space-1) 0 var(--space-2);
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  .segment-empty {
    padding: var(--space-5) var(--space-3);
    text-align: center;
  }

  .segment-empty-title {
    margin: 0 0 var(--space-1);
    font-size: var(--text-base);
    font-weight: 600;
    color: var(--fg);
  }

  .segment-empty-sub {
    margin: 0;
    font-size: var(--text-base);
    line-height: 1.5;
    color: var(--muted);
  }

  /* ── Right conversation pane ────────────────────────────────────────── */

  .pane {
    flex: 1;
    display: flex;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
  }

  .pane-empty {
    flex: 1;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: var(--space-6);
  }

  .pane-empty p {
    margin: 0;
    font-size: var(--text-base);
    color: var(--muted);
    text-align: center;
  }

  .pane-header {
    display: flex;
    align-items: baseline;
    gap: var(--space-3);
    padding: var(--space-4) var(--space-5) var(--space-3);
    border-bottom: 1px solid var(--border);
    background: var(--surface-panel);
    flex-shrink: 0;
  }

  .pane-header h2 {
    margin: 0;
    font-family: var(--font-display);
    font-size: var(--text-base);
    font-weight: 600;
    letter-spacing: -0.01em;
    color: var(--fg);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .pane-sub {
    margin-left: auto;
    font-family: var(--font-mono);
    font-size: var(--text-micro);
    color: var(--muted);
    white-space: nowrap;
  }
</style>
