import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import {
  __INTERNALS__,
  clearMeetingsCache,
  getMeetingsCacheAgeMs,
  loadMeetingsCache,
  saveMeetingsCache,
  type MeetingsSnapshot,
} from './meetingsCache';

// vitest runs in node env (see vite.config.ts) — localStorage doesn't exist
// natively, so we stub a minimal in-memory implementation onto globalThis
// for each test. Mirrors the only three methods the cache module touches
// (`getItem` / `setItem` / `removeItem`), reset between tests so cases
// stay isolated and don't bleed cached entries into each other.
//
// We deliberately don't pull in jsdom or happy-dom just for this — they'd
// double the install footprint of a Tauri webview that ships its own
// localStorage at runtime, and the module's only DOM touchpoint is this
// flat key/value surface.

interface MemStorage {
  store: Map<string, string>;
  getItem(k: string): string | null;
  setItem(k: string, v: string): void;
  removeItem(k: string): void;
  clear(): void;
  readonly length: number;
  key(_: number): string | null;
}

function makeMemStorage(): MemStorage {
  const store = new Map<string, string>();
  return {
    store,
    getItem(k) {
      return store.get(k) ?? null;
    },
    setItem(k, v) {
      store.set(k, String(v));
    },
    removeItem(k) {
      store.delete(k);
    },
    clear() {
      store.clear();
    },
    get length() {
      return store.size;
    },
    key(_: number) {
      return null;
    },
  };
}

let mem: MemStorage;

// Type-only alias for the global. Avoids `declare global { var ... }`
// (which clashes with lib.dom's `localStorage: Storage`) and keeps every
// assignment + delete site cast-free.
const g = globalThis as unknown as { localStorage?: Storage };

beforeEach(() => {
  mem = makeMemStorage();
  g.localStorage = mem as unknown as Storage;
});

afterEach(() => {
  delete g.localStorage;
});

// Fixture used across multiple tests — exercises every Map / Set encoded
// field so the roundtrip really proves we can rebuild the live in-memory
// shape, not just shallow scalars.
function makeSnapshot(): MeetingsSnapshot<
  { id: string; summary: string },
  { botId: string; status: string },
  { accountId: string; email: string },
  { id: string; summary: string }
> {
  return {
    events: [
      { id: 'evt-1', summary: 'Standup' },
      { id: 'evt-2', summary: 'Demo' },
    ],
    botsByEventId: [['evt-1', { botId: 'b-1', status: 'recording' }]],
    companyNamesByUid: [['cmp_abc', 'Indigo']],
    accounts: [{ accountId: 'acct-1', email: 'a@example.com' }],
    accountEmailById: [['acct-1', 'a@example.com']],
    calendarsByAccount: [
      ['acct-1', [{ id: 'cal-primary', summary: 'a@example.com' }]],
    ],
    enabledCalIdsByAccount: [['acct-1', ['cal-primary', 'cal-team']]],
    calendarSummaryByKey: [['acct-1|cal-primary', 'a@example.com']],
  };
}

