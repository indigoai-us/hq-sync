import { invoke } from '@tauri-apps/api/core';
import type { CompanySummary } from './company-summary.svelte';
import type { CompanyBoard } from './company-board.svelte';
import type { DeploymentEntry } from '../components/DeploymentRow.svelte';
import type { SecretEnv } from '../components/SecretEnvRow.svelte';

// Poll cadence for the background warm-cache refresh. Mirrors meetings-store:
// long enough to be cheap, short enough that a tab opened a minute later is
// already current.
const POLL_INTERVAL_MS = 30_000;

// ---------------------------------------------------------------------------
// Company workspace preload cache (Board / Activity / Deployments / Secrets).
//
// Why this exists: DesktopApp wraps the routed page in {#key routeKey} and
// CompanyPage wraps each tab panel in {#key `${slug}:${activeTab}`}, so every
// company nav AND every tab switch remounts the panel. Each panel runs a
// blocking `get_company_*` invoke on mount, which is what made the screen sit
// on a skeleton for 5-10s every time. This module loads all four datasets for
// every known company ONCE at app launch (startCompanyStore, called from
// DesktopApp.loadWorkspaces once the real slugs are known) and keeps them warm
// with a 30s poll + focus refresh. Panels read the warm value synchronously on
// mount (instant paint), still run their own invoke to revalidate, and write
// the fresh result back here.
//
// Unlike meetings-store these caches are intentionally NON-reactive plain Maps:
// the panels keep ownership of their own invoke + local $state (so the story
// contracts that pin those invokes stay intact), and they only read this cache
// once per mount. Keeping it non-reactive means a background poll can't retrigger
// a mounted panel's effect into a refetch loop — it simply freshens the cache
// for the next mount.
// ---------------------------------------------------------------------------

const summaryBySlug = new Map<string, CompanySummary>();
const boardBySlug = new Map<string, CompanyBoard>();
// Activity's payload type (CompanyActivity) is declared inline in ActivityPanel
// and not exported, so the cache holds the raw invoke result untyped; the panel
// normalizes it on read.
const activityBySlug = new Map<string, unknown>();
const deploymentsBySlug = new Map<string, Partial<DeploymentEntry>[]>();
const secretsBySlug = new Map<string, Partial<SecretEnv>[]>();

// Slugs we warm + poll. Reconciled on every startCompanyStore call so companies
// that appear after the first workspace load still get warmed.
let warmSlugs: string[] = [];

let started = false;
let pollTimer: ReturnType<typeof setInterval> | null = null;

/**
 * Fetch all four company datasets in parallel and populate the cache. Failures
 * are logged and swallowed per-command so one bad command can't blank the rest;
 * a panel's own invoke surfaces the real error to the user when mounted.
 */
async function loadCompany(slug: string): Promise<void> {
  if (!slug) return;
  await Promise.all([
    invoke<CompanySummary>('get_company_summary', { slug })
      .then((result) => {
        summaryBySlug.set(slug, result);
      })
      .catch((err) => console.error('get_company_summary preload failed:', err)),
    invoke<CompanyBoard>('get_company_board', { slug })
      .then((result) => {
        boardBySlug.set(slug, result);
      })
      .catch((err) => console.error('get_company_board preload failed:', err)),
    invoke('get_company_activity', { slug })
      .then((result) => {
        activityBySlug.set(slug, result);
      })
      .catch((err) => console.error('get_company_activity preload failed:', err)),
    invoke<Partial<DeploymentEntry>[]>('get_company_deployments', { slug })
      .then((result) => {
        deploymentsBySlug.set(slug, Array.isArray(result) ? result : []);
      })
      .catch((err) => console.error('get_company_deployments preload failed:', err)),
    invoke<Partial<SecretEnv>[]>('get_company_secrets', { slug })
      .then((result) => {
        secretsBySlug.set(slug, Array.isArray(result) ? result : []);
      })
      .catch((err) => console.error('get_company_secrets preload failed:', err)),
  ]);
}

function refreshAll(): void {
  for (const slug of warmSlugs) {
    void loadCompany(slug);
  }
}

/**
 * Add a single company to the warm set and load it immediately if it's new.
 * Idempotent — a slug already being warmed is a no-op for the warm set but
 * still kicks a fresh load.
 */
function ensureCompany(slug: string): void {
  if (!slug) return;
  if (!warmSlugs.includes(slug)) warmSlugs.push(slug);
  void loadCompany(slug);
}

/**
 * Start the singleton once for the app's lifetime. Called from
 * DesktopApp.loadWorkspaces (after the workspace list resolves) so the company
 * tab data is warm before the user navigates into any company. Safe to call
 * repeatedly: each call reconciles the slug set and immediately warms any
 * newly-seen companies; the 30s poll + focus refresh are wired only once.
 */
export function startCompanyStore(slugs: string[]): void {
  const fresh: string[] = [];
  for (const slug of slugs) {
    if (slug && !warmSlugs.includes(slug)) {
      warmSlugs.push(slug);
      fresh.push(slug);
    }
  }
  for (const slug of fresh) {
    void loadCompany(slug);
  }

  if (started) return;
  started = true;

  pollTimer = setInterval(refreshAll, POLL_INTERVAL_MS);
  window.addEventListener('focus', refreshAll);
}

/**
 * Tear down the poll + listener. Unused in the app (the store lives for the
 * whole session) but exported so tests can reset between runs.
 */
export function stopCompanyStore(): void {
  if (pollTimer !== null) {
    clearInterval(pollTimer);
    pollTimer = null;
  }
  window.removeEventListener('focus', refreshAll);
  warmSlugs = [];
  started = false;
}

// Read/write surface. Reads return null when a slug hasn't been warmed yet, so
// a consumer can paint its empty state and show the skeleton (loading = warm
// === null) exactly once, on a true cold cache.
export const companyStore = {
  summary(slug: string): CompanySummary | null {
    return summaryBySlug.get(slug) ?? null;
  },
  board(slug: string): CompanyBoard | null {
    return boardBySlug.get(slug) ?? null;
  },
  activity(slug: string): unknown {
    return activityBySlug.has(slug) ? activityBySlug.get(slug) : null;
  },
  deployments(slug: string): Partial<DeploymentEntry>[] | null {
    return deploymentsBySlug.get(slug) ?? null;
  },
  secrets(slug: string): Partial<SecretEnv>[] | null {
    return secretsBySlug.get(slug) ?? null;
  },
  ensureCompany,
  setSummary(slug: string, value: CompanySummary): void {
    summaryBySlug.set(slug, value);
  },
  setBoard(slug: string, value: CompanyBoard): void {
    boardBySlug.set(slug, value);
  },
  setActivity(slug: string, value: unknown): void {
    activityBySlug.set(slug, value);
  },
  setDeployments(slug: string, value: Partial<DeploymentEntry>[]): void {
    deploymentsBySlug.set(slug, value);
  },
  setSecrets(slug: string, value: Partial<SecretEnv>[]): void {
    secretsBySlug.set(slug, value);
  },
};
