//! Pure orchestration for the `meeting:detected` → (popover row +
//! notification) decision, extracted from `App.svelte` so the bot-dedup
//! rule is unit-testable without mounting the component or mocking Tauri
//! IPC.
//!
//! ## The bug this guards
//!
//! A meeting already covered by an active hq-pro bot — a scheduled calendar
//! bot, or one already in the call — must surface **neither** a recordable
//! popover row **nor** a macOS notification. The bot is already handling it.
//!
//! The previous inline handler added the "detected" row *unconditionally,
//! before* the bot check, and only the notification path honoured the
//! dedup (`if (bot) return`). So a fully-scheduled calendar meeting still
//! showed "you could record this" in the popover even while its bot was
//! recording. This module makes the row and the notification share one
//! decision.

/** Raw `meeting:detected` event payload — the subset the handler reads. */
export interface MeetingDetectedPayload {
  /** The meeting URL (Zoom/Meet/Teams) or a synthetic `recall-window:<id>`. */
  meetingUrl?: string;
  /** Lowercase platform discriminator from the SDK (`zoom`, `meet`, …). */
  platform?: string;
  /** Meeting title / calendar summary, if known. */
  summary?: string;
  /** SDK source calendar-event id — secondary dedup key alongside the URL. */
  sourceEventId?: string;
  /** Canonical SDK window handle (newer bridge versions include it directly). */
  windowId?: string;
}

/**
 * Row seed handed to the store when a detection should surface as a
 * recordable meeting. Carries every non-optional `ActiveMeeting` field plus
 * the two seed defaults (`state: 'detected'`, `companyUserSet: false`), so
 * it is assignable to `App.svelte`'s `ActiveMeeting`.
 */
export interface DetectedMeetingSeed {
  windowId: string;
  platform: string;
  meetingUrl: string;
  detectedAt: string;
  state: 'detected';
  companyUid: string | null;
  companyUserSet: false;
}

/** Payload forwarded to the `meetings_notify_detected` Tauri command. */
export interface NotifyDetectedPayload {
  meetingUrl: string | null;
  windowId: string | null;
  platform: string | null;
  summary: string | null;
  sourceEventId: string | null;
}

/**
 * Side-effecting collaborators, injected so the handler stays pure and the
 * test can observe every decision with plain fakes.
 */
export interface MeetingDetectedDeps {
  /**
   * Resolve whether hq-pro already has an *active* bot for this meeting.
   * Implementations should resolve `false` for the "no bot" case; a thrown
   * error is treated as **fail-open** (surface the detection) — better to
   * over-surface once than silently swallow a real meeting.
   */
  checkActiveBot: (meetingUrl: string, eventId: string | null) => Promise<boolean>;
  /** Add/update the in-popover "detected" row (the Record affordance). */
  upsertRow: (seed: DetectedMeetingSeed) => void;
  /** Remove a row by window id (clears a stale row once a bot is found). */
  removeRow: (windowId: string) => void;
  /** Fire the macOS "Meeting detected" notification. */
  notify: (payload: NotifyDetectedPayload) => Promise<void>;
  /** Current valid default recording company UID (or null = Personal). */
  resolveValidDefault: () => string | null;
  /** ISO-8601 "now" — injected so tests are deterministic. */
  now: () => string;
  /** Optional diagnostic sink for a failed (fail-open) bot check. */
  warn?: (msg: string, err: unknown) => void;
}

/**
 * Derive the stable window id for a detection.
 *
 * Prefers the explicit `windowId` field (canonical SDK handle on newer
 * bridges). Falls back to extracting it from a synthetic
 * `recall-window:<id>` URL (URL-less detections / older bridge), and last
 * of all uses the real `meetingUrl` itself as a dedup-only key.
 */
export function resolveWindowId(payload: MeetingDetectedPayload): {
  windowId: string;
  isSyntheticUrl: boolean;
} {
  const { meetingUrl } = payload;
  const isSyntheticUrl =
    typeof meetingUrl === 'string' && meetingUrl.startsWith('recall-window:');
  const windowId =
    payload.windowId ??
    (isSyntheticUrl
      ? meetingUrl!.slice('recall-window:'.length)
      : (meetingUrl ?? ''));
  return { windowId, isSyntheticUrl };
}

/**
 * Handle a single `meeting:detected` event.
 *
 * Flow:
 *   1. Resolve the window id.
 *   2. If the URL is real (not synthetic), ask hq-pro whether an active bot
 *      already covers it. Synthetic `recall-window:<id>` URLs can never have
 *      a bot, so skip the lookup. A failed lookup fails open.
 *   3. **Covered by a bot** → clear any stale row for this window and return
 *      without notifying. Neither surface appears.
 *   4. **Not covered** → seed the recordable row and fire the notification.
 */
export async function handleMeetingDetected(
  payload: MeetingDetectedPayload,
  deps: MeetingDetectedDeps,
): Promise<void> {
  const { meetingUrl, platform, summary, sourceEventId } = payload;
  const { windowId, isSyntheticUrl } = resolveWindowId(payload);

  let hasActiveBot = false;
  if (meetingUrl && !isSyntheticUrl) {
    try {
      hasActiveBot = await deps.checkActiveBot(meetingUrl, sourceEventId ?? null);
    } catch (botErr) {
      deps.warn?.('meetings_check_bot_for_url failed, continuing to notify:', botErr);
    }
  }

  if (hasActiveBot) {
    // Already handled by a bot. Clear any row an earlier detection of this
    // same window may have added (e.g. detected before the scheduled bot
    // joined), then bail — no recordable row, no notification.
    if (windowId) deps.removeRow(windowId);
    return;
  }

  if (windowId) {
    // Seed the row with the current valid default. May be null if the
    // default-company context hasn't loaded yet — `companyUserSet: false`
    // marks this as a non-explicit seed so the loader's back-fill and the
    // start-time resolver can both safely overwrite it.
    deps.upsertRow({
      windowId,
      platform: platform ?? 'other',
      meetingUrl: meetingUrl ?? '',
      detectedAt: deps.now(),
      state: 'detected',
      companyUid: deps.resolveValidDefault(),
      companyUserSet: false,
    });
  }

  await deps.notify({
    meetingUrl: meetingUrl ?? null,
    // Pass through so the notification's action-button thread can route a
    // Record click back to start_recording.
    windowId: windowId || null,
    platform: platform ?? null,
    summary: summary ?? null,
    sourceEventId: sourceEventId ?? null,
  });
}
