import { describe, it, expect } from 'vitest';

import {
  HQ_CONSOLE_BASE,
  companyConsoleUrl,
  companySettingsUrl,
  companyInviteUrl,
  HQ_CONSOLE_INTEGRATIONS_URL,
  HQ_CONSOLE_CREATORS_URL,
  creatorProfileUrl,
} from './hq-console';

describe('hq-console URLs', () => {
  // Regression: company links must carry the `/companies/` path segment. The
  // console namespaces every company surface under `/companies/{slug}`; a link
  // to `${HQ_CONSOLE_BASE}/${slug}` 404s. (Settings button shipped broken.)
  it('company console home includes the /companies/ prefix', () => {
    expect(companyConsoleUrl('indigo')).toBe(`${HQ_CONSOLE_BASE}/companies/indigo`);
  });

  it('company console home is NEVER the bare /{slug} form (the bug)', () => {
    expect(companyConsoleUrl('indigo')).not.toBe(`${HQ_CONSOLE_BASE}/indigo`);
    expect(companyConsoleUrl('indigo')).toContain('/companies/');
  });

  it('settings link points to the dedicated /companies/{slug}/settings page', () => {
    expect(companySettingsUrl('indigo')).toBe(
      `${HQ_CONSOLE_BASE}/companies/indigo/settings`,
    );
    expect(companySettingsUrl('indigo')).toContain('/companies/');
  });

  it('invite link points to the company Team → Invites surface', () => {
    expect(companyInviteUrl('indigo')).toBe(
      `${HQ_CONSOLE_BASE}/companies/indigo/team/invites`,
    );
    expect(companyInviteUrl('indigo')).toContain('/companies/');
  });

  it('encodes slugs that need escaping', () => {
    expect(companyConsoleUrl('a b/c')).toBe(
      `${HQ_CONSOLE_BASE}/companies/a%20b%2Fc`,
    );
    expect(companySettingsUrl('a b')).toBe(
      `${HQ_CONSOLE_BASE}/companies/a%20b/settings`,
    );
  });

  it('non-company console links are unchanged', () => {
    expect(HQ_CONSOLE_INTEGRATIONS_URL).toBe(`${HQ_CONSOLE_BASE}/integrations`);
    expect(HQ_CONSOLE_CREATORS_URL).toBe(`${HQ_CONSOLE_BASE}/creators`);
    expect(creatorProfileUrl('jane')).toBe(`${HQ_CONSOLE_BASE}/creators/jane`);
  });
});
