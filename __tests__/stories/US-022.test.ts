import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { describe, expect, it } from 'vitest';

// US-022 "Threads UI" is wired across the shared Conversation primitive, a new
// ThreadPanel, the MessagesShell surface (DM + channel panes), the Tauri thread
// commands in dm_notify.rs, their registration in main.rs, and the messages
// window capability. These are source-contract assertions (mirroring the other
// US-* story tests) so capability drift or a dropped wire fails fast.

const read = (p: string) => readFileSync(resolve(process.cwd(), p), 'utf8');
const normalize = (s: string) => s.replace(/\s+/g, ' ');

const conversation = read('src/components/messaging/Conversation.svelte');
const threadPanel = read('src/components/messaging/ThreadPanel.svelte');
const shell = read('src/components/messaging/MessagesShell.svelte');
const channelView = read('src/components/messaging/ChannelView.svelte');
const dmNotify = read('src-tauri/src/commands/dm_notify.rs');
const mainRs = read('src-tauri/src/main.rs');
const capability = read('src-tauri/capabilities/messages.json');

describe('US-022: reply-count affordance in Conversation', () => {
  it('renders a tap-visible {n} replies affordance under root bubbles with replies', () => {
    const c = normalize(conversation);
    // The affordance is a real button (tap-visible, not hover-gated) gated on
    // hasReplies (rootEventId + replyCount > 0).
    expect(c).toContain('function hasReplies(msg: ConversationMessage): boolean');
    expect(c).toContain('return !!msg.rootEventId && (msg.replyCount ?? 0) > 0;');
    expect(c).toContain('{#if hasReplies(msg)}');
    expect(c).toContain('class="thread-affordance"');
    expect(c).toContain('onclick={() => openThread(msg.rootEventId)}');
    // It shows the count and a "last {time}" relative stamp.
    expect(c).toContain("{(msg.replyCount ?? 0) === 1 ? 'reply' : 'replies'}");
    expect(c).toContain('· last {formatRelative(msg.lastReplyAt)}');
  });

  it('clicking the affordance calls onopenthread with the rootEventId', () => {
    const c = normalize(conversation);
    expect(c).toContain('onopenthread?: (rootEventId: string) => void;');
    expect(c).toContain('function openThread(rootEventId: string | null | undefined): void');
    expect(c).toContain('if (id) onopenthread?.(id);');
  });

  it('highlights the active thread root bubble', () => {
    const c = normalize(conversation);
    expect(c).toContain('activeRootEventId?: string | null;');
    expect(c).toContain(
      "class:dm-bubble-thread-active={!!activeRootEventId && msg.rootEventId === activeRootEventId}",
    );
    expect(c).toContain('.dm-bubble-thread-active {');
  });

  it('extends ConversationMessage with rootEventId, replyCount, lastReplyAt', () => {
    const c = normalize(conversation);
    expect(c).toContain('rootEventId?: string | null;');
    expect(c).toContain('replyCount?: number | null;');
    expect(c).toContain('lastReplyAt?: string | null;');
  });
});

describe('US-022: ThreadPanel right-side panel', () => {
  it('pins the root, reuses Conversation for replies, and has its own composer', () => {
    const p = normalize(threadPanel);
    expect(p).toContain('class="thread-root"');
    expect(p).toContain('<p class="thread-root-body">{root.body}</p>');
    // Replies + composer reuse the shared Conversation.
    expect(p).toContain('<Conversation');
    expect(p).toContain('messages={replies}');
    expect(p).toContain('onsend={sendReply}');
    // Close/back affordance returns to the main conversation.
    expect(p).toContain('onclick={onclose}');
  });

  it('loads via fetch_thread and posts replies with rootEventId set', () => {
    const p = normalize(threadPanel);
    expect(p).toContain("invoke<ThreadView>('fetch_thread'");
    expect(p).toContain("invoke('send_thread_reply'");
    expect(p).toContain('rootEventId,');
  });

  it('registers/clears the open thread with set_active_thread', () => {
    const p = normalize(threadPanel);
    expect(p).toContain("invoke('set_active_thread'");
    expect(p).toContain("invoke('set_active_thread', { rootEventId: null });");
  });

  it('updates live on thread:new-reply (append + bump count)', () => {
    const p = normalize(threadPanel);
    expect(p).toContain("'thread:new-reply'");
    expect(p).toContain('if (e.payload.rootEventId !== rootEventId) return;');
    expect(p).toContain('appendReply(e.payload.reply);');
    expect(p).toContain('onreplycount?.(rootEventId, replyCount);');
  });
});