describe('meetingsCache', () => {
  describe('roundtrip', () => {
    it('returns null when nothing has been cached yet', () => {
      expect(loadMeetingsCache()).toBeNull();
    });

    it('saves and loads back the same snapshot shape', () => {
      const snap = makeSnapshot();
      saveMeetingsCache(snap);
      const loaded = loadMeetingsCache<
        { id: string; summary: string },
        { botId: string; status: string },
        { accountId: string; email: string },
        { id: string; summary: string }
      >();
      expect(loaded).not.toBeNull();
      expect(loaded?.events).toEqual(snap.events);
      expect(loaded?.botsByEventId).toEqual(snap.botsByEventId);
      expect(loaded?.companyNamesByUid).toEqual(snap.companyNamesByUid);
      expect(loaded?.accounts).toEqual(snap.accounts);
      expect(loaded?.accountEmailById).toEqual(snap.accountEmailById);
      expect(loaded?.calendarsByAccount).toEqual(snap.calendarsByAccount);
      expect(loaded?.enabledCalIdsByAccount).toEqual(snap.enabledCalIdsByAccount);
      expect(loaded?.calendarSummaryByKey).toEqual(snap.calendarSummaryByKey);
    });

    it('can rebuild Maps and Sets from the serialized arrays', () => {
      // Sanity that the [k, v][] shape this module returns plugs directly
      // into `new Map(...)` and `new Set(...)` the way MeetingsWindow uses
      // it at script-init. Catches the silent-failure case where Map
      // serialization regresses to `JSON.stringify(new Map())` -> "{}".
      const snap = makeSnapshot();
      saveMeetingsCache(snap);
      const loaded = loadMeetingsCache()!;
      const bots = new Map(loaded.botsByEventId as Array<[string, unknown]>);
      const calIds = new Map(
        (loaded.enabledCalIdsByAccount as Array<[string, string[]]>).map(
          ([acct, ids]) => [acct, new Set(ids)],
        ),
      );
      expect(bots.get('evt-1')).toEqual({ botId: 'b-1', status: 'recording' });
      expect(calIds.get('acct-1')?.has('cal-team')).toBe(true);
      expect(calIds.get('acct-1')?.has('cal-other')).toBe(false);
    });
  });

  describe('invalidation', () => {
    it('returns null when the schema version does not match', () => {
      // Hand-write an envelope with a bogus version so we exercise the
      // version-mismatch path without bumping SCHEMA_VERSION in production.
      mem.setItem(
        __INTERNALS__.STORAGE_KEY,
        JSON.stringify({
          version: __INTERNALS__.SCHEMA_VERSION + 99,
          cachedAt: Date.now(),
          snapshot: makeSnapshot(),
        }),
      );
      expect(loadMeetingsCache()).toBeNull();
    });

    it('returns null when the entry is older than MAX_AGE_MS', () => {
      mem.setItem(
        __INTERNALS__.STORAGE_KEY,
        JSON.stringify({
          version: __INTERNALS__.SCHEMA_VERSION,
          // 1ms past the cap — proves the boundary check is `>` not `>=`
          // doesn't matter, but proves we actually walk the timestamp.
          cachedAt: Date.now() - __INTERNALS__.MAX_AGE_MS - 1,
          snapshot: makeSnapshot(),
        }),
      );
      expect(loadMeetingsCache()).toBeNull();
    });

    it('returns null on corrupt JSON', () => {
      mem.setItem(__INTERNALS__.STORAGE_KEY, '{not json');
      expect(loadMeetingsCache()).toBeNull();
    });

    it('returns null when envelope is missing cachedAt', () => {
      mem.setItem(
        __INTERNALS__.STORAGE_KEY,
        JSON.stringify({
          version: __INTERNALS__.SCHEMA_VERSION,
          snapshot: makeSnapshot(),
        }),
      );
      expect(loadMeetingsCache()).toBeNull();
    });

    it('clearMeetingsCache removes the entry', () => {
      saveMeetingsCache(makeSnapshot());
      expect(loadMeetingsCache()).not.toBeNull();
      clearMeetingsCache();
      expect(loadMeetingsCache()).toBeNull();
    });
  });

  describe('localStorage shim', () => {
    it('does not throw when localStorage is undefined', () => {
      // Drop the stub mid-test to simulate a privacy-mode browser /
      // pre-mount node context — the module's `typeof localStorage`
      // guard should short-circuit every accessor.
      delete g.localStorage;
      expect(() => saveMeetingsCache(makeSnapshot())).not.toThrow();
      expect(loadMeetingsCache()).toBeNull();
      expect(getMeetingsCacheAgeMs()).toBeNull();
      expect(() => clearMeetingsCache()).not.toThrow();
    });

    it('swallows setItem errors so refresh path never breaks', () => {
      // Simulate a quota-exceeded / SecurityError throw on write —
      // proves the saveMeetingsCache try/catch keeps the live refresh
      // codepath safe even when the underlying storage rejects us.
      g.localStorage = {
        ...mem,
        setItem: () => {
          throw new Error('QuotaExceededError');
        },
      } as unknown as Storage;
      expect(() => saveMeetingsCache(makeSnapshot())).not.toThrow();
    });
  });

  describe('getMeetingsCacheAgeMs', () => {
    it('returns null when there is no cached entry', () => {
      expect(getMeetingsCacheAgeMs()).toBeNull();
    });

    it('returns a non-negative number after a save', () => {
      saveMeetingsCache(makeSnapshot());
      const age = getMeetingsCacheAgeMs();
      expect(age).not.toBeNull();
      expect(age!).toBeGreaterThanOrEqual(0);
      expect(age!).toBeLessThan(1000); // brand new — should be near zero
    });
  });

  describe('storage key', () => {
    it('is namespaced with the schema version', () => {
      // Keeps `vN` in the key so a future SCHEMA_VERSION bump doesn't have
      // to remember to manually evict the old payload — old key just rots
      // until the browser GCs it.
      expect(__INTERNALS__.STORAGE_KEY).toContain(
        `v${__INTERNALS__.SCHEMA_VERSION}`,
      );
      expect(__INTERNALS__.STORAGE_KEY).toMatch(/^hq-sync:meetings-window:v\d+$/);
    });
  });
});
