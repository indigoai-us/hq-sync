import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { get, writable } from 'svelte/store';

export type ActiveMeetingState = 'detected' | 'starting' | 'recording' | 'stopping' | 'error';

export interface ActiveMeeting {
  windowId: string;
  platform: string;
  meetingUrl: string;
  detectedAt: string;
  state: ActiveMeetingState;
  recordingId?: string;
  error?: string;
  companyUid: string | null;
  summary?: string;
  sourceEventId?: string;
}

interface MeetingDetectedPayload {
  meetingUrl?: string;
  platform?: string;
  summary?: string;
  sourceEventId?: string;
  windowId?: string;
}

export const activeMeetings = writable<ActiveMeeting[]>([]);

let unlisteners: UnlistenFn[] | null = null;
let listenerPromise: Promise<() => void> | null = null;

export function upsertActiveMeeting(meeting: ActiveMeeting): void {
  activeMeetings.update((rows) => {
    const idx = rows.findIndex((row) => row.windowId === meeting.windowId);
    if (idx < 0) return [...rows, meeting];
    const next = rows.slice();
    next[idx] = meeting;
    return next;
  });
}

export function updateActiveMeeting(windowId: string, patch: Partial<ActiveMeeting>): void {
  activeMeetings.update((rows) =>
    rows.map((row) => (row.windowId === windowId ? { ...row, ...patch } : row)),
  );
}

export function removeActiveMeeting(windowId: string): void {
  activeMeetings.update((rows) => rows.filter((row) => row.windowId !== windowId));
}

export async function startRecording(windowId: string): Promise<void> {
  updateActiveMeeting(windowId, { state: 'starting', error: undefined });
  const row = get(activeMeetings).find((meeting) => meeting.windowId === windowId);
  try {
    const recordingId = await invoke<string>('start_recording', {
      windowId,
      companyUid: row?.companyUid ?? null,
    });
    updateActiveMeeting(windowId, { recordingId });
  } catch (err) {
    console.error('start_recording failed:', err);
    updateActiveMeeting(windowId, {
      state: 'error',
      error: typeof err === 'string' ? err : String(err),
    });
  }
}

export async function stopRecording(windowId: string): Promise<void> {
  updateActiveMeeting(windowId, { state: 'stopping' });
  try {
    await invoke('stop_recording', { windowId });
  } catch (err) {
    console.error('stop_recording failed:', err);
    updateActiveMeeting(windowId, {
      state: 'recording',
      error: typeof err === 'string' ? err : String(err),
    });
  }
}

export function ensureActiveMeetingListeners(): Promise<() => void> {
  if (unlisteners) return Promise.resolve(stopActiveMeetingListeners);
  if (listenerPromise) return listenerPromise;

  listenerPromise = installActiveMeetingListeners();
  return listenerPromise;
}

export function stopActiveMeetingListeners(): void {
  unlisteners?.forEach((unlisten) => unlisten());
  unlisteners = null;
  listenerPromise = null;
}

async function installActiveMeetingListeners(): Promise<() => void> {
  const offs = await Promise.all([
    listen<MeetingDetectedPayload>('meeting:detected', handleMeetingDetected),
    listen<{ windowId: string; platform: string; startedAt: string }>(
      'recording:started',
      (event) => {
        updateActiveMeeting(event.payload.windowId, {
          state: 'recording',
          error: undefined,
        });
      },
    ),
    listen<{ windowId: string; platform: string; endedAt: string }>(
      'recording:ended',
      (event) => {
        removeActiveMeeting(event.payload.windowId);
      },
    ),
    listen<{ cmd: string; windowId: string; message: string }>('recording:error', (event) => {
      updateActiveMeeting(event.payload.windowId, {
        state: 'error',
        error: `${event.payload.cmd}: ${event.payload.message}`,
      });
    }),
    listen<{ windowId: string; platform: string; closedAt: string }>(
      'meeting:closed',
      (event) => {
        removeActiveMeeting(event.payload.windowId);
      },
    ),
    listen<{ action: string; windowId: string; platform: string }>(
      'notification:meeting-action',
      async (event) => {
        const { action, windowId } = event.payload;
        if (action === 'record' && windowId) {
          await startRecording(windowId);
          invoke('meetings_clear_prompt_badge').catch(() => undefined);
          return;
        }
        if (action === 'open') {
          invoke('show_main_window').catch(() => undefined);
          invoke('meetings_clear_prompt_badge').catch(() => undefined);
        }
      },
    ),
  ]);

  unlisteners = offs;
  return stopActiveMeetingListeners;
}

async function handleMeetingDetected(event: { payload: MeetingDetectedPayload }): Promise<void> {
  const { meetingUrl, platform, summary, sourceEventId } = event.payload;
  const isSyntheticUrl =
    typeof meetingUrl === 'string' && meetingUrl.startsWith('recall-window:');
  const windowId =
    event.payload.windowId ??
    (isSyntheticUrl ? meetingUrl.slice('recall-window:'.length) : (meetingUrl ?? ''));

  if (windowId) {
    upsertActiveMeeting({
      windowId,
      platform: platform ?? 'other',
      meetingUrl: meetingUrl ?? '',
      detectedAt: new Date().toISOString(),
      state: 'detected',
      companyUid: null,
      summary,
      sourceEventId,
    });
  }

  try {
    if (meetingUrl && !isSyntheticUrl) {
      try {
        const bot = await invoke<{ botId: string } | null>('meetings_check_bot_for_url', {
          meetingUrl,
          eventId: sourceEventId ?? null,
        });
        if (bot) return;
      } catch (botErr) {
        console.warn('meetings_check_bot_for_url failed, continuing to notify:', botErr);
      }
    }
    await invoke('meetings_notify_detected', {
      payload: {
        meetingUrl: meetingUrl ?? null,
        windowId: windowId || null,
        platform: platform ?? null,
        summary: summary ?? null,
        sourceEventId: sourceEventId ?? null,
      },
    });
  } catch (err) {
    console.error('meeting:detected handler error:', err);
  }
}
