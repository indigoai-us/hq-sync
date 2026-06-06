import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { get } from 'svelte/store';

// ── Tauri API mocks ───────────────────────────────────────────────────────────
//
// `activeMeetings.ts` imports `invoke` (commands) and `listen` (events). We mock
// both: `invoke` is stubbed per-test; `listen` records every handler the module
// registers so a test can fire a synthetic Tauri event (e.g. the Rust-side
// `recording:error`, or the bridge's `meeting:closed`) straight at the real
// consumer.
//
// NOTE: we deliberately do NOT mock `./stopWatchdog` here. The bridge-death /
// stop-hang suite asserts the REAL watchdog behaviour (armed count, the 12s
// timeout escalation, the timeout message), so the genuine module has to run.
// The `meeting:closed` suite drives the real `stopRecording`, which arms a real
// watchdog — its block uses fake timers + `clearStopWatchdog` cleanup so those
// timers never leak.

const invokeMock = vi.fn();
type Handler = (event: { payload: unknown }) => void;
const handlers = new Map<string, Handler>();

vi.mock('@tauri-apps/api/core', () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));

vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn((name: string, handler: Handler) => {
    handlers.set(name, handler);
    // `listen` resolves to an unlisten fn.
    return Promise.resolve(() => handlers.delete(name));
  }),
}));

import {
  activeMeetings,
  ensureActiveMeetingListeners,
  seedActiveMeetingsFromBackend,
  stopActiveMeetingListeners,
  stopRecording,
  upsertActiveMeeting,
  type ActiveMeeting,
  type ActiveMeetingState,
} from './activeMeetings';
import {
  STOP_TIMEOUT_MESSAGE,
  STOP_WATCHDOG_MS,
  activeStopWatchdogCount,
  clearStopWatchdog,
} from './stopWatchdog';

/** Emit a synthetic Tauri event to the handler `activeMeetings.ts` registered. */
function emit(name: string, payload: unknown): void {
  const handler = handlers.get(name);
  if (!handler) throw new Error(`no handler registered for ${name}`);
  handler({ payload });
}

function seedRow(windowId: string, state: ActiveMeetingState): ActiveMeeting {
  const row: ActiveMeeting = {
    windowId,
    platform: 'meet',
    meetingUrl: `recall-window:${windowId}`,
    detectedAt: '2026-06-03T10:00:00.000Z',
    state,
    companyUid: null,
  };
  upsertActiveMeeting(row);
  return row;
}

function rowState(windowId: string): ActiveMeeting | undefined {
  return get(activeMeetings).find((m) => m.windowId === windowId);
}

describe('activeMeetings — desktop-alt stop path arms the watchdog', () => {
  beforeEach(() => {
    vi.useFakeTimers();
    invokeMock.mockReset();
    activeMeetings.set([]);
  });

  afterEach(() => {
    for (const id of ['win-1', 'win-2']) clearStopWatchdog(id);
    vi.clearAllTimers();
    vi.useRealTimers();
  });

  it('stopRecording arms a watchdog and the spinner resolves to error on timeout', async () => {
    // This is the exact path MeetingsPage.svelte (desktop-alt) calls via
    // `onstop={stopRecording}`. With no terminal event arriving, the row must
    // not hang in `stopping` — the watchdog escalates it to `error`.
    invokeMock.mockResolvedValue(undefined); // stop_recording bridge ack
    seedRow('win-1', 'recording');

    await stopRecording('win-1');

    // Row flipped to `stopping` and a watchdog is armed.
    expect(rowState('win-1')?.state).toBe('stopping');
    expect(activeStopWatchdogCount()).toBe(1);
    expect(invokeMock).toHaveBeenCalledWith('stop_recording', { windowId: 'win-1' });

    // No SDK confirmation ever comes (bridge stalled). Watchdog fires.
    vi.advanceTimersByTime(STOP_WATCHDOG_MS);

    const row = rowState('win-1');
    expect(row?.state).toBe('error');
    expect(row?.error).toBe(STOP_TIMEOUT_MESSAGE);
    expect(activeStopWatchdogCount()).toBe(0);
  });

  it('a bridge ack failure rolls the row back to recording and disarms the watchdog', async () => {
    // If the bridge rejects the stop command outright, we're still recording —
    // the watchdog must be cancelled so it can't later fire a bogus `error`.
    invokeMock.mockRejectedValue('bridge not running');
    seedRow('win-1', 'recording');

    await stopRecording('win-1');

    expect(rowState('win-1')?.state).toBe('recording');
    expect(activeStopWatchdogCount()).toBe(0);
    vi.advanceTimersByTime(STOP_WATCHDOG_MS * 2);
    expect(rowState('win-1')?.state).toBe('recording');
  });
});

