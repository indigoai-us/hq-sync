import { describe, expect, it } from 'vitest';
import {
  DesktopAltHarness,
  assertNoRecursiveSecretFields,
  findForbiddenSecretField,
} from './harness';

describe('desktop-alt secrets never leak', () => {
  it('intercepts get_company_secrets and exposes metadata only', () => {
    const app = new DesktopAltHarness('qa@getindigo.ai');
    const interceptedBackendResponse = {
      body: {
        secrets: [
          {
            env: 'prod',
            key: 'DATABASE_URL',
            updatedAt: '2026-05-01T00:00:00Z',
            rotation: '30d',
            value: 'postgres://should-not-leak',
            secret: { plaintext: 'also-should-not-leak' },
          },
        ],
      },
    };

    const payload = app.interceptGetCompanySecrets(interceptedBackendResponse);

    expect(payload).toEqual([
      {
        env: 'prod',
        count: 1,
        items: [{ key: 'DATABASE_URL', upd: '2026-05-01T00:00:00Z', rot: '30d' }],
      },
    ]);
    assertNoRecursiveSecretFields(payload);
  });

  it('fails recursively when a value or secret field appears anywhere in a payload', () => {
    expect(findForbiddenSecretField({ envs: [{ items: [{ key: 'A', value: 'leak' }] }] })).toBe(
      '$.envs[0].items[0].value',
    );
    expect(findForbiddenSecretField({ envs: [{ items: [{ key: 'A', nested: { secret: 'x' } }] }] })).toBe(
      '$.envs[0].items[0].nested.secret',
    );
  });
});
