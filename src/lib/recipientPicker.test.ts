import { describe, expect, it } from 'vitest';
import {
  buildSuggestions,
  flattenRows,
  isValidEmail,
  matchesQuery,
  type ContactLike,
} from './recipientPicker';

describe('isValidEmail', () => {
  it('accepts well-formed addresses', () => {
    expect(isValidEmail('a@b.com')).toBe(true);
    expect(isValidEmail('  ada.lovelace@getindigo.ai  ')).toBe(true);
  });
  it('rejects malformed strings', () => {
    expect(isValidEmail('')).toBe(false);
    expect(isValidEmail('ada')).toBe(false);
    expect(isValidEmail('ada@local')).toBe(false); // no dotted domain
    expect(isValidEmail('a b@c.com')).toBe(false); // space
    expect(isValidEmail('a@@b.com')).toBe(false);
  });
});

describe('matchesQuery', () => {
  const c: ContactLike = {
    personUid: 'prs_1',
    email: 'ada@getindigo.ai',
    displayName: 'Ada Lovelace',
  };
  it('matches on name and email substrings, case-insensitive', () => {
    expect(matchesQuery(c, 'ada')).toBe(true);
    expect(matchesQuery(c, 'LOVE')).toBe(true);
    expect(matchesQuery(c, 'getindigo')).toBe(true);
  });
  it('empty query matches everything', () => {
    expect(matchesQuery(c, '   ')).toBe(true);
  });
  it('non-matching query returns false', () => {
    expect(matchesQuery(c, 'zzz')).toBe(false);
  });
});

const contacts: ContactLike[] = [
  {
    personUid: 'prs_ada',
    email: 'ada@getindigo.ai',
    displayName: 'Ada Lovelace',
    connectionState: 'active',
  },
];

const membersByCompany: Record<string, ContactLike[]> = {
  ent_acme: [
    {
      personUid: 'prs_grace',
      email: 'grace@acme.com',
      displayName: 'Grace Hopper',
      companyUid: 'ent_acme',
      connectionState: 'none',
    },
    // Ada is also an Acme member, but already shown under Contacts → must dedupe.
    {
      personUid: 'prs_ada',
      email: 'ada@getindigo.ai',
      displayName: 'Ada Lovelace',
      companyUid: 'ent_acme',
      connectionState: 'active',
    },
  ],
};

const companies = [{ companyUid: 'ent_acme', companyName: 'Acme Inc.' }];

describe('buildSuggestions', () => {
  it('groups contacts first, then per-company members labeled "From {name}"', () => {
    const groups = buildSuggestions({
      query: '',
      contacts,
      membersByCompany,
      companies,
    });
    expect(groups[0].label).toBe('Contacts');
    expect(groups[0].rows[0].recipient.personUid).toBe('prs_ada');
    const companyGroup = groups.find((g) => g.key === 'company:ent_acme');
    expect(companyGroup?.label).toBe('From Acme Inc.');
    // Grace appears, Ada does NOT (deduped against the Contacts group).
    const uids = companyGroup?.rows.map((r) => r.recipient.personUid);
    expect(uids).toContain('prs_grace');
    expect(uids).not.toContain('prs_ada');
  });

  it('filters company members by a partial name query', () => {
    const groups = buildSuggestions({
      query: 'grac',
      contacts,
      membersByCompany,
      companies,
    });
    const rows = flattenRows(groups);
    expect(rows).toHaveLength(1);
    expect(rows[0].recipient.personUid).toBe('prs_grace');
  });

  it('offers a free-text "Send to {email}" row for a valid unknown email', () => {
    const groups = buildSuggestions({
      query: 'stranger@elsewhere.com',
      contacts,
      membersByCompany,
      companies,
    });
    const free = groups.find((g) => g.key === 'freetext');
    expect(free).toBeDefined();
    expect(free?.rows[0].primary).toBe('Send to stranger@elsewhere.com');
    expect(free?.rows[0].recipient.email).toBe('stranger@elsewhere.com');
    expect(free?.rows[0].recipient.connectionState).toBe('none');
    expect(free?.rows[0].freeText).toBe(true);
  });

  it('does NOT offer a free-text row when the email already matches a contact', () => {
    const groups = buildSuggestions({
      query: 'ada@getindigo.ai',
      contacts,
      membersByCompany,
      companies,
    });
    expect(groups.find((g) => g.key === 'freetext')).toBeUndefined();
  });

  it('does NOT offer a free-text row for an invalid email', () => {
    const groups = buildSuggestions({
      query: 'not-an-email',
      contacts,
      membersByCompany,
      companies,
    });
    expect(groups.find((g) => g.key === 'freetext')).toBeUndefined();
  });

  it('defaults an absent connectionState to "none"', () => {
    const groups = buildSuggestions({
      query: '',
      contacts: [{ personUid: 'prs_x', email: 'x@y.com', displayName: 'X' }],
      membersByCompany: {},
      companies: [],
    });
    expect(groups[0].rows[0].recipient.connectionState).toBe('none');
  });
});
