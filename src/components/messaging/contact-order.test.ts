import { describe, expect, it } from 'vitest';
import {
  contactPreviewAt,
  contactPreviewText,
  mergeContactPreviews,
  mergeConversations,
  previewFromMessages,
  sortContactsByRecentActivity,
  type ChannelRecencyFields,
  type ContactPreviewFields,
  type ContactRecencyFields,
} from './contact-order';

const contact = (
  personUid: string,
  displayName: string,
  overrides: Partial<ContactPreviewFields> = {},
): ContactPreviewFields => ({
  personUid,
  displayName,
  email: `${personUid}@example.com`,
  ...overrides,
});

const channel = (
  channelId: string,
  name: string,
  overrides: Partial<ChannelRecencyFields> = {},
): ChannelRecencyFields => ({ channelId, name, ...overrides });

describe('sortContactsByRecentActivity', () => {
  it('sorts contacts by their newest server-supplied conversation timestamp', () => {
    const sorted = sortContactsByRecentActivity([
      contact('prs_old', 'Old', { lastMessageAt: '2026-06-10T00:00:00Z' }),
      contact('prs_new', 'New', { lastActivityAt: '2026-06-12T00:00:00Z' }),
      contact('prs_mid', 'Mid', { lastDmAt: '2026-06-11T00:00:00Z' }),
    ]);

    expect(sorted.map((c) => c.personUid)).toEqual(['prs_new', 'prs_mid', 'prs_old']);
  });

  it('uses notification-history DMs when the contacts endpoint has no activity fields', () => {
    const sorted = sortContactsByRecentActivity(
      [
        contact('prs_ada', 'Ada', { email: 'ada@getindigo.ai' }),
        contact('prs_grace', 'Grace', { email: 'grace@getindigo.ai' }),
        contact('prs_alan', 'Alan', { email: 'alan@example.com' }),
      ],
      [
        {
          fromPersonUid: 'prs_alan',
          fromEmail: 'alan@example.com',
          createdAt: '2026-06-13T00:00:00Z',
        },
        {
          fromPersonUid: 'prs_grace',
          fromEmail: 'grace@getindigo.ai',
          createdAt: '2026-06-12T00:00:00Z',
        },
      ],
    );

    expect(sorted.map((c) => c.personUid)).toEqual(['prs_alan', 'prs_grace', 'prs_ada']);
  });

  it('falls back to display name when nobody has known activity', () => {
    const sorted = sortContactsByRecentActivity([
      contact('prs_grace', 'Grace'),
      contact('prs_ada', 'Ada'),
      contact('prs_alan', 'Alan'),
    ]);

    expect(sorted.map((c) => c.displayName)).toEqual(['Ada', 'Alan', 'Grace']);
  });
});

describe('conversation previews', () => {
  it('merges latest notification body onto the matching contact', () => {
    const [row] = mergeContactPreviews(
      [contact('prs_alan', 'Alan', { email: 'alan@example.com' }) as ContactPreviewFields],
      [
        {
          fromPersonUid: 'prs_alan',
          fromEmail: 'alan@example.com',
          body: '  Latest\nmessage  body ',
          createdAt: '2026-06-13T10:00:00Z',
        },
      ],
    );

    expect(contactPreviewText(row)).toBe('Latest message body');
    expect(contactPreviewAt(row)).toBe('2026-06-13T10:00:00Z');
  });

  it('keeps inbox-preview rows sorted by their preview timestamp after hydration', () => {
    const rows = mergeContactPreviews(
      [
        contact('prs_old', 'Old', {
          lastMessageAt: '2026-06-09T10:00:00Z',
          lastMessageBody: 'Old thread',
        }),
        contact('prs_new', 'New'),
      ],
      [
        {
          fromPersonUid: 'prs_new',
          body: 'Fresh inbox',
          createdAt: '2026-06-13T10:00:00Z',
        },
      ],
    );

    expect(sortContactsByRecentActivity(rows).map((c) => c.personUid)).toEqual([
      'prs_new',
      'prs_old',
    ]);
  });

  it('keeps a newer server-provided contact preview over older history', () => {
    const [row] = mergeContactPreviews(
      [
        contact('prs_ada', 'Ada', {
          lastMessageAt: '2026-06-13T11:00:00Z',
          lastMessageBody: 'Server wins',
          lastMessageDirection: 'out',
        }) as ContactPreviewFields,
      ],
      [
        {
          fromPersonUid: 'prs_ada',
          body: 'Older inbox copy',
          createdAt: '2026-06-13T10:00:00Z',
        },
      ],
    );

    expect(contactPreviewText(row)).toBe('You: Server wins');
  });

  it('builds a preview from the newest loaded thread message', () => {
    const preview = previewFromMessages([
      { body: 'Older', createdAt: '2026-06-13T10:00:00Z', direction: 'in' },
      { body: 'Newest', createdAt: '2026-06-13T11:00:00Z', direction: 'out' },
    ]);

    expect(preview).toEqual({
      body: 'Newest',
      createdAt: '2026-06-13T11:00:00Z',
      direction: 'out',
    });
  });
});

