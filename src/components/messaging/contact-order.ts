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
