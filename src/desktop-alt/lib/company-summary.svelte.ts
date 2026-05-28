import { invoke } from '@tauri-apps/api/core';

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

export function useCompanySummary(options: { slug: () => string | null }) {
  let summary = $state<CompanySummary>(emptyCompanySummary());
  let loading = $state(false);
  let error = $state<string | null>(null);

  $effect(() => {
    const slug = options.slug();
    summary = emptyCompanySummary();
    error = null;

    if (!slug) {
      loading = false;
      return;
    }

    let cancelled = false;
    loading = true;

    void invoke<CompanySummary>('get_company_summary', { slug })
      .then((result) => {
        if (!cancelled) {
          summary = result;
        }
      })
      .catch((err) => {
        console.error('get_company_summary failed:', err);
        if (!cancelled) {
          error = String(err);
          summary = emptyCompanySummary();
        }
      })
      .finally(() => {
        if (!cancelled) {
          loading = false;
        }
      });

    return () => {
      cancelled = true;
    };
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
