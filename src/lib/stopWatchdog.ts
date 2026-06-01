import type { ActiveMeetingState } from './activeMeetings';

// Stop-recording watchdog.
//
// When the user hits Stop we set the row to `stopping` and wait for the SDK to
// confirm via `recording:ended` (drop the row) or `recording:error` (→ error).
// If the SDK never confirms — it crashed, the bridge stalled, or Rust dropped
// the event — the row would otherwise sit in `stopping` forever (the exact bug
// users hit: "Stopping…" hangs). This watchdog is the framework-agnostic
// backstop: arm a timer on stop; if the row is STILL `stopping` when it fires,
// force it to `error`. The bridge-side terminal-error synthesis usually fires
// first on a crash — this covers every other way the confirmation can go
// missing.

export const STOP_WATCHDOG_MS = 12_000;

export const STOP_TIMEOUT_MESSAGE =
  'Stop timed out — the recording engine stopped responding. The recording may not have been saved.';

type TimerHandle = ReturnType<typeof setTimeout>;

// windowId -> pending timer. Module-level so the arm site (stop handler) and the
// clear sites (terminal-event listeners) share one registry within a window's
// JS context. The classic popover and the desktop-alt window are separate
// webviews with separate module instances — each manages only its own rows.
const timers = new Map<string, TimerHandle>();

/**
 * Arm (or re-arm) the stop watchdog for a window. Re-arming replaces any prior
 * timer for the same window so it can only fire once.
 */
export function armStopWatchdog(
  windowId: string,
  onExpire: (windowId: string) => void,
  ms: number = STOP_WATCHDOG_MS,
): void {
  clearStopWatchdog(windowId);
  const handle = setTimeout(() => {
    timers.delete(windowId);
    onExpire(windowId);
  }, ms);
  timers.set(windowId, handle);
}

/** Cancel a pending watchdog (terminal event arrived, or stop was rolled back). */
export function clearStopWatchdog(windowId: string): void {
  const handle = timers.get(windowId);
  if (handle !== undefined) {
    clearTimeout(handle);
    timers.delete(windowId);
  }
}

/** Number of armed watchdogs — inspection/test aid. */
export function activeStopWatchdogCount(): number {
  return timers.size;
}

/**
 * Decide the row patch when a watchdog fires. Only escalate if the row is STILL
 * `stopping`; if a terminal event already moved it (removed → undefined,
 * `recording`, or `error`) the watchdog lost the race and must be a no-op.
 * Returns null when no action is needed.
 */
export function resolveStopTimeout(
  state: ActiveMeetingState | undefined,
): { state: ActiveMeetingState; error: string } | null {
  if (state !== 'stopping') return null;
  return { state: 'error', error: STOP_TIMEOUT_MESSAGE };
}
