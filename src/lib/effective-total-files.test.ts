import { describe, it, expect } from 'vitest';
import { effectiveTotalFiles } from './effective-total-files';

describe('effectiveTotalFiles', () => {
  it('uses the strict transfer count once the plan has arrived', () => {
    expect(
      effectiveTotalFiles({
        planReceived: true,
        syncPlanTotalFiles: 42,
        syncTotalFiles: 13_660,
      }),
    ).toBe(42);
  });

  it('REGRESSION: an up-to-date tree shows 0 — not the full pre-walk', () => {
    // The exact field report: a fully-synced HQ rendered
    // "Syncing Personal … of 13,660 files" (the whole vault) because the
    // old `plan > 0 ? plan : total` rule fell back to the pre-walk whenever
    // the transfer count was 0. With the plan received and 0 to transfer,
    // the denominator MUST be 0 so the UI can say "Up to date".
    expect(
      effectiveTotalFiles({
        planReceived: true,
        syncPlanTotalFiles: 0,
        syncTotalFiles: 13_660,
      }),
    ).toBe(0);
  });

  it('falls back to the pre-walk before any plan event (legacy runner)', () => {
    // Older runners (pre hq-cloud@5.5.0) never emit `sync:plan`. Until a plan
    // arrives we have no transfer count, so the pre-walk is the only
    // denominator available — better an over-count than dividing by zero.
    expect(
      effectiveTotalFiles({
        planReceived: false,
        syncPlanTotalFiles: 0,
        syncTotalFiles: 1_247,
      }),
    ).toBe(1_247);
  });

  it('pre-plan window: ignores a not-yet-populated plan total', () => {
    // planReceived flips true only when a plan event is processed; while it is
    // still false we never read syncPlanTotalFiles, even if some other code
    // path nudged it.
    expect(
      effectiveTotalFiles({
        planReceived: false,
        syncPlanTotalFiles: 5,
        syncTotalFiles: 900,
      }),
    ).toBe(900);
  });

  it('reflects accumulated transfer work across multiple companies', () => {
    // syncPlanTotalFiles is summed across every company/direction plan event;
    // this function just trusts that sum once the plan is in.
    expect(
      effectiveTotalFiles({
        planReceived: true,
        syncPlanTotalFiles: 137,
        syncTotalFiles: 50_000,
      }),
    ).toBe(137);
  });
});
