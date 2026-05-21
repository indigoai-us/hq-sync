<script lang="ts">
  import { invoke } from '@tauri-apps/api/core';
  import { listen, type UnlistenFn } from '@tauri-apps/api/event';
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
  let stagingUpdateChannel = $state(false);
  let loading = $state(true);
  let savedFeedback = $state(false);
  let savedTimeout: ReturnType<typeof setTimeout> | null = null;

  // Staging-channel update state. Populated by the `hq-core-staging-update:available`
  // event (emitted by the Rust background checker when staging is ahead of the
  // local hqVersion) AND by the manual one-shot `check_hq_core_staging_update`
  // call this component runs whenever the toggle flips ON.
  let stagingUpdate = $state<{ local: string | null; latestTag: string; latestSemver: string } | null>(null);
  let stagingCheckInFlight = $state(false);
  // Manifest of what the apply will overwrite — read at confirmation time so
  // the modal lists exactly what's about to change. Lazily loaded.
  let stagingManifest = $state<{ paths: string[]; preserveSubpaths: string[] } | null>(null);
  let stagingConfirmOpen = $state(false);
  let stagingApplyRunning = $state(false);
  // Streaming progress log — bounded to last N lines so the popover doesn't
  // grow unbounded during a long apply.
  let stagingApplyLog = $state<{ stream: string; line: string }[]>([]);
  const STAGING_APPLY_LOG_MAX = 200;
  let stagingApplyError = $state<string | null>(null);
  let stagingApplyDone = $state<string | null>(null);
  let unlistenStagingAvailable: UnlistenFn | null = null;
  let unlistenStagingProgress: UnlistenFn | null = null;

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
          stagingUpdateChannel: boolean | null;
        }>('get_settings'),
        invoke<boolean>('get_autostart_enabled'),
      ]);

      hqPath = settings.hqPath;
      syncOnLaunch = settings.syncOnLaunch ?? false;
      notifications = settings.notifications ?? true;
      startAtLogin = settings.startAtLogin ?? autostart;
      realtimeSync = settings.realtimeSync ?? true;
      stagingUpdateChannel = settings.stagingUpdateChannel ?? false;
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
          stagingUpdateChannel,
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

  async function handleToggleStartAtLogin() {
    startAtLogin = !startAtLogin;
    try {
      await invoke('set_autostart_enabled', { enabled: startAtLogin });
    } catch (err) {
      console.error('Failed to set autostart:', err);
    }
    await saveAll();
  }

  // Staging-channel toggle flip. Two distinct effects on transitions:
  //   ON  → save, then fire a one-shot manual `check_hq_core_staging_update`
  //         so the user gets immediate feedback rather than waiting up to
  //         6h for the background loop to tick.
  //   OFF → save, clear any cached delta so the row hides immediately
  //         instead of waiting for the next event.
  async function handleToggleStagingChannel() {
    stagingUpdateChannel = !stagingUpdateChannel;
    await saveAll();
    if (stagingUpdateChannel) {
      try {
        stagingCheckInFlight = true;
        const info = await invoke<{
          local: string | null;
          latestTag: string;
          latestSemver: string;
        } | null>('check_hq_core_staging_update');
        stagingUpdate = info;
      } catch (err) {
        console.error('check_hq_core_staging_update failed:', err);
      } finally {
        stagingCheckInFlight = false;
      }
    } else {
      stagingUpdate = null;
      stagingManifest = null;
      stagingConfirmOpen = false;
    }
  }

  async function openStagingConfirm() {
    if (!stagingUpdate || stagingApplyRunning) return;
    try {
      stagingManifest = await invoke<{ paths: string[]; preserveSubpaths: string[] }>(
        'read_replace_from_staging_manifest'
      );
    } catch (err) {
      console.error('read_replace_from_staging_manifest failed:', err);
      stagingApplyError = String(err);
      return;
    }
    stagingApplyError = null;
    stagingApplyDone = null;
    stagingApplyLog = [];
    stagingConfirmOpen = true;
  }

  function closeStagingConfirm() {
    if (stagingApplyRunning) return;
    stagingConfirmOpen = false;
  }

  async function confirmStagingApply() {
    if (!stagingUpdate || stagingApplyRunning) return;
    stagingApplyRunning = true;
    stagingApplyError = null;
    stagingApplyDone = null;
    try {
      const result = await invoke<{ exitCode: number; tag: string }>('apply_hq_core_staging', {
        tag: stagingUpdate.latestTag,
      });
      stagingApplyDone = `Applied ${result.tag}`;
      // After a successful apply, hide the "update available" row — the
      // local hqVersion has moved up; the next background check will
      // re-surface a newer beta only when there actually is one.
      stagingUpdate = null;
    } catch (err) {
      stagingApplyError = String(err);
    } finally {
      stagingApplyRunning = false;
    }
  }

  function pushApplyLog(entry: { stream: string; line: string }) {
    // Append to a fresh array so Svelte's reactivity picks up the change.
    const next = stagingApplyLog.concat(entry);
    stagingApplyLog =
      next.length > STAGING_APPLY_LOG_MAX
        ? next.slice(next.length - STAGING_APPLY_LOG_MAX)
        : next;
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

    // Background channel for the staging poller. Fires whenever the Rust
    // checker (every 6h while toggle is on) finds staging ahead of local.
    // We don't rely on this alone — the toggle's ON transition also fires
    // a one-shot check synchronously — but this catches transitions that
    // happen mid-session without the user touching Settings.
    listen<{ local: string | null; latestTag: string; latestSemver: string }>(
      'hq-core-staging-update:available',
      (event) => {
        if (stagingUpdateChannel) {
          stagingUpdate = event.payload;
        }
      }
    )
      .then((unlisten) => {
        unlistenStagingAvailable = unlisten;
      })
      .catch((err) => console.error('listen hq-core-staging-update:available failed:', err));

    listen<{ stream: string; line: string }>('hq-core-staging-apply:progress', (event) => {
      pushApplyLog(event.payload);
    })
      .then((unlisten) => {
        unlistenStagingProgress = unlisten;
      })
      .catch((err) => console.error('listen hq-core-staging-apply:progress failed:', err));

    return () => {
      if (savedTimeout) clearTimeout(savedTimeout);
      if (updateResultTimeout) clearTimeout(updateResultTimeout);
      if (unlistenStagingAvailable) unlistenStagingAvailable();
      if (unlistenStagingProgress) unlistenStagingProgress();
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

      <!-- Staging update channel — feature-flagged toggle. When ON, the
           menubar polls indigoai-us/hq-core-staging's latest release every
           6h and renders an "Update from staging" button below this row
           when staging is ahead of local core/core.yaml#hqVersion. Replaces
           local .agents/.codex/.claude/core/.obsidian/AGENTS.md from the
           staging tag via the personal:replace-from-staging script (the
           menubar shells out to it with --paths from
           core.yaml#replace_from_staging.paths). Defaults OFF — opt-in
           bleeding-edge channel; production users should leave it off. -->
      <div class="setting-row">
        <div class="setting-info">
          <label class="setting-label" for="toggle-staging-channel">Staging Channel</label>
          <span class="setting-desc">Show updates from `hq-core-staging` betas</span>
        </div>
        <button
          id="toggle-staging-channel"
          class="toggle"
          class:active={stagingUpdateChannel}
          onclick={handleToggleStagingChannel}
          role="switch"
          aria-checked={stagingUpdateChannel}
          aria-label="Staging update channel"
        >
          <span class="toggle-knob"></span>
        </button>
      </div>

      {#if stagingUpdateChannel && (stagingUpdate || stagingCheckInFlight || stagingApplyDone)}
        <div class="setting-row staging-update-row">
          <div class="setting-info">
            <span class="setting-label">Staging update</span>
            <span class="setting-desc">
              {#if stagingCheckInFlight}
                Checking…
              {:else if stagingApplyDone}
                {stagingApplyDone}
              {:else if stagingUpdate}
                {stagingUpdate.local ?? '—'} → {stagingUpdate.latestTag}
              {/if}
            </span>
          </div>
          {#if stagingUpdate && !stagingApplyRunning && !stagingApplyDone}
            <button class="change-button" onclick={openStagingConfirm}>
              Update from staging…
            </button>
          {/if}
        </div>
      {/if}

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

  {#if stagingConfirmOpen && stagingManifest}
    <!-- Modal overlay: confirmation + live progress in one surface. The user
         can't dismiss while an apply is in flight (Cancel hidden during run)
         because cancelling mid-rsync would leave the HQ root in a half-state. -->
    <div
      class="staging-modal-backdrop"
      role="presentation"
      onclick={closeStagingConfirm}
    ></div>
    <div class="staging-modal" role="dialog" aria-modal="true" aria-labelledby="staging-modal-title">
      <h2 id="staging-modal-title">Update from staging — {stagingUpdate?.latestTag ?? ''}</h2>
      <p class="staging-modal-lead">
        This will replace the following top-level entries inside your HQ folder
        with their contents from <code>indigoai-us/hq-core-staging@{stagingUpdate?.latestTag ?? ''}</code>.
        Everything else (your <code>companies/</code>, <code>personal/</code>,
        <code>workspace/</code>, <code>.git/</code>, etc.) is left untouched.
      </p>
      <ul class="staging-modal-list">
        {#each stagingManifest.paths as p (p)}
          <li><code>{p}</code></li>
        {/each}
      </ul>
      {#if stagingManifest.preserveSubpaths.length > 0}
        <p class="staging-modal-lead">
          These sub-paths inside that set are backed up and restored across the overlay:
        </p>
        <ul class="staging-modal-list">
          {#each stagingManifest.preserveSubpaths as sp (sp)}
            <li><code>{sp}</code></li>
          {/each}
        </ul>
      {/if}
      <p class="staging-modal-warning">
        Nothing is committed — the script leaves the changes staged in <code>git status</code>
        for you to review and commit (or revert) by hand.
      </p>

      {#if stagingApplyLog.length > 0 || stagingApplyRunning || stagingApplyError || stagingApplyDone}
        <div
          class="staging-modal-log"
          role="log"
          aria-live="polite"
          aria-label="Apply progress"
        >
          {#each stagingApplyLog as entry, i (i)}
            <div class="staging-modal-log-line staging-modal-log-{entry.stream}">{entry.line}</div>
          {/each}
        </div>
      {/if}

      {#if stagingApplyError}
        <p class="staging-modal-error">Error: {stagingApplyError}</p>
      {/if}
      {#if stagingApplyDone}
        <p class="staging-modal-done">{stagingApplyDone}. Review the diff in your editor.</p>
      {/if}

      <div class="staging-modal-actions">
        {#if !stagingApplyRunning && !stagingApplyDone}
          <button class="change-button" onclick={closeStagingConfirm}>Cancel</button>
          <button
            class="change-button staging-modal-confirm"
            onclick={confirmStagingApply}
            disabled={!stagingUpdate}
          >
            Overwrite from {stagingUpdate?.latestTag ?? '…'}
          </button>
        {:else if stagingApplyRunning}
          <button class="change-button" disabled>Running…</button>
        {:else if stagingApplyDone}
          <button class="change-button" onclick={closeStagingConfirm}>Close</button>
        {/if}
      </div>
    </div>
  {/if}
</div>

<style>
  .settings {
    position: relative;
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

  /* Staging update row — appears under the Staging Channel toggle when an
     update is available. Visually similar to the HQ folder row (info + CTA
     on the right) so it reads as a "thing you can act on" rather than a
     toggle. */
  .setting-row.staging-update-row .setting-desc {
    font-family: ui-monospace, SFMono-Regular, Menlo, Monaco, monospace;
  }

  /* Modal backdrop + sheet. Positioned absolute against the .settings popover
     (which is itself the Tauri window), so the modal sits flush over the
     popover surface rather than escaping to the OS-window edges. */
  .staging-modal-backdrop {
    position: absolute;
    inset: 0;
    background: rgba(0, 0, 0, 0.45);
    border-radius: 18px;
    z-index: 10;
  }

  .staging-modal {
    position: absolute;
    inset: 8px;
    z-index: 11;
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
    padding: 0.875rem 1rem 1rem;
    overflow-y: auto;
    background: var(--popover-bg, rgba(28, 28, 32, 0.96));
    backdrop-filter: var(--popover-blur, blur(28px) saturate(1.45));
    -webkit-backdrop-filter: var(--popover-blur, blur(28px) saturate(1.45));
    border: 1px solid var(--popover-border, rgba(255, 255, 255, 0.18));
    border-radius: 14px;
    box-shadow: 0 10px 30px rgba(0, 0, 0, 0.45);
  }

  .staging-modal h2 {
    font-size: 0.875rem;
    font-weight: 600;
    color: var(--popover-text-heading, #ffffff);
    margin: 0;
  }

  .staging-modal-lead {
    font-size: 0.75rem;
    color: var(--popover-text, #e0e0e0);
    line-height: 1.4;
    margin: 0;
  }

  .staging-modal-list {
    margin: 0;
    padding-left: 1.1rem;
    font-size: 0.75rem;
    color: var(--popover-text, #e0e0e0);
    line-height: 1.4;
  }

  .staging-modal-list code,
  .staging-modal-lead code,
  .staging-modal-warning code {
    font-family: ui-monospace, SFMono-Regular, Menlo, Monaco, monospace;
    font-size: 0.6875rem;
    background: var(--popover-surface, rgba(255, 255, 255, 0.08));
    padding: 0 4px;
    border-radius: 4px;
  }

  .staging-modal-warning {
    font-size: 0.6875rem;
    color: var(--popover-text-muted, #a0a0b0);
    line-height: 1.4;
    margin: 0;
  }

  .staging-modal-log {
    flex: 1 1 auto;
    min-height: 100px;
    max-height: 200px;
    overflow-y: auto;
    background: rgba(0, 0, 0, 0.35);
    border: 1px solid var(--popover-divider, rgba(255, 255, 255, 0.06));
    border-radius: 8px;
    padding: 0.5rem;
    font-family: ui-monospace, SFMono-Regular, Menlo, Monaco, monospace;
    font-size: 0.6875rem;
    line-height: 1.35;
  }

  .staging-modal-log-line {
    color: var(--popover-text, #e0e0e0);
    white-space: pre-wrap;
    word-break: break-word;
  }

  .staging-modal-log-stderr {
    color: #ffb3a0;
  }

  .staging-modal-log-info {
    color: #a8c7ff;
  }

  .staging-modal-log-error {
    color: #ff9a9a;
    font-weight: 600;
  }

  .staging-modal-error {
    font-size: 0.75rem;
    color: #ff9a9a;
    margin: 0;
  }

  .staging-modal-done {
    font-size: 0.75rem;
    color: #a8e6a3;
    margin: 0;
  }

  .staging-modal-actions {
    display: flex;
    justify-content: flex-end;
    gap: 0.5rem;
    margin-top: 0.5rem;
  }

  .staging-modal-confirm {
    background: var(--popover-primary, #ffffff);
    color: var(--popover-primary-text, #111113);
    border-color: var(--popover-primary, #ffffff);
  }

  .staging-modal-confirm:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
</style>
