/**
 * Per-window localStorage cache for the Meetings window.
 *
 * The Meetings window historically fetched four Tauri commands in parallel
 * on mount (`meetings_list_upcoming`, `_scheduled_bots`, `_memberships`,
 * `_accounts`) plus a per-account calendar fan-out, then flipped a skeleton
 * to the populated list. On a typical install that's a noticeable hang —
 * users see "loading…" for a few hundred ms each time they reopen the
 * window, even though the data they saw on last open was almost certainly
 * still valid.
 *
 * This module is the stale-while-revalidate seam: snapshot last-known state
 * on every successful refresh, replay it synchronously at next script-init
 * so the first paint already has rows, then let the in-flight refresh swap
 * in fresh data when it lands. Maps and Sets are serialized as arrays so
 * the cached payload survives `JSON.stringify`.
 *
 * Kept pure (no Tauri/Svelte imports) so it can be unit-tested in isolation
 * and so the hydration path can't fail in a way that takes the whole window
 * down — every accessor swallows `localStorage` errors and returns null,
 * which the caller treats as "no cache, render the normal skeleton".
 */

/** Bump on every breaking change to the cached shape so old entries from a
 *  previous app version are treated as a cache miss instead of crashing
 *  the hydration path with a shape mismatch. */
const SCHEMA_VERSION = 1;

/** localStorage key. Namespaced with the schema version so a future bump
 *  doesn't have to manually delete the prior entry — old keys just rot
 *  harmlessly until the browser evicts them. */
const STORAGE_KEY = `hq-sync:meetings-window:v${SCHEMA_VERSION}`;

/** Hard upper bound on cache age. Past this we ignore the cache and let
 *  the normal cold-start skeleton render — better than showing meetings
 *  from yesterday when the user opens the window after a weekend. Refresh
 *  still runs in parallel so the user sees fresh data within a beat. */
const MAX_AGE_MS = 24 * 60 * 60 * 1000;

/** Shape of one snapshot. Mirrors the `$state` variables in
 *  MeetingsWindow.svelte that `refresh()` populates. Maps and Sets are
 *  encoded as their `Array.from()` form so `JSON.stringify` roundtrips
 *  cleanly — `JSON.stringify(new Map())` returns `"{}"`, which is useless. */
export interface MeetingsSnapshot<
  TEvent = unknown,
  TBot = unknown,
  TAccount = unknown,
  TCalendar = unknown,
> {
  events: TEvent[];
  /** [calendarEventId, bot] entries — deserialized into a Map by the caller. */
  botsByEventId: Array<[string, TBot]>;
  /** [companyUid, companyName] entries. */
  companyNamesByUid: Array<[string, string]>;
  accounts: TAccount[];
  /** [accountId, email] entries. */
  accountEmailById: Array<[string, string]>;
  /** [accountId, calendars] entries. */
  calendarsByAccount: Array<[string, TCalendar[]]>;
  /** [accountId, calendarIds] entries — calendarIds is the Set encoded
   *  as a plain array. */
  enabledCalIdsByAccount: Array<[string, string[]]>;
  /** [calKey, summary] entries — calKey is `${accountId}|${calendarId}`. */
  calendarSummaryByKey: Array<[string, string]>;
}

interface CacheEnvelope<TSnap> {
  version: number;
  cachedAt: number;
  snapshot: TSnap;
}

/**
 * Read the last snapshot. Returns null on any failure path so callers can
 * treat hydration as best-effort — a corrupt entry, a privacy-mode browser
 * with localStorage disabled, or a schema-version mismatch all collapse to
 * the same "no cache" outcome and the normal cold-start skeleton renders.
 */
export function loadMeetingsCache<
  TEvent = unknown,
  TBot = unknown,
  TAccount = unknown,
  TCalendar = unknown,
>(): MeetingsSnapshot<TEvent, TBot, TAccount, TCalendar> | null {
  try {
    const raw = safeGetItem(STORAGE_KEY);
    if (!raw) return null;
    const parsed = JSON.parse(raw) as CacheEnvelope<
      MeetingsSnapshot<TEvent, TBot, TAccount, TCalendar>
    >;
    if (!parsed || typeof parsed !== 'object') return null;
    if (parsed.version !== SCHEMA_VERSION) return null;
    if (typeof parsed.cachedAt !== 'number') return null;
    if (Date.now() - parsed.cachedAt > MAX_AGE_MS) return null;
    if (!parsed.snapshot || typeof parsed.snapshot !== 'object') return null;
    return parsed.snapshot;
  } catch {
    return null;
  }
}

/**
 * Write the snapshot. Best-effort — silently swallows quota errors and
 * other localStorage failures so a write failure can never break the
 * refresh path (the user still sees the freshly-loaded data, they just
 * won't get the cache benefit on next open).
 */
export function saveMeetingsCache<
  TEvent = unknown,
  TBot = unknown,
  TAccount = unknown,
  TCalendar = unknown,
>(snapshot: MeetingsSnapshot<TEvent, TBot, TAccount, TCalendar>): void {
  try {
    const envelope: CacheEnvelope<typeof snapshot> = {
      version: SCHEMA_VERSION,
      cachedAt: Date.now(),
      snapshot,
    };
    safeSetItem(STORAGE_KEY, JSON.stringify(envelope));
  } catch {
    // No-op — see function-level comment.
  }
}

/** Wipe the cached snapshot. Exposed for tests and for any future
 *  "sign out" path that needs to drop user-scoped data. */
export function clearMeetingsCache(): void {
  try {
    safeRemoveItem(STORAGE_KEY);
  } catch {
    // No-op.
  }
}

/**
 * Hours-since-cache helper — useful for tests, for future telemetry, or
 * for a debug surface that shows "cache age" in the diagnostics drawer.
 * Returns null when there is no cached entry.
 */
export function getMeetingsCacheAgeMs(): number | null {
  try {
    const raw = safeGetItem(STORAGE_KEY);
    if (!raw) return null;
    const parsed = JSON.parse(raw) as { cachedAt?: unknown };
    if (typeof parsed?.cachedAt !== 'number') return null;
    return Date.now() - parsed.cachedAt;
  } catch {
    return null;
  }
}

// ─────────────────────────────────────────────────────────────────────────
// localStorage shims
//
// Wrap localStorage access so the module never throws on the server (vitest
// `jsdom` env has localStorage, but node-env tests won't) and so we always
// have a single chokepoint to swallow `SecurityError` / `QuotaExceededError`
// — the two common failure modes when running inside a hardened webview.
// ─────────────────────────────────────────────────────────────────────────

function safeGetItem(key: string): string | null {
  if (typeof localStorage === 'undefined') return null;
  return localStorage.getItem(key);
}

function safeSetItem(key: string, value: string): void {
  if (typeof localStorage === 'undefined') return;
  localStorage.setItem(key, value);
}

function safeRemoveItem(key: string): void {
  if (typeof localStorage === 'undefined') return;
  localStorage.removeItem(key);
}

/** Exposed for tests — the storage key isn't part of the public API but
 *  tests need to assert on it (and on the schema-version namespacing). */
export const __INTERNALS__ = {
  STORAGE_KEY,
  SCHEMA_VERSION,
  MAX_AGE_MS,
};
