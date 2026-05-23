<script lang="ts">
  /**
   * Compact banner shown when meeting detection is blocked by missing macOS
   * permissions. Renders at the top of the Popover (and MeetingsWindow) so
   * the user sees the friction the moment they open the app.
   *
   * Per-permission click opens System Settings directly to the right pane
   * via the `permissions_open_settings` Tauri command. macOS won't re-prompt
   * for permissions the user has already denied once — the deep-link is the
   * only path back to a granted state.
   */
  import { invoke } from '@tauri-apps/api/core';
  import {
    permissions,
    permissionState,
    missingPermissions,
    PERMISSION_LABELS,
    type PermissionKey,
  } from '../lib/permissionState.svelte';

  // Visible only after:
  //   1. the Phase-0 eligibility check has resolved truthy (the project owner
  //      / HQ_SYNC_MEETING_DETECT_FORCE=1), AND
  //   2. the bridge has reported at least one status (avoids a banner flash on
  //      a correctly-configured machine).
  //
  // For users outside the allowlist the SDK never spawns, so they would never
  // see a status report — without (1) the banner would silently stay hidden
  // via (2), but keeping the check explicit makes the gate obvious to future
  // readers and stays correct if the SDK ever boots earlier than the gate.
  const visible = $derived(
    permissionState.meetingDetectEligible === true &&
      permissionState.initialized &&
      !permissionState.allGranted,
  );

  const missing = $derived(missingPermissions());

  async function openPermission(perm: PermissionKey) {
    try {
      // Force HQ Sync.app to be in the TCC list before opening Settings.
      // Accessibility + Screen Recording need the main app process to call
      // their native macOS APIs once — the SDK's child binary doesn't count
      // because TCC has cached denials for it from earlier dev runs.
      // Best-effort: a failure here is fine; just open Settings anyway.
      if (perm === 'accessibility' || perm === 'screen-capture') {
        try {
          await invoke('permissions_force_native_register');
        } catch (err) {
          console.warn('permissions_force_native_register:', err);
        }
      }
      await invoke('permissions_open_settings', { permission: perm });
    } catch (err) {
      console.error('permissions_open_settings failed:', err);
    }
  }
</script>

{#if visible}
  <div class="permissions-banner" role="alert">
    <div class="banner-title">
      Meeting detection needs {missing.length} macOS
      permission{missing.length === 1 ? '' : 's'}
    </div>
    <div class="banner-rows">
      {#each missing as perm (perm)}
        <button
          type="button"
          class="permission-row"
          onclick={() => openPermission(perm)}
          title="Open System Settings → Privacy & Security → {PERMISSION_LABELS[perm]}"
        >
          <span class="permission-label">{PERMISSION_LABELS[perm]}</span>
          <span class="permission-status">
            {permissions[perm] === 'denied'
              ? 'Denied'
              : permissions[perm] === 'not-determined'
                ? 'Not set'
                : permissions[perm] === 'restricted'
                  ? 'Restricted'
                  : 'Needed'}
          </span>
          <span class="permission-cta">Open Settings →</span>
        </button>
      {/each}
    </div>
    <div class="banner-hint">
      After granting, return to HQ Sync — the banner will update
      automatically.
    </div>
  </div>
{/if}

<style>
  .permissions-banner {
    margin: 8px 12px 0;
    padding: 10px 12px;
    border-radius: 8px;
    background: rgba(255, 159, 28, 0.12);
    border: 1px solid rgba(255, 159, 28, 0.35);
    color: rgba(255, 255, 255, 0.92);
    font-size: 12px;
    line-height: 1.4;
  }

  .banner-title {
    font-weight: 600;
    margin-bottom: 6px;
    color: rgba(255, 200, 120, 1);
  }

  .banner-rows {
    display: flex;
    flex-direction: column;
    gap: 4px;
    margin-bottom: 6px;
  }

  .permission-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    padding: 6px 8px;
    border-radius: 6px;
    background: rgba(255, 255, 255, 0.05);
    border: 1px solid rgba(255, 255, 255, 0.08);
    color: inherit;
    font: inherit;
    text-align: left;
    cursor: pointer;
    transition: background 0.12s ease;
  }

  .permission-row:hover {
    background: rgba(255, 255, 255, 0.1);
  }

  .permission-label {
    flex: 1 1 auto;
    font-weight: 500;
  }

  .permission-status {
    flex: 0 0 auto;
    font-size: 11px;
    opacity: 0.7;
    text-transform: uppercase;
    letter-spacing: 0.02em;
  }

  .permission-cta {
    flex: 0 0 auto;
    font-size: 11px;
    color: rgba(120, 180, 255, 1);
  }

  .banner-hint {
    font-size: 11px;
    opacity: 0.6;
  }
</style>
