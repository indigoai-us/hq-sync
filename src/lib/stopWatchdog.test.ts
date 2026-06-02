import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import {
  STOP_TIMEOUT_MESSAGE,
  STOP_WATCHDOG_MS,
  activeStopWatchdogCount,
  armStopWatchdog,
  clearStopWatchdog,
  resolveStopTimeout,
} from './stopWatchdog';

// Regression for the "Stopping…" hang: if the SDK never confirms a stop, the
// row would sit in `stopping` forever. The watchdog must force it to `error`
// after STOP_WATCHDOG_MS — but only if a terminal event hasn't already resolved
// the row first.

describe('stopWatchdog timers', () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });

  afterEach(() => {
    // Drop any timers a test left armed so the module-level map can't leak
    // across tests, then restore real timers.
    for (const id of ['win-1', 'win-a', 'win-b']) clearStopWatchdog(id);
    vi.clearAllTimers();
    vi.useRealTimers();
  });

  it('fires onExpire with the windowId after the timeout and self-clears', () => {
    const onExpire = vi.fn();
    armStopWatchdog('win-1', onExpire);
    expect(activeStopWatchdogCount()).toBe(1);
    expect(onExpire).not.toHaveBeenCalled();

    vi.advanceTimersByTime(STOP_WATCHDOG_MS);

    expect(onExpire).toHaveBeenCalledTimes(1);
    expect(onExpire).toHaveBeenCalledWith('win-1');
    expect(activeStopWatchdogCount()).toBe(0);
  });

  it('clearStopWatchdog cancels a pending watchdog', () => {
    const onExpire = vi.fn();
    armStopWatchdog('win-1', onExpire);
    clearStopWatchdog('win-1');
    expect(activeStopWatchdogCount()).toBe(0);

    vi.advanceTimersByTime(STOP_WATCHDOG_MS * 2);
    expect(onExpire).not.toHaveBeenCalled();
  });

  it('re-arming the same window replaces the prior timer (fires once)', () => {
    const onExpire = vi.fn();
    armStopWatchdog('win-1', onExpire);
    armStopWatchdog('win-1', onExpire);
    expect(activeStopWatchdogCount()).toBe(1);

    vi.advanceTimersByTime(STOP_WATCHDOG_MS);
    expect(onExpire).toHaveBeenCalledTimes(1);
  });

  it('tracks independent windows on independent timers', () => {
    const a = vi.fn();
    const b = vi.fn();
    armStopWatchdog('win-a', a);
    armStopWatchdog('win-b', b, STOP_WATCHDOG_MS * 2);
    expect(activeStopWatchdogCount()).toBe(2);

    vi.advanceTimersByTime(STOP_WATCHDOG_MS);
    expect(a).toHaveBeenCalledTimes(1);
    expect(b).not.toHaveBeenCalled();

    vi.advanceTimersByTime(STOP_WATCHDOG_MS);
    expect(b).toHaveBeenCalledTimes(1);
  });
});

describe('resolveStopTimeout', () => {
  it('escalates a still-stopping row to error', () => {
    expect(resolveStopTimeout('stopping')).toEqual({
      state: 'error',
      error: STOP_TIMEOUT_MESSAGE,
    });
  });

  it('is a no-op once a terminal event moved the row off stopping', () => {
    expect(resolveStopTimeout('recording')).toBeNull();
    expect(resolveStopTimeout('error')).toBeNull();
    expect(resolveStopTimeout('detected')).toBeNull();
    expect(resolveStopTimeout('starting')).toBeNull();
    // Row was removed (recording:ended) before the watchdog fired.
    expect(resolveStopTimeout(undefined)).toBeNull();
  });
});
