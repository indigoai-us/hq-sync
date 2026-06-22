export interface ContactRecencyFields {
  personUid: string;
  email?: string | null;
  displayName?: string | null;
  lastMessageAt?: string | null;
  lastActivityAt?: string | null;
  lastDmAt?: string | null;
}

export interface ConversationEventRecencyFields {
  personUid?: string | null;
  fromPersonUid?: string | null;
  toPersonUid?: string | null;
  peerPersonUid?: string | null;
  withPersonUid?: string | null;
  email?: string | null;
  fromEmail?: string | null;
  toEmail?: string | null;
  peerEmail?: string | null;
  createdAt?: string | null;
  sentAt?: string | null;
  lastMessageAt?: string | null;
  body?: string | null;
  direction?: string | null;
}

export interface ContactPreviewFields extends ContactRecencyFields {
  lastMessageBody?: string | null;
  lastMessagePreview?: string | null;
  lastMessageText?: string | null;
  lastMessageDirection?: string | null;
  previewBody?: string | null;
  previewAt?: string | null;
  previewDirection?: string | null;
}

export interface ConversationPreview {
  body: string;
  createdAt: string | null;
  direction: string | null;
}

function parseTimestamp(value: string | null | undefined): number {
  if (!value) return 0;
  const time = Date.parse(value);
  return Number.isFinite(time) ? time : 0;
}

function ownContactTime(contact: ContactRecencyFields): number {
  return Math.max(
    parseTimestamp(contact.lastMessageAt),
    parseTimestamp(contact.lastActivityAt),
    parseTimestamp(contact.lastDmAt),
  );
}

function eventTime(event: ConversationEventRecencyFields): number {
  return Math.max(
    parseTimestamp(event.createdAt),
    parseTimestamp(event.sentAt),
    parseTimestamp(event.lastMessageAt),
  );
}

function personKey(value: string | null | undefined): string | null {
  const normalized = value?.trim();
  return normalized ? `person:${normalized}` : null;
}

function emailKey(value: string | null | undefined): string | null {
  const normalized = value?.trim().toLowerCase();
  return normalized ? `email:${normalized}` : null;
}

function addLatest(map: Map<string, number>, key: string | null, time: number): void {
  if (!key || time <= 0) return;
  map.set(key, Math.max(map.get(key) ?? 0, time));
}

function contactKeys(contact: ContactRecencyFields): string[] {
  return [
    personKey(contact.personUid),
    emailKey(contact.email),
  ].filter((key): key is string => Boolean(key));
}

function eventKeys(event: ConversationEventRecencyFields): string[] {
  return [
    personKey(event.personUid),
    personKey(event.fromPersonUid),
    personKey(event.toPersonUid),
    personKey(event.peerPersonUid),
    personKey(event.withPersonUid),
    emailKey(event.email),
    emailKey(event.fromEmail),
    emailKey(event.toEmail),
    emailKey(event.peerEmail),
  ].filter((key): key is string => Boolean(key));
}

function indexEvents(events: ConversationEventRecencyFields[]): Map<string, number> {
  const map = new Map<string, number>();
  for (const event of events) {
    const time = eventTime(event);
    for (const key of eventKeys(event)) addLatest(map, key, time);
  }
  return map;
}

function label(contact: ContactRecencyFields): string {
  return (contact.displayName?.trim() || contact.email?.trim() || contact.personUid).toLowerCase();
}

function contactTime(contact: ContactRecencyFields, eventIndex: Map<string, number>): number {
  let latest = ownContactTime(contact);
  for (const key of contactKeys(contact)) {
    latest = Math.max(latest, eventIndex.get(key) ?? 0);
  }
  return latest;
}

function eventPreview(event: ConversationEventRecencyFields): ConversationPreview | null {
  const body = event.body?.replace(/\s+/g, ' ').trim();
  if (!body) return null;
  const createdAt = event.createdAt ?? event.sentAt ?? event.lastMessageAt ?? null;
  return {
    body,
    createdAt,
    direction: event.direction ?? null,
  };
}

function contactPreviewTimestamp(contact: ContactPreviewFields): number {
  return Math.max(parseTimestamp(contact.previewAt), ownContactTime(contact));
}

function firstText(values: Array<string | null | undefined>): string | null {
  for (const value of values) {
    const text = value?.replace(/\s+/g, ' ').trim();
    if (text) return text;
  }
  return null;
}

