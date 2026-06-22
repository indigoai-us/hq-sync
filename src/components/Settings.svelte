<script lang="ts">
  import { invoke } from '@tauri-apps/api/core';
  import { getVersion } from '@tauri-apps/api/app';
  import { open as openUrl } from '@tauri-apps/plugin-shell';
  import { permissionState, loadMeetingPermissions } from '../lib/permissionState.svelte';

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
  // Default company UID for SDK-local recordings (US-010). `null` means
  // Personal (no company attribution). Mirrors the URL-invite dropdown in
  // MeetingsWindow where `null` is the Personal option. The popover's
  // active-meetings row reads this on detection and presets its own
  // dropdown; per-recording overrides don't write back here.
  let defaultRecordingCompanyUid = $state<string | null>(null);
  // Memberships for the dropdown, loaded via `meetings_list_memberships`
  // (same Tauri command MeetingsWindow uses). Empty for non-Indigo users —
  // the row still renders so they can confirm "Personal" is the implicit
  // destination, but the only option is Personal.
  type CompanyMembership = {
    companyUid: string;
    companyName: string | null;
    role: string | null;
    status: string;
  };
  let memberships = $state<CompanyMembership[]>([]);
  // Share notifications — shown to ALL signed-in users (the @getindigo.ai
  // dogfood gate was removed; share_notify.rs polls universally). Re-read on
  // each poll cycle in share_notify.rs so the toggle takes effect immediately
  // without app restart.
  let shareNotifications = $state(true);
  // DM notifications — shown to ALL signed-in users. dm_notify.rs polls
  // universally and instant delivery (dm_mqtt.rs) is GA for everyone. Re-read
  // on each poll cycle so the toggle takes effect without restart.
  let dmNotifications = $state(true);
  // CLI auto-update — shown to ALL signed-in users. hq_cli_update.rs re-reads
  // this from menubar.json on each background check, so the toggle takes
  // effect without restart. Default ON: the app keeps the HQ CLI current.
  let cliAutoUpdate = $state(true);
  // Usage telemetry — shown to ALL signed-in users. OFF by default (opt-in):
  // telemetry.rs re-reads `telemetryEnabled` untyped from menubar.json after
  // each sync, so the toggle takes effect without restart. The authoritative
  // gate is still the server-side opt-in; this is the local fallback.
  let telemetryEnabled = $state(false);
  // Shared @getindigo.ai gate. No longer gates the notification toggles
  // (those are now universal) — still used by the staging-channel toggle and
  // the release-channel picker below. Populated at mount from
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
      const [settings, autostart, indigoUser, channels, memberships_] = await Promise.all([
        invoke<{
          hqPath: string | null;
          syncOnLaunch: boolean | null;
          notifications: boolean | null;
          startAtLogin: boolean | null;
          realtimeSync: boolean | null;
          personalSyncEnabled: boolean | null;
          instantSync: boolean | null;
          shareNotifications: boolean | null;
          dmNotifications: boolean | null;
          cliAutoUpdate: boolean | null;
          stagingChannel: boolean | null;
          releaseChannel: string | null;
          meetingDetectNotify?: {
            enabled: boolean | null;
            platforms: string[] | null;
          } | null;
          defaultRecordingCompanyUid?: string | null;
          telemetryEnabled?: boolean | null;
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
        // Memberships drive the default-recording-company dropdown. Same
        // Tauri command MeetingsWindow uses for its URL-invite picker.
        // Errors degrade to an empty list (Personal-only); we never want
        // a vault hiccup to block the rest of Settings from rendering.
        invoke<CompanyMembership[]>('meetings_list_memberships').catch(() => []),
      ]);

      hqPath = settings.hqPath;
      syncOnLaunch = settings.syncOnLaunch ?? false;
      notifications = settings.notifications ?? true;
      startAtLogin = settings.startAtLogin ?? autostart;
      realtimeSync = settings.realtimeSync ?? true;
      personalSyncEnabled = settings.personalSyncEnabled ?? true;
      instantSync = settings.instantSync ?? true;
      shareNotifications = settings.shareNotifications ?? true;
      dmNotifications = settings.dmNotifications ?? true;
      cliAutoUpdate = settings.cliAutoUpdate ?? true;
      telemetryEnabled = settings.telemetryEnabled ?? false;
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
      // Keep only active memberships for the dropdown — pending / revoked
      // memberships are filtered server-side too, but defense-in-depth
      // here lets the UI degrade gracefully if a stale row sneaks through.
      memberships = (memberships_ ?? []).filter((m) => m.status === 'active');
      // Validate the stored default against the live membership list. If
      // the user's lost access (or never had a default), drop to null
      // (Personal) — same fallback shape as MeetingsWindow's company
      // picker after a stale invite is revoked.
      const storedUid = settings.defaultRecordingCompanyUid ?? null;
      defaultRecordingCompanyUid = storedUid && memberships.some((m) => m.companyUid === storedUid)
        ? storedUid
        : null;
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
          dmNotifications,
          cliAutoUpdate,
          telemetryEnabled,
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
          // Round-trip the raw value — `null` is Personal, a `co_…` uid
          // is a specific company. Rust skip_serializing_if=None drops
          // `null` from disk so older builds reading the file don't
          // see an unknown key.
          defaultRecordingCompanyUid,
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

  async function handleToggleDmNotifications() {
    dmNotifications = !dmNotifications;
    await saveAll();
  }

  async function handleToggleCliAutoUpdate() {
    cliAutoUpdate = !cliAutoUpdate;
    await saveAll();
  }

  async function handleToggleTelemetry() {
    telemetryEnabled = !telemetryEnabled;
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

  async function handleOpenMeetingPermissionsWizard() {
    try {
      await invoke('open_meeting_permissions_window');
    } catch (err) {
      console.error('open_meeting_permissions_window failed:', err);
    }
  }

  async function handleChangeDefaultRecordingCompany(next: string | null) {
    // The `<select>` is one-way (`value={…}` + this `onchange`), not
    // `bind:value`: the `onchange` normalizes the empty Personal option back
    // to `null` before persisting, so this is the single place the change
    // lands in menubar.json on selection (no save button — same pattern as
    // every other toggle in this view).
    defaultRecordingCompanyUid = next;
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
    // Read the meeting-permissions snapshot so the Meeting permissions row
    // can show the current grant state. The response is non-prompting and
    // cheap. The Meetings section is shown to all users now (no eligibility
    // gate), so this always populates the row's status line.
    loadMeetingPermissions();
    getVersion()
      .then((v) => {
        appVersion = v;
      })
      .catch((err) => console.error('Failed to read app version:', err));
    // Re-read permission state whenever the window regains focus —
    // covers the common flow of granting/blocking in System Settings
    // then returning. Both notification permission AND meeting
    // permissions refresh on focus so the pills track macOS reality.
    const onFocus = () => {
      loadNotifPermission();
      loadMeetingPermissions();
    };
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

  {#if loading}
    <div class="settings-loading">
      <span class="dot-spinner"></span>
    </div>
  {:else}
    <div class="settings-body">
      <!-- ===== Group: Sync ============================================
           HQ folder + the four sync-behavior toggles. Ordered most- to
           least-touched. Rendered as a grouped inset list (macOS System
           Settings idiom): a surface card with hairline dividers between
           rows only, a muted section header above. -->
      <section class="settings-group-wrap">
        <h2 class="settings-group-title">Sync</h2>
        <div class="settings-group">
          <!-- HQ Folder Path -->
          <div class="setting-row">
            <div class="setting-info">
              <span class="setting-label">HQ Folder</span>
              <span class="setting-path" title={hqPath ?? ''}>{pathDisplay}</span>
            </div>
            <button class="change-button" onclick={handlePickFolder}>Change...</button>
          </div>

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

          <!-- Auto-sync — runs hq-sync-runner in --watch mode via the
               existing daemon Tauri commands, fanning out to every
               membership the user has (same as the Sync Now button). -->
          <div class="setting-row">
            <div class="setting-info">
              <label class="setting-label" for="toggle-realtime-sync">Auto-sync</label>
              <span class="setting-desc">Syncs every 15 seconds with no clicks needed</span>
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

          <!-- Instant sync (event-driven) — when ON, the daemon spawns the
               hq-sync-runner with --event-push so local edits upload within
               seconds of the filesystem event instead of waiting for the
               10-minute poll. The backend only honors this for @getindigo.ai
               identities (Phase 1 rollout); other users stay poll-only
               regardless. See src-tauri/src/commands/daemon.rs. -->
          <div class="setting-row">
            <div class="setting-info">
              <label class="setting-label" for="toggle-instant-sync">Instant sync</label>
              <span class="setting-desc">Push local edits within seconds instead of waiting for the periodic sync</span>
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

          <!-- Sync personal vault — when OFF, the menubar passes
               --skip-personal to the spawned hq-sync-runner so the personal
               target is dropped from the --companies fanout. Only
               cloud-enabled company memberships sync. Defaults ON to
               preserve pre-5.25 behavior. -->
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
        </div>
      </section>

      <!-- ===== Group: Notifications =================================== -->
      <section class="settings-group-wrap">
        <h2 class="settings-group-title">Notifications</h2>
        <div class="settings-group">
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

          <!-- macOS permission monitor — reflects the OS authorization
               (separate from the in-app toggle above). Persistent: re-read
               on focus so it tracks changes made in System Settings. Hidden
               until first read. -->
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

          <!-- Share + DM notifications — available to ALL signed-in users.
               Both poll universally on the backend (share_notify.rs /
               dm_notify.rs; the former @getindigo.ai dogfood gate was removed
               2026-05-26) and instant DM delivery (dm_mqtt.rs) is GA for
               everyone. These toggles let any user turn the banners on/off;
               persisted in menubar.json and re-read each poll cycle so a
               change takes effect without an app restart. -->
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

          <div class="setting-row">
            <div class="setting-info">
              <label class="setting-label" for="toggle-dm-notifications">Direct messages</label>
              <span class="setting-desc">Show a notification when a teammate sends you a message</span>
            </div>
            <button
              id="toggle-dm-notifications"
              class="toggle"
              class:active={dmNotifications}
              onclick={handleToggleDmNotifications}
              role="switch"
              aria-checked={dmNotifications}
              aria-label="Direct message notifications"
            >
              <span class="toggle-knob"></span>
            </button>
          </div>

          <div class="setting-row">
            <div class="setting-info">
              <label class="setting-label" for="toggle-cli-auto-update">Auto-update HQ CLI</label>
              <span class="setting-desc">Keep the <code>hq</code> command-line tool up to date automatically</span>
            </div>
            <button
              id="toggle-cli-auto-update"
              class="toggle"
              class:active={cliAutoUpdate}
              onclick={handleToggleCliAutoUpdate}
              role="switch"
              aria-checked={cliAutoUpdate}
              aria-label="Automatically update HQ CLI"
            >
              <span class="toggle-knob"></span>
            </button>
          </div>

          <div class="setting-row">
            <div class="setting-info">
              <label class="setting-label" for="toggle-telemetry">Usage telemetry</label>
              <span class="setting-desc">Share anonymized usage counts to help improve HQ. Off by default.</span>
            </div>
            <button
              id="toggle-telemetry"
              class="toggle"
              class:active={telemetryEnabled}
              onclick={handleToggleTelemetry}
              role="switch"
              aria-checked={telemetryEnabled}
              aria-label="Usage telemetry"
            >
              <span class="toggle-knob"></span>
            </button>
          </div>
        </div>
      </section>

      <!-- ===== Group: Meetings ========================================
           Detect upcoming meetings + per-recording attribution +
           permissions wizard. Shown to ALL users — no eligibility gate —
           so anyone can reach the "Meeting permissions" → Manage area and
           grant the macOS TCC permissions the SDK needs. Permissions are
           never requested on launch; this section is the only place they're
           asked for. The Rust side
           (`commands::recall_sdk::meeting_detect_eligible`) still gates
           whether the SDK actually spawns / records — this section is UX. -->
      <section class="settings-group-wrap">
        <h2 class="settings-group-title">Meetings</h2>
        <div class="settings-group">
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

          <!-- Default recording company — preselects the company-attribution
               dropdown in the popover's active-meetings row. The user can
               still override per-recording via that dropdown. Empty / no
               selection = Personal vault (no company tag on the Recall
               metadata). Memberships come from `meetings_list_memberships`
               — same source as MeetingsWindow's URL-invite picker, so the
               two surfaces stay in lockstep when a membership is added or
               revoked.

               Layout deviates from the other `.setting-row` entries: the
               label sits above a full-width select instead of sharing a row
               with it. The default-company description is awkward to
               compress and the constrained `max-width: 140px` pill on the
               right read as cramped + ugly when the company name was long.
               Stacking keeps the select wide enough to show full names and
               gives the row visual room to breathe. -->
          <div class="setting-row setting-row-stacked">
            <div class="setting-info">
              <label class="setting-label" for="default-recording-company">Default recording</label>
              <span class="setting-desc">
                Attribution for new recordings. Changeable per-recording.
              </span>
            </div>
            <!-- value MUST coerce `null` (Personal) to `''` so it matches the
                 empty Personal <option>. A raw `null` makes Svelte set
                 `select.__value = null`, which no option carries, so the control
                 renders BLANK in the popover's WKWebView instead of "Personal".
                 Mirrors the desktop-alt SettingsPage, which already coerces. -->
            <select
              id="default-recording-company"
              class="default-recording-company"
              aria-label="Default recording company"
              value={defaultRecordingCompanyUid ?? ''}
              onchange={(e) => {
                const v = (e.currentTarget as HTMLSelectElement).value;
                void handleChangeDefaultRecordingCompany(v === '' ? null : v);
              }}
            >
              <option value="">Personal</option>
              {#each memberships as m (m.companyUid)}
                <option value={m.companyUid}>{m.companyName ?? m.companyUid}</option>
              {/each}
            </select>
          </div>

          <!-- Meeting permissions monitor — opens the wizard window where
               the user can grant (or revoke) each macOS TCC permission
               the SDK needs. The button label stays "Manage" regardless
               of current state, since the wizard handles both directions:
               - all granted  → user may want to audit / revoke
               - some missing → user grants the missing ones

               The descriptive text below the label still reflects the
               current state so the Settings overview reads correctly at a
               glance without opening the wizard. -->
          <div class="setting-row">
            <div class="setting-info">
              <span class="setting-label">Meeting permissions</span>
              <span class="setting-desc">
                {#if !permissionState.meetingPermissions}
                  Checking macOS privacy grants…
                {:else if permissionState.meetingPermissions.allRequiredGranted}
                  Accessibility, screen recording &amp; microphone all granted
                {:else}
                  One or more macOS permissions need attention
                {/if}
              </span>
            </div>
            <button class="change-button" onclick={handleOpenMeetingPermissionsWizard}>
              Manage
            </button>
          </div>
        </div>
      </section>

      <!-- ===== Group: Updates =========================================
           Everything that controls what the Update pill targets and which
           build the auto-updater pulls. Check for Updates is always present
           so the group never renders empty for non-@indigo users. -->
      <section class="settings-group-wrap">
        <h2 class="settings-group-title">Updates</h2>
        <div class="settings-group">
          <!-- Staging channel — @getindigo.ai-only toggle. When ON (default
               for @indigo builders), the popover's Update pill targets
               `hq-core-staging` and shows the staging-flavored drift count.
               When OFF, the staging-replace + staging-drift checks both
               return None and the popover falls through to the prod release
               channel. See `commands/hq_core_staging.rs`. -->
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
          {/if}

          <!-- Release channel — only rendered when the backend exposes more
               than one channel (i.e. signed-in user is @getindigo.ai).
               Non-indigo users have updates pinned to stable on the Rust
               side regardless of what's stored, so showing the picker would
               be misleading. Persisted via save_settings on every change so
               the next 6-hour updater poll picks up the new endpoint. -->
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
        </div>
      </section>

      <!-- ===== Group: General ========================================= -->
      <section class="settings-group-wrap">
        <h2 class="settings-group-title">General</h2>
        <div class="settings-group">
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

          <!-- Packages are managed in the unified Library → Installed surface
               (US-009 — the standalone Packages window was removed). The desktop
               window's Library tab now hosts installed + marketplace packs in one
               place, so there is no separate Settings destination here. -->

          <!-- Version — read-only; sourced from tauri.conf.json via getVersion() -->
          <div class="setting-row">
            <div class="setting-info">
              <span class="setting-label">Version</span>
            </div>
            <span class="version-value">{appVersion ? `v${appVersion}` : '—'}</span>
          </div>
        </div>
      </section>
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

  /* Body — vertical stack of labeled groups. Groups are separated by
     space, not dividers; dividers live only between rows inside a group. */
  .settings-body {
    display: flex;
    flex-direction: column;
    gap: var(--space-5);
    padding: var(--space-2) var(--space-3) var(--space-4);
  }

  /* Grouped inset list (macOS System Settings idiom). A muted uppercase
     header sits above a surface card; rows live inside the card with
     hairline dividers between them. */
  .settings-group-wrap {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  .settings-group-title {
    margin: 0;
    padding: 0 var(--space-2);
    font-size: var(--text-xs);
    font-weight: 600;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--popover-text-muted, #a0a0b0);
  }

  .settings-group {
    display: flex;
    flex-direction: column;
    background: var(--popover-surface, rgba(255, 255, 255, 0.08));
    border: 1px solid var(--popover-divider, rgba(255, 255, 255, 0.06));
    border-radius: var(--radius-lg);
    overflow: hidden;
  }

  /* Divider between consecutive rows within a group only. CSS adjacency
     ignores Svelte's {#if} anchor comments, so gated rows still divide
     correctly. */
  .settings-group > .setting-row + .setting-row {
    border-top: 1px solid var(--popover-divider, rgba(255, 255, 255, 0.06));
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
    gap: var(--space-3);
    padding: var(--space-3);
  }

  /* Vertical variant — label/desc on top, control on a second row spanning
     full width. Used by the Default recording row where a long company name
     would otherwise look cramped inside the 140px right-column pill. */
  .setting-row-stacked {
    flex-direction: column;
    align-items: stretch;
    gap: var(--space-2);
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

  /* Native <select> styled to read as the row's primary control. Sits on
     its own line inside `.setting-row-stacked` so the full width is
     available — long company names no longer clip behind the chevron.
     Caret colour follows the muted text so the control reads as inactive
     until hovered/focused. */
  .default-recording-company {
    font-size: 0.8125rem;
    font-family: inherit;
    padding: 0.4375rem 1.75rem 0.4375rem 0.625rem;
    background: var(--popover-surface, rgba(255, 255, 255, 0.08));
    color: var(--popover-text, #e0e0e0);
    border: 1px solid var(--popover-divider, rgba(255, 255, 255, 0.06));
    border-radius: 9px;
    cursor: pointer;
    width: 100%;
    text-overflow: ellipsis;
    overflow: hidden;
    appearance: none;
    -webkit-appearance: none;
    background-image: url("data:image/svg+xml,%3Csvg width='8' height='6' viewBox='0 0 8 6' xmlns='http://www.w3.org/2000/svg'%3E%3Cpath d='M1 1l3 3 3-3' stroke='%23a0a0b0' stroke-width='1.2' fill='none' stroke-linecap='round' stroke-linejoin='round'/%3E%3C/svg%3E");
    background-repeat: no-repeat;
    background-position: right 0.5rem center;
  }
  .default-recording-company:hover {
    background-color: var(--popover-action-hover, rgba(255, 255, 255, 0.05));
    border-color: var(--popover-border, rgba(255, 255, 255, 0.18));
  }
  .default-recording-company:focus {
    outline: none;
    border-color: var(--popover-border, rgba(255, 255, 255, 0.18));
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
