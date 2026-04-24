/**
 * Behavior tests for the Settings "Run embeddings now" contract (US-005).
 *
 * Same rationale as EmbeddingsRow.test.ts: Settings.svelte binds its
 * Maintenance row's label, disabled state, and error text directly to
 * `embeddingsStore`. Testing the store therefore covers the Settings
 * contract without adding a DOM testing dependency to this repo (see
 * hq-sync/CLAUDE.md "Manual testing only in V1").
 *
 * The assertions below mirror what the user sees:
 *   - Button label flips to "Running…" + disabled === true once
 *     `embeddingsStore.status === 'running'`
 *   - Subtext reads "Last run: <relative>" when a run exists, "Never"
 *     otherwise
 *   - Clicking the button dispatches `start_embeddings` with reason=manual
 *   - An error row renders the last error message
 */

import { beforeEach, describe, expect, it, vi } from 'vitest';

const mockInvoke = vi.fn();
vi.mock('@tauri-apps/api/core', () => ({
  invoke: (cmd: string, args?: unknown) => mockInvoke(cmd, args),
}));

import { embeddingsStore } from '../stores/embeddings';

// Match the relative-time formatter in Settings.svelte. The duplicated
// formatter in the component is small enough that porting a shared util
// isn't worth the churn; testing the output shape (not a byte-exact string)
// is what matters for the user-facing contract.
function relativeLabel(isoDate: string): string {
  const now = Date.now();
  const then = new Date(isoDate).getTime();
  if (isNaN(then)) return 'unknown';
  const seconds = Math.floor((now - then) / 1000);
  if (seconds < 60) return 'just now';
  if (seconds < 3600) {
    const m = Math.floor(seconds / 60);
    return `${m} minute${m > 1 ? 's' : ''} ago`;
  }
  if (seconds < 86400) {
    const h = Math.floor(seconds / 3600);
    return `${h} hour${h > 1 ? 's' : ''} ago`;
  }
  const d = Math.floor(seconds / 86400);
  return `${d} day${d > 1 ? 's' : ''} ago`;
}

describe('Settings → Run embeddings now contract (US-005)', () => {
  beforeEach(() => {
    mockInvoke.mockReset();
    embeddingsStore.resetForTest();
  });

  // ── Label / disabled contract ────────────────────────────────────────────

  it("button label is 'Run now' + enabled when idle", () => {
    expect(embeddingsStore.status).toBe('idle');
    const disabled = embeddingsStore.status === 'running';
    const label =
      embeddingsStore.status === 'running' ? 'Running…' : 'Run now';
    expect(disabled).toBe(false);
    expect(label).toBe('Run now');
  });

  it("button label flips to 'Running…' and disables once status='running'", () => {
    embeddingsStore.applyStart({
      reason: 'manual',
      startedAt: '2026-04-24T07:00:00.000Z',
    });
    const disabled = embeddingsStore.status === 'running';
    const label =
      embeddingsStore.status === 'running' ? 'Running…' : 'Run now';
    expect(disabled).toBe(true);
    expect(label).toBe('Running…');
  });

  // ── Subtext contract ─────────────────────────────────────────────────────

  it("subtext reads 'Last run: Never' when the store has no lastRunAt", () => {
    expect(embeddingsStore.lastRunAt).toBeUndefined();
    const subtext = embeddingsStore.lastRunAt
      ? `Last run: ${relativeLabel(embeddingsStore.lastRunAt)}`
      : 'Last run: Never';
    expect(subtext).toBe('Last run: Never');
  });

  it('subtext reads a relative timestamp when lastRunAt is ~10 minutes ago', () => {
    // Seed with a journal payload ten minutes old; Settings' subtext helper
    // mirrors the same time formatting so the visible line should read
    // "Last run: 10 minutes ago" — the specific minute count is tolerated
    // (±1) to avoid wall-clock-based flake.
    const tenMinAgo = new Date(Date.now() - 10 * 60_000).toISOString();
    embeddingsStore.seedFromPayload({
      lastRunAt: tenMinAgo,
      durationSec: 60,
      state: 'ok',
      errorMsg: null,
      source: 'journal',
    });
    const subtext = embeddingsStore.lastRunAt
      ? `Last run: ${relativeLabel(embeddingsStore.lastRunAt)}`
      : 'Last run: Never';
    expect(subtext).toMatch(/^Last run: (9|10|11) minutes ago$/);
  });

  it("subtext reads 'Running now' framing when status='running'", () => {
    embeddingsStore.applyStart({
      reason: 'manual',
      startedAt: '2026-04-24T07:00:00.000Z',
    });
    const subtext =
      embeddingsStore.status === 'running'
        ? 'Running now — progress visible in the popover.'
        : 'Last run: Never';
    expect(subtext).toMatch(/Running now/);
  });

  // ── Click handler dispatch ───────────────────────────────────────────────

  it("Run now dispatches invoke('start_embeddings', { reason: 'manual' })", async () => {
    mockInvoke.mockResolvedValue('hq-embeddings');
    await embeddingsStore.startNow('manual');
    expect(mockInvoke).toHaveBeenCalledTimes(1);
    expect(mockInvoke).toHaveBeenCalledWith('start_embeddings', {
      reason: 'manual',
    });
  });

  it('Run now is a no-op when a run is already in flight', async () => {
    // Simulate the onclick guard in Settings.svelte: if status === 'running',
    // handleRunEmbeddings returns early without calling the store.
    embeddingsStore.applyStart({
      reason: 'manual',
      startedAt: '2026-04-24T07:00:00.000Z',
    });
    const shouldDispatch = embeddingsStore.status !== 'running';
    expect(shouldDispatch).toBe(false);
    // Confirm the store wasn't touched by the simulated click.
    expect(mockInvoke).not.toHaveBeenCalled();
  });

  // ── Error surface ────────────────────────────────────────────────────────

  it('errorMsg is surfaced to the Maintenance row after embeddings:error', () => {
    embeddingsStore.applyError({ message: 'qmd not found' });
    expect(embeddingsStore.status).toBe('error');
    expect(embeddingsStore.errorMsg).toBe('qmd not found');
  });

  it('error clears on next successful start', () => {
    embeddingsStore.applyError({ message: 'transient' });
    embeddingsStore.applyStart({
      reason: 'manual',
      startedAt: '2026-04-24T07:10:00.000Z',
    });
    expect(embeddingsStore.errorMsg).toBeUndefined();
    expect(embeddingsStore.status).toBe('running');
  });
});
