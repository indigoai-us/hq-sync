import { describe, expect, it } from 'vitest';
import {
  contactPreviewAt,
  contactPreviewText,
  mergeContactPreviews,
  previewFromMessages,
  sortContactsByRecentActivity,
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
