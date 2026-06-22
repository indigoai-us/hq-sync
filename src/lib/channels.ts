// Pure helpers for the Channels UI (US-018).
//
// The Channels segment (ChannelList) renders the caller's channels grouped
// under a "Personal" header and one header per company, each row showing the
// channel `#name` with a scope chip (a personal glyph vs the company name).
// Keeping the grouping/labeling/membership logic here (not inside the .svelte
// components) makes it unit-testable without a DOM — mirrors lib/dmRequests.ts
// and lib/recipientPicker.ts. The components own the invoke() calls + rendering.

/** A channel's posting policy — who may post into it. Tolerant of server
 * additions: the UI only branches on the values it knows. */
export type ChannelPostPolicy = 'all' | 'owner' | string;

/** Channel visibility. `company` channels are discoverable by anyone in the
 * company; `private` channels are invite-only. */
export type ChannelVisibility = 'company' | 'private' | string;

/** The caller's membership state in a channel. `joined` = full participant;
 * `invited` = invited but not yet joined (show the join CTA, not the composer);
 * `none` = discoverable but the caller has no relationship yet. Absent on older
 * payloads → treated as 'joined' (the list only returns channels you can see). */
export type ChannelMembership = 'joined' | 'invited' | 'none' | string;

/** One channel the caller can see. Mirrors the Rust `Channel` wire shape
 * (camelCase). `companyUid` is present only for company-scoped channels. */
export interface Channel {
  channelId: string;
  name: string;
  /** "personal" | "company" | "group". Group DMs are unnamed, participant-keyed. */
  scope: 'personal' | 'company' | 'group' | string;
  companyUid?: string | null;
  /** Company display name (server-supplied for company channels), used for the
   * group header + scope chip. Falls back to companyUid when absent. */
  companyName?: string | null;
  postPolicy?: ChannelPostPolicy;
  visibility?: ChannelVisibility;
  membership?: ChannelMembership;
  /** Unread message count for this channel (drives the left-rail badge). */
  unread?: number;
  /** Member count (shown on the header member-count button). */
  memberCount?: number;
  /** Optional server-supplied activity timestamps (ISO-8601). Older servers omit
   * them; when present they let a channel interleave with DMs by recency in the
   * unified rail (`mergeConversations`). Absent → the channel falls back to an
   * unread-aware ordering. */
  lastActivityAt?: string | null;
  lastMessageAt?: string | null;
  /** Client-only epoch-ms stamp of when this channel first entered the rail,
   * set once by `upsertChannel` on insert. NOT part of the server wire shape —
   * it lets a brand-new channel with no server timestamps surface as recent in
   * the unified rail (`mergeConversations`) instead of sinking to the bottom.
   * Never serialized back to the server. */
  arrivedAt?: number | null;
  /** Server-supplied channel creation timestamp (ISO-8601). Present on the list
   * payload for every scope; used as a fallback ordering signal for group DMs,
   * which ship no activity timestamp (`mergeConversations`). */
  createdAt?: string | null;
  /** Group-DM participant roster (the OTHER members — caller excluded),
   * server-supplied on the list endpoint so an unnamed group DM can be named by
   * its people ("Stefan, Hassaan"). Absent for named scopes and on older server
   * payloads. */
  members?: ChannelParticipant[];
}

/** A group-DM participant as surfaced on the channels list payload — just enough
 * to label the conversation by its people. */
export interface ChannelParticipant {
  personUid: string;
  displayName: string;
}

/** One member of a channel. Mirrors the Rust `ChannelMember` wire shape. */
export interface ChannelMember {
  personUid: string;
  email: string;
  displayName: string;
  /** "owner" | "member" — owners can remove members + invite. */
  role: 'owner' | 'member' | string;
}

/** A group of channels under one header (Personal, or a company name). */
export interface ChannelGroup {
  /** Stable key for `{#each}`. */
  key: string;
  /** Header label shown above the group. */
  label: string;
  /** "personal" | "company" | "group" — drives the header glyph. */
  scope: 'personal' | 'company' | 'group';
  /** companyUid for company groups (used to scope new-channel creation). */
  companyUid?: string | null;
  channels: Channel[];
}