describe('US-022: MessagesShell wires onopenthread for DM + channel panes', () => {
  it('opens ThreadPanel for a DM root via the conversation pane', () => {
    const s = normalize(shell);
    expect(s).toContain("import ThreadPanel from './ThreadPanel.svelte';");
    expect(s).toContain('function handleOpenDmThread(rootEventId: string): void');
    expect(s).toContain("scope: 'dm',");
    expect(s).toContain('onopenthread={handleOpenDmThread}');
  });

  it('opens ThreadPanel for a channel root via ChannelView', () => {
    const s = normalize(shell);
    const cv = normalize(channelView);
    expect(s).toContain('function handleOpenChannelThread(rootEventId: string): void');
    expect(s).toContain("scope: 'channel',");
    expect(s).toContain('onopenthread={handleOpenChannelThread}');
    // ChannelView forwards onopenthread + activeRootEventId to its Conversation.
    expect(cv).toContain('onopenthread?: (rootEventId: string) => void;');
    expect(cv).toContain('{onopenthread}');
    expect(cv).toContain('{activeRootEventId}');
  });

  it('renders the ThreadPanel as a right-side column/overlay and bumps live count', () => {
    const s = normalize(shell);
    expect(s).toContain('{#if openThread}');
    expect(s).toContain('class="thread-column"');
    expect(s).toContain('<ThreadPanel');
    expect(s).toContain('onreplycount={handleThreadReplyCount}');
    expect(s).toContain('function handleThreadReplyCount(rootEventId: string, replyCount: number): void');
    // Overlay on narrow widths, third column on wide.
    expect(s).toContain('@media (max-width: 720px)');
  });
});

describe('US-022: Rust commands + poll path + registration + capability', () => {
  it('defines fetch_thread, send_thread_reply, set_active_thread commands', () => {
    const r = normalize(dmNotify);
    expect(r).toContain('pub async fn fetch_thread(');
    expect(r).toContain('pub async fn send_thread_reply(');
    expect(r).toContain('pub fn set_active_thread(');
    expect(r).toContain('GET /v1/notify/threads');
  });

  it('emits thread:new-reply from the single poll path', () => {
    const r = normalize(dmNotify);
    expect(r).toContain('pub const EVENT_THREAD_NEW_REPLY: &str = "thread:new-reply";');
    expect(r).toContain('async fn poll_active_thread(');
    // Folded into the single do_poll path, not a parallel poller.
    expect(r).toContain('poll_active_thread(app, &base_url, &access_token).await;');
    expect(r).toContain('app.emit(EVENT_THREAD_NEW_REPLY,');
  });

  it('registers both commands in main.rs invoke_handler + manages ActiveThreadState', () => {
    const m = normalize(mainRs);
    expect(m).toContain('commands::dm_notify::fetch_thread');
    expect(m).toContain('commands::dm_notify::send_thread_reply');
    expect(m).toContain('commands::dm_notify::set_active_thread');
    expect(m).toContain('commands::dm_notify::ActiveThreadState::new()');
  });

  it('covers fetch_thread + send_thread_reply in the messages capability description', () => {
    expect(capability).toContain('fetch_thread');
    expect(capability).toContain('send_thread_reply');
    expect(capability).toContain('set_active_thread');
    expect(capability).toContain('thread:new-reply');
  });
});
