/**
 * Reactive controller for the reactions UI (US-025), shared by every Conversation
 * host (MessagesShell DM pane, ChannelView, ThreadPanel, DmDetail).
 *
 * Each host creates one controller for its open conversation's messageScope and:
 *   - feeds it the visible message ids (`setMessages`) so it can load reactions
 *     and register the active conversation with the SINGLE Rust poll path
 *     (`set_active_conversation`) — that's what lets a "reaction" wake re-fetch
 *     and emit `message:reaction`;
 *   - binds `controller.map` into `<Conversation reactions={…} />`;
 *   - passes `controller.toggle` as `ontogglereaction`;
 *   - calls `controller.applyEvent` from its `message:reaction` listener;
 *   - calls `controller.dispose()` on teardown to clear the active conversation.
 *
 * The optimistic toggle is local + instant; the authoritative `message:reaction`
 * event reconciles it. A failed toggle_reaction rolls the optimistic change back.
 *
 * All pure logic lives in `reactions.ts` (unit-tested without a DOM); this module
 * only owns the reactive `$state`, the `invoke()` calls, and the listener glue.
 */

import { invoke } from '@tauri-apps/api/core';
import {
  type ReactionAggregate,
  type ReactionEvent,
  type ReactionMap,
  applyReactionEvent,
  buildReactionMap,
  setMessageReactions,
  toggleIsAdd,
  toggleReaction as toggleAggregates,
} from './reactions';

export class ReactionController {
  /** messageId → sorted aggregates. Bound into `<Conversation reactions={…} />`. */
  map = $state<ReactionMap>({});

  /** The open conversation's messageScope (`dm:…` | `chan:…`). */
  private scope: string;
  /** The message ids currently registered as active (for stale-guarding). */
  private messageIds: string[] = [];
  /**
   * Whether `dispose()` clears the Rust active-conversation slot. The primary
   * pane (DM/channel) owns the slot lifecycle and clears on teardown; a secondary
   * host that shares the same scope (the ThreadPanel, whose replies merge into the
   * same slot) passes `false` so closing it doesn't wipe the still-open pane's
   * registration.
   */
  private clearOnDispose: boolean;

  constructor(scope: string, clearOnDispose = true) {
    this.scope = scope;
    this.clearOnDispose = clearOnDispose;
  }

  /**
   * Declare the conversation's currently-visible message ids. Loads their
   * reactions (initial aggregate sets) and registers the active conversation
   * with the Rust poll path so a realtime wake can re-fetch + emit. Call whenever
   * the host's message list changes (and once on open).
   */
  async setMessages(messageIds: string[]): Promise<void> {
    this.messageIds = [...messageIds];

    // Register the active conversation first so a wake mid-load still re-fetches.
    try {
      await invoke('set_active_conversation', {
        scope: this.scope,
        messageIds: this.messageIds,
      });
    } catch (err) {
      console.error('reactions: set_active_conversation failed', err);
    }

    // Load each message's current reactions (best-effort, in parallel).
    try {
      const rows = await Promise.all(
        this.messageIds.map(async (messageId) => {
          try {
            const reactions = await invoke<ReactionAggregate[]>('fetch_reactions', {
              messageScope: this.scope,
              messageId,
            });
            return { messageId, reactions: reactions ?? [] };
          } catch (err) {
            console.error('reactions: fetch_reactions failed', messageId, err);
            return { messageId, reactions: [] as ReactionAggregate[] };
          }
        }),
      );
      this.map = buildReactionMap(rows);
    } catch (err) {
      console.error('reactions: initial load failed', err);
    }
  }

  /**
   * Optimistically toggle the caller's `emoji` reaction on `messageId`, then
   * persist via `toggle_reaction`. Rolls the optimistic change back on failure;
   * the `message:reaction` event reconciles the authoritative count either way.
   */
  toggle = (messageId: string, emoji: string): void => {
    const before = this.map[messageId];
    const add = toggleIsAdd(before, emoji);
    // Optimistic local update.
    this.map = setMessageReactions(this.map, messageId, toggleAggregates(before, emoji));

    void invoke('toggle_reaction', {
      messageScope: this.scope,
      messageId,
      emoji,
      add,
    }).catch((err) => {
      console.error('reactions: toggle_reaction failed', err);
      // Roll back to the pre-toggle aggregates for this message.
      this.map = setMessageReactions(this.map, messageId, before ?? []);
    });
  };

  /**
   * Apply a `message:reaction` Tauri event payload. Ignores events for a
   * different scope (the pure helper guards this), so a host only reconciles its
   * own conversation.
   */
  applyEvent = (event: ReactionEvent): void => {
    this.map = applyReactionEvent(this.map, this.scope, event);
  };

  /** Clear the active conversation on the Rust side (host teardown / close).
   * No-op when `clearOnDispose` is false (a secondary host sharing the slot). */
  dispose(): void {
    if (!this.clearOnDispose) return;
    void invoke('set_active_conversation', { scope: null, messageIds: [] }).catch(() => {
      /* best-effort */
    });
  }
}
