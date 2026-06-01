import { describe, expect, it } from 'vitest';
import { createRecordingTracker, SDK_CRASH_CMD } from './recording-tracker.mjs';

// Regression for the "Stopping…" hang: when the Recall SDK server process
// SIGABRT-loops mid-recording (ORC/GStreamer JIT denied under the hardened
// runtime) it never fires `recording-ended`, so the bridge never emits
// `recording:ended` and the UI is stranded forever. The tracker lets the bridge
// synthesize a terminal `recording:error` for every in-flight recording on a
// fatal SDK event. These tests pin that behaviour without booting the real SDK.

describe('createRecordingTracker', () => {
  it('tracks started recordings and clears ended ones', () => {
    const t = createRecordingTracker();
    t.started('win-1');
    t.started('win-2');
    expect(t.activeWindowIds().sort()).toEqual(['win-1', 'win-2']);

    t.ended('win-1');
    expect(t.activeWindowIds()).toEqual(['win-2']);
  });

  it('ignores empty / non-string window ids', () => {
    const t = createRecordingTracker();
    t.started('');
    t.started(undefined);
    t.started(null);
    expect(t.activeWindowIds()).toEqual([]);
  });

  it('drainOnFatal emits one recording:error per in-flight recording', () => {
    const t = createRecordingTracker();
    t.started('win-1');
    t.started('win-2');

    const events = t.drainOnFatal('Recording engine crashed: SIGABRT');

    expect(events).toEqual([
      {
        type: 'recording:error',
        cmd: SDK_CRASH_CMD,
        windowId: 'win-1',
        message: 'Recording engine crashed: SIGABRT',
      },
      {
        type: 'recording:error',
        cmd: SDK_CRASH_CMD,
        windowId: 'win-2',
        message: 'Recording engine crashed: SIGABRT',
      },
    ]);
  });

  it('drainOnFatal is idempotent — the paired error+shutdown a crash fires only emits once', () => {
    const t = createRecordingTracker();
    t.started('win-1');

    const first = t.drainOnFatal('crash');
    const second = t.drainOnFatal('crash again');

    expect(first).toHaveLength(1);
    expect(second).toEqual([]); // already drained; no double-fire
    expect(t.activeWindowIds()).toEqual([]);
  });

  it('drainOnFatal with no active recordings emits nothing', () => {
    const t = createRecordingTracker();
    expect(t.drainOnFatal('crash')).toEqual([]);
  });

  it('falls back to a default message when no reason is given', () => {
    const t = createRecordingTracker();
    t.started('win-1');
    const [event] = t.drainOnFatal('');
    expect(event.message).toBe('Recording engine stopped unexpectedly');
  });

  it('a normally-ended recording is not failed by a later crash', () => {
    const t = createRecordingTracker();
    t.started('win-1');
    t.ended('win-1'); // clean stop confirmed by the SDK
    expect(t.drainOnFatal('later crash')).toEqual([]);
  });
});
