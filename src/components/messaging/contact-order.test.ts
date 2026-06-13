import { describe, expect, it } from 'vitest';
import { sortContactsByRecentActivity, type ContactRecencyFields } from './contact-order';

const contact = (
  personUid: string,
  displayName: string,
  overrides: Partial<ContactRecencyFields> = {},
): ContactRecencyFields => ({
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
