/**
 * recording-tracker — pure, SDK-agnostic bookkeeping for in-flight recordings.
 *
 * Why this exists: when the Recall Desktop SDK server process dies mid-recording
 * (e.g. an ORC/GStreamer SIGABRT under a misconfigured hardened runtime), it
 * never fires `recording-ended`, so the bridge never emits `recording:ended` and
 * the Svelte UI is stranded in the `stopping` (or `recording`) state forever.
 *
 * This tracker lets bridge.mjs remember which windows are actively recording so
 * that on a fatal SDK event it can synthesize a terminal `recording:error` for
 * each one — unwedging the UI with an explanatory error instead of an infinite
 * spinner. Kept dependency-free and side-effect-free so it is unit-testable
 * without mocking the SDK (see recording-tracker.test.mjs).
 */

/** Marker emitted as the `cmd` field of a synthesized terminal error so the UI
 *  (and logs) can distinguish an SDK crash from a real command failure. */
export const SDK_CRASH_CMD = 'sdk-crash';

export function createRecordingTracker() {
  /** windowId -> true for every recording the SDK has confirmed started and
   *  not yet ended. A Set keyed by windowId (the canonical SDK handle). */
  const active = new Set();

  return {
    /** Mark a window as actively recording. No-op for a falsy/empty id. */
    started(windowId) {
      if (typeof windowId === 'string' && windowId) active.add(windowId);
    },

    /** Clear a window once the SDK confirms its recording ended (normal stop
     *  path) or it errors out. No-op if it wasn't tracked. */
    ended(windowId) {
      if (typeof windowId === 'string' && windowId) active.delete(windowId);
    },

    /** Snapshot of the currently-recording window ids (test/inspection aid). */
    activeWindowIds() {
      return [...active];
    },

    /**
     * The SDK process died (shutdown / fatal `process` error). Produce one
     * terminal `recording:error` ndjson object per in-flight recording so the
     * caller can emit them, then clear tracking so a follow-on fatal event
     * (the SDK fires both `error` and `shutdown` on a crash) doesn't double-fire.
     *
     * Shape matches RecordingErrorEvent in src-tauri/src/events.rs
     * ({type, cmd, windowId, message}); the existing recording:error listener
     * in App.svelte / activeMeetings.ts already routes it to the `error` state.
     *
     * @param {string} reason human-readable cause, surfaced in the UI error.
     * @returns {Array<{type:string,cmd:string,windowId:string,message:string}>}
     */
    drainOnFatal(reason) {
      const message =
        typeof reason === 'string' && reason
          ? reason
          : 'Recording engine stopped unexpectedly';
      const events = [...active].map((windowId) => ({
        type: 'recording:error',
        cmd: SDK_CRASH_CMD,
        windowId,
        message,
      }));
      active.clear();
      return events;
    },
  };
}

/**
 * Auto-stop a recording when its meeting CALL ends.
 *
 * The Recall Desktop SDK's only signal that a call has ended (host ended it /
 * everyone left) is the `meeting-closed` event — there is NO `participant-left`
 * / `call-ended` event. The bridge wrongly assumed the SDK auto-stops the
 * recording when the meeting window closes (see the now-corrected comment in
 * src-tauri/src/events.rs), but the SDK's own CHANGELOG shows that auto-stop is
 * unreliable per-platform, so a call ending could leave the recording running
 * indefinitely. This turns the existing `meeting-closed` subscription into an
 * action: if the closing window is a currently-tracked active recording, ask
 * the SDK to stop it.
 *
 * `RecallAiSdk.stopRecording` is a documented no-op when the window isn't
 * recording, so this is idempotent. We do NOT clear the tracker here — the
 * resulting `recording-ended` event flows normally and its handler calls
 * `tracker.ended(windowId)`, the single place tracking is cleared on a clean
 * stop.
 *
 * Kept here (not inline in the meeting-closed listener) so it's unit-testable
 * against a mocked SDK without booting the real one — see
 * recording-tracker.test.mjs and bridge.test.mjs.
 *
 * @param {{activeWindowIds: () => string[]}} tracker the recording tracker.
 * @param {{stopRecording: (arg: {windowId: string}) => Promise<unknown>}} sdk
 *   the Recall SDK (only `stopRecording` is used).
 * @param {string} windowId the window whose meeting just closed.
 * @returns {Promise<boolean>} true if a stop was issued (window was tracked).
 */
export async function stopRecordingIfActive(tracker, sdk, windowId) {
  if (typeof windowId !== 'string' || !windowId) return false;
  if (!tracker.activeWindowIds().includes(windowId)) return false;
  await sdk.stopRecording({ windowId });
  return true;
}
