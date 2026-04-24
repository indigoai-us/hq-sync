/**
 * Behavior tests for the EmbeddingsRow contract (US-003).
 *
 * The row itself is a thin Svelte template bound to `embeddingsStore`; all
 * of its render-state branching (`running` / `ok` / `error` / `pending` /
 * `idle`) is driven by the store's `status` value, and its Retry button
 * calls `embeddingsStore.startNow('manual')`. Testing the store covers both
 * — with the added benefit that these tests stay green if we ever swap the
 * Svelte component for a different renderer (e.g. a React snapshot in a
 * hypothetical web build).
 *
 * We deliberately do NOT mount the component here: hq-sync has no DOM
 * testing library configured (see `CLAUDE.md` "Manual testing only in V1"),
 * and adding one just for this story is scope creep. If DOM rendering is
 * needed in a future iteration, `@testing-library/svelte` + `happy-dom` can
 * be dropped in without rewriting these tests.
 */

import { beforeEach, describe, expect, it, vi } from 'vitest';

// Mock the Tauri invoke surface BEFORE importing the store — the store
// imports it at module-evaluation time, so the mock has to be in place.
const mockInvoke = vi.fn();
vi.mock('@tauri-apps/api/core', () => ({
  invoke: (cmd: string, args?: unknown) => mockInvoke(cmd, args),
}));

import { embeddingsStore } from '../stores/embeddings';

