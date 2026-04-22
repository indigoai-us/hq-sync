import { describe, expect, it } from 'vitest';
import { shouldSkipSignIn } from './auth';

describe('shouldSkipSignIn', () => {
  it('skips when get_auth_state says authenticated', () => {
    expect(shouldSkipSignIn(false, { authenticated: true })).toBe(true);
  });

  it('skips when a stored token is present even if get_auth_state is unauthenticated', () => {
    // get_auth_state's silent-refresh can flip to unauthenticated for a
    // network blip; stored-token presence is the documented skip signal.
    expect(shouldSkipSignIn(true, { authenticated: false })).toBe(true);
  });

  it('shows sign-in when neither signal fires', () => {
    expect(shouldSkipSignIn(false, { authenticated: false })).toBe(false);
  });

  it('skips when both signals fire (no regression)', () => {
    expect(shouldSkipSignIn(true, { authenticated: true })).toBe(true);
  });
});
