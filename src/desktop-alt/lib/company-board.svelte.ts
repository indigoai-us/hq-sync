import { invoke } from '@tauri-apps/api/core';
import { companyStore } from './company-store.svelte';

export interface CompanyBoardCard {
  id: string;
  title: string;
  subtitle?: string | null;
  href?: string | null;
  labels?: string[];
  assigneeInitials?: string | null;
  tag?: string | null;
  age?: string | null;
  [key: string]: unknown;
}

export interface CompanyBoard {
  inbox: CompanyBoardCard[];
  doing: CompanyBoardCard[];
  review: CompanyBoardCard[];
  done: CompanyBoardCard[];
}

export const emptyCompanyBoard = (): CompanyBoard => ({
  inbox: [],
  doing: [],
  review: [],
  done: [],
});

// Normalize a raw board payload (from invoke or the warm cache) into the four
// always-present columns. Shared by the warm-read paint and the invoke commit
// so both go through identical shaping.
function shapeBoard(raw: CompanyBoard): CompanyBoard {
  return {
    inbox: raw.inbox ?? [],
    doing: raw.doing ?? [],
    review: raw.review ?? [],
    done: raw.done ?? [],
  };
}

export function useCompanyBoard(options: { slug: () => string | null; enabled?: () => boolean }) {
  let board = $state<CompanyBoard>(emptyCompanyBoard());
  let loading = $state(false);
  let error = $state<string | null>(null);
  let reloadToken = $state(0);

  $effect(() => {
    const slug = options.slug();
    const enabled = options.enabled?.() ?? true;
    reloadToken;
    board = emptyCompanyBoard();
    error = null;

    if (!slug || !enabled) {
      loading = false;
      return;
    }

    let cancelled = false;

    const warm = companyStore.board(slug);
    board = warm ? shapeBoard(warm) : emptyCompanyBoard();
    loading = warm === null;

    void invoke<CompanyBoard>('get_company_board', { slug })
      .then((result) => {
        if (!cancelled) {
          board = shapeBoard(result);
          companyStore.setBoard(slug, result);
        }
      })
      .catch((err) => {
        console.error('get_company_board failed:', err);
        if (!cancelled) {
          error = String(err);
          board = emptyCompanyBoard();
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
    get board() {
      return board;
    },
    get loading() {
      return loading;
    },
    get error() {
      return error;
    },
    retry() {
      reloadToken += 1;
    },
  };
}
