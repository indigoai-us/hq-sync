/**
 * Decide whether the sign-in step can be skipped. The onboarding contract:
 * presence of a non-empty cognito-tokens.json token is the signal to skip.
 * `get_auth_state` remains the source of truth when it returns authenticated,
 * and its verdict is preferred so a token that was just refreshed still sets
 * `expiresAt` for the UI. When it returns unauthenticated (e.g. silent
 * refresh failed), we fall through to raw token presence — if that stored
 * token turns out to be unusable, downstream sync flows surface
 * `sync:auth-error` and route the user back through sign-in.
 */
export function shouldSkipSignIn(
  hasToken: boolean,
  state: { authenticated: boolean },
): boolean {
  return state.authenticated || hasToken;
}
