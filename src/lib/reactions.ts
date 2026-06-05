// Pure helpers for the emoji-reactions UI (US-025).
//
// ReactionBar / EmojiPicker render reaction pills under every message bubble in
// the shared <Conversation/> (DMs, channels, threads). The aggregate shape,
// optimistic toggle, message-scope derivation, and the live `message:reaction`
// reconcile all live here so they're unit-testable without a DOM — the .svelte
// components own the invoke() calls + rendering. Mirrors the lib/dmRequests.ts
// and lib/recipientPicker.ts split.
//
// ## Wire contract (hq-pro US-024)
//
//   POST   /v1/notify/reactions { messageScope, messageId, emoji }   (add)
//   DELETE /v1/notify/reactions { messageScope, messageId, emoji }   (remove)
//   GET    /v1/notify/reactions?messageScope=&messageId=
//            → [{ emoji, count, reactedByMe }]
//
//   messageScope = "dm:" + pairKey    (a DM conversation)
//                | "chan:" + channelId (a channel)
//
// The realtime "reaction" wake ({type:"reaction", messageScope, messageId,
// emoji, op}) lands on the person topic; the Rust single-poll path re-fetches the
// open conversation's reactions and emits a `message:reaction` Tauri event the
// hosts apply via `applyReactionEvent`.

/** One emoji's aggregate on a single message, as returned by
 * `GET /v1/notify/reactions` and mirrored by the Rust `ReactionAggregate`. */
export interface ReactionAggregate {
  emoji: string;
  count: number;
  /** True when the signed-in caller is one of the reactors (drives the
   * highlighted pill + toggle direction). */
  reactedByMe: boolean;
}

/** The payload of the `message:reaction` Tauri event emitted by the single poll
 * path. `reactions` is the full re-aggregated set for `messageId` in
 * `messageScope` (newest server truth), so the host replaces wholesale. */
export interface ReactionEvent {
  messageScope: string;
  messageId: string;
  reactions: ReactionAggregate[];
}

/** Map of messageId → its reaction aggregates, owned by each Conversation host
 * and passed down to <Conversation/> → <ReactionBar/>. */
export type ReactionMap = Record<string, ReactionAggregate[]>;

/** Build the `messageScope` string for a DM conversation. The DM scope is keyed
 * by the conversation pair; the client passes the peer's personUid and the
 * server resolves the symmetric pairKey, echoing back the same scope on the
 * realtime wake so optimistic + reconcile stay aligned. */
export function dmScope(peerPersonUid: string): string {
  return `dm:${peerPersonUid.trim()}`;
}

/** Build the `messageScope` string for a channel. */
export function channelScope(channelId: string): string {
  return `chan:${channelId.trim()}`;
}

/** Curated emoji set for the picker. Kept intentionally small (~24) and inline
 * so we never pull a multi-MB emoji-data dependency (the bundle budget is
 * <15MB — see tests/PERF.md). Ordered by rough frequency of use. */
export const CURATED_EMOJI: readonly string[] = [
  '👍',
  '❤️',
  '😂',
  '🎉',
  '🙏',
  '🔥',
  '👀',
  '✅',
  '🚀',
  '💯',
  '😄',
  '😍',
  '🤔',
  '😢',
  '😮',
  '👏',
  '🙌',
  '💪',
  '👌',
  '🤝',
  '💡',
  '⭐',
  '❓',
  '👎',
] as const;

/** Find an emoji's aggregate within a message's list (undefined if absent). */
export function findAggregate(
  list: ReactionAggregate[] | undefined,
  emoji: string,
): ReactionAggregate | undefined {
  return (list ?? []).find((r) => r.emoji === emoji);
}

/** True when the caller has already reacted with `emoji` on this message — the
 * pill is highlighted and a click will REMOVE the reaction. */
export function hasReacted(
  list: ReactionAggregate[] | undefined,
  emoji: string,
): boolean {
  return !!findAggregate(list, emoji)?.reactedByMe;
}

