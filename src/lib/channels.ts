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
  /** "personal" | "company". */
  scope: 'personal' | 'company' | string;
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
  /** "personal" | "company" — drives the header glyph. */
  scope: 'personal' | 'company';
  /** companyUid for company groups (used to scope new-channel creation). */
  companyUid?: string | null;
  channels: Channel[];
}

/** Best display name for a channel — its `name`, `#`-stripped + trimmed. */
export function channelDisplayName(c: Channel): string {
  return c.name.trim().replace(/^#+/, '') || c.channelId;
}

/** The scope chip text: a personal glyph for personal channels, else the
 * company name (falling back to the companyUid, then a generic label). */
export function scopeChipLabel(c: Channel): string {
  if (c.scope === 'personal') return 'Personal';
  return c.companyName?.trim() || c.companyUid?.trim() || 'Company';
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

  const personal = channels.filter((c) => c.scope === 'personal').slice().sort(byName);

  // Bucket company channels by companyUid.
  const companyBuckets = new Map<string, Channel[]>();
  for (const c of channels) {
    if (c.scope === 'personal') continue;
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
    return uid === '__unknown__' ? 'Company' : uid;
  };

  const groups: ChannelGroup[] = [];
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
 * preserving order. Returns a new array (callers reassign $state). */
export function upsertChannel(list: Channel[], next: Channel): Channel[] {
  const idx = list.findIndex((c) => c.channelId === next.channelId);
  if (idx === -1) return [...list, next];
  const copy = list.slice();
  copy[idx] = next;
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
