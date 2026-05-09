/**
 * Auto-sync (Beta) feature flag.
 *
 * The toggle in Settings is hidden unless the signed-in user's email ends in
 * `@getindigo.ai`. The check is case-insensitive (Cognito stores casing
 * verbatim, so users may sign in with mixed case) and is a strict suffix
 * match — substring matches like `@notgetindigo.ai` must NOT pass.
 *
 * This is a UI affordance, not a security gate: anyone with devtools can
 * call the underlying Tauri command directly. Fine for an internal beta;
 * promote to a backend check before turning the feature on for non-Indigo
 * accounts.
 */
const ALLOWED_DOMAIN = 'getindigo.ai';

export function canEnableRealtimeSync(email: string | null): boolean {
  if (email == null) return false;
  // Strict: no trim. Whitespace in a Cognito claim is a data issue we want
  // to surface, not silently tolerate.
  if (email !== email.trim()) return false;
  if (email.length === 0) return false;

  const at = email.lastIndexOf('@');
  if (at < 0) return false;
  const domain = email.slice(at + 1).toLowerCase();
  return domain === ALLOWED_DOMAIN;
}
