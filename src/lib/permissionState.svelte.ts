/**
 * Reactive state for meeting-detect feature gating and TCC permission status.
 *
 * Originally this module also tracked per-permission TCC status via the
 * Recall Desktop SDK's `permission:status` events, but that surface was
 * removed on 2026-05-25: the SDK only emits status events on boot and
 * after explicit `requestPermission()` calls, so the popover banner /
 * Settings permissions section never noticed when the user granted a
 * permission in System Settings after the fact (System Audio in
 * particular stayed stuck at NEEDED forever).
 *
 * The current model:
 * - `meetingDetectEligible`  — Phase-0 feature gate (Cognito email claim).
 *   Resolved once on mount; never changes during a session.
 * - `meetingPermissions`     — TCC status snapshot read via the Rust
 *   `meetings_permissions_state` command. Re-read on Settings mount + on
 *   window focus, so returning from System Settings refreshes the pill.
 *   Reading is non-prompting and idempotent.
 *
 * The Settings row + the wizard window both subscribe to `meetingPermissions`.
 */

import { invoke } from '@tauri-apps/api/core';

/**
 * Status of a single TCC permission. Tri-state plus `unknown` for the
 * permissions where macOS has no public read API (Full Disk Access).
 *
 * Kept in serde-kebab-case to match the Rust `PermStatus` enum.
 */
export type PermStatus = 'granted' | 'denied' | 'prompt' | 'unknown';

/**
 * Full meeting-permissions status as returned by `meetings_permissions_state`.
 * Mirrors `MeetingPermissionsState` in `src-tauri/src/commands/permissions.rs`.
 */
export interface MeetingPermissionsSnapshot {
  accessibility: PermStatus;
  screenCapture: PermStatus;
  microphone: PermStatus;
  systemAudio: PermStatus;
  fullDiskAccess: PermStatus;
  allRequiredGranted: boolean;
}

export const permissionState = $state({
  /**
   * Phase-0 eligibility flag — `null` while the gate is being computed,
   * `false` for users outside the allowlist, `true` for @getindigo.ai
   * accounts (and anyone with `HQ_SYNC_MEETING_DETECT_FORCE=1` set).
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

  /**
   * Meeting permissions snapshot — `null` until the first
   * `loadMeetingPermissions()` resolves. Settings and the wizard window
   * both read from here; the wizard refreshes on its own focus events.
   *
   * Non-eligible users never call the loader (the Settings row is gated
   * on `meetingDetectEligible === true` first), so this stays `null` for
   * them and the row is hidden.
   */
  meetingPermissions: null as MeetingPermissionsSnapshot | null,
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

/**
 * Read the meeting-permissions snapshot via the Rust
 * `meetings_permissions_state` command and cache it on `permissionState`.
 *
 * Non-prompting — safe to call repeatedly without re-nagging the user
 * with system dialogs. The Settings row calls this on mount + on window
 * focus, so returning from System Settings updates the pill without a
 * restart.
 *
 * On error, leaves the previous snapshot in place (don't blow away a
 * good value because of a transient Tauri IPC hiccup).
 */
export async function loadMeetingPermissions(): Promise<MeetingPermissionsSnapshot | null> {
  try {
    const snapshot = await invoke<MeetingPermissionsSnapshot>('meetings_permissions_state');
    permissionState.meetingPermissions = snapshot;
    return snapshot;
  } catch (err) {
    console.error('meetings_permissions_state failed:', err);
    return permissionState.meetingPermissions;
  }
}