describe('activeMeetings — Rust bridge-death terminal event resolves the row', () => {
  beforeEach(async () => {
    vi.useFakeTimers();
    invokeMock.mockReset();
    invokeMock.mockResolvedValue(undefined);
    activeMeetings.set([]);
    handlers.clear();
    stopActiveMeetingListeners();
    // Install the real listeners (registers the `recording:error` consumer).
    await ensureActiveMeetingListeners();
  });

  afterEach(() => {
    for (const id of ['win-1', 'win-2']) clearStopWatchdog(id);
    stopActiveMeetingListeners();
    vi.clearAllTimers();
    vi.useRealTimers();
  });

  it('registers a recording:error listener', () => {
    expect(handlers.has('recording:error')).toBe(true);
  });

  it('transitions a hung `stopping` row to error and cancels the watchdog', async () => {
    // The actual B3 fix: on an unexpected sidecar death, Rust synthesizes a
    // terminal `recording:error` (cmd `bridge-exit`). The UI must leave
    // `stopping` for `error` immediately — NOT wait out the 12s watchdog.
    seedRow('win-1', 'recording');
    await stopRecording('win-1');
    expect(rowState('win-1')?.state).toBe('stopping');
    expect(activeStopWatchdogCount()).toBe(1);

    // Rust ProcessEvent::Exit → recording:error for the in-flight window.
    emit('recording:error', {
      cmd: 'bridge-exit',
      windowId: 'win-1',
      message: 'Recording engine exited unexpectedly — the recording may not have been saved.',
    });

    const row = rowState('win-1');
    expect(row?.state).toBe('error');
    expect(row?.error).toContain('exited unexpectedly');
    // Watchdog cancelled by the terminal event — it must not also fire.
    expect(activeStopWatchdogCount()).toBe(0);
    vi.advanceTimersByTime(STOP_WATCHDOG_MS * 2);
    expect(rowState('win-1')?.error).toContain('exited unexpectedly');
  });

  it('resolves a still-`recording` row (user never pressed Stop) on bridge death', () => {
    // Bridge dies mid-recording with no Stop pressed → no watchdog was armed.
    // The terminal event is the ONLY thing that can move the row, and it must.
    seedRow('win-2', 'recording');
    expect(activeStopWatchdogCount()).toBe(0);

    emit('recording:error', {
      cmd: 'bridge-exit',
      windowId: 'win-2',
      message: 'Recording engine exited unexpectedly (signal 9)',
    });

    const row = rowState('win-2');
    expect(row?.state).toBe('error');
    expect(row?.error).toContain('bridge-exit');
  });

  it('recording:ended for the same window still clears the watchdog (clean stop wins)', async () => {
    // Sanity: the normal clean-stop terminal path remains intact — a
    // `recording:ended` arriving before the watchdog removes the row.
    seedRow('win-1', 'recording');
    await stopRecording('win-1');
    expect(activeStopWatchdogCount()).toBe(1);

    emit('recording:ended', {
      windowId: 'win-1',
      platform: 'meet',
      endedAt: '2026-06-03T10:30:00.000Z',
    });

    expect(rowState('win-1')).toBeUndefined();
    expect(activeStopWatchdogCount()).toBe(0);
  });
});

