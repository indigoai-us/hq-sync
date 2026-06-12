import { invoke } from '@tauri-apps/api/core';
import { companyStore } from './company-store.svelte';

export interface CompanySummary {
  board: number;
  activity: {
    last7d: number;
  };
  deployments: number;
  secrets: number;
}

export const emptyCompanySummary = (): CompanySummary => ({
  board: 0,
  activity: {
    last7d: 0,
  },
  deployments: 0,
  secrets: 0,
});

export function useCompanySummary(options: { slug: () => string | null; enabled?: () => boolean }) {
  let summary = $state<CompanySummary>(emptyCompanySummary());
  let loading = $state(false);
  let error = $state<string | null>(null);

  // The effect tracks `options.slug()` (i.e. `company.slug`), which re-fires
  // whenever the parent hands down a NEW `company` object — even when the slug
  // VALUE is unchanged. The desktop shell reassigns the whole workspace list on
  // every focus/refresh, so `page.activeCompany` churns identity constantly.
  // We must NOT cancel + refetch on those churns: the previous structure tied a
  // `cancelled` flag to the effect cleanup, so each churn cancelled the
  // in-flight `get_company_summary` before its (correct) counts could commit —
  // leaving the UI stuck on zeros while the backend returned real data.
  //
  // Fix: only react to an actual slug-VALUE change. A monotonic request id (not
  // an effect-cleanup flag) discards out-of-order completions when the slug
  // changes rapidly, so the loaded summary sticks across identity churn.
  let activeSlug: string | null = null;
  let requestId = 0;

  $effect(() => {
    const slug = options.slug();
    const enabled = options.enabled?.() ?? true;
    if (!enabled) {
      activeSlug = slug;
      requestId += 1;
      summary = emptyCompanySummary();
      error = null;
      loading = false;
      return;
    }
    if (slug === activeSlug) {
      return; // same company — keep the loaded summary, don't refetch/cancel
    }
    activeSlug = slug;
    const myRequest = ++requestId;

    summary = emptyCompanySummary();
    error = null;

    if (!slug) {
      loading = false;
      return;
    }

    const warm = companyStore.summary(slug);
    summary = warm ?? emptyCompanySummary();
    loading = warm === null;

    void invoke<CompanySummary>('get_company_summary', { slug })
      .then((result) => {
        if (myRequest === requestId) {
          summary = result;
          companyStore.setSummary(slug, result);
        }
      })
      .catch((err) => {
        console.error('get_company_summary failed:', err);
        if (myRequest === requestId) {
          error = String(err);
          summary = emptyCompanySummary();
        }
      })
      .finally(() => {
        if (myRequest === requestId) {
          loading = false;
        }
      });
  });

  return {
    get summary() {
      return summary;
    },
    get loading() {
      return loading;
    },
    get error() {
      return error;
    },
  };
}
