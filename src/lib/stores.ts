// Svelte stores for cross-component state.
//
// `companiesState` tracks the per-company list surfaced in the popover
// (rendered as one `CompanyRow` each). Shape is deliberately flat so
// `{#each $companiesState.companies}` reads cleanly in templates.
//
// Per-slug transient state (promoting / lastError) is kept alongside the
// snapshot so CompanyRow doesn't have to thread multiple stores. A plain
// `Set<string>` is enough for `promoting`: mutation goes through `update`
// with a fresh Set so Svelte's reactivity picks it up.
import { writable } from 'svelte/store';

export interface CompanyInfo {
  slug: string;
  name: string;
  uid: string | null;
  source: 'aws' | 'local' | 'both';
}

export interface CompaniesState {
  companies: CompanyInfo[];
  loading: boolean;
  error?: string;
  /** Slugs currently promoting. First-seen wins to dedupe the double
   *  `promote:start` (synchronous + runner-streamed — see US-005 PRD). */
  promoting: Set<string>;
  /** Last promote:error message per slug, cleared on retry start. */
  lastError: Map<string, string>;
  /** ISO-8601 `lastSync` per company slug, seeded from the journal scan on
   *  mount and then stamped per-slug by `sync:complete` events. Plain
   *  object (not Map) so `$companiesState.lastSyncedPerSlug[slug]` reads
   *  cleanly from Svelte templates without a `.get()` wrapper. */
  lastSyncedPerSlug: Record<string, string>;
}

function createCompaniesStore() {
  const initial: CompaniesState = {
    companies: [],
    loading: true,
    error: undefined,
    promoting: new Set(),
    lastError: new Map(),
    lastSyncedPerSlug: {},
  };
  const { subscribe, set, update } = writable<CompaniesState>(initial);

  return {
    subscribe,
    set,
    update,

    /** Snapshot from `list_all_companies`. Resets loading + error. */
    setCompanies(companies: CompanyInfo[]) {
      update((s) => ({ ...s, companies, loading: false, error: undefined }));
    },

    /** Surface a backend error. UI treats this as terminal for the load. */
    setError(message: string) {
      update((s) => ({ ...s, loading: false, error: message }));
    },

    /** Optimistic start: mark a slug as promoting. First-seen wins — if
     *  already in flight, the second `promote:start` is a no-op (dedupes
     *  the synchronous-vs-streamed double-emit). */
    startPromote(slug: string) {
      update((s) => {
        if (s.promoting.has(slug)) return s;
        const promoting = new Set(s.promoting);
        promoting.add(slug);
        const lastError = new Map(s.lastError);
        lastError.delete(slug);
        return { ...s, promoting, lastError };
      });
    },

    /** Terminal: clear promoting flag (success or failure). */
    endPromote(slug: string) {
      update((s) => {
        if (!s.promoting.has(slug)) return s;
        const promoting = new Set(s.promoting);
        promoting.delete(slug);
        return { ...s, promoting };
      });
    },

    /** On `promote:complete`: update the row's source + uid in-place so the
     *  badge flips from 'Local' → 'Synced' without a full re-fetch. */
    markPromoted(slug: string, uid: string) {
      update((s) => {
        const companies = s.companies.map((c) =>
          c.slug === slug ? { ...c, uid, source: 'both' as const } : c
        );
        const promoting = new Set(s.promoting);
        promoting.delete(slug);
        return { ...s, companies, promoting };
      });
    },

    /** On `promote:error`: record message + clear promoting flag so the
     *  row can render an inline error + Retry button. */
    setPromoteError(slug: string, message: string) {
      update((s) => {
        const promoting = new Set(s.promoting);
        promoting.delete(slug);
        const lastError = new Map(s.lastError);
        lastError.set(slug, message);
        return { ...s, promoting, lastError };
      });
    },

    /** Clear the last error for a slug (called when the user clicks Retry). */
    clearPromoteError(slug: string) {
      update((s) => {
        if (!s.lastError.has(slug)) return s;
        const lastError = new Map(s.lastError);
        lastError.delete(slug);
        return { ...s, lastError };
      });
    },

    /** Seed from the `list_sync_journals` mount-time scan. Replaces the
     *  whole map — stale entries (e.g. a company the user left) shouldn't
     *  linger between mounts. */
    setLastSyncedMap(map: Record<string, string>) {
      update((s) => ({ ...s, lastSyncedPerSlug: { ...map } }));
    },

    /** Stamp one slug with a fresh ISO timestamp — called on per-company
     *  `sync:complete`. Immediate UI update without waiting for the journal
     *  round-trip. */
    updateLastSynced(slug: string, iso: string) {
      update((s) => ({
        ...s,
        lastSyncedPerSlug: { ...s.lastSyncedPerSlug, [slug]: iso },
      }));
    },

    /** Test-only reset. */
    reset() {
      set({
        companies: [],
        loading: true,
        error: undefined,
        promoting: new Set(),
        lastError: new Map(),
        lastSyncedPerSlug: {},
      });
    },
  };
}

export const companiesState = createCompaniesStore();
