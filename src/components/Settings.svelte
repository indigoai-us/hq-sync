<script lang="ts">
  import { invoke } from '@tauri-apps/api/core';
  import { getVersion } from '@tauri-apps/api/app';

  interface Props {
    onback: () => void;
  }

  let { onback }: Props = $props();

  let hqPath = $state<string | null>(null);
  let syncOnLaunch = $state(false);
  let notifications = $state(true);
  let startAtLogin = $state(true);
  let realtimeSync = $state(true);
  // Defaults to true so a brand-new install matches pre-5.25 behavior.
  // When false, Sync Now (and Auto-sync) drop the personal target from
  // the spawned hq-sync-runner's fanout — only cloud-enabled company
  // memberships sync. See src-tauri/src/commands/sync.rs.
  let personalSyncEnabled = $state(true);
  // Instant sync (event-driven). Kept a DISTINCT toggle from Auto-sync rather
  // than folded into it (the PRD allowed either) — Auto-sync controls whether
  // the daemon runs at all, Instant-sync controls whether it pushes on every
  // local edit vs the 10-minute poll, so two switches read clearer than one
  // overloaded control. Defaults true to match the realtime_sync default-on
  // convention; the backend only honors it for @getindigo.ai (Phase 1
  // event_push_eligible) identities. See src-tauri/src/commands/daemon.rs.
  let instantSync = $state(true);
  let loading = $state(true);
  let savedFeedback = $state(false);
  let savedTimeout: ReturnType<typeof setTimeout> | null = null;

  // Updater UI state. `checking` blocks the button and shows a spinner;
  // `result` is a transient status line ("Up to date" / "v0.1.8 ready").
  // Backend is authoritative — if it emits `update:available`, App.svelte's
  // listener shows the install banner regardless of what we render here.
  let updateChecking = $state(false);
  let updateResult = $state<string | null>(null);
  let updateResultTimeout: ReturnType<typeof setTimeout> | null = null;

  // App version pulled from tauri.conf.json at runtime via the Tauri API.
  // Sourced from a single place (the Rust bundle metadata) so it stays in
  // sync with the binary the user is actually running.
  let appVersion = $state<string>('');

  let pathDisplay = $derived(
    hqPath ? hqPath.replace(/^\/Users\/[^/]+/, '~') : '~/hq'
  );

  async function loadSettings() {
    try {
      const [settings, autostart] = await Promise.all([
        invoke<{
          hqPath: string | null;
          syncOnLaunch: boolean | null;
          notifications: boolean | null;
          startAtLogin: boolean | null;
          realtimeSync: boolean | null;
          personalSyncEnabled: boolean | null;
          instantSync: boolean | null;
        }>('get_settings'),
        invoke<boolean>('get_autostart_enabled'),
      ]);

      hqPath = settings.hqPath;
      syncOnLaunch = settings.syncOnLaunch ?? false;
      notifications = settings.notifications ?? true;
      startAtLogin = settings.startAtLogin ?? autostart;
      realtimeSync = settings.realtimeSync ?? true;
      personalSyncEnabled = settings.personalSyncEnabled ?? true;
      instantSync = settings.instantSync ?? true;
    } catch (err) {
      console.error('Failed to load settings:', err);
    } finally {
      loading = false;
    }
  }

  function showSaved() {
    if (savedTimeout) clearTimeout(savedTimeout);
    savedFeedback = true;
    savedTimeout = setTimeout(() => {
      savedFeedback = false;
    }, 1000);
  }

  async function saveAll() {
    try {
      await invoke('save_settings', {
        prefs: {
          hqPath,
          syncOnLaunch,
          notifications,
          startAtLogin,
          realtimeSync,
          personalSyncEnabled,
          instantSync,
        },
      });
      showSaved();
    } catch (err) {
      console.error('Failed to save settings:', err);
    }
  }

  async function handlePickFolder() {
    try {
      const picked = await invoke<string | null>('pick_folder');
      if (picked !== null) {
        hqPath = picked;
        await saveAll();
      }
    } catch (err) {
      console.error('Failed to pick folder:', err);
    }
  }

  async function handleToggleSyncOnLaunch() {
    syncOnLaunch = !syncOnLaunch;
    await saveAll();
  }

  async function handleToggleNotifications() {
    notifications = !notifications;
    await saveAll();
  }

  async function handleToggleRealtimeSync() {
    realtimeSync = !realtimeSync;
    await saveAll();
    try {
      if (realtimeSync) {
        await invoke('start_daemon');
      } else {
        await invoke('stop_daemon');
      }
    } catch (err) {
      // Surface in console — the toggle's persisted state is still authoritative,
      // and main.rs auto-starts the daemon on next launch when the flag is set.
      console.error('Auto-sync daemon command failed:', err);
    }
  }

  async function handleTogglePersonalSync() {
    personalSyncEnabled = !personalSyncEnabled;
    await saveAll();
  }

  async function handleToggleInstantSync() {
    instantSync = !instantSync;
    await saveAll();
    // The instant-sync flag is read at daemon spawn time
    // (build_watch_runner_args). If Auto-sync is already running, bounce the
    // daemon so the new `--event-push` argv takes effect immediately; the
    // backend still gates it to event_push_eligible() identities. If Auto-sync
    // is off, the persisted setting is picked up the next time the daemon starts.
    if (!realtimeSync) return;
    try {
      await invoke('stop_daemon');
      await invoke('start_daemon');
    } catch (err) {
      console.error('Instant-sync daemon restart failed:', err);
    }
  }

  async function handleToggleStartAtLogin() {
    startAtLogin = !startAtLogin;
    try {
      await invoke('set_autostart_enabled', { enabled: startAtLogin });
    } catch (err) {
      console.error('Failed to set autostart:', err);
    }
    await saveAll();
  }

  async function handleCheckForUpdates() {
    if (updateChecking) return;
    updateChecking = true;
    updateResult = null;
    if (updateResultTimeout) clearTimeout(updateResultTimeout);
    try {
      const info = await invoke<{ version: string; body?: string; date?: string } | null>(
        'check_for_updates'
      );
      updateResult = info ? `v${info.version} ready` : 'Up to date';
    } catch (err) {
      console.error('check_for_updates failed:', err);
      updateResult = 'Check failed';
    } finally {
      updateChecking = false;
      // Clear the result after a few seconds so it doesn't linger forever
      updateResultTimeout = setTimeout(() => {
        updateResult = null;
      }, 4000);
    }
  }

  $effect(() => {
    loadSettings();
    getVersion()
      .then((v) => {
        appVersion = v;
      })
      .catch((err) => console.error('Failed to read app version:', err));
    return () => {
      if (savedTimeout) clearTimeout(savedTimeout);
      if (updateResultTimeout) clearTimeout(updateResultTimeout);
    };
  });