/** Whether a click on `emoji` should ADD (true) or REMOVE (false) the caller's
 * reaction, given the current aggregates. Pure mirror of the optimistic toggle
 * so the .svelte caller and the toggle helper agree on direction. */
export function toggleIsAdd(
  list: ReactionAggregate[] | undefined,
  emoji: string,
): boolean {
  return !hasReacted(list, emoji);
}

/**
 * Apply the caller's optimistic toggle of `emoji` on a message's aggregates,
 * returning a NEW list (never mutates the input). Adds the caller's reaction
 * (count +1, reactedByMe true) when they hadn't reacted, or removes it (count
 * -1, reactedByMe false; pill dropped when the count hits 0) when they had.
 *
 * The optimistic result is reconciled by the authoritative `message:reaction`
 * event (see `applyReactionEvent`), so a transient over/under-count self-heals.
 */
export function toggleReaction(
  list: ReactionAggregate[] | undefined,
  emoji: string,
): ReactionAggregate[] {
  const current = list ?? [];
  const existing = findAggregate(current, emoji);

  // No existing aggregate → the caller is the first reactor with this emoji.
  if (!existing) {
    return [...current, { emoji, count: 1, reactedByMe: true }];
  }

  if (existing.reactedByMe) {
    // Caller is un-reacting. Drop their +1; remove the pill if it empties.
    const nextCount = existing.count - 1;
    if (nextCount <= 0) {
      return current.filter((r) => r.emoji !== emoji);
    }
    return current.map((r) =>
      r.emoji === emoji ? { ...r, count: nextCount, reactedByMe: false } : r,
    );
  }

  // Caller is adding to an emoji others already used.
  return current.map((r) =>
    r.emoji === emoji ? { ...r, count: r.count + 1, reactedByMe: true } : r,
  );
}

/** Sort aggregates for stable display: highest count first, then emoji order so
 * ties don't reshuffle on every re-render. Returns a new array. */
export function sortAggregates(list: ReactionAggregate[]): ReactionAggregate[] {
  return [...list].sort((a, b) => b.count - a.count || a.emoji.localeCompare(b.emoji));
}

/**
 * Apply an authoritative `message:reaction` event to a host's reaction map,
 * returning a NEW map. The event carries the full re-aggregated set for one
 * message, so we replace that message's entry wholesale (and drop the key when
 * the message has no reactions left, keeping the map tidy). Events for a
 * different scope are ignored so a host only reconciles its own conversation.
 */
export function applyReactionEvent(
  map: ReactionMap,
  scope: string,
  event: ReactionEvent,
): ReactionMap {
  if (event.messageScope !== scope) return map;
  const next: ReactionMap = { ...map };
  const sorted = sortAggregates(event.reactions ?? []);
  if (sorted.length === 0) {
    delete next[event.messageId];
  } else {
    next[event.messageId] = sorted;
  }
  return next;
}

/** Set/replace one message's aggregates in a host's reaction map, returning a
 * NEW map (used for the optimistic local update before the server reconciles). */
export function setMessageReactions(
  map: ReactionMap,
  messageId: string,
  reactions: ReactionAggregate[],
): ReactionMap {
  const next: ReactionMap = { ...map };
  if (reactions.length === 0) {
    delete next[messageId];
  } else {
    next[messageId] = reactions;
  }
  return next;
}

/** Build a reaction map keyed by messageId from a flat fetch result of
 * `{ messageId, reactions }` rows (the initial load for a conversation). */
export function buildReactionMap(
  rows: { messageId: string; reactions: ReactionAggregate[] }[],
): ReactionMap {
  const map: ReactionMap = {};
  for (const row of rows) {
    const sorted = sortAggregates(row.reactions ?? []);
    if (sorted.length > 0) map[row.messageId] = sorted;
  }
  return map;
}