// Regression for B2: when a meeting CALL ends, the bridge emits `meeting:closed`.
// If the bridge's auto-stop was missed, the UI must NOT silently drop a row that
// is still recording (which would leak a running recording) — it must finalize
// the row through the normal stop path (`invoke('stop_recording', {windowId})`).
// A row that isn't actively recording is still just removed. These tests pin the
// `meeting:closed` listener behaviour against the real `stopRecording`/watchdog.
describe('activeMeetings meeting:closed listener', () => {
  /** Dispatch a synthetic `meeting:closed` and let the void stopRecording settle. */
  async function emitMeetingClosed(windowId: string): Promise<void> {
    const handler = handlers.get('meeting:closed');
    if (!handler) throw new Error('meeting:closed listener not registered');
    handler({ payload: { windowId, platform: 'zoom', closedAt: '2026-06-03T11:00:00.000Z' } });
    // Let any microtasks (the void stopRecording promise) settle.
    await Promise.resolve();
  }

  beforeEach(async () => {
    // The close path routes recording/starting/stopping rows through the real
    // `stopRecording`, which arms a real watchdog — fake timers + the afterEach
    // cleanup keep those 12s timers from leaking across tests.
    vi.useFakeTimers();
    invokeMock.mockReset();
    invokeMock.mockResolvedValue(undefined);
    activeMeetings.set([]);
    handlers.clear();
    stopActiveMeetingListeners();
    await ensureActiveMeetingListeners();
  });

  afterEach(() => {
    for (const id of ['win-1', 'win-2']) clearStopWatchdog(id);
    stopActiveMeetingListeners();
    activeMeetings.set([]);
    vi.clearAllTimers();
    vi.useRealTimers();
  });

  it('finalizes a recording row via stop_recording instead of removing it', async () => {
    seedRow('win-1', 'recording');

    await emitMeetingClosed('win-1');

    expect(invokeMock).toHaveBeenCalledWith('stop_recording', { windowId: 'win-1' });
    // Row is NOT dropped — it transitions to stopping (handled by stopRecording),
    // so the still-running recording is finalized rather than leaked.
    const row = get(activeMeetings).find((m) => m.windowId === 'win-1');
    expect(row).toBeDefined();
    expect(row?.state).toBe('stopping');
  });

  it.each<ActiveMeetingState>(['starting', 'stopping'])(
    'also routes a %s row through stop_recording (mid-transition close)',
    async (state) => {
      seedRow('win-1', state);

      await emitMeetingClosed('win-1');

      expect(invokeMock).toHaveBeenCalledWith('stop_recording', { windowId: 'win-1' });
      expect(get(activeMeetings).find((m) => m.windowId === 'win-1')).toBeDefined();
    },
  );

  it('removes a non-recording (detected) row without calling stop_recording', async () => {
    seedRow('win-1', 'detected');

    await emitMeetingClosed('win-1');

    expect(invokeMock).not.toHaveBeenCalledWith('stop_recording', expect.anything());
    expect(get(activeMeetings).find((m) => m.windowId === 'win-1')).toBeUndefined();
  });

  it('removes an errored row without calling stop_recording', async () => {
    seedRow('win-1', 'error');

    await emitMeetingClosed('win-1');

    expect(invokeMock).not.toHaveBeenCalledWith('stop_recording', expect.anything());
    expect(get(activeMeetings).find((m) => m.windowId === 'win-1')).toBeUndefined();
  });
});

describe('seedActiveMeetingsFromBackend — overlays live recording state (desktop-alt late-open)', () => {
  beforeEach(() => {
    invokeMock.mockReset();
    activeMeetings.set([]);
  });

  it('flips a seeded detection to recording when the ledger says it is recording', async () => {
    // The disconnect this fixes: the on-demand desktop-alt window opens AFTER a
    // recording started (user clicked Record on the notification first), so it
    // missed the live `recording:started`. Detections seed `detected`; the
    // recordings ledger says win-1 is recording → the row must show `recording`,
    // carrying the ledger's recordingId + company, with detection metadata kept.
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'meetings_list_active_detections') {
        return Promise.resolve([
          {
            windowId: 'win-1',
            platform: 'zoom',
            meetingUrl: 'https://zoom.us/j/1',
            detectedAt: '2026-06-06T14:56:59Z',
          },
        ]);
      }
      if (cmd === 'meetings_list_active_recordings') {
        return Promise.resolve([
          {
            windowId: 'win-1',
            recordingId: 'rec_1',
            companyUid: 'cmp_1',
            startedAt: '2026-06-06T14:57:06Z',
          },
        ]);
      }
      return Promise.resolve([]);
    });

    await seedActiveMeetingsFromBackend();

    const row = rowState('win-1');
    expect(row?.state).toBe('recording');
    expect(row?.recordingId).toBe('rec_1');
    expect(row?.companyUid).toBe('cmp_1');
    expect(row?.platform).toBe('zoom'); // detection metadata preserved
  });

  it('creates a recording row when the ledger has a recording with no retained detection', async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'meetings_list_active_detections') return Promise.resolve([]);
      if (cmd === 'meetings_list_active_recordings') {
        return Promise.resolve([
          {
            windowId: 'win-9',
            recordingId: 'rec_9',
            companyUid: null,
            startedAt: '2026-06-06T15:00:00Z',
          },
        ]);
      }
      return Promise.resolve([]);
    });

    await seedActiveMeetingsFromBackend();

    const row = rowState('win-9');
    expect(row?.state).toBe('recording');
    expect(row?.recordingId).toBe('rec_9');
  });

  it('leaves a detection as detected when the ledger reports no active recordings', async () => {
    invokeMock.mockImplementation((cmd: string) => {
      if (cmd === 'meetings_list_active_detections') {
        return Promise.resolve([
          {
            windowId: 'win-1',
            platform: 'meet',
            meetingUrl: 'https://meet.google.com/x',
            detectedAt: '2026-06-06T14:00:00Z',
          },
        ]);
      }
      if (cmd === 'meetings_list_active_recordings') return Promise.resolve([]);
      return Promise.resolve([]);
    });

    await seedActiveMeetingsFromBackend();

    expect(rowState('win-1')?.state).toBe('detected');
  });
});
