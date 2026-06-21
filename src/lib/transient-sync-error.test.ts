/**
 * Regression for HQ-SYNC-WEB-18 ("[sync] personal first-push failed: list person
 * entities: transport error: error sending request for url (…)", error level,
 * captured by App.svelte's `sync:error` defence-in-depth handler).
 *
 * A transport-level reqwest failure on a background sync step (the request never
 * reached a response) is recoverable noise and must NOT capture as a Sentry
 * error — but the matcher must stay NARROW so a genuine, server-answered failure
 * still surfaces.
 */
import { describe, it, expect } from 'vitest';
import { isTransientSyncTransportError } from './transient-sync-error';

describe('isTransientSyncTransportError', () => {
  it('flags the HQ-SYNC-WEB-18 personal first-push transport error', () => {
    expect(
      isTransientSyncTransportError(
        'personal first-push failed: list person entities: transport error: error sending request for url (https://hqapi.getindigo.ai/entity/by-type/person)',
      ),
    ).toBe(true);
  });

  it('flags a bare reqwest "error sending request" message', () => {
    expect(
      isTransientSyncTransportError(
        'error sending request for url (https://hqapi.getindigo.ai/sts/vend-self)',
      ),
    ).toBe(true);
  });

  it('flags the "transport error" marker on its own', () => {
    expect(isTransientSyncTransportError('transport error: connection reset')).toBe(
      true,
    );
  });

  it('is case-insensitive', () => {
    expect(isTransientSyncTransportError('Transport Error: dns error')).toBe(true);
  });

  // ── Negatives: the swallow must stay narrow ──────────────────────────────
  it('does NOT flag a server-answered HTTP failure', () => {
    expect(
      isTransientSyncTransportError(
        'personal first-push failed: list person entities: HTTP 500 Internal Server Error',
      ),
    ).toBe(false);
  });

  it('does NOT flag a forbidden / auth failure the server actually returned', () => {
    expect(
      isTransientSyncTransportError('first-push failed: 403 Forbidden'),
    ).toBe(false);
  });

  it('does NOT flag an unrelated logic error', () => {
    expect(
      isTransientSyncTransportError('first-push failed: serialize body: invalid utf-8'),
    ).toBe(false);
  });
});
