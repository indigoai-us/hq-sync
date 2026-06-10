import { describe, expect, it } from 'vitest';
import { transferCountDelta, type TransferCountEvent } from './transfer-count';
import { liveProgressCaption } from './live-progress-caption';

describe('transferCountDelta', () => {
  describe('regression: personal walk events must not be counted as "transferred"', () => {
    // Pre-fix bug: the `sync:personal-first-push-progress` listener bumped
    // `syncFilesProgressed` once per event whenever `currentFile` was set.
    // The Rust walker (run_personal_first_push) emits that event for EVERY
    // file it examines — including files it then skips because the journal
    // hash matches. On a fully-in-sync HQ with a ~2,500-file personal scope,
    // a Sync click moved 0 files but the popover read "2,503 transferred".
    it('returns 0 for a personal walk event, even with a currentFile', () => {
      expect(
        transferCountDelta({
          kind: 'personal-first-push-progress',
          currentFile: 'personal/notes/todo.md',
        })
      ).toBe(0);
    });

    it('fully-in-sync run: 2,503 walk events + 0 uploads accumulate to 0', () => {
      const events: TransferCountEvent[] = [
        ...Array.from({ length: 2_503 }, (_, i) => ({
          kind: 'personal-first-push-progress' as const,
          currentFile: `personal/file-${i}.md`,
        })),
        { kind: 'personal-first-push-complete', filesUploaded: 0 },
      ];
      const progressed = events.reduce((n, e) => n + transferCountDelta(e), 0);
      expect(progressed).toBe(0);

      // And the caption must not read "2,503 transferred": with nothing
      // progressed and no plan work, the up-to-date / fanout branches win.
      const caption = liveProgressCaption({
        syncFilesProgressed: progressed,
        syncPlanTotalFiles: 0,
        syncTotalFiles: 0,
        fanoutTotal: 4,
        fanoutDoneCount: 4,
        personalFilesDone: 2_503,
        personalFilesTotal: 2_503,
      });
      expect(caption.kind).toBe('up-to-date');
    });
  });

  it('counts each runner per-file progress event as one transfer', () => {
    expect(transferCountDelta({ kind: 'runner-progress' })).toBe(1);
  });

  it('credits real personal uploads once, from the complete event', () => {
    // The walker only reports actual uploads on completion; a run that
    // examined 2,503 files but uploaded 8 must show exactly 8 transferred.
    const events: TransferCountEvent[] = [
      ...Array.from({ length: 2_503 }, (_, i) => ({
        kind: 'personal-first-push-progress' as const,
        currentFile: `personal/file-${i}.md`,
      })),
      { kind: 'personal-first-push-complete', filesUploaded: 8 },
    ];
    const progressed = events.reduce((n, e) => n + transferCountDelta(e), 0);
    expect(progressed).toBe(8);
  });

  it('mixed run: personal uploads + runner transfers sum without walk noise', () => {
    const events: TransferCountEvent[] = [
      { kind: 'personal-first-push-progress', currentFile: 'personal/a.md' },
      { kind: 'personal-first-push-progress', currentFile: 'personal/b.md' },
      { kind: 'personal-first-push-complete', filesUploaded: 1 },
      { kind: 'runner-progress' },
      { kind: 'runner-progress' },
    ];
    const progressed = events.reduce((n, e) => n + transferCountDelta(e), 0);
    expect(progressed).toBe(3);
  });
});