</script>

<div class="settings">
  <!-- Header -->
  <header class="settings-header">
    <button class="back-button" onclick={onback} aria-label="Back to main view">
      <svg width="16" height="16" viewBox="0 0 16 16" fill="none" xmlns="http://www.w3.org/2000/svg" aria-hidden="true">
        <path d="M10 12L6 8l4-4" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" />
      </svg>
    </button>
    <h1>Settings</h1>
    <span class="saved-indicator" class:visible={savedFeedback}>Saved</span>
  </header>

  <div class="settings-divider"></div>

  {#if loading}
    <div class="settings-loading">
      <span class="dot-spinner"></span>
    </div>
  {:else}
    <div class="settings-body">
      <!-- HQ Folder Path -->
      <div class="setting-row">
        <div class="setting-info">
          <span class="setting-label">HQ Folder</span>
          <span class="setting-path" title={hqPath ?? ''}>{pathDisplay}</span>
        </div>
        <button class="change-button" onclick={handlePickFolder}>Change...</button>
      </div>

      <div class="settings-divider"></div>

      <!-- Sync on Launch -->
      <div class="setting-row">
        <div class="setting-info">
          <label class="setting-label" for="toggle-sync-launch">Sync on Launch</label>
          <span class="setting-desc">Automatically sync when app starts</span>
        </div>
        <button
          id="toggle-sync-launch"
          class="toggle"
          class:active={syncOnLaunch}
          onclick={handleToggleSyncOnLaunch}
          role="switch"
          aria-checked={syncOnLaunch}
          aria-label="Sync on Launch"
        >
          <span class="toggle-knob"></span>
        </button>
      </div>

      <!-- Auto-sync — runs hq-sync-runner in --watch mode via the existing
           daemon Tauri commands, fanning out to every membership the user
           has (same as the Sync Now button). -->
      <div class="settings-divider"></div>

      <div class="setting-row">
        <div class="setting-info">
          <label class="setting-label" for="toggle-realtime-sync">Auto-sync</label>
          <span class="setting-desc">Syncs every 10 minutes with no clicks needed</span>
        </div>
        <button
          id="toggle-realtime-sync"
          class="toggle"
          class:active={realtimeSync}
          onclick={handleToggleRealtimeSync}
          role="switch"
          aria-checked={realtimeSync}
          aria-label="Auto-sync"
        >
          <span class="toggle-knob"></span>
        </button>
      </div>

      <div class="settings-divider"></div>

      <!-- Instant sync (event-driven) — when ON, the daemon spawns the
           hq-sync-runner with --event-push so local edits upload within
           seconds of the filesystem event instead of waiting for the
           10-minute poll. The backend only honors this for @getindigo.ai
           identities (Phase 1 rollout); other users stay poll-only
           regardless. See src-tauri/src/commands/daemon.rs. -->
      <div class="setting-row">
        <div class="setting-info">
          <label class="setting-label" for="toggle-instant-sync">Instant sync</label>
          <span class="setting-desc">Push local edits within seconds instead of every 10 minutes</span>
        </div>
        <button
          id="toggle-instant-sync"
          class="toggle"
          class:active={instantSync}
          onclick={handleToggleInstantSync}
          role="switch"
          aria-checked={instantSync}
          aria-label="Instant sync"
        >
          <span class="toggle-knob"></span>
        </button>
      </div>

      <div class="settings-divider"></div>

      <!-- Sync personal vault — when OFF, the menubar passes --skip-personal
           to the spawned hq-sync-runner so the personal target is dropped
           from the --companies fanout. Only cloud-enabled company
           memberships sync. Defaults ON to preserve pre-5.25 behavior. -->
      <div class="setting-row">
        <div class="setting-info">
          <label class="setting-label" for="toggle-personal-sync">Sync personal vault</label>
          <span class="setting-desc">Sync your personal HQ files in addition to company memberships</span>
        </div>
        <button
          id="toggle-personal-sync"
          class="toggle"
          class:active={personalSyncEnabled}
          onclick={handleTogglePersonalSync}
          role="switch"
          aria-checked={personalSyncEnabled}
          aria-label="Sync personal vault"
        >
          <span class="toggle-knob"></span>
        </button>
      </div>

      <div class="settings-divider"></div>

      <!-- Notifications -->
      <div class="setting-row">
        <div class="setting-info">
          <label class="setting-label" for="toggle-notifications">Notifications</label>
          <span class="setting-desc">Show notifications for sync events</span>
        </div>
        <button
          id="toggle-notifications"
          class="toggle"
          class:active={notifications}
          onclick={handleToggleNotifications}
          role="switch"
          aria-checked={notifications}
          aria-label="Notifications"
        >
          <span class="toggle-knob"></span>
        </button>
      </div>

      <div class="settings-divider"></div>

      <!-- Start at Login -->
      <div class="setting-row">
        <div class="setting-info">
          <label class="setting-label" for="toggle-start-login">Start at Login</label>
          <span class="setting-desc">Launch HQ Sync when you log in</span>
        </div>
        <button
          id="toggle-start-login"
          class="toggle"
          class:active={startAtLogin}
          onclick={handleToggleStartAtLogin}
          role="switch"
          aria-checked={startAtLogin}
          aria-label="Start at Login"
        >
          <span class="toggle-knob"></span>
        </button>
      </div>

      <div class="settings-divider"></div>

      <!-- Check for Updates — manual trigger; background checker runs every 6h -->
      <div class="setting-row">
        <div class="setting-info">
          <span class="setting-label">Check for Updates</span>
          <span class="setting-desc">
            {updateResult ?? 'Background checks run every 6 hours'}
          </span>
        </div>
        <button
          class="change-button"
          onclick={handleCheckForUpdates}
          disabled={updateChecking}
        >
          {updateChecking ? 'Checking…' : 'Check Now'}
        </button>
      </div>

      <div class="settings-divider"></div>

      <!-- Version — read-only; sourced from tauri.conf.json via getVersion() -->
      <div class="setting-row">
        <div class="setting-info">
          <span class="setting-label">Version</span>
        </div>
        <span class="version-value">{appVersion ? `v${appVersion}` : '—'}</span>
      </div>
    </div>
  {/if}
</div>

<style>
  .settings {
    display: flex;
    flex-direction: column;
    width: 320px;
    max-height: 480px;
    background: var(--popover-bg, rgba(18, 18, 20, 0.68));
    backdrop-filter: var(--popover-blur, blur(28px) saturate(1.45));
    -webkit-backdrop-filter: var(--popover-blur, blur(28px) saturate(1.45));
    color: var(--popover-text, #e0e0e0);
    overflow-y: auto;
    border-radius: 18px;
    border: 1px solid var(--popover-border, rgba(255, 255, 255, 0.18));
    box-sizing: border-box;
  }

  /* Header */
  .settings-header {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    padding: 0.875rem 1rem;
  }

  .settings-header h1 {
    font-size: 0.9375rem;
    font-weight: 600;
    color: var(--popover-text-heading, #ffffff);
    margin: 0;
    line-height: 1.3;
    flex: 1;
  }

  .back-button {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 28px;
    height: 28px;
    padding: 0;
    background: none;
    border: none;
    border-radius: 9px;
    color: var(--popover-text-muted, #a0a0b0);
    cursor: pointer;
    transition: background-color 0.1s ease, color 0.1s ease;
    flex-shrink: 0;
  }

  .back-button:hover {
    background: var(--popover-action-hover, rgba(255, 255, 255, 0.05));
    color: var(--popover-text, #e0e0e0);
  }

  .saved-indicator {
    font-size: 0.6875rem;
    color: var(--popover-text-heading, #ffffff);
    opacity: 0;
    transition: opacity 0.2s ease;
    flex-shrink: 0;
  }

  .saved-indicator.visible {
    opacity: 1;
  }

  /* Divider */
  .settings-divider {
    height: 1px;
    background: var(--popover-divider, rgba(255, 255, 255, 0.06));
    margin: 0 0.75rem;
  }

  /* Body */
  .settings-body {
    display: flex;
    flex-direction: column;
    padding: 0.25rem 0;
  }

  .settings-loading {
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 2rem;
  }

  .dot-spinner {
    display: inline-block;
    width: 20px;
    height: 20px;
    border: 2.5px solid var(--popover-progress-track, rgba(255, 255, 255, 0.14));
    border-top-color: var(--popover-progress-fill, #ffffff);
    border-radius: 50%;
    animation: spin 0.7s linear infinite;
  }

  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }

  /* Setting row */
  .setting-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 0.75rem;
    padding: 0.75rem 1rem;
  }

  .setting-info {
    display: flex;
    flex-direction: column;
    gap: 0.125rem;
    min-width: 0;
    flex: 1;
  }

  .setting-label {
    font-size: 0.8125rem;
    font-weight: 500;
    color: var(--popover-text, #e0e0e0);
    cursor: default;
  }

  .setting-desc {
    font-size: 0.6875rem;
    color: var(--popover-text-muted, #a0a0b0);
    line-height: 1.3;
  }

  .setting-path {
    font-size: 0.6875rem;
    color: var(--popover-text-muted, #a0a0b0);
    line-height: 1.3;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  /* Change button */
  .change-button {
    font-size: 0.75rem;
    font-family: inherit;
    padding: 0.25rem 0.625rem;
    background: var(--popover-surface, rgba(255, 255, 255, 0.08));
    color: var(--popover-text-muted, #a0a0b0);
    border: 1px solid var(--popover-divider, rgba(255, 255, 255, 0.06));
    border-radius: 9px;
    cursor: pointer;
    transition: background-color 0.1s ease, color 0.1s ease, border-color 0.1s ease;
    white-space: nowrap;
    flex-shrink: 0;
  }

  .change-button:hover {
    background: var(--popover-action-hover, rgba(255, 255, 255, 0.05));
    color: var(--popover-text, #e0e0e0);
    border-color: var(--popover-border, rgba(255, 255, 255, 0.18));
  }

  /* Version value — monospace, subdued, aligned to the right like a
     value column. Not a button — purely informational. */
  .version-value {
    font-family: ui-monospace, SFMono-Regular, Menlo, Monaco, monospace;
    font-size: 0.75rem;
    color: var(--popover-text-muted, #a0a0b0);
    white-space: nowrap;
    flex-shrink: 0;
  }

  /* Toggle switch */
  .toggle {
    position: relative;
    width: 36px;
    height: 20px;
    padding: 0;
    background: var(--popover-surface, rgba(255, 255, 255, 0.08));
    border: 1px solid var(--popover-divider, rgba(255, 255, 255, 0.06));
    border-radius: 10px;
    cursor: pointer;
    transition: background-color 0.2s ease, border-color 0.2s ease;
    flex-shrink: 0;
  }

  .toggle.active {
    background: var(--popover-primary, #ffffff);
    border-color: var(--popover-primary, #ffffff);
  }

  .toggle-knob {
    position: absolute;
    top: 2px;
    left: 2px;
    width: 14px;
    height: 14px;
    background: #ffffff;
    border-radius: 50%;
    transition: transform 0.2s ease;
    pointer-events: none;
  }

  .toggle.active .toggle-knob {
    transform: translateX(16px);
    /* Active pill is `--popover-primary` (white in dark mode, black in light).
       Default knob is also white, so on dark mode the knob disappeared into
       the pill. Flip the knob to the inverted contrast color when active so
       it stays visible against the filled pill. */
    background: var(--popover-primary-text, #111113);
  }
</style>
