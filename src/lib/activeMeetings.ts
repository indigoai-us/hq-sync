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
import { armStopWatchdog, clearStopWatchdog, resolveStopTimeout } from './stopWatchdog';

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

/** Shape of `MeetingDetectedEvent` returned by `meetings_list_active_detections`
 *  (serde `rename_all = "camelCase"`). Mirrors the live `meeting:detected`
 *  payload plus the registry-only `detectedAt`/`source` fields. */
interface BackendDetection {
  windowId?: string;
  detectionId?: string;
  meetingUrl?: string;
  platform?: string;
  detectedAt?: string;
  source?: string;
  sourceEventId?: string;
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
  // Backstop the SDK confirmation: if no recording:ended/recording:error
  // arrives, force the row out of `stopping` so it can't hang forever.
  armStopWatchdog(windowId, (id) => {
    const row = get(activeMeetings).find((m) => m.windowId === id);
    const patch = resolveStopTimeout(row?.state);
    if (patch) updateActiveMeeting(id, patch);
  });
  try {
    await invoke('stop_recording', { windowId });
  } catch (err) {
    console.error('stop_recording failed:', err);
    // The bridge errored before the SDK got the stop — we're still recording,
    // so cancel the watchdog and roll back rather than letting it fire `error`.
    clearStopWatchdog(windowId);
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
        clearStopWatchdog(event.payload.windowId);
        updateActiveMeeting(event.payload.windowId, {
          state: 'recording',
          error: undefined,
        });
      },
    ),
    listen<{ windowId: string; platform: string; endedAt: string }>(
      'recording:ended',
      (event) => {
        clearStopWatchdog(event.payload.windowId);
        removeActiveMeeting(event.payload.windowId);
      },
    ),
    // Terminal failure for a recording. Two producers, one consumer:
    //  1. the bridge's own failActiveRecordings (an in-SDK crash while the
    //     bridge is still alive), and
    //  2. the Rust ProcessEvent::Exit handler (recall_sdk.rs), which
    //     synthesizes `cmd: "bridge-exit"` when the sidecar *process* dies
    //     unexpectedly and so cannot report anything itself (B3 residual).
    // Either way we leave `stopping`/`recording` for `error` immediately and
    // cancel any stop-watchdog — the real terminal event resolves the row, so
    // it no longer has to wait out the 12s timeout.
    listen<{ cmd: string; windowId: string; message: string }>('recording:error', (event) => {
      clearStopWatchdog(event.payload.windowId);
      updateActiveMeeting(event.payload.windowId, {
        state: 'error',
        error: `${event.payload.cmd}: ${event.payload.message}`,
      });
    }),
    listen<{ windowId: string; platform: string; closedAt: string }>(
      'meeting:closed',
      (event) => {
        const { windowId } = event.payload;
        // The call ended (host ended it / everyone left) — the SDK's only
        // call-ended signal. Defense-in-depth: if the bridge's auto-stop was
        // missed and this row is still recording (or mid start/stop), finalize
        // it through the normal stop path rather than silently dropping the row
        // and leaking a still-running recording. `stopRecording` owns the
        // watchdog, so don't pre-clear it here. A row that isn't actively
        // recording is just removed (the user closed it without recording).
        const row = get(activeMeetings).find((m) => m.windowId === windowId);
        if (row && (row.state === 'recording' || row.state === 'starting' || row.state === 'stopping')) {
          void stopRecording(windowId);
          return;
        }
        clearStopWatchdog(windowId);
        removeActiveMeeting(windowId);
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
          // Open the desktop-alt "HQ Meetings" window (Indigo-gated) on the
          // Meetings screen — the click came from a meeting prompt. If that
          // command is unavailable/denied, fall back to focusing the popover
          // so the click is never a dead end. NB: this warm path only fires
          // for the legacy mac-notification-sys Click response; UN-delivered
          // banners are handled entirely by the Rust delegate (un_notify.rs),
          // which opens the same window directly (idempotent — no double open).
          invoke('open_desktop_alt_window', { route: 'meetings' }).catch(() => {
            invoke('show_main_window').catch(() => undefined);
          });
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

  // NOTE: this listener intentionally does NOT call `meetings_notify_detected`.
  //
  // The SDK emits `meeting:detected` once, but Tauri fans it out to every
  // webview — so this listener (installed in the on-demand desktop-alt window)
  // AND the popover/main listener in `App.svelte` both wake for the same event.
  // The popover/main window is the always-present owner of the OS notification
  // (it runs `handleMeetingDetected` in `lib/meetingDetection.ts`, which does the
  // bot check + `meetings_notify_detected` fire); this window only owns the
  // in-app `$activeMeetings` store row above. Letting BOTH fire the notify was a
  // source of the double-notification bug. The Rust `claim_notify` lock is the
  // authoritative guard, but scoping the notify to one window here removes the
  // race at its source (defence-in-depth). The bot check that used to gate this
  // notify lives entirely in the popover path now.
}

/**
 * Seed `$activeMeetings` from the backend active-detection registry. The
 * desktop-alt window is created on-demand (after launch), so its JS context
 * misses any `meeting:detected` events that fired before it existed — which is
 * why a meeting detected before the window opened didn't show up. This pulls
 * the currently-active detections the SDK has recorded and runs each through
 * the same idempotent `upsertActiveMeeting` the live listener uses, so a
 * meeting detected while the window was closed appears (with a Record control)
 * the moment the window opens. Unlike the live handler this does NOT re-notify
 * or re-check bots — these detections already went through that path. Fail-soft:
 * a backend hiccup must never blank the Meetings UX.
 */
export async function seedActiveMeetingsFromBackend(): Promise<void> {
  let detections: BackendDetection[];
  try {
    detections = await invoke<BackendDetection[]>('meetings_list_active_detections');
  } catch (err) {
    console.warn('meetings_list_active_detections failed; skipping seed:', err);
    return;
  }
  for (const d of detections ?? []) {
    const meetingUrl = d.meetingUrl;
    const isSyntheticUrl =
      typeof meetingUrl === 'string' && meetingUrl.startsWith('recall-window:');
    const windowId =
      d.windowId ??
      (isSyntheticUrl ? meetingUrl!.slice('recall-window:'.length) : (meetingUrl ?? ''));
    if (!windowId) continue;
    const existing = get(activeMeetings).find((meeting) => meeting.windowId === windowId);
    upsertActiveMeeting({
      ...existing,
      windowId,
      platform: d.platform ?? existing?.platform ?? 'other',
      meetingUrl: meetingUrl ?? existing?.meetingUrl ?? '',
      detectedAt: d.detectedAt ?? existing?.detectedAt ?? new Date().toISOString(),
      state: existing?.state ?? 'detected',
      // Same attribution as the live handler: keep an explicit choice, else seed
      // the validated default (back-filled later by loadRecordingCompanyContext
      // if the context hasn't resolved yet).
      companyUid: existing?.companyUserSet
        ? (existing.companyUid ?? null)
        : (resolveValidDefault(defaultRecordingCompanyUid, get(recordingMemberships)) ??
          existing?.companyUid ??
          null),
      companyUserSet: existing?.companyUserSet ?? false,
      summary: existing?.summary,
      sourceEventId: d.sourceEventId ?? existing?.sourceEventId,
    });
  }
}
