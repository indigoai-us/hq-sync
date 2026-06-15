/**
 * Canonical HQ web-console URLs — the single source of truth for every external
 * link the desktop window opens into the HQ console.
 *
 * Centralised so the "all links resolve to the right place" guarantee lives in
 * one file rather than scattered string literals (CompanyPage, MeetingsPage,
 * MarketplacePanel, the shell's secondary-sidebar footer all consume these).
 * Every link opens in the system browser via `@tauri-apps/plugin-shell`'s
 * `open()`.
 */

/** Production HQ console host. */
export const HQ_CONSOLE_BASE = 'https://hq.getindigo.ai';

/** A company's console home — also its settings / admin surface. */
export function companyConsoleUrl(slug: string): string {
  return `${HQ_CONSOLE_BASE}/${encodeURIComponent(slug)}`;
}

/**
 * Company settings live in the console (sync rules, members, roles) — the
 * console company page is that surface, so this is an alias of
 * {@link companyConsoleUrl} with an intent-revealing name at the call site.
 */
export function companySettingsUrl(slug: string): string {
  return companyConsoleUrl(slug);
}

/** Company invite flow in the console. */
export function companyInviteUrl(slug: string): string {
  return `${companyConsoleUrl(slug)}/invite`;
}

/** Console Integrations page (calendar / meeting-bot connect). */
export const HQ_CONSOLE_INTEGRATIONS_URL = `${HQ_CONSOLE_BASE}/integrations`;

/** Console creators index. */
export const HQ_CONSOLE_CREATORS_URL = `${HQ_CONSOLE_BASE}/creators`;

/** A single creator's public profile in the console. */
export function creatorProfileUrl(handle: string): string {
  return `${HQ_CONSOLE_CREATORS_URL}/${encodeURIComponent(handle)}`;
}
