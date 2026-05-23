/**
 * Module-level reactive store for macOS permission state.
 *
 * The Recall Desktop SDK bridge emits `permission:status` Tauri events on
 * boot and whenever the user toggles a permission in System Settings. We
 * cache the latest status per permission here so any component can render
 * the current state without re-listening.
 *
 * Use:
 *   import { permissions, missingPermissions } from '$lib/permissionState.svelte';
 *   // permissions.accessibility === 'granted' | 'denied' | 'not-determined' | undefined
 *   // missingPermissions() returns the kebab-case keys that aren't granted
 */
import { listen } from '@tauri-apps/api/event';
import { invoke } from '@tauri-apps/api/core';

/**
 * The five macOS permissions the SDK requires. Kebab-case matches the
 * `Permission` enum in @recallai/desktop-sdk and the `RecallPermission`
 * enum on the Rust side (serde rename_all = "kebab-case").
 */
export const REQUIRED_PERMISSIONS = [
  'accessibility',
  'screen-capture',
  'microphone',
  'system-audio',
  'full-disk-access',
] as const;

export type PermissionKey = (typeof REQUIRED_PERMISSIONS)[number];

/**
 * Status strings the SDK reports. Anything other than 'granted' counts as
 * missing for UI purposes.
 */
export type PermissionStatus = 'granted' | 'denied' | 'not-determined' | 'restricted' | string;

/**
 * Human-readable labels keyed by permission. Used in the banner + Settings
 * row. Kept here so a single source of truth covers both surfaces.
 */
export const PERMISSION_LABELS: Record<PermissionKey, string> = {
  'accessibility': 'Accessibility',
  'screen-capture': 'Screen Recording',
  'microphone': 'Microphone',
  'system-audio': 'System Audio',
  'full-disk-access': 'Full Disk Access',
};

/**
 * Short explanation per permission for the Settings row. Helps the user
 * understand why each is needed.
 */
export const PERMISSION_EXPLAINERS: Record<PermissionKey, string> = {
  'accessibility': 'Detect when a meeting window opens in Zoom, Meet, Teams, etc.',
  'screen-capture': 'Record the meeting video and transcribe what is said.',
  'microphone': 'Capture your audio for the transcript.',
  'system-audio': 'Capture audio from other participants (paired with Screen Recording).',
  'full-disk-access': 'Read calendar files and store recordings locally.',
};

/**
 * Reactive map of permission → latest status. `undefined` means the SDK
 * hasn't reported on it yet (e.g. before the bridge has booted).
 *
 * Svelte 5 `$state.raw` because we replace the whole object on each
 * update rather than mutating nested keys — keeps the dep graph cheap.
 */
export const permissions = $state<Partial<Record<PermissionKey, PermissionStatus>>>({});

/**
 * True until the bridge has reported at least one permission status — used
 * to suppress the "missing permissions" banner during the brief startup
 * window before the bridge boots. Without this the banner flashes even
 * on a perfectly-configured machine.
 */
export const permissionState = $state({
  initialized: false,
  allGranted: false,
  /**
   * Phase-0 eligibility flag — `null` while the gate is being computed,
   * `false` for users outside the allowlist, `true` for the project owner
   * (and anyone with `HQ_SYNC_MEETING_DETECT_FORCE=1` set).
   *
   * Resolved by `loadMeetingDetectEligible()` on app mount via the
   * `meeting_detect_feature_enabled` Tauri command. Components read this to
   * hide the meeting-detect toggle, permissions banner, and Settings
   * permissions section when the feature isn't available to this user.
   *
   * Defaults to `null` (not yet resolved) rather than `false` so the brief
   * window before the command returns doesn't flash "not eligible" UI for an
   * eligible user.
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

/**
 * Returns the permissions that are NOT granted. Empty when everything is
 * set up correctly. Pure function — call it inside `$derived` if you want
 * a reactive subscriber.
 */
export function missingPermissions(): PermissionKey[] {
  return REQUIRED_PERMISSIONS.filter((p) => permissions[p] !== 'granted');
}

/**
 * Start listening to permission events from the Rust side. Idempotent —
 * subsequent calls are no-ops. Returns an unlisten function for cleanup.
 */
let registered = false;
let unlisteners: Array<() => void> = [];

export async function startPermissionListeners(): Promise<() => void> {
  if (registered) {
    return () => {};
  }
  registered = true;

  const unlistenStatus = await listen<{ permission: PermissionKey; status: PermissionStatus }>(
    'permission:status',
    (event) => {
      const { permission, status } = event.payload;
      // Replace-whole pattern keeps the reactive deps simple.
      Object.assign(permissions, { [permission]: status });
      permissionState.initialized = true;
      // Recompute allGranted on every status change.
      permissionState.allGranted = REQUIRED_PERMISSIONS.every(
        (p) => permissions[p] === 'granted',
      );
    },
  );
  unlisteners.push(unlistenStatus);

  const unlistenAll = await listen('permissions:all-granted', () => {
    for (const p of REQUIRED_PERMISSIONS) {
      Object.assign(permissions, { [p]: 'granted' });
    }
    permissionState.initialized = true;
    permissionState.allGranted = true;
  });
  unlisteners.push(unlistenAll);

  return () => {
    for (const u of unlisteners) {
      u();
    }
    unlisteners = [];
    registered = false;
  };
}
