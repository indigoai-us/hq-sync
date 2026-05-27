<script lang="ts">
  /**
   * MeetingPermissionsWindow — secondary window that walks the user through
   * granting every macOS TCC permission the Recall Desktop SDK needs.
   *
   * Architecture mirrors `MeetingsWindow.svelte`: this view self-fetches
   * its state via Tauri commands (no main-window event handshake) and
   * re-reads the snapshot on `focus` so returning from System Settings
   * refreshes the pills.
   *
   * Permission groups:
   *   - Required (gating): accessibility, screen-capture, microphone
   *   - System Audio: tied to screen-capture on macOS Sequoia+, surfaced
   *     for clarity even though it isn't separately gateable
   *   - Optional: full-disk-access. The SDK degrades gracefully without
   *     it for our current capture mode, but we surface it so the user
   *     can verify against the SDK's diagnostic output.
   */
  import { invoke } from '@tauri-apps/api/core';
  import { getCurrentWindow } from '@tauri-apps/api/window';
  import { onMount, onDestroy } from 'svelte';
  import {
    permissionState,
    loadMeetingPermissions,
    type PermStatus,
  } from '../lib/permissionState.svelte';

  // Keyed list rendered by the template. Order is the suggested grant
  // order for a new user:
  //   1. Microphone — the only one of the three required perms whose
  //      tri-state Prompt is meaningful (the others can't distinguish
  //      Prompt from Denied via the public API). Asking for the mic
  //      first triggers the native prompt before the user is anywhere
  //      near System Settings.
  //   2. Screen recording (covers System Audio on Sequoia+).
  //   3. Accessibility — separate pane; user has to flip the toggle and
  //      then macOS asks them to quit + relaunch the app.
  //   4. Full disk access — optional / advisory; last so the user isn't
  //      gated on it.
  const PERMISSIONS = [
    {
      id: 'microphone',
      title: 'Microphone',
      reason: 'Captures the meeting audio. Without this, the SDK can’t record anything you say.',
      key: 'microphone' as const,
      required: true,
    },
    {
      id: 'screen-capture',
      title: 'Screen Recording & System Audio',
      reason: 'Reads the meeting window so the SDK knows which call you’re on, and captures the other participants’ audio.',
      key: 'screenCapture' as const,
      required: true,
    },
    {
      id: 'accessibility',
      title: 'Accessibility',
      reason: 'Lets HQ Sync see which app is currently in the foreground so it can detect when you’ve joined a meeting.',
      key: 'accessibility' as const,
      required: true,
    },
    {
      id: 'full-disk-access',
      title: 'Full Disk Access',
      reason: 'Optional. Some advanced SDK capture modes need it; the current meeting-detect path works without it.',
      key: 'fullDiskAccess' as const,
      required: false,
    },
  ] as const;

  type PermissionId = (typeof PERMISSIONS)[number]['id'];

  let opening = $state<PermissionId | null>(null);
  let refreshing = $state(false);
  let lastFetchedAt = $state<number | null>(null);

  // Convenience accessors that pull from the shared reactive store.
  const snapshot = $derived(permissionState.meetingPermissions);
  const allGranted = $derived(snapshot?.allRequiredGranted ?? false);

  function statusOf(key: PermissionId): PermStatus {
    if (!snapshot) return 'unknown';
    switch (key) {
      case 'microphone': return snapshot.microphone;
      case 'screen-capture': return snapshot.screenCapture;
      case 'accessibility': return snapshot.accessibility;
      case 'full-disk-access': return snapshot.fullDiskAccess;
    }
  }

  function pillLabel(status: PermStatus): string {
    switch (status) {
      case 'granted': return 'Granted';
      case 'denied':  return 'Not granted';
      case 'prompt':  return 'Not asked yet';
      case 'unknown': return 'Check';
    }
  }

  function pillClass(status: PermStatus): string {
    switch (status) {
      case 'granted': return 'pill pill-ok';
      case 'denied':  return 'pill pill-needs';
      case 'prompt':  return 'pill pill-prompt';
      case 'unknown': return 'pill pill-unknown';
    }
  }

  async function handleOpen(id: PermissionId) {
    opening = id;
    try {
      await invoke('permissions_open_settings', { permission: id });
    } catch (err) {
      console.error('permissions_open_settings failed:', err);
    } finally {
      // Brief delay so the chrome of the button registers the click;
      // then clear so a re-click is responsive.
      setTimeout(() => {
        if (opening === id) opening = null;
      }, 250);
    }
  }

  async function handleRefresh() {
    refreshing = true;
    try {
      await loadMeetingPermissions();
      lastFetchedAt = Date.now();
    } finally {
      refreshing = false;
    }
  }

  async function handleRunNativeRegister() {
    // Triggers AVCaptureDevice.requestAccess(audio), CGRequestScreenCaptureAccess,
    // and AXIsProcessTrustedWithOptions(null) from the menubar process so
    // TCC attributes the request to the .app bundle (not a sub-process or
    // SDK helper). On macOS Sequoia+ this also covers System Audio.
    //
    // Safe to call repeatedly; first call shows native prompts for any
    // permission still in NotDetermined, subsequent calls are silent.
    try {
      await invoke('permissions_force_native_register');
      await loadMeetingPermissions();
    } catch (err) {
      console.error('permissions_force_native_register failed:', err);
    }
  }

  function handleClose() {
    getCurrentWindow().close();
  }

  let unlistenFocus: (() => void) | null = null;

  onMount(() => {
    handleRefresh();
    // Re-read the snapshot when this window regains focus — covers the
    // common flow of opening System Settings, granting, then bringing
    // this wizard back to the foreground. macOS does NOT emit any
    // permission-change event we can subscribe to; focus is the
    // canonical "user might have just changed something" hook.
    const onFocus = () => handleRefresh();
    window.addEventListener('focus', onFocus);
    unlistenFocus = () => window.removeEventListener('focus', onFocus);
  });

  onDestroy(() => {
    unlistenFocus?.();
  });
