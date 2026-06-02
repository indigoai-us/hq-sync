import { describe, expect, it, vi } from 'vitest';
import { createRecordingTracker, stopRecordingIfActive } from './recording-tracker.mjs';

// Regression for B2: when a meeting CALL ends (host ends it / everyone leaves)
// the Recall SDK fires only `meeting-closed` — there is no participant-left /
// call-ended event — and the SDK does NOT reliably auto-stop the recording
// (its CHANGELOG documents auto-stop as unreliable per-platform). bridge.mjs
// used to merely emit `meeting:closed` so NOBODY stopped the recording and it
// kept running. The fix turns the `meeting-closed` subscription into an action
// via `stopRecordingIfActive`. These tests pin that behaviour against a mocked
// SDK without booting the real one (bridge.mjs boots the SDK at import time, so
// the stop decision is factored into this pure helper to keep it testable).

describe('stopRecordingIfActive (meeting-closed → stopRecording)', () => {
  it('stops the recording exactly once when the closed window is tracked', async () => {
    const tracker = createRecordingTracker();
    tracker.started('win-1');
    const sdk = { stopRecording: vi.fn().mockResolvedValue(undefined) };

    const stopped = await stopRecordingIfActive(tracker, sdk, 'win-1');

    expect(stopped).toBe(true);
    expect(sdk.stopRecording).toHaveBeenCalledTimes(1);
    expect(sdk.stopRecording).toHaveBeenCalledWith({ windowId: 'win-1' });
  });

  it('does NOT stop when the closed window is not a tracked recording', async () => {
    const tracker = createRecordingTracker();
    tracker.started('win-1'); // a different window is recording
    const sdk = { stopRecording: vi.fn().mockResolvedValue(undefined) };

    const stopped = await stopRecordingIfActive(tracker, sdk, 'win-2');

    expect(stopped).toBe(false);
    expect(sdk.stopRecording).not.toHaveBeenCalled();
  });

  it('does NOT stop when no recording is tracked at all', async () => {
    const tracker = createRecordingTracker();
    const sdk = { stopRecording: vi.fn().mockResolvedValue(undefined) };

    const stopped = await stopRecordingIfActive(tracker, sdk, 'win-1');

    expect(stopped).toBe(false);
    expect(sdk.stopRecording).not.toHaveBeenCalled();
  });

  it('ignores empty / non-string window ids (no stop attempted)', async () => {
    const tracker = createRecordingTracker();
    tracker.started('win-1');
    const sdk = { stopRecording: vi.fn().mockResolvedValue(undefined) };

    expect(await stopRecordingIfActive(tracker, sdk, '')).toBe(false);
    expect(await stopRecordingIfActive(tracker, sdk, undefined)).toBe(false);
    expect(await stopRecordingIfActive(tracker, sdk, null)).toBe(false);
    expect(sdk.stopRecording).not.toHaveBeenCalled();
  });

  it('does NOT clear the tracker — recording-ended is what clears it', async () => {
    // The bridge intentionally leaves tracking in place so the normal
    // `recording-ended` handler (which calls tracker.ended) remains the single
    // place tracking is cleared on a clean stop.
    const tracker = createRecordingTracker();
    tracker.started('win-1');
    const sdk = { stopRecording: vi.fn().mockResolvedValue(undefined) };

    await stopRecordingIfActive(tracker, sdk, 'win-1');

    expect(tracker.activeWindowIds()).toEqual(['win-1']);
  });
});