/** Best display name for a channel — its `name`, `#`-stripped + trimmed. Group
 * DMs are unnamed, so they're labeled by their participants. */
export function channelDisplayName(c: Channel): string {
  const trimmed = c.name.trim().replace(/^#+/, '');
  if (trimmed) return trimmed;
  if (c.scope === 'group') return groupDmLabel(c);
  return c.channelId;
}

/** Label an unnamed group DM by its people — "Stefan, Hassaan", truncated to
 * "Stefan, Hassaan +2" when the roster is long. Falls back to a member-count
 * label ("Group · N"), then "Group DM" when the server supplied no participant
 * info (older payloads). */
function groupDmLabel(c: Channel): string {
  const names = (c.members ?? [])
    .map((m) => m.displayName?.trim())
    .filter((n): n is string => !!n);
  if (names.length > 0) {
    const shown = names.slice(0, 3);
    const extra = names.length - shown.length;
    return extra > 0 ? `${shown.join(', ')} +${extra}` : shown.join(', ');
  }
  const n = c.memberCount ?? 0;
  return n > 0 ? `Group · ${n}` : 'Group DM';
}

/** The scope chip text: a personal glyph for personal channels, "Group" for a
 * group DM, else the company NAME. Never the raw `cmp_…` UID — an unresolved
 * company degrades to the generic "Company" label, not an opaque identifier. */
export function scopeChipLabel(c: Channel): string {
  if (c.scope === 'personal') return 'Personal';
  if (c.scope === 'group') return 'Group';
  return c.companyName?.trim() || 'Company';
}

/** Resolve a channel's company display NAME for the unified rail chip. Tries the
 * channel's own `companyName`, then the caller's membership labels, and finally
 * the generic "Company" — it NEVER returns the raw `cmp_…` UID (the bug where a
 * channel row rendered `cmp_01KQ2RYAH…` as a pill). Personal/group channels have
 * no company chip and return null. */
export function companyNameFor(c: Channel, companies: CompanyLabel[] = []): string | null {
  if (c.scope === 'personal' || c.scope === 'group') return null;
  const fromChannel = c.companyName?.trim();
  if (fromChannel) return fromChannel;
  const uid = c.companyUid?.trim();
  if (uid) {
    const fromList = companies.find((co) => co.companyUid === uid)?.companyName?.trim();
    if (fromList) return fromList;
  }
  return 'Company';
}

/** True when the caller is invited to a channel but hasn't joined yet — the
 * ChannelView shows a join CTA instead of the composer for these. */
export function isInvitedNotJoined(c: Channel): boolean {
  return (c.membership ?? 'joined') === 'invited';
}

/** True when the caller may post into the channel: they're joined AND the
 * post policy permits them. `owner`-only channels need the caller to be the
 * owner — which we can't determine from the Channel alone, so a non-owner in an
 * owner-only channel is gated by the roster check in the view, not here; this
 * helper only blocks the not-yet-joined case (the universal composer gate). */
export function canPost(c: Channel): boolean {
  return (c.membership ?? 'joined') === 'joined';
}

/** Display label for a company in a group header / chip. */
export interface CompanyLabel {
  companyUid: string;
  companyName: string | null;
}

/**
 * Group channels for the Channels segment.
 *
 * Ordering:
 *   1. A "Personal" group first (all `scope === "personal"` channels), only when
 *      non-empty.
 *   2. One group per company that has at least one channel, in `companies`
 *      order; companies not in that list are appended after (sorted by label) so
 *      a channel for a company the caller can't currently enumerate still shows.
 *
 * Each group's channels are sorted by display name (case-insensitive). The
 * company display name is resolved from (in order): the channel's own
 * `companyName`, the `companies` lookup, then the raw companyUid.
 */
export function groupChannels(
  channels: Channel[],
  companies: CompanyLabel[] = [],
): ChannelGroup[] {
  const byName = (a: Channel, b: Channel): number =>
    channelDisplayName(a).toLowerCase().localeCompare(channelDisplayName(b).toLowerCase());

  const groupDms = channels.filter((c) => c.scope === 'group').slice().sort(byName);
  const personal = channels.filter((c) => c.scope === 'personal').slice().sort(byName);

  // Bucket company channels by companyUid.
  const companyBuckets = new Map<string, Channel[]>();
  for (const c of channels) {
    if (c.scope === 'personal' || c.scope === 'group') continue;
    const uid = (c.companyUid ?? '').trim() || '__unknown__';
    const bucket = companyBuckets.get(uid) ?? [];
    bucket.push(c);
    companyBuckets.set(uid, bucket);
  }

  const labelFor = (uid: string, sample: Channel | undefined): string => {
    const fromChannel = sample?.companyName?.trim();
    if (fromChannel) return fromChannel;
    const fromList = companies.find((co) => co.companyUid === uid)?.companyName?.trim();
    if (fromList) return fromList;
    // Never surface the raw `cmp_…` UID as a header label.
    return 'Company';
  };

  const groups: ChannelGroup[] = [];
  // Group DMs first under a "Direct" header — the most conversational surface.
  if (groupDms.length > 0) {
    groups.push({ key: 'group', label: 'Direct', scope: 'group', channels: groupDms });
  }
  if (personal.length > 0) {
    groups.push({ key: 'personal', label: 'Personal', scope: 'personal', channels: personal });
  }

  // Companies the caller knows about, in their declared order, then any
  // remaining buckets sorted by resolved label.
  const orderedUids: string[] = [];
  for (const co of companies) {
    if (companyBuckets.has(co.companyUid)) orderedUids.push(co.companyUid);
  }
  const remaining = [...companyBuckets.keys()]
    .filter((uid) => !orderedUids.includes(uid))
    .sort((a, b) => labelFor(a, companyBuckets.get(a)?.[0]).localeCompare(labelFor(b, companyBuckets.get(b)?.[0])));

  for (const uid of [...orderedUids, ...remaining]) {
    const bucket = (companyBuckets.get(uid) ?? []).slice().sort(byName);
    if (bucket.length === 0) continue;
    groups.push({
      key: `company:${uid}`,
      label: labelFor(uid, bucket[0]),
      scope: 'company',
      companyUid: uid === '__unknown__' ? null : uid,
      channels: bucket,
    });
  }

  return groups;
}

/** Total unread across all channels — feeds the popover badge accent. */
export function totalChannelUnread(channels: Channel[]): number {
  return channels.reduce((sum, c) => sum + (c.unread ?? 0), 0);
}

/** Upsert a channel into a list by channelId (replace if present, else append),
 * preserving order. Returns a new array (callers reassign $state).
 *
 * On a NEW insert the channel is stamped with `arrivedAt` (epoch-ms, default
 * `Date.now()`) so a brand-new channel with no server timestamps still surfaces
 * as recent in the unified rail (`mergeConversations`) — fixing the bug where an
 * externally-/self-created group DM (unread 0, no `lastActivityAt`) sank to the
 * bottom and looked like it never arrived. On a REPLACE the prior `arrivedAt` is
 * preserved (a re-poll of an already-known channel must not re-float it →
 * no flicker, no reorder), unless the incoming payload already carries one.
 * `now` is injectable for deterministic tests. */
export function upsertChannel(list: Channel[], next: Channel, now: number = Date.now()): Channel[] {
  const idx = list.findIndex((c) => c.channelId === next.channelId);
  if (idx === -1) {
    return [...list, { ...next, arrivedAt: next.arrivedAt ?? now }];
  }
  const copy = list.slice();
  // Preserve the original arrival stamp across re-polls so an existing channel
  // keeps its place; honor an explicit incoming `arrivedAt` if the caller set one.
  copy[idx] = { ...next, arrivedAt: next.arrivedAt ?? list[idx].arrivedAt };
  return copy;
}

/** Apply an unread delta to one channel by id (clamped at 0). Returns a new
 * array. Used when a `channel:new-message` event lands for a channel the user
 * isn't currently viewing. */
export function bumpChannelUnread(list: Channel[], channelId: string, delta: number): Channel[] {
  return list.map((c) =>
    c.channelId === channelId ? { ...c, unread: Math.max(0, (c.unread ?? 0) + delta) } : c,
  );
}

/** Clear unread for one channel by id (when the user opens/reads it). */
export function clearChannelUnread(list: Channel[], channelId: string): Channel[] {
  return list.map((c) => (c.channelId === channelId ? { ...c, unread: 0 } : c));
}
