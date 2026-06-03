import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { get } from 'svelte/store';

// Regression for B2: when a meeting CALL ends, the bridge emits `meeting:closed`.
// If the bridge's auto-stop was missed, the UI must NOT silently drop a row that
// is still recording (which would leak a running recording) — it must finalize
// the row through the normal stop path (`invoke('stop_recording', {windowId})`).
// A row that isn't actively recording is still just removed. These tests pin the
// `meeting:closed` listener behaviour, mocking the Tauri `listen`/`invoke` bridge.

// --- Mock the Tauri bridge ---
// `listen` captures each event handler so the test can dispatch synthetic
// events; `invoke` is a spy resolving benignly (start/stop/settings/memberships
// all flow through it).
const listeners = new Map<string, (event: { payload: unknown }) => void>();
const invokeMock = vi.fn();

vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async (event: string, handler: (e: { payload: unknown }) => void) => {
    listeners.set(event, handler);
    return () => listeners.delete(event);
  }),
}));

vi.mock('@tauri-apps/api/core', () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));

// Mock the watchdog so the listener path doesn't arm real timers (the watchdog
// itself is covered by stopWatchdog.test.ts). We only care that the close path
// routes to the stop command vs. removal here.
vi.mock('./stopWatchdog', () => ({
  armStopWatchdog: vi.fn(),
  clearStopWatchdog: vi.fn(),
  resolveStopTimeout: vi.fn(() => null),
}));

// Import after mocks are registered (vi.mock is hoisted, but keep the import
// here for clarity — the module reads the mocked Tauri bridge on listener setup).
import {
  activeMeetings,
  ensureActiveMeetingListeners,
  stopActiveMeetingListeners,
  upsertActiveMeeting,
  type ActiveMeeting,
  type ActiveMeetingState,
} from './activeMeetings';

function seedRow(windowId: string, state: ActiveMeetingState): ActiveMeeting {
  const row: ActiveMeeting = {
    windowId,
    platform: 'zoom',
    meetingUrl: `recall-window:${windowId}`,
    detectedAt: new Date().toISOString(),
    state,
    companyUid: null,
  };
  upsertActiveMeeting(row);
  return row;
}

async function emitMeetingClosed(windowId: string): Promise<void> {
  const handler = listeners.get('meeting:closed');
  if (!handler) throw new Error('meeting:closed listener not registered');
  handler({ payload: { windowId, platform: 'zoom', closedAt: new Date().toISOString() } });
  // Let any microtasks (the void stopRecording promise) settle.
  await Promise.resolve();
}

describe('activeMeetings meeting:closed listener', () => {
  beforeEach(async () => {
    invokeMock.mockReset();
    invokeMock.mockResolvedValue(undefined);
    activeMeetings.set([]);
    listeners.clear();
    await ensureActiveMeetingListeners();
  });

  afterEach(() => {
    stopActiveMeetingListeners();
    activeMeetings.set([]);
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
