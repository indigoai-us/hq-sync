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
