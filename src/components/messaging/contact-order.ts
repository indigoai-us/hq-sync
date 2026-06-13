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
