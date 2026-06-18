import { invoke } from '@tauri-apps/api/core';
import type { AgencyTeam, AgencyQuestion } from './agency';

// ---------------------------------------------------------------------------
// Agency store (Mission Control). Module-level runes singleton — the same shape
// as sessions-store. There is no backend poll event for the agency surface, so
// this drives a light JS interval refresh (the data is just on-disk chat files).
// Consumers read the reactive getters inside their own $derived/template.
// ---------------------------------------------------------------------------

let teams = $state<AgencyTeam[]>([]);
let questions = $state<AgencyQuestion[]>([]);
let loading = $state(true);
let error = $state('');

let started = false;
let timer: ReturnType<typeof setInterval> | null = null;

const REFRESH_MS = 4000;

async function refresh(): Promise<void> {
  try {
    const [t, q] = await Promise.all([
      invoke<AgencyTeam[]>('list_agency_teams'),
      invoke<AgencyQuestion[]>('list_agency_questions'),
    ]);
    teams = t ?? [];
    questions = q ?? [];
    error = '';
    loading = false;
  } catch (err) {
    console.error('agency refresh failed:', err);
    error = 'Could not load agency teams.';
    loading = false;
  }
}

/** Idempotent lifetime singleton — starts the interval refresh. */
export function startAgencyStore(): void {
  if (started) return;
  started = true;
  void refresh();
  timer = setInterval(() => void refresh(), REFRESH_MS);
}

export function stopAgencyStore(): void {
  if (timer) clearInterval(timer);
  timer = null;
  started = false;
}

/** Answer a question — writes back to the manager inbox, then refreshes so the
 *  answered card disappears. Returns `'delivered'` | `'already-answered'`. */
export async function submitAnswer(q: AgencyQuestion, answer: string): Promise<string> {
  const res = await invoke<string>('answer_agency_question', {
    company: q.company,
    team: q.team,
    id: q.id,
    answer,
  });
  await refresh();
  return res;
}

export const agencyStore = {
  get teams() {
    return teams;
  },
  get questions() {
    return questions;
  },
  get loading() {
    return loading;
  },
  get error() {
    return error;
  },
};