function indexEventPreviews(
  events: ConversationEventRecencyFields[],
): Map<string, ConversationPreview> {
  const map = new Map<string, ConversationPreview>();
  for (const event of events) {
    const preview = eventPreview(event);
    if (!preview) continue;
    const time = eventTime(event);
    for (const key of eventKeys(event)) {
      if (!key) continue;
      const current = map.get(key);
      const currentTime = current ? parseTimestamp(current.createdAt) : 0;
      if (!current || time > currentTime) map.set(key, preview);
    }
  }
  return map;
}

export function sortContactsByRecentActivity<T extends ContactRecencyFields>(
  contacts: T[],
  events: ConversationEventRecencyFields[] = [],
): T[] {
  const eventIndex = indexEvents(events);
  return [...contacts].sort((a, b) => {
    const recencyDelta = contactTime(b, eventIndex) - contactTime(a, eventIndex);
    if (recencyDelta !== 0) return recencyDelta;
    return label(a).localeCompare(label(b)) || a.personUid.localeCompare(b.personUid);
  });
}

export function mergeContactPreviews<T extends ContactPreviewFields>(
  contacts: T[],
  events: ConversationEventRecencyFields[] = [],
): T[] {
  const previews = indexEventPreviews(events);
  return contacts.map((contact) => {
    let newest: ConversationPreview | null = null;
    for (const key of contactKeys(contact)) {
      const preview = previews.get(key);
      if (!preview) continue;
      if (!newest || parseTimestamp(preview.createdAt) > parseTimestamp(newest.createdAt)) {
        newest = preview;
      }
    }

    const existingBody = firstText([
      contact.previewBody,
      contact.lastMessagePreview,
      contact.lastMessageBody,
      contact.lastMessageText,
    ]);
    if (!newest) {
      return existingBody ? { ...contact, previewBody: existingBody } : contact;
    }

    const existingTime = contactPreviewTimestamp(contact);
    const newestTime = parseTimestamp(newest.createdAt);
    if (existingBody && existingTime > newestTime) {
      return { ...contact, previewBody: existingBody };
    }

    return {
      ...contact,
      previewBody: newest.body,
      previewAt: newest.createdAt ?? contact.previewAt ?? contact.lastMessageAt ?? null,
      previewDirection: newest.direction ?? contact.previewDirection ?? null,
      lastMessageAt: newest.createdAt ?? contact.lastMessageAt ?? null,
    };
  });
}

export function contactPreviewText(contact: ContactPreviewFields): string | null {
  const body = firstText([
    contact.previewBody,
    contact.lastMessagePreview,
    contact.lastMessageBody,
    contact.lastMessageText,
  ]);
  if (!body) return null;

  const direction = contact.previewDirection ?? contact.lastMessageDirection ?? null;
  return direction === 'out' ? `You: ${body}` : body;
}

export function contactPreviewAt(contact: ContactPreviewFields): string | null {
  return contact.previewAt ?? contact.lastMessageAt ?? contact.lastActivityAt ?? contact.lastDmAt ?? null;
}

/** Recency-relevant fields a channel contributes to the unified rail. Channels
 * may not carry a server timestamp yet (older servers omit `lastActivityAt` /
 * `lastMessageAt`); when absent, an unread channel still floats up via the
 * `now` fallback in `mergeConversations`, and a freshly-arrived channel floats
 * up via its client-stamped `arrivedAt` (see `upsertChannel`). */
export interface ChannelRecencyFields {
  channelId: string;
  name?: string | null;
  unread?: number | null;
  lastActivityAt?: string | null;
  lastMessageAt?: string | null;
  /** Server-supplied channel creation timestamp (ISO-8601). A fallback ordering
   * signal for channels the server returns with NO activity timestamps — notably
   * group DMs, whose list payload carries `createdAt` but no `lastMessageAt`. Lets
   * them sort by when they were created instead of sinking to `time: 0` below
   * every contact. */
  createdAt?: string | null;
  /** Client-only epoch-ms stamp of when this channel FIRST entered the rail
   * (set once by `upsertChannel` on insert, never on re-poll). Lets a brand-new
   * channel with no server timestamps and `unread: 0` — e.g. a group DM the
   * signed-in user just created via `hq dm` — surface as recent instead of
   * sinking to `time: 0`. Never sent to the server; absent on a channel built
   * directly (not through `upsertChannel`), so steady-state ordering is
   * unchanged. */
  arrivedAt?: number | null;
}