describe('mergeConversations', () => {
  const NOW = Date.parse('2026-06-13T12:00:00Z');

  it('interleaves channels and DMs by recency, newest first', () => {
    const merged = mergeConversations(
      [
        contact('prs_ada', 'Ada', { lastMessageAt: '2026-06-13T11:00:00Z' }),
        contact('prs_alan', 'Alan', { lastMessageAt: '2026-06-10T11:00:00Z' }),
      ],
      [
        channel('chn_crew', 'crew', { lastActivityAt: '2026-06-12T11:00:00Z' }),
      ],
      { now: NOW },
    );

    // Ada (06-13) > #crew (06-12) > Alan (06-10) — fully interleaved.
    expect(merged.map((m) => m.key)).toEqual(['dm:prs_ada', 'ch:chn_crew', 'dm:prs_alan']);
  });

  it('floats an unread, timestamp-less channel up among recent items', () => {
    const merged = mergeConversations(
      [contact('prs_old', 'Old', { lastMessageAt: '2026-06-01T00:00:00Z' })],
      [channel('chn_general', 'general', { unread: 3 })], // no timestamp, but unread
      { now: NOW },
    );

    // The unread channel (no timestamp) is treated as "now" and sorts above the
    // stale DM.
    expect(merged[0].key).toBe('ch:chn_general');
    expect(merged[0].unread).toBe(3);
    expect(merged[1].key).toBe('dm:prs_old');
  });

  it('sinks a read, timestamp-less channel below DMs', () => {
    const merged = mergeConversations(
      [contact('prs_recent', 'Recent', { lastMessageAt: '2026-06-13T11:00:00Z' })],
      [channel('chn_quiet', 'quiet', { unread: 0 })], // read + no timestamp → time 0
      { now: NOW },
    );

    expect(merged.map((m) => m.key)).toEqual(['dm:prs_recent', 'ch:chn_quiet']);
  });

  it('tags each item with its kind and carries the source object', () => {
    const merged = mergeConversations(
      [contact('prs_x', 'X', { lastMessageAt: '2026-06-13T00:00:00Z' })],
      [channel('chn_y', 'y', { lastActivityAt: '2026-06-12T00:00:00Z' })],
      { now: NOW },
    );

    const dm = merged.find((m) => m.kind === 'dm');
    const ch = merged.find((m) => m.kind === 'channel');
    expect(dm?.contact?.personUid).toBe('prs_x');
    expect(dm?.channel).toBeUndefined();
    expect(ch?.channel?.channelId).toBe('chn_y');
    expect(ch?.contact).toBeUndefined();
  });

  it('is stable for empty inputs', () => {
    expect(mergeConversations([], [], { now: NOW })).toEqual([]);
  });

  it('includes a group DM and orders it by createdAt when it has no activity stamp (REGRESSION)', () => {
    // Group DMs come back from the list endpoint with no lastMessageAt/lastActivityAt
    // and unread 0 — they used to collapse to time 0 and sink below every contact,
    // reading as "missing". They must still appear, ordered by their createdAt.
    const merged = mergeConversations(
      [
        contact('prs_old', 'Old', { lastMessageAt: '2026-06-01T00:00:00Z' }),
        contact('prs_new', 'New', { lastMessageAt: '2026-06-13T11:00:00Z' }),
      ],
      [channel('chn_grp', '', { createdAt: '2026-06-10T00:00:00Z' })],
      { now: NOW },
    );

    // The group DM is present, and createdAt (06-10) sorts it between the two DMs.
    expect(merged.map((m) => m.key)).toContain('ch:chn_grp');
    expect(merged.map((m) => m.key)).toEqual(['dm:prs_new', 'ch:chn_grp', 'dm:prs_old']);
  });
});
