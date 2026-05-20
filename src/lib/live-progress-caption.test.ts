import { describe, expect, it } from 'vitest';
import { liveProgressCaption, type LiveProgressInputs } from './live-progress-caption';

const base: LiveProgressInputs = {
  syncFilesProgressed: 0,
  syncPlanTotalFiles: 0,
  syncTotalFiles: 0,
  fanoutTotal: 0,
  fanoutDoneCount: 0,
  personalFilesDone: 0,
  personalFilesTotal: null,
};

describe('liveProgressCaption', () => {
  describe('regression: tree-walk total must not be shown as "transferred"', () => {
    // Pre-fix bug: the caption used `syncTotalFiles` (Rust pre-walk =
    // every file in the local tree) as the denominator, so a sync of a
    // 50,000-file tree that actually needed to move 0 files showed
    // "0 of 50,000 transferred" — making it read as if all files were
    // being transferred. The denominator must come from `sync:plan`
    // events instead (strict transfer count).
    it('does NOT use the pre-walk total when plan events have not landed', () => {
      const r = liveProgressCaption({
        ...base,
        syncFilesProgressed: 0,
        syncTotalFiles: 50_000, // pre-walk: large tree
        syncPlanTotalFiles: 0, // plan events not in yet
        fanoutTotal: 3,
      });
      // Must NOT be transferred-of with planTotal = 50_000 — that's the bug.
      expect(r.kind).not.toBe('transferred-of');
    });

    it('shows the plan total (not pre-walk total) once plan events land', () => {
      const r = liveProgressCaption({
        ...base,
        syncFilesProgressed: 5,
        syncTotalFiles: 50_000, // pre-walk: large tree
        syncPlanTotalFiles: 47, // plan: only 47 files actually move
        fanoutTotal: 3,
      });
      expect(r).toEqual({ kind: 'transferred-of', progressed: 5, planTotal: 47 });
    });
  });

  it('renders the strict "N of M transferred" caption when plan total is known', () => {
    const r = liveProgressCaption({
      ...base,
      syncFilesProgressed: 12,
      syncPlanTotalFiles: 100,
    });
    expect(r).toEqual({ kind: 'transferred-of', progressed: 12, planTotal: 100 });
  });

  it('falls back to count-only when the bar overshoots the plan total', () => {
    // Plan estimate can under-count (e.g. dynamic discoveries during sync);
    // when numerator exceeds denominator, show the honest running count.
    const r = liveProgressCaption({
      ...base,
      syncFilesProgressed: 110,
      syncPlanTotalFiles: 100,
    });
    expect(r).toEqual({ kind: 'transferred', progressed: 110 });
  });

  it('falls back to count-only when on a pre-5.5.0 runner (no plan events)', () => {
    const r = liveProgressCaption({
      ...base,
      syncFilesProgressed: 7,
      syncPlanTotalFiles: 0,
      syncTotalFiles: 1234,
    });
    expect(r).toEqual({ kind: 'transferred', progressed: 7 });
  });

  it('shows "Up to date — finalizing…" when pre-walk says 0 and fanout started', () => {
    const r = liveProgressCaption({
      ...base,
      syncFilesProgressed: 0,
      syncPlanTotalFiles: 0,
      syncTotalFiles: 0,
      fanoutTotal: 2,
    });
    expect(r).toEqual({ kind: 'up-to-date' });
  });

  it('shows fanout fallback when nothing else qualifies', () => {
    const r = liveProgressCaption({
      ...base,
      fanoutTotal: 3,
      fanoutDoneCount: 1,
      syncTotalFiles: 100, // pre-walk has data, but no progress yet
    });
    expect(r).toEqual({ kind: 'fanout', current: 2, total: 3, progressed: 0 });
  });

  it('clamps fanout current to fanoutTotal once the last company finishes', () => {
    const r = liveProgressCaption({
      ...base,
      fanoutTotal: 3,
      fanoutDoneCount: 3,
      syncTotalFiles: 100,
    });
    expect(r).toEqual({ kind: 'fanout', current: 3, total: 3, progressed: 0 });
  });

  it('shows the personal phase caption when only personal first-push is active', () => {
    const r = liveProgressCaption({
      ...base,
      personalFilesDone: 12,
      personalFilesTotal: 47,
    });
    expect(r).toEqual({ kind: 'personal', done: 12, total: 47 });
  });

  it('renders nothing when no phase has data', () => {
    expect(liveProgressCaption(base)).toEqual({ kind: 'none' });
  });

  it('prefers transferred-of over personal-phase when both have data', () => {
    // Personal phase can leak personalFilesTotal into the runner phase
    // (App.svelte intentionally doesn't clear it on handoff). The strict
    // transferred caption must win.
    const r = liveProgressCaption({
      ...base,
      syncFilesProgressed: 5,
      syncPlanTotalFiles: 200,
      personalFilesDone: 47,
      personalFilesTotal: 47,
    });
    expect(r).toEqual({ kind: 'transferred-of', progressed: 5, planTotal: 200 });
  });
});
