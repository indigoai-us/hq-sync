<script lang="ts">
  import { invoke } from '@tauri-apps/api/core';
  import { getVersion } from '@tauri-apps/api/app';
  import { open as openUrl } from '@tauri-apps/plugin-shell';
  import { permissionState } from '../lib/permissionState.svelte';

  interface Props {
    onback: () => void;
  }

  let { onback }: Props = $props();

  const ALL_PLATFORMS = ['zoom', 'meet', 'teams', 'slack', 'webex'] as const;
  type Platform = (typeof ALL_PLATFORMS)[number];

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
  // Meeting detect-notify state (US-007) — only ever applied on @getindigo.ai
  // accounts, gate enforced in Rust. The Settings UI hides the Meetings
  // section entirely when the user isn't eligible.
  let meetingDetectEnabled = $state(true);
  let meetingDetectPlatforms = $state<string[]>([...ALL_PLATFORMS]);
  // Share notifications — dogfood gate: section only rendered for @getindigo.ai
  // users. Same gate as meetings_feature_enabled (both call is_indigo_user()
  // on the Rust side). Re-read on each poll cycle in share_notify.rs so the
  // toggle takes effect immediately without app restart.
  let shareNotifications = $state(true);
  // Shared @getindigo.ai gate, used by BOTH the share-notify section and
  // the staging-channel toggle below. Populated at mount from
  // `meetings_feature_enabled` (cached process-lifetime on the Rust side).
  let isIndigoUser = $state(false);
  // Staging channel — @getindigo.ai-only toggle (visibility gated on
  // `isIndigoUser`). Distinct from the release-channel picker below:
  // this controls which hq-core SOURCE the in-app rescue + drift
  // classifier targets (staging vs prod), while `release_channel`
  // controls which hq-sync BUILD the auto-updater pulls. When ON
  // (default), the popover renders "Update to Staging" and the rescue
  // script targets hq-core-staging. When OFF, the `coreState` Rust
  // command falls through to the prod release channel (same surface
  // non-@indigo users see).
  let stagingChannel = $state(true);

  // Release channel picker — only rendered when the backend reports >1
  // channel (i.e. the signed-in user is @getindigo.ai). The Rust-side
  // updater.rs coerces non-indigo users to "stable" regardless of what's
  // stored here, so this UI is purely the convenience surface; a tampered
  // frontend cannot escalate a user into beta/alpha because the resolver
  // re-applies the gate at every check (see updater::resolve_endpoint_url
  // -> util::release_channel::effective_channel).
  //
  // Two-state model (Codex P1 review on PR #120):
  //   - `storedChannel` is the raw value persisted in menubar.json.
  //     `null` = the user has never explicitly chosen a channel; the
  //     updater will resolve it identity-aware on the Rust side. This
  //     gets round-tripped through save_settings UNTOUCHED on non-picker
  //     toggles, so flipping e.g. Auto-sync doesn't lock an indigo user
  //     into "beta" by side effect.
  //   - `displayedChannel` is what the picker shows. Derived from
  //     `storedChannel` when set, otherwise falls back to the first
  //     non-stable option (`beta` for indigo users, `stable` for
  //     everyone — same defaulting the Rust `effective_channel` does).
  type Channel = 'stable' | 'beta' | 'alpha';
  let storedChannel = $state<Channel | null>(null);
  let availableChannels = $state<Channel[]>(['stable']);
  // Derived: what the user sees in the segmented control.
  let displayedChannel = $derived<Channel>(
    storedChannel ?? (availableChannels.includes('beta') ? 'beta' : 'stable')
  );

  // OS-level macOS notification authorization, distinct from the in-app
  // `notifications` preference above. `'unknown'` = not yet read (renders
  // nothing); the backend returns 'granted' | 'denied' | 'prompt'.
  let notifPermission = $state<'granted' | 'denied' | 'prompt' | 'unknown'>('unknown');
  let notifRequesting = $state(false);
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
      const [settings, autostart, indigoUser, channels] = await Promise.all([
        invoke<{
          hqPath: string | null;
          syncOnLaunch: boolean | null;
          notifications: boolean | null;
          startAtLogin: boolean | null;
          realtimeSync: boolean | null;
          personalSyncEnabled: boolean | null;
          instantSync: boolean | null;
          shareNotifications: boolean | null;
          stagingChannel: boolean | null;
          releaseChannel: string | null;
          meetingDetectNotify?: {
            enabled: boolean | null;
            platforms: string[] | null;
          } | null;
        }>('get_settings'),
        invoke<boolean>('get_autostart_enabled'),
        // Shared @getindigo.ai gate for share-notify section AND
        // staging-channel toggle visibility. Rust side caches the
        // decision process-lifetime so this is effectively free after
        // first call.
        invoke<boolean>('meetings_feature_enabled').catch(() => false),
        // Returns ["stable"] for non-indigo users, ["stable","beta","alpha"]
        // for @getindigo.ai. The picker only renders when length > 1.
        invoke<string[]>('available_channels'),
      ]);

      hqPath = settings.hqPath;
      syncOnLaunch = settings.syncOnLaunch ?? false;
      notifications = settings.notifications ?? true;
      startAtLogin = settings.startAtLogin ?? autostart;
      realtimeSync = settings.realtimeSync ?? true;
      personalSyncEnabled = settings.personalSyncEnabled ?? true;
      instantSync = settings.instantSync ?? true;
      shareNotifications = settings.shareNotifications ?? true;
      stagingChannel = settings.stagingChannel ?? true;
      isIndigoUser = indigoUser;
      availableChannels = (channels.filter(
        (c) => c === 'stable' || c === 'beta' || c === 'alpha'
      ) as Channel[]) ?? ['stable'];
      // Raw on-disk value: `null` when the user has never touched the
      // picker. The displayed channel is derived in `displayedChannel`.
      const raw = settings.releaseChannel as Channel | null;
      storedChannel = raw && availableChannels.includes(raw) ? raw : null;
      meetingDetectEnabled = settings.meetingDetectNotify?.enabled ?? true;
      meetingDetectPlatforms = settings.meetingDetectNotify?.platforms ?? [...ALL_PLATFORMS];
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
          shareNotifications,
          stagingChannel,
          // Round-trip the RAW stored value (null when never explicitly
          // chosen). The Rust side serializes `null` -> absent via
          // skip_serializing_if=None, so an indigo user toggling Auto-sync
          // never accidentally writes `releaseChannel: "beta"` to disk
          // and locks in the resolved default. Only `handleChannelChange`
          // mutates `storedChannel`.
          releaseChannel: storedChannel,
          meetingDetectNotify: {
            enabled: meetingDetectEnabled,
            platforms: meetingDetectPlatforms,
          },
        },
      });
      showSaved();
    } catch (err) {
      console.error('Failed to save settings:', err);
    }
  }

  // Flip the staging-channel toggle. Backend's `check_core_state`
  // reads the persisted value on every call, so the next popover open
  // reflects the new state. No daemon bounce needed — unlike
  // instant-sync, this doesn't change a long-running process's argv.
  async function handleToggleStagingChannel() {
    stagingChannel = !stagingChannel;
    await saveAll();
  }

  async function handleChannelChange(next: Channel) {
    if (next === displayedChannel) return;
    if (!availableChannels.includes(next)) return;
    // Explicit user choice — persist the raw value going forward.
    storedChannel = next;
    await saveAll();
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

  async function handleToggleShareNotifications() {
    shareNotifications = !shareNotifications;
    await saveAll();
  }

  // Read the current OS permission without prompting. Called on mount and on
  // window focus (so returning from System Settings refreshes the pill).
  async function loadNotifPermission() {
    try {
      notifPermission = await invoke<'granted' | 'denied' | 'prompt'>(
        'notification_permission_state'
      );
    } catch (err) {
      console.error('Failed to read notification permission:', err);
      notifPermission = 'unknown';
    }
  }

  async function handleEnableNotifications() {
    if (notifRequesting) return;
    // Once macOS has recorded a denial it will NOT re-show the system dialog,
    // so request_permission() would be a silent no-op. The only way back is the
    // System Settings > Notifications pane — deep-link the user straight there.
    if (notifPermission === 'denied') {
      try {
        await openUrl('x-apple.systempreferences:com.apple.preference.notifications');
      } catch (err) {
        console.error('Failed to open System Settings:', err);
      }
      return;
    }
    notifRequesting = true;
    try {
      notifPermission = await invoke<'granted' | 'denied' | 'prompt'>(
        'notification_request_permission'
      );
    } catch (err) {
      console.error('Failed to request notification permission:', err);
    } finally {
      notifRequesting = false;
    }
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

  async function handleToggleMeetingDetect() {
    meetingDetectEnabled = !meetingDetectEnabled;
    await saveAll();
  }

  async function handleTogglePlatform(platform: Platform) {
    if (meetingDetectPlatforms.includes(platform)) {
      meetingDetectPlatforms = meetingDetectPlatforms.filter((p) => p !== platform);
    } else {
      meetingDetectPlatforms = [...meetingDetectPlatforms, platform];
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
    loadNotifPermission();
    getVersion()
      .then((v) => {
        appVersion = v;
      })
      .catch((err) => console.error('Failed to read app version:', err));
    // Re-read permission whenever the window regains focus — covers the
    // common flow of granting/blocking in System Settings then returning.
    const onFocus = () => loadNotifPermission();
    window.addEventListener('focus', onFocus);
    return () => {
      window.removeEventListener('focus', onFocus);
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

      <!-- Staging channel — @getindigo.ai-only toggle. When ON (default
           for @indigo builders), the popover's Update pill targets
           `hq-core-staging` and shows the staging-flavored drift count.
           When OFF, the staging-replace + staging-drift checks both
           return None and the popover falls through to the prod release
           channel (the same surface non-@indigo users see). Visibility
           gated on `isIndigoUser` (shared @getindigo.ai gate; same
           predicate the share-notify section uses, via
           `meetings_feature_enabled`). See `commands/hq_core_staging.rs`. -->
      {#if isIndigoUser}
        <div class="setting-row">
          <div class="setting-info">
            <label class="setting-label" for="toggle-staging-channel">Use staging channel</label>
            <span class="setting-desc">Target hq-core-staging for the Update pill instead of the released hq-core tag</span>
          </div>
          <button
            id="toggle-staging-channel"
            class="toggle"
            class:active={stagingChannel}
            onclick={handleToggleStagingChannel}
            role="switch"
            aria-checked={stagingChannel}
            aria-label="Use staging channel"
          >
            <span class="toggle-knob"></span>
          </button>
        </div>

        <div class="settings-divider"></div>
      {/if}

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

      <!-- macOS permission monitor — reflects the OS authorization (separate
           from the in-app toggle above). Persistent: re-read on focus so it
           tracks changes made in System Settings. Hidden until first read. -->
      {#if notifPermission !== 'unknown'}
        <div class="setting-row">
          <div class="setting-info">
            <span class="setting-label">System permission</span>
            <span class="setting-desc">
              {#if notifPermission === 'granted'}
                macOS is allowing notifications from HQ Sync
              {:else if notifPermission === 'denied'}
                Blocked in macOS — open System Settings to allow
              {:else}
                Not enabled yet — allow to see sync &amp; share alerts
              {/if}
            </span>
          </div>
          {#if notifPermission === 'granted'}
            <span class="perm-pill">Enabled</span>
          {:else}
            <button
              class="change-button"
              onclick={handleEnableNotifications}
              disabled={notifRequesting}
            >
              {#if notifRequesting}
                Requesting…
              {:else if notifPermission === 'denied'}
                Open Settings
              {:else}
                Enable
              {/if}
            </button>
          {/if}
        </div>
      {/if}

      <!-- Share notifications — dogfood gate: only rendered for @getindigo.ai
           users. Persists shareNotifications in menubar.json; the poll in
           share_notify.rs re-reads on each cycle so the toggle takes effect
           on the next sync:complete without restart. -->
      {#if isIndigoUser}
        <div class="settings-divider"></div>

        <div class="setting-row">
          <div class="setting-info">
            <label class="setting-label" for="toggle-share-notifications">Share notifications</label>
            <span class="setting-desc">Show a notification when someone shares files with you</span>
          </div>
          <button
            id="toggle-share-notifications"
            class="toggle"
            class:active={shareNotifications}
            onclick={handleToggleShareNotifications}
            role="switch"
            aria-checked={shareNotifications}
            aria-label="Share notifications"
          >
            <span class="toggle-knob"></span>
          </button>
        </div>
      {/if}

      <!-- Meeting Detection (Phase 0). Gated on
           `permissionState.meetingDetectEligible` so users outside the
           allowlist don't see a toggle for an SDK that won't spawn. The
           Rust side (`commands::recall_sdk::meeting_detect_eligible`) is the
           authoritative gate; this is UX-only. macOS permissions are
           handled by native first-use prompts — no parallel UI here. -->
      {#if permissionState.meetingDetectEligible}
        <div class="settings-divider"></div>

        <div class="setting-row">
          <div class="setting-info">
            <label class="setting-label" for="toggle-meeting-detect">Detect upcoming meetings</label>
            <span class="setting-desc">Notify when a new meeting is detected</span>
          </div>
          <button
            id="toggle-meeting-detect"
            class="toggle"
            class:active={meetingDetectEnabled}
            onclick={handleToggleMeetingDetect}
            role="switch"
            aria-checked={meetingDetectEnabled}
            aria-label="Detect upcoming meetings"
          >
            <span class="toggle-knob"></span>
          </button>
        </div>

        {#if meetingDetectEnabled}
          <div class="platform-rows">
            {#each ALL_PLATFORMS as platform}
              {@const checked = meetingDetectPlatforms.includes(platform)}
              <div class="platform-row">
                <label class="platform-label" for="platform-{platform}">{platform}</label>
                <button
                  id="platform-{platform}"
                  class="platform-check"
                  class:checked
                  onclick={() => handleTogglePlatform(platform)}
                  role="checkbox"
                  aria-checked={checked}
                  aria-label="Enable {platform} meeting detection"
                >
                  {#if checked}
                    <svg width="10" height="10" viewBox="0 0 10 10" fill="none" aria-hidden="true">
                      <path d="M2 5l2.5 2.5L8 3" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"/>
                    </svg>
                  {/if}
                </button>
              </div>
            {/each}
            <div class="ledger-path-row">
              <span class="setting-desc">Ledger: ~/.hq/meeting-notify-ledger.json</span>
            </div>
          </div>
        {/if}
      {/if}

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

      <!-- Release channel — only rendered when the backend exposes more than
           one channel (i.e. signed-in user is @getindigo.ai). Non-indigo
           users have updates pinned to stable on the Rust side regardless
           of what's stored, so showing the picker would be misleading.

           The segmented control renders one button per available channel;
           the selected button is highlighted, the others are click targets.
           Persisted via save_settings on every change so the next 6-hour
           updater poll picks up the new endpoint. -->
      {#if availableChannels.length > 1}
        <div class="setting-row channel-row">
          <div class="setting-info">
            <span class="setting-label">Release channel</span>
            <span class="setting-desc">
              {#if displayedChannel === 'stable'}
                Stable updates only
              {:else if displayedChannel === 'beta'}
                Includes beta builds — early access, mostly stable
              {:else}
                Includes alpha builds — bleeding edge, may break
              {/if}
            </span>
          </div>
          <div class="channel-segments" role="radiogroup" aria-label="Release channel">
            {#each availableChannels as channel (channel)}
              <button
                type="button"
                class="channel-segment"
                class:active={displayedChannel === channel}
                role="radio"
                aria-checked={displayedChannel === channel}
                onclick={() => handleChannelChange(channel)}
              >
                {channel === 'stable' ? 'Stable' : channel === 'beta' ? 'Beta' : 'Alpha'}
              </button>
            {/each}
          </div>
        </div>

        <div class="settings-divider"></div>
      {/if}

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

  .change-button:disabled {
    opacity: 0.5;
    cursor: default;
  }

  /* Permission status pill — informational, green-tinted "Enabled" state.
     Mirrors .version-value sizing so the value column stays aligned. */
  .perm-pill {
    font-size: 0.6875rem;
    font-weight: 600;
    padding: 0.1875rem 0.5rem;
    border-radius: 9px;
    background: rgba(52, 199, 89, 0.16);
    color: #5fd27a;
    white-space: nowrap;
    flex-shrink: 0;
  }

  /* Release-channel segmented picker. Each segment is a button; the active
     one is highlighted in the same primary tone as a "Saved" pill. Sized
     to fit Stable / Beta / Alpha in one row without truncation; the row
     re-flows to a column on narrow popovers via `.channel-row`. */
  .channel-segments {
    display: flex;
    gap: 2px;
    padding: 2px;
    background: var(--popover-surface, rgba(255, 255, 255, 0.08));
    border: 1px solid var(--popover-divider, rgba(255, 255, 255, 0.06));
    border-radius: 9px;
    flex-shrink: 0;
  }

  .channel-segment {
    font-size: 0.6875rem;
    font-family: inherit;
    font-weight: 500;
    padding: 0.1875rem 0.5rem;
    background: transparent;
    color: var(--popover-text-muted, #a0a0b0);
    border: none;
    border-radius: 7px;
    cursor: pointer;
    transition: background-color 0.12s ease, color 0.12s ease;
    white-space: nowrap;
  }

  .channel-segment:hover {
    color: var(--popover-text, #e0e0e0);
  }

  .channel-segment.active {
    background: var(--popover-primary, #ffffff);
    color: var(--popover-primary-text, #111113);
  }

  /* The channel row's value column carries a wider control than the other
     rows (3 segments, ~140px wide vs. a 36px toggle), so loosen the gap
     and let the value column shrink the description if needed. */
  .channel-row {
    align-items: center;
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

  /* Platform sub-rows (shown when meeting detection is enabled) */
  .platform-rows {
    display: flex;
    flex-direction: column;
    padding: 0 1rem 0.25rem 1.75rem;
    gap: 0.125rem;
  }

  .platform-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 0.2rem 0;
  }

  .platform-label {
    font-size: 0.75rem;
    color: var(--popover-text-muted, #a0a0b0);
    text-transform: capitalize;
    cursor: default;
  }

  .platform-check {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 18px;
    height: 18px;
    padding: 0;
    background: var(--popover-surface, rgba(255, 255, 255, 0.08));
    border: 1px solid var(--popover-divider, rgba(255, 255, 255, 0.12));
    border-radius: 5px;
    cursor: pointer;
    color: var(--popover-primary-text, #111113);
    transition: background-color 0.15s ease, border-color 0.15s ease;
    flex-shrink: 0;
  }

  .platform-check.checked {
    background: var(--popover-primary, #ffffff);
    border-color: var(--popover-primary, #ffffff);
  }

  .ledger-path-row {
    padding-top: 0.375rem;
  }
</style>
