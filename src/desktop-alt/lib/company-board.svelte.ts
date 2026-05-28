import { invoke } from '@tauri-apps/api/core';

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

export function useCompanyBoard(options: { slug: () => string | null }) {
  let board = $state<CompanyBoard>(emptyCompanyBoard());
  let loading = $state(false);
  let error = $state<string | null>(null);
  let reloadToken = $state(0);

  $effect(() => {
    const slug = options.slug();
    reloadToken;
    board = emptyCompanyBoard();
    error = null;

    if (!slug) {
      loading = false;
      return;
    }

    let cancelled = false;
    loading = true;

    void invoke<CompanyBoard>('get_company_board', { slug })
      .then((result) => {
        if (!cancelled) {
          board = {
            inbox: result.inbox ?? [],
            doing: result.doing ?? [],
            review: result.review ?? [],
            done: result.done ?? [],
          };
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
