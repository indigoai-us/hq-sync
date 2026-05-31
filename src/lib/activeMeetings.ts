import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { get, writable } from 'svelte/store';
import {
  activeMemberships,
  resolveStartCompany,
  resolveValidDefault,
  shouldBackfill,
  type RecordingMembership,
} from './recordingCompany';

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
  /** True once the user has explicitly picked a company for this row (incl. an
   *  explicit "Personal" = null). Guards the resolved-default + back-fill paths
   *  from clobbering a deliberate choice. */
  companyUserSet?: boolean;
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

// Recording-company context, mirrored from the classic popover. The picker in
// LiveNowCard reads `recordingMemberships` (active-filtered); `startRecording`
// and the detection seed resolve attribution against the validated default.
// Loaded via `loadRecordingCompanyContext` at store start + on focus.
export const recordingMemberships = writable<RecordingMembership[]>([]);
let defaultRecordingCompanyUid: string | null = null;

let unlisteners: UnlistenFn[] | null = null;
let listenerPromise: Promise<() => void> | null = null;

export function upsertActiveMeeting(meeting: ActiveMeeting): void {
  activeMeetings.update((rows) => {
    const idx = rows.findIndex((row) => row.windowId === meeting.windowId);
    if (idx < 0) return [...rows, meeting];
    const next = rows.slice();
    const existing = rows[idx];
    next[idx] =
      meeting.state === 'detected' && existing.state !== 'detected'
        ? {
            ...meeting,
            state: existing.state,
            recordingId: existing.recordingId,
            error: existing.error,
          }
        : meeting;
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
  // Resolve attribution the same way the classic popover does: an explicit
  // user choice wins, else the validated default, else whatever the row had.
  // Persist the resolved uid back so the row reflects the company we recorded
  // under (e.g. a detected row that only had the default reflected applied).
  const companyUid = resolveStartCompany(row, defaultRecordingCompanyUid, get(recordingMemberships));
  if (row && row.companyUid !== companyUid) {
    updateActiveMeeting(windowId, { companyUid });
  }
  try {
    const recordingId = await invoke<string>('start_recording', {
      windowId,
      companyUid,
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

/**
 * Record an explicit per-meeting company choice from the LiveNowCard picker.
 * `companyUserSet: true` marks it deliberate so the resolved-default and the
 * focus-time back-fill leave it alone. `null` is a valid choice (Personal).
 */
export function setRecordingCompany(windowId: string, companyUid: string | null): void {
  updateActiveMeeting(windowId, { companyUid, companyUserSet: true });
}

/**
 * Load the recording-company context (active memberships + validated default)
 * from the backend, then back-fill any already-detected rows that loaded
 * before this resolved. Mirrors classic App.svelte's memberships/default load.
 *
 * Both invokes fail soft to an empty/neutral result: the picker is an additive
 * affordance, so a transient backend hiccup must never blank the live meeting
 * or throw on the focus path.
 */
export async function loadRecordingCompanyContext(): Promise<void> {
  const [list, settings] = await Promise.all([
    invoke<RecordingMembership[]>('meetings_list_memberships').catch(
      () => [] as RecordingMembership[],
    ),
    invoke<{ defaultRecordingCompanyUid?: string | null }>('get_settings').catch(
      () => ({}) as { defaultRecordingCompanyUid?: string | null },
    ),
  ]);
  const active = activeMemberships(list ?? []);
  recordingMemberships.set(active);
  defaultRecordingCompanyUid = resolveValidDefault(
    settings?.defaultRecordingCompanyUid ?? null,
    active,
  );
  // Seed the resolved default onto rows detected before this loaded — without
  // overwriting an explicit user choice (shouldBackfill guards that).
  for (const m of get(activeMeetings)) {
    if (shouldBackfill(m, defaultRecordingCompanyUid)) {
      updateActiveMeeting(m.windowId, { companyUid: defaultRecordingCompanyUid });
    }
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
    const existing = get(activeMeetings).find((meeting) => meeting.windowId === windowId);
    upsertActiveMeeting({
      ...existing,
      windowId,
      platform: platform ?? existing?.platform ?? 'other',
      meetingUrl: meetingUrl ?? existing?.meetingUrl ?? '',
      detectedAt: new Date().toISOString(),
      state: existing?.state ?? 'detected',
      // Keep an explicit choice; otherwise seed the validated default so a
      // fresh detection is attributed correctly out of the gate (back-filled
      // later if the context wasn't loaded yet).
      companyUid: existing?.companyUserSet
        ? (existing.companyUid ?? null)
        : (resolveValidDefault(defaultRecordingCompanyUid, get(recordingMemberships)) ??
          existing?.companyUid ??
          null),
      companyUserSet: existing?.companyUserSet ?? false,
      summary: summary ?? existing?.summary,
      sourceEventId: sourceEventId ?? existing?.sourceEventId,
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
