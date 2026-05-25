/**
 * Reactive store for meeting-detect feature gating.
 *
 * Previously this module also tracked per-permission TCC status (via the
 * Recall Desktop SDK's `permission:status` events), but that surface was
 * removed on 2026-05-25: the SDK only emits status events on boot and
 * after explicit `requestPermission()` calls, so the popover banner /
 * Settings permissions section never noticed when the user granted a
 * permission in System Settings after the fact (System Audio in
 * particular stayed stuck at NEEDED forever). Native macOS prompts fire
 * the first time the SDK calls each TCC API and that's enough on its
 * own — no parallel UI needed.
 *
 * What's left is the Phase-0 eligibility gate, which IS reliable
 * (resolved once on mount via a Tauri command, value baked from the
 * Cognito email claim) and which other components still read to hide
 * meeting-detect surfaces for users outside the allowlist.
 */

import { invoke } from '@tauri-apps/api/core';

export const permissionState = $state({
  /**
   * Phase-0 eligibility flag — `null` while the gate is being computed,
   * `false` for users outside the allowlist, `true` for the project owner
   * (and anyone with `HQ_SYNC_MEETING_DETECT_FORCE=1` set).
   *
   * Resolved by `loadMeetingDetectEligible()` on app mount via the
   * `meeting_detect_feature_enabled` Tauri command. Components read this
   * to hide the meeting-detect toggle in Settings for users outside the
   * allowlist.
   *
   * Defaults to `null` (not yet resolved) rather than `false` so the brief
   * window before the command returns doesn't flash "not eligible" UI for
   * an eligible user.
   */
  meetingDetectEligible: null as boolean | null,
});

/**
 * Resolve the Phase-0 meeting-detect eligibility flag and cache it on
 * `permissionState`. Idempotent — re-calling is harmless (re-invokes the
 * Tauri command which is itself cached on the Rust side).
 *
 * Errors are caught and treated as not-eligible (closed-by-default). The
 * gate is best-effort UX gating; the authoritative gate is the Rust side
 * which never spawns the SDK for ineligible users regardless of UI state.
 */
export async function loadMeetingDetectEligible(): Promise<boolean> {
  try {
    const eligible = await invoke<boolean>('meeting_detect_feature_enabled');
    permissionState.meetingDetectEligible = eligible;
    return eligible;
  } catch (err) {
    console.error('meeting_detect_feature_enabled failed:', err);
    permissionState.meetingDetectEligible = false;
    return false;
  }
}