</script>

<div class="window">
  <header>
    <div>
      <h1>Meeting Permissions</h1>
      <p class="subtitle">
        HQ Sync needs three macOS privacy grants to detect meetings and record them locally.
        {#if allGranted}
          <strong>All set</strong> — you can close this window.
        {/if}
      </p>
    </div>
    <button class="close-btn" onclick={handleClose} aria-label="Close window">
      <svg width="14" height="14" viewBox="0 0 14 14" fill="none" aria-hidden="true">
        <path d="M3 3l8 8M11 3l-8 8" stroke="currentColor" stroke-width="1.5" stroke-linecap="round"/>
      </svg>
    </button>
  </header>

  <!-- Quick-prompt button — fires the native macOS prompts for any
       permission still in NotDetermined. Safer first move than dragging
       the user through System Settings; only does anything when there's
       actually an open prompt to satisfy. -->
  {#if snapshot && !allGranted && (snapshot.microphone === 'prompt' || snapshot.accessibility === 'denied' || snapshot.screenCapture === 'denied')}
    <div class="quick-prompt">
      <div class="quick-text">
        <strong>Try the quick path first.</strong>
        Click below — HQ Sync will trigger the native macOS prompt for anything still pending.
      </div>
      <button class="primary-btn" onclick={handleRunNativeRegister}>
        Trigger prompts
      </button>
    </div>
  {/if}

  <ul class="perm-list">
    {#each PERMISSIONS as perm}
      {@const status = statusOf(perm.id)}
      <li class="perm-row" data-status={status}>
        <div class="perm-meta">
          <div class="perm-head">
            <span class="perm-title">{perm.title}</span>
            {#if !perm.required}
              <span class="optional-tag">Optional</span>
            {/if}
          </div>
          <p class="perm-reason">{perm.reason}</p>
        </div>
        <div class="perm-controls">
          <span class={pillClass(status)}>{pillLabel(status)}</span>
          <!-- Always render the Open Settings button — granted users may want
               to revoke or re-grant after a TCC reset, so we shouldn't hide
               the path forward. The button label adapts to whether the
               action is grant-flavoured ("Open Settings") or
               revoke-flavoured ("Manage in Settings") so the user knows
               what to expect on the other side. -->
          <button
            class="open-btn"
            onclick={() => handleOpen(perm.id)}
            disabled={opening === perm.id}
          >
            {#if opening === perm.id}
              Opening…
            {:else if status === 'granted'}
              Manage in Settings
            {:else}
              Open Settings
            {/if}
          </button>
        </div>
      </li>
    {/each}
  </ul>

  <footer>
    <div class="footer-meta">
      {#if lastFetchedAt}
        Last checked {new Date(lastFetchedAt).toLocaleTimeString()}
      {:else}
        Loading…
      {/if}
    </div>
    <button class="refresh-btn" onclick={handleRefresh} disabled={refreshing}>
      {refreshing ? 'Refreshing…' : 'Refresh'}
    </button>
  </footer>
</div>

<style>
  /* Scope :global(html, body) to this window's `data-window` attribute
     (set in src/main.ts on mount). Without the scope, the rule bundles
     into the shared CSS file and bleeds into the popover — popover font
     family / background / size all snap to this window's values. See
     commit acc6bd5 ("scope global html/body styles per-window to stop CSS
     bleed") for the prior incident this mirrors. */
  :global(html[data-window='meeting-permissions']),
  :global(html[data-window='meeting-permissions'] body) {
    margin: 0;
    padding: 0;
    background: #18181b;
    color: #fafafa;
    font-family: -apple-system, BlinkMacSystemFont, 'Inter', system-ui, sans-serif;
    font-size: 13px;
    line-height: 1.5;
    -webkit-font-smoothing: antialiased;
  }
  :global(html[data-window='meeting-permissions'] #app) {
    height: 100vh;
  }

  .window {
    display: flex;
    flex-direction: column;
    height: 100vh;
    overflow: hidden;
  }

  header {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    padding: 18px 22px 14px;
    border-bottom: 1px solid rgba(255, 255, 255, 0.08);
  }
  header h1 {
    margin: 0 0 4px;
    font-size: 17px;
    font-weight: 500;
    letter-spacing: -0.01em;
  }
  header .subtitle {
    margin: 0;
    font-size: 12.5px;
    color: rgba(250, 250, 250, 0.62);
    max-width: 420px;
  }
  header .subtitle strong {
    color: #4ade80;
    font-weight: 500;
  }
  .close-btn {
    background: transparent;
    border: 0;
    padding: 6px;
    color: rgba(250, 250, 250, 0.55);
    cursor: pointer;
    border-radius: 6px;
  }
  .close-btn:hover {
    background: rgba(255, 255, 255, 0.08);
    color: rgba(250, 250, 250, 0.95);
  }

  .quick-prompt {
    margin: 14px 22px 0;
    padding: 12px 14px;
    background: rgba(129, 140, 248, 0.08);
    border: 1px solid rgba(129, 140, 248, 0.32);
    border-radius: 8px;
    display: flex;
    align-items: center;
    gap: 14px;
    justify-content: space-between;
  }
  .quick-text {
    font-size: 12.5px;
    color: rgba(250, 250, 250, 0.85);
  }
  .quick-text strong {
    display: block;
    margin-bottom: 2px;
    color: #fafafa;
    font-weight: 500;
  }
  .primary-btn {
    background: #818cf8;
    color: #0a0a0c;
    border: 0;
    padding: 7px 14px;
    border-radius: 6px;
    font-size: 12px;
    font-weight: 500;
    cursor: pointer;
    flex-shrink: 0;
  }
  .primary-btn:hover {
    background: #a5b4fc;
  }

  .perm-list {
    list-style: none;
    margin: 14px 0 0;
    padding: 0;
    overflow-y: auto;
    flex: 1;
  }
  .perm-row {
    display: grid;
    grid-template-columns: 1fr auto;
    gap: 14px;
    align-items: center;
    padding: 14px 22px;
    border-bottom: 1px solid rgba(255, 255, 255, 0.06);
  }
  .perm-row:last-child {
    border-bottom: 0;
  }
  .perm-head {
    display: flex;
    align-items: center;
    gap: 10px;
  }
  .perm-title {
    font-size: 13.5px;
    font-weight: 500;
    color: #fafafa;
  }
  .optional-tag {
    font-size: 10px;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: rgba(250, 250, 250, 0.42);
    background: rgba(255, 255, 255, 0.06);
    padding: 2px 6px;
    border-radius: 3px;
  }
  .perm-reason {
    margin: 4px 0 0;
    font-size: 12.5px;
    color: rgba(250, 250, 250, 0.62);
    max-width: 360px;
  }
  .perm-controls {
    display: flex;
    align-items: center;
    gap: 10px;
  }

  .pill {
    display: inline-flex;
    align-items: center;
    padding: 3px 8px;
    border-radius: 999px;
    font-size: 11px;
    font-weight: 500;
    letter-spacing: 0.01em;
  }
  .pill-ok {
    background: rgba(74, 222, 128, 0.15);
    color: #4ade80;
  }
  .pill-needs {
    background: rgba(248, 113, 113, 0.15);
    color: #f87171;
  }
  .pill-prompt {
    background: rgba(250, 204, 21, 0.15);
    color: #facc15;
  }
  .pill-unknown {
    background: rgba(255, 255, 255, 0.08);
    color: rgba(250, 250, 250, 0.55);
  }

  .open-btn {
    background: rgba(255, 255, 255, 0.08);
    color: #fafafa;
    border: 0;
    padding: 6px 12px;
    border-radius: 6px;
    font-size: 12px;
    cursor: pointer;
    font-weight: 500;
  }
  .open-btn:hover:not(:disabled) {
    background: rgba(255, 255, 255, 0.14);
  }
  .open-btn:disabled {
    opacity: 0.55;
    cursor: default;
  }

  footer {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 12px 22px;
    border-top: 1px solid rgba(255, 255, 255, 0.08);
    background: rgba(0, 0, 0, 0.18);
  }
  .footer-meta {
    font-size: 11.5px;
    color: rgba(250, 250, 250, 0.42);
  }
  .refresh-btn {
    background: transparent;
    color: rgba(250, 250, 250, 0.85);
    border: 1px solid rgba(255, 255, 255, 0.14);
    padding: 5px 11px;
    border-radius: 5px;
    font-size: 12px;
    cursor: pointer;
  }
  .refresh-btn:hover:not(:disabled) {
    background: rgba(255, 255, 255, 0.06);
  }
  .refresh-btn:disabled {
    opacity: 0.55;
    cursor: default;
  }
</style>