describe('EmbeddingsRow → embeddingsStore contract (US-003)', () => {
  beforeEach(() => {
    mockInvoke.mockReset();
    embeddingsStore.resetForTest();
  });

  // ── Render state: running ────────────────────────────────────────────────

  it('renders `running` after embeddings:start', () => {
    embeddingsStore.applyStart({
      reason: 'post-install',
      startedAt: '2026-04-24T07:00:00.000Z',
    });
    expect(embeddingsStore.status).toBe('running');
    // errorMsg from a previous failed run must be cleared on new start.
    expect(embeddingsStore.errorMsg).toBeUndefined();
  });

  // ── Render state: live progress tail ─────────────────────────────────────

  it('captures live progress lines and stays in running state', () => {
    embeddingsStore.applyStart({
      reason: 'manual',
      startedAt: '2026-04-24T07:00:00.000Z',
    });
    embeddingsStore.applyProgress({ line: 'Indexing chunk 42/100' });
    expect(embeddingsStore.status).toBe('running');
    expect(embeddingsStore.liveLine).toBe('Indexing chunk 42/100');
  });

  it('progress without prior start lifts state to running', () => {
    // Defensive: if events arrive out-of-order (rare: OS event reordering
    // under load), the store must not be stuck in `idle` while stdout is
    // flowing. Progress alone is enough to assert an active run.
    embeddingsStore.applyProgress({ line: 'Loading qmd model…' });
    expect(embeddingsStore.status).toBe('running');
    expect(embeddingsStore.liveLine).toBe('Loading qmd model…');
  });

  // ── Render state: ok ──────────────────────────────────────────────────────

  it('transitions to `ok` after embeddings:complete', () => {
    embeddingsStore.applyStart({
      reason: 'post-install',
      startedAt: '2026-04-24T07:00:00.000Z',
    });
    embeddingsStore.applyComplete({ durationSec: 45 });
    expect(embeddingsStore.status).toBe('ok');
    expect(embeddingsStore.durationSec).toBe(45);
    // Client-side timestamp placeholder until refresh() overwrites from
    // the journal — still a valid ISO8601 parseable date.
    expect(embeddingsStore.lastRunAt).toBeDefined();
    expect(Number.isNaN(Date.parse(embeddingsStore.lastRunAt as string))).toBe(
      false
    );
    // Live progress is cleared once the run completes.
    expect(embeddingsStore.liveLine).toBeUndefined();
  });

  // ── Render state: error ──────────────────────────────────────────────────

  it('transitions to `error` after embeddings:error with message', () => {
    embeddingsStore.applyStart({
      reason: 'manual',
      startedAt: '2026-04-24T07:00:00.000Z',
    });
    embeddingsStore.applyError({ message: 'qmd not found' });
    expect(embeddingsStore.status).toBe('error');
    expect(embeddingsStore.errorMsg).toBe('qmd not found');
  });

  it('error cleared on next start', () => {
    embeddingsStore.applyError({ message: 'first failure' });
    expect(embeddingsStore.errorMsg).toBe('first failure');
    embeddingsStore.applyStart({
      reason: 'manual',
      startedAt: '2026-04-24T07:10:00.000Z',
    });
    expect(embeddingsStore.errorMsg).toBeUndefined();
    expect(embeddingsStore.status).toBe('running');
  });

  // ── Retry contract ───────────────────────────────────────────────────────

  it("Retry dispatches invoke('start_embeddings', { reason: 'manual' })", async () => {
    mockInvoke.mockResolvedValue('hq-embeddings');
    await embeddingsStore.startNow('manual');
    expect(mockInvoke).toHaveBeenCalledTimes(1);
    expect(mockInvoke).toHaveBeenCalledWith('start_embeddings', {
      reason: 'manual',
    });
  });

  it('Retry surfaces invoke rejections to the caller (so EmbeddingsRow can log)', async () => {
    mockInvoke.mockRejectedValue(new Error('already running'));
    await expect(embeddingsStore.startNow('manual')).rejects.toThrow(
      'already running'
    );
  });

  // ── Seeding from journal (mount-time refresh) ────────────────────────────

  it('seedFromPayload with source=none leaves status=idle and lastRunAt undefined', () => {
    embeddingsStore.seedFromPayload({
      lastRunAt: null,
      durationSec: 0,
      state: 'unknown',
      errorMsg: null,
      source: 'none',
    });
    expect(embeddingsStore.status).toBe('idle');
    expect(embeddingsStore.lastRunAt).toBeUndefined();
  });

  it('seedFromPayload with state=ok transitions to ok and preserves lastRunAt', () => {
    embeddingsStore.seedFromPayload({
      lastRunAt: '2026-04-24T06:30:00.000Z',
      durationSec: 90,
      state: 'ok',
      errorMsg: null,
      source: 'journal',
    });
    expect(embeddingsStore.status).toBe('ok');
    expect(embeddingsStore.lastRunAt).toBe('2026-04-24T06:30:00.000Z');
    expect(embeddingsStore.durationSec).toBe(90);
  });

  it('seedFromPayload with source=marker transitions to pending', () => {
    // Codex P2 fix: when a pending marker is on disk but the journal
    // hasn't been written yet (brief window between installer exit and
    // auto-trigger firing, OR persistent when qmd is missing), the
    // Rust-side `get_embeddings_status` returns source="marker". The
    // store must render this as `pending` so the popover shows the
    // user that work is queued instead of "never".
    embeddingsStore.seedFromPayload({
      lastRunAt: null,
      durationSec: 0,
      state: 'pending',
      errorMsg: null,
      source: 'marker',
    });
    expect(embeddingsStore.status).toBe('pending');
    expect(embeddingsStore.lastRunAt).toBeUndefined();
  });

  it('seedFromPayload with state=error transitions to error and sets errorMsg', () => {
    embeddingsStore.seedFromPayload({
      lastRunAt: '2026-04-24T06:30:00.000Z',
      durationSec: 2,
      state: 'error',
      errorMsg: 'transient failure',
      source: 'journal',
    });
    expect(embeddingsStore.status).toBe('error');
    expect(embeddingsStore.errorMsg).toBe('transient failure');
  });

  it('seedFromPayload does NOT override a running state mid-run', () => {
    // A mount-time refresh races against a fresh start — we must not flip
    // `running` back to the previous journal's `ok`/`error` just because
    // the refresh call happens to land a millisecond later.
    embeddingsStore.applyStart({
      reason: 'post-install',
      startedAt: '2026-04-24T07:00:00.000Z',
    });
    embeddingsStore.seedFromPayload({
      lastRunAt: '2026-04-23T12:00:00.000Z',
      durationSec: 60,
      state: 'ok',
      errorMsg: null,
      source: 'journal',
    });
    expect(embeddingsStore.status).toBe('running');
  });

  // ── refresh() integration with invoke ────────────────────────────────────

  it('refresh() calls get_embeddings_status and seeds from the payload', async () => {
    mockInvoke.mockResolvedValue({
      lastRunAt: '2026-04-24T06:30:00.000Z',
      durationSec: 55,
      state: 'ok',
      errorMsg: null,
      source: 'journal',
    });
    await embeddingsStore.refresh();
    expect(mockInvoke).toHaveBeenCalledWith('get_embeddings_status', undefined);
    expect(embeddingsStore.status).toBe('ok');
    expect(embeddingsStore.lastRunAt).toBe('2026-04-24T06:30:00.000Z');
  });

  it('refresh() swallows invoke rejections (row stays on its last state)', async () => {
    embeddingsStore.applyComplete({ durationSec: 30 });
    mockInvoke.mockRejectedValue(new Error('Tauri not ready'));
    await embeddingsStore.refresh();
    // Still 'ok' — the prior state is preserved when refresh can't improve it.
    expect(embeddingsStore.status).toBe('ok');
  });

  // ── subscribe/notify ─────────────────────────────────────────────────────

  it('subscribers fire on every apply* transition', () => {
    const fn = vi.fn();
    const unsub = embeddingsStore.subscribe(fn);
    embeddingsStore.applyStart({
      reason: 'post-install',
      startedAt: '2026-04-24T07:00:00.000Z',
    });
    embeddingsStore.applyProgress({ line: 'x' });
    embeddingsStore.applyComplete({ durationSec: 10 });
    expect(fn).toHaveBeenCalledTimes(3);
    unsub();
    embeddingsStore.applyError({ message: 'post-unsub' });
    // After unsub, the listener should not fire again.
    expect(fn).toHaveBeenCalledTimes(3);
  });
});
