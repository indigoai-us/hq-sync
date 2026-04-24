import { invoke } from '@tauri-apps/api/core';

/**
 * UI-facing status for the `qmd embed` background job.
 *
 * - `idle`    — initial state, no journal read yet
 * - `pending` — marker exists but auto-trigger hasn't fired; narrow window
 *               between installer exit and sync's 2s startup grace (US-004)
 * - `running` — `embeddings:start` received, `embeddings:complete` not yet
 * - `ok`      — last run succeeded; `lastRunAt` populated
 * - `error`   — last run failed; `errorMsg` populated
 */
export type EmbeddingsStatusValue =
  | 'idle'
  | 'pending'
  | 'running'
  | 'ok'
  | 'error';

/**
 * Shape returned by the Rust `get_embeddings_status` command. Mirrors
 * `EmbeddingsStatus` in `src-tauri/src/commands/embeddings.rs`.
 */
export interface EmbeddingsStatusPayload {
  lastRunAt: string | null;
  durationSec: number;
  /** Journal state string: "ok" | "error" | "unknown" */
  state: string;
  errorMsg: string | null;
  /** Journal presence marker: "journal" | "none" */
  source: string;
}

/**
 * Subscribe-pattern store for the `qmd embed` background job.
 *
 * Matches the class/subscriber shape of `conflictStore` so components can
 * consume via a local `$state` mirror updated in `$effect`. Svelte rune state
 * inside a `.svelte.ts` module isn't used here because tests run under plain
 * vitest (no svelte preprocessor in the test pipeline), and the manual
 * subscriber pattern keeps component tests unit-testable without DOM.
 */
class EmbeddingsStore {
  private _status: EmbeddingsStatusValue = 'idle';
  private _lastRunAt: string | undefined = undefined;
  private _durationSec: number = 0;
  private _errorMsg: string | undefined = undefined;
  private _liveLine: string | undefined = undefined;
  private _listeners: Set<() => void> = new Set();

  // ── getters (read-only view for components) ────────────────────────────

  get status(): EmbeddingsStatusValue {
    return this._status;
  }
  get lastRunAt(): string | undefined {
    return this._lastRunAt;
  }
  get durationSec(): number {
    return this._durationSec;
  }
  get errorMsg(): string | undefined {
    return this._errorMsg;
  }
  get liveLine(): string | undefined {
    return this._liveLine;
  }

  // ── subscribe (for component $effect bridges) ──────────────────────────

  subscribe(fn: () => void) {
    this._listeners.add(fn);
    return () => this._listeners.delete(fn);
  }

  private notify() {
    for (const fn of this._listeners) fn();
  }

  // ── event appliers (called by App.svelte's Tauri listeners) ────────────

  /**
   * `embeddings:start`. Flips the UI to `running` and clears any stale error
   * text. `reason` isn't persisted because the UI doesn't surface it today;
   * if we ever need "auto-retried after install" in the row, we'd add it here.
   */
  applyStart(_payload: { reason: string; startedAt: string }) {
    this._status = 'running';
    this._errorMsg = undefined;
    this._liveLine = undefined;
    this.notify();
  }

  /** `embeddings:progress`. Tail the last stdout/stderr line for display. */
  applyProgress(payload: { line: string }) {
    this._liveLine = payload.line;
    // Stay in running — progress can arrive before start (rare) so lift the
    // state if needed, but don't downgrade from running.
    if (this._status !== 'running') this._status = 'running';
    this.notify();
  }

  /** `embeddings:complete`. Transition to `ok`; server sets `lastRunAt`. */
  applyComplete(payload: { durationSec: number }) {
    this._status = 'ok';
    this._durationSec = payload.durationSec;
    // Assume lastRunAt == now for the common case; a subsequent seed() call
    // (triggered by the component mounting) will overwrite with the journal's
    // precise timestamp. This keeps the timestamp fresh in the running →
    // complete transition without waiting for a round-trip.
    this._lastRunAt = new Date().toISOString();
    this._errorMsg = undefined;
    this._liveLine = undefined;
    this.notify();
  }

  /** `embeddings:error`. Transition to `error` and surface the message. */
  applyError(payload: { message: string }) {
    this._status = 'error';
    this._errorMsg = payload.message;
    this._liveLine = undefined;
    this.notify();
  }

  /**
   * Seed the store from a fresh `get_embeddings_status` call. Called on row
   * mount so the "Up to date · 2 minutes ago" line survives popover
   * close/re-open and app restarts (journal is the durable source of truth).
   *
   * Does NOT override `running` status — the journal only records completed
   * runs, so seeding mid-run would spuriously flip to `ok`/`error` based on
   * the PREVIOUS run.
   */
  seedFromPayload(payload: EmbeddingsStatusPayload) {
    if (this._status === 'running') return;
    this._lastRunAt = payload.lastRunAt ?? undefined;
    this._durationSec = payload.durationSec;
    this._errorMsg = payload.errorMsg ?? undefined;
    // Source + state fan-out:
    //   - source=none  → idle (no prior run, no marker)
    //   - source=marker → pending (marker on disk, no journal yet —
    //                     auto-trigger may not have fired, or qmd is missing)
    //   - source=journal + state=ok → ok
    //   - source=journal + state=error → error
    if (payload.source === 'none') {
      this._status = 'idle';
    } else if (payload.source === 'marker' || payload.state === 'pending') {
      this._status = 'pending';
    } else if (payload.state === 'ok') {
      this._status = 'ok';
    } else if (payload.state === 'error') {
      this._status = 'error';
    } else {
      this._status = 'idle';
    }
    this.notify();
  }

  /** Fetch + seed in one step. Swallows invoke errors (caller can't act on them). */
  async refresh() {
    try {
      const payload = await invoke<EmbeddingsStatusPayload>(
        'get_embeddings_status'
      );
      this.seedFromPayload(payload);
    } catch (err) {
      // Keep existing state on failure — the row's current display is still
      // the best guess we have.
      console.error('[embeddings] get_embeddings_status failed:', err);
    }
  }

  /** Trigger a new run via Tauri. Thin wrapper so components don't import `invoke`. */
  async startNow(reason: string): Promise<void> {
    await invoke('start_embeddings', { reason });
  }

  /** Test-only helper so tests can rewind state between cases. */
  resetForTest() {
    this._status = 'idle';
    this._lastRunAt = undefined;
    this._durationSec = 0;
    this._errorMsg = undefined;
    this._liveLine = undefined;
    this._listeners.clear();
  }
}

export const embeddingsStore = new EmbeddingsStore();