export type ConversationKind = 'dm' | 'channel';

/** One row in the unified conversation rail — a DM (contact) or a channel,
 * carrying the resolved sort `time` and `unread` so the view renders without
 * re-deriving order. Exactly one of `contact` / `channel` is set. */
export interface UnifiedConversationItem<C, Ch> {
  key: string;
  kind: ConversationKind;
  time: number;
  unread: number;
  contact?: C;
  channel?: Ch;
}

function channelLabel(channel: ChannelRecencyFields): string {
  return (channel.name?.trim() || channel.channelId).toLowerCase();
}

/**
 * Merge DMs (contacts) and channels into ONE recency-sorted list — the unified
 * Messages rail (channels alongside people, no separate Channels tab).
 *
 * Sort: newest activity first. A DM's time folds in its own timestamps + the
 * local notification `events` index (same source as `sortContactsByRecentActivity`)
 * and its hydrated `previewAt`. A channel's time is its server `lastActivityAt` /
 * `lastMessageAt` when present; when absent, it falls back (in order) to its
 * client `arrivedAt` (a brand-new channel that just entered the rail surfaces as
 * recent — see `upsertChannel`), then to `now` for an UNREAD timeless channel
 * (it needs attention), and finally a read, never-arrival-stamped timeless
 * channel sinks to the bottom. Ties break by unread desc, then label.
 *
 * Pure + DOM-free so it's unit-tested like the other helpers here. Pass `now`
 * for deterministic tests.
 */
export function mergeConversations<
  C extends ContactPreviewFields,
  Ch extends ChannelRecencyFields,
>(
  contacts: C[],
  channels: Ch[],
  options: { events?: ConversationEventRecencyFields[]; now?: number } = {},
): UnifiedConversationItem<C, Ch>[] {
  const events = options.events ?? [];
  const now = options.now ?? Date.now();
  const eventIndex = indexEvents(events);

  const dmItems: UnifiedConversationItem<C, Ch>[] = contacts.map((contact) => ({
    key: `dm:${contact.personUid}`,
    kind: 'dm',
    time: Math.max(contactTime(contact, eventIndex), contactPreviewTimestamp(contact)),
    unread: 0,
    contact,
  }));

  const channelItems: UnifiedConversationItem<C, Ch>[] = channels.map((channel) => {
    const stamp = Math.max(
      parseTimestamp(channel.lastActivityAt),
      parseTimestamp(channel.lastMessageAt),
    );
    const unread = channel.unread ?? 0;
    const arrivedAt = channel.arrivedAt ?? 0;
    return {
      key: `ch:${channel.channelId}`,
      kind: 'channel',
      // Server stamp wins (real ordering untouched). Else a freshly-arrived
      // channel surfaces at its `arrivedAt`. Else fall back to the channel's
      // `createdAt` (group DMs ship this but no activity stamp, so they order by
      // creation instead of sinking to 0). Else an unread timeless channel floats
      // to `now`. Else (read + timeless + never-arrival/created-stamped) → 0.
      time: stamp || arrivedAt || parseTimestamp(channel.createdAt) || (unread > 0 ? now : 0),
      unread,
      channel,
    };
  });

  const labelOf = (item: UnifiedConversationItem<C, Ch>): string =>
    item.contact ? label(item.contact) : item.channel ? channelLabel(item.channel) : item.key;

  return [...channelItems, ...dmItems].sort((a, b) => {
    if (b.time !== a.time) return b.time - a.time;
    if (b.unread !== a.unread) return b.unread - a.unread;
    return labelOf(a).localeCompare(labelOf(b)) || a.key.localeCompare(b.key);
  });
}

export function previewFromMessages<T extends {
  body?: string | null;
  createdAt?: string | null;
  direction?: string | null;
}>(messages: T[]): ConversationPreview | null {
  let latest: ConversationPreview | null = null;
  for (const message of messages) {
    const body = message.body?.replace(/\s+/g, ' ').trim();
    if (!body) continue;
    const createdAt = message.createdAt ?? null;
    if (!latest || parseTimestamp(createdAt) >= parseTimestamp(latest.createdAt)) {
      latest = {
        body,
        createdAt,
        direction: message.direction ?? null,
      };
    }
  }
  return latest;
}
