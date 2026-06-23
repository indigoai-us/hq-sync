<script lang="ts">
  import { invoke } from '@tauri-apps/api/core';
  import type { SettingsTab } from '../route';
  import { permissionState, loadMeetingPermissions } from '../../lib/permissionState.svelte';
  import '../v4/tokens.css';

  // The secondary sidebar drives which section is in view; this page renders all
  // sections in one scroll and reacts to `activeTab` by scrolling it into view.
  let { activeTab = 'sync' }: { activeTab?: SettingsTab } = $props();

  type Channel = 'stable' | 'beta' | 'alpha';
  type Platform = 'zoom' | 'meet' | 'teams' | 'slack' | 'webex';

  interface SettingsWire {
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
  }

  const platforms: Platform[] = ['zoom', 'meet', 'teams', 'slack', 'webex'];

  // Memberships drive the default-recording-company dropdown (mirrors the
  // classic Settings + MeetingsWindow URL-invite picker). Empty for users with
  // no company memberships — the dropdown still renders so Personal is selectable.
  type CompanyMembership = {
    companyUid: string;
    companyName: string | null;
    role: string | null;
    status: string;
  };
  let memberships = $state<CompanyMembership[]>([]);

  let loading = $state(true);
  let saved = $state(false);
  let error = $state<string | null>(null);
  // GA gate (any signed-in user) — from `meetings_feature_enabled`. Gates the
  // Meeting-detection row, which graduated out of the Indigo dogfood.
  let isIndigoUser = $state(false);
  // True @getindigo.ai gate — from `is_indigo_user`. Gates the builder-only
  // staging-channel row. Kept separate from `isIndigoUser` above because
  // `meetings_feature_enabled` is now GA: keying the staging row off it would
  // hand the @getindigo.ai-only control to every signed-in user.
  let isIndigoBuilder = $state(false);
  let availableChannels = $state<Channel[]>(['stable']);

  let hqPath = $state<string | null>(null);
  // Default ON — mirrors the backend get_settings default; a fresh install
  // syncs on launch out of the box.
  let syncOnLaunch = $state(true);
  let realtimeSync = $state(true);
  let personalSyncEnabled = $state(true);
  let instantSync = $state(true);
  let notifications = $state(true);
  let shareNotifications = $state(true);
  let dmNotifications = $state(true);
  let cliAutoUpdate = $state(true);
  let stagingChannel = $state(true);
  let releaseChannel = $state<Channel | null>(null);
  let startAtLogin = $state(true);
  let meetingDetectEnabled = $state(true);
  let meetingDetectPlatforms = $state<string[]>([...platforms]);
  let defaultRecordingCompanyUid = $state<string | null>(null);
  // Telemetry is opt-in — defaults OFF until the user explicitly turns it on.
  let telemetryEnabled = $state(false);

  const displayedChannel = $derived<Channel>(
    releaseChannel ?? (availableChannels.includes('beta') ? 'beta' : 'stable'),
  );
  const hqPathLabel = $derived(hqPath ? hqPath.replace(/^\/Users\/[^/]+/, '~') : 'HQ folder not set');

  $effect(() => {
    void loadSettings();
    // Non-prompting read so the Meeting permissions row reflects the current
    // macOS grant state; refreshed on focus (returning from System Settings).
    void loadMeetingPermissions();
    const onFocus = () => void loadMeetingPermissions();
    window.addEventListener('focus', onFocus);
    return () => window.removeEventListener('focus', onFocus);
  });

  // Scroll the active section into view when the sidebar selection changes (and
  // once sections first render after load). No-op for the default top section.
  $effect(() => {
    const id = activeTab;
    if (loading) return;
    document.getElementById(id)?.scrollIntoView({ behavior: 'smooth', block: 'start' });
  });

  async function loadSettings() {
    loading = true;
    error = null;
    try {
      const [settings, indigoUser, indigoBuilder, channels, memberships_] = await Promise.all([
        invoke<SettingsWire>('get_settings'),
        invoke<boolean>('meetings_feature_enabled').catch(() => false),
        // True @getindigo.ai gate for the staging-channel row (NOT the GA
        // `meetings_feature_enabled` above, which admits any signed-in user).
        invoke<boolean>('is_indigo_user').catch(() => false),
        invoke<string[]>('available_channels').catch(() => ['stable']),
        // Drives the default-recording-company dropdown. A vault hiccup must not
        // block the rest of Settings from rendering → degrade to Personal-only.
        invoke<CompanyMembership[]>('meetings_list_memberships').catch(() => []),
      ]);
      hqPath = settings.hqPath;
      syncOnLaunch = settings.syncOnLaunch ?? true;
      realtimeSync = settings.realtimeSync ?? true;
      personalSyncEnabled = settings.personalSyncEnabled ?? true;
      instantSync = settings.instantSync ?? true;
      notifications = settings.notifications ?? true;
      shareNotifications = settings.shareNotifications ?? true;
      dmNotifications = settings.dmNotifications ?? true;
      cliAutoUpdate = settings.cliAutoUpdate ?? true;
      stagingChannel = settings.stagingChannel ?? true;
      startAtLogin = settings.startAtLogin ?? true;
      meetingDetectEnabled = settings.meetingDetectNotify?.enabled ?? true;
      meetingDetectPlatforms = settings.meetingDetectNotify?.platforms ?? [...platforms];
      // Keep only active memberships; validate the stored default against the
      // live list so a revoked-access default falls back to Personal (null).
      memberships = (memberships_ ?? []).filter((m) => m.status === 'active');
      const storedUid = settings.defaultRecordingCompanyUid ?? null;
      defaultRecordingCompanyUid =
        storedUid && memberships.some((m) => m.companyUid === storedUid) ? storedUid : null;
      telemetryEnabled = settings.telemetryEnabled ?? false;
      isIndigoUser = indigoUser;
      isIndigoBuilder = indigoBuilder;
      availableChannels = channels.filter(isChannel);
      releaseChannel = isChannel(settings.releaseChannel) ? settings.releaseChannel : null;
    } catch (err) {
      error = String(err);
    } finally {
      loading = false;
    }
  }

  function isChannel(value: unknown): value is Channel {
    return value === 'stable' || value === 'beta' || value === 'alpha';
  }

  function togglePlatform(platform: Platform) {
    meetingDetectPlatforms = meetingDetectPlatforms.includes(platform)
      ? meetingDetectPlatforms.filter((item) => item !== platform)
      : [...meetingDetectPlatforms, platform];
    void saveSettings();
  }

  // Re-tether the HQ folder — mirrors the classic Settings "Change…" button so a
  // user who moved their HQ folder can fix it without opening the menubar popover
  // or hand-editing menubar.json.
  async function handlePickFolder() {
    try {
      const picked = await invoke<string | null>('pick_folder');
      if (picked !== null) {
        hqPath = picked;
        await saveSettings();
      }
    } catch (err) {
      error = String(err);
    }
  }

  // Open the macOS TCC permissions wizard. Without this row the desktop window
  // had no way to grant the Accessibility/Screen-Recording/Microphone access the
  // meeting SDK needs — users had to open the classic popover to reach it.
  async function handleOpenMeetingPermissionsWizard() {
    try {
      await invoke('open_meeting_permissions_window');
    } catch (err) {
      error = String(err);
    }
  }

  async function saveSettings() {
    error = null;
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
          stagingChannel,
          releaseChannel,
          meetingDetectNotify: {
            enabled: meetingDetectEnabled,
            platforms: meetingDetectPlatforms,
          },
          defaultRecordingCompanyUid,
          telemetryEnabled,
        },
      });
      saved = true;
      window.setTimeout(() => (saved = false), 1000);
    } catch (err) {
      error = String(err);
    }
  }

  // Three toggles carry live backend side-effects beyond persistence — without
  // them, flipping the switch in this window only writes menubar.json and the
  // running process keeps its old behavior until the next launch. These mirror
  // the classic popover Settings (src/components/Settings.svelte) so both
  // surfaces apply identically. `bind:checked` has already flipped the bound
  // value by the time onchange fires, so each reads the NEW state directly.

  // Auto-sync drives whether the background daemon runs at all. Start or stop
  // it immediately so the change takes effect without an app restart.
  async function applyRealtimeSync() {
    await saveSettings();
    try {
      if (realtimeSync) {
        await invoke('start_daemon');
      } else {
        await invoke('stop_daemon');
      }
    } catch (err) {
      // Persisted state is authoritative; main.rs reconciles on next launch.
      console.error('Auto-sync daemon command failed:', err);
    }
  }

  // Instant-sync is read at daemon spawn time (the --event-push argv). If the
  // daemon is already running, bounce it so the new flag takes effect now; if
  // Auto-sync is off there's no process to bounce — the next start picks it up.
  async function applyInstantSync() {
    await saveSettings();
    if (!realtimeSync) return;
    try {
      await invoke('stop_daemon');
      await invoke('start_daemon');
    } catch (err) {
      console.error('Instant-sync daemon restart failed:', err);
    }
  }

  // Start-at-login must reconcile the macOS LaunchAgent plist, not just persist
  // the flag — otherwise the login item never actually changes.
  async function applyStartAtLogin() {
    try {
      await invoke('set_autostart_enabled', { enabled: startAtLogin });
    } catch (err) {
      console.error('Failed to set autostart:', err);
    }
    await saveSettings();
  }

  // Re-read just the persisted prefs (no loading flash) so a change made in the
  // menubar popover while this window is open reflects here on focus. The indigo
  // gate, channel list, and memberships are process-stable, so they're skipped.
  async function refreshSettingsSilently() {
    try {
      const settings = await invoke<SettingsWire>('get_settings');
      hqPath = settings.hqPath;
      syncOnLaunch = settings.syncOnLaunch ?? true;
      realtimeSync = settings.realtimeSync ?? true;
      personalSyncEnabled = settings.personalSyncEnabled ?? true;
      instantSync = settings.instantSync ?? true;
      notifications = settings.notifications ?? true;
      shareNotifications = settings.shareNotifications ?? true;
      dmNotifications = settings.dmNotifications ?? true;
      cliAutoUpdate = settings.cliAutoUpdate ?? true;
      stagingChannel = settings.stagingChannel ?? true;
      startAtLogin = settings.startAtLogin ?? true;
      meetingDetectEnabled = settings.meetingDetectNotify?.enabled ?? true;
      meetingDetectPlatforms = settings.meetingDetectNotify?.platforms ?? [...platforms];
      defaultRecordingCompanyUid = settings.defaultRecordingCompanyUid ?? null;
      telemetryEnabled = settings.telemetryEnabled ?? false;
      releaseChannel = isChannel(settings.releaseChannel) ? settings.releaseChannel : null;
    } catch {
      // Non-fatal — keep showing the last-known values.
    }
  }

  $effect(() => {
    const onFocus = () => {
      void refreshSettingsSilently();
    };
    window.addEventListener('focus', onFocus);
    return () => window.removeEventListener('focus', onFocus);
  });
</script>

<section class="settings-page" aria-labelledby="settings-title" aria-busy={loading}>
  <main class="settings-main">
    <header class="page-header">
      <div>
        <p>{saved ? 'Saved' : 'menubar.json'}</p>
        <h1 id="settings-title">Settings</h1>
      </div>
    </header>

    {#if error}
      <p class="error" role="alert">{error}</p>
    {/if}

    <section id="sync" class="settings-section">
      <h2>Sync</h2>
      <div class="settings-card">
        <div class="setting-row">
          <div><strong>HQ folder</strong><span>{hqPathLabel}</span></div>
          <button type="button" class="row-button" onclick={handlePickFolder}>Change…</button>
        </div>
        <label class="setting-row"><span><strong>Sync on launch</strong><small>Run a sync when the app starts.</small></span><input type="checkbox" bind:checked={syncOnLaunch} onchange={saveSettings} /></label>
        <label class="setting-row"><span><strong>Auto-sync</strong><small>Sync every few minutes in the background.</small></span><input type="checkbox" bind:checked={realtimeSync} onchange={applyRealtimeSync} /></label>
        <label class="setting-row"><span><strong>Instant sync</strong><small>Push local edits within seconds when eligible.</small></span><input type="checkbox" bind:checked={instantSync} onchange={applyInstantSync} /></label>
        <label class="setting-row"><span><strong>Sync personal vault</strong><small>Include personal HQ files in the fanout.</small></span><input type="checkbox" bind:checked={personalSyncEnabled} onchange={saveSettings} /></label>
      </div>
    </section>

    <section id="notifications" class="settings-section">
      <h2>Notifications</h2>
      <div class="settings-card">
        <label class="setting-row"><span><strong>Sync notifications</strong><small>Notify when sync needs attention.</small></span><input type="checkbox" bind:checked={notifications} onchange={saveSettings} /></label>
        <label class="setting-row"><span><strong>Share notifications</strong><small>Show file-share activity from teammates.</small></span><input type="checkbox" bind:checked={shareNotifications} onchange={saveSettings} /></label>
        <label class="setting-row"><span><strong>DM notifications</strong><small>Show direct messages in the menu bar.</small></span><input type="checkbox" bind:checked={dmNotifications} onchange={saveSettings} /></label>
      </div>
    </section>

    <section id="updates" class="settings-section">
      <h2>Updates</h2>
      <div class="settings-card">
        <label class="setting-row"><span><strong>CLI auto-update</strong><small>Keep the bundled HQ CLI current.</small></span><input type="checkbox" bind:checked={cliAutoUpdate} onchange={saveSettings} /></label>
        <label class="setting-row gated-row"><span><strong>HQ core staging channel</strong><small>@getindigo.ai only. Changes rescue and drift targets.</small></span><input type="checkbox" disabled={!isIndigoBuilder} bind:checked={stagingChannel} onchange={saveSettings} /><em>Gated</em></label>
        <label class="setting-row gated-row"><span><strong>Release channel</strong><small>@getindigo.ai only. Stable is enforced for everyone else.</small></span><select disabled={availableChannels.length <= 1} bind:value={releaseChannel} onchange={saveSettings}><option value={null}>Default ({displayedChannel})</option>{#each availableChannels as channel (channel)}<option value={channel}>{channel}</option>{/each}</select><em>Gated</em></label>
      </div>
    </section>

    <section id="general" class="settings-section">
      <h2>General</h2>
      <div class="settings-card">
        <label class="setting-row"><span><strong>Start at login</strong><small>Open HQ Sync when macOS starts.</small></span><input type="checkbox" bind:checked={startAtLogin} onchange={applyStartAtLogin} /></label>
        <label class="setting-row"><span><strong>Usage telemetry</strong><small>Share anonymized usage counts to improve HQ. Off by default.</small></span><input type="checkbox" bind:checked={telemetryEnabled} onchange={saveSettings} /></label>
      </div>
    </section>

    <section id="meetings" class="settings-section">
      <h2>Meetings</h2>
      <div class="settings-card">
        <label class="setting-row gated-row"><span><strong>Meeting detection</strong><small>Detect active meeting apps and surface recording actions.</small></span><input type="checkbox" disabled={!isIndigoUser} bind:checked={meetingDetectEnabled} onchange={saveSettings} /><em>Gated</em></label>
        {#if meetingDetectEnabled}
          <!-- Only shown when detection is on — otherwise the platform toggles
               looked actionable but changed nothing (detection was off). -->
          <div class="setting-row platform-row">
            <span><strong>Platforms</strong><small>Choose which meeting apps are watched.</small></span>
            <div class="platforms">
              {#each platforms as platform (platform)}
                <button type="button" class:active={meetingDetectPlatforms.includes(platform)} onclick={() => togglePlatform(platform)}>{platform}</button>
              {/each}
            </div>
          </div>
        {/if}
        <!-- A validated dropdown of the caller's active memberships, not a
             free-text UID field. The old text input saved an arbitrary,
             unvalidated string (and only on blur, so it often never saved) — a
             bad UID then silently fell back to Personal at recording time. -->
        <label class="setting-row">
          <span><strong>Default recording company</strong><small>Attribution for new recordings. Changeable per-recording.</small></span>
          <select
            value={defaultRecordingCompanyUid ?? ''}
            aria-label="Default recording company"
            onchange={(event) => {
              const v = event.currentTarget.value;
              defaultRecordingCompanyUid = v === '' ? null : v;
              void saveSettings();
            }}
          >
            <option value="">Personal</option>
            {#each memberships as m (m.companyUid)}
              <option value={m.companyUid}>{m.companyName?.trim() || 'Company'}</option>
            {/each}
          </select>
        </label>
        <!-- Meeting permissions monitor — the only place to grant the macOS TCC
             permissions the SDK needs. Without it the desktop window couldn't
             grant them at all. -->
        <div class="setting-row">
          <span>
            <strong>Meeting permissions</strong>
            <small>
              {#if !permissionState.meetingPermissions}
                Checking macOS privacy grants…
              {:else if permissionState.meetingPermissions.allRequiredGranted}
                Accessibility, screen recording &amp; microphone all granted
              {:else}
                One or more macOS permissions need attention
              {/if}
            </small>
          </span>
          <button type="button" class="row-button" onclick={handleOpenMeetingPermissionsWizard}>Manage</button>
        </div>
      </div>
    </section>
  </main>
</section>

<style>
  .settings-page {
    display: block;
    min-width: 0;
    height: 100%;
    color: var(--v4-text-1);
  }

  .settings-section h2,
  .page-header p {
    margin: 0;
    color: var(--v4-text-3);
    font-size: var(--text-base);
    font-weight: 500;
    line-height: 1.25;
  }

  .settings-main {
    display: flex;
    flex-direction: column;
    gap: 18px;
    min-width: 0;
    overflow: auto;
  }

  h1 {
    margin: 2px 0 0;
    font-size: var(--text-lg);
    font-weight: 500;
  }

  .settings-section {
    display: grid;
    gap: 8px;
    scroll-margin-top: 12px;
  }

  .settings-card {
    display: grid;
    overflow: hidden;
    border: 1px solid var(--v4-hairline);
    border-radius: 8px;
    background: var(--v4-raised);
  }

  .setting-row {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto auto;
    align-items: center;
    gap: 12px;
    min-height: 48px;
    padding: 10px 12px;
    border-top: 1px solid var(--v4-rowline);
  }

  .setting-row:first-child {
    border-top: 0;
  }

  .setting-row span,
  .setting-row div {
    display: grid;
    gap: 2px;
    min-width: 0;
  }

  strong,
  small {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  strong {
    color: var(--v4-text-1);
    font-size: var(--text-base);
    font-weight: 500;
  }

  small,
  .setting-row div span,
  .error {
    color: var(--v4-text-3);
    font-size: var(--text-base);
    line-height: 1.35;
  }

  input,
  select {
    min-width: 0;
  }

  /* macOS-style toggle pill — the one place green is allowed as a control fill
     (SPEC §5/§6: "26×16 pills, on = green fill — the one non-dot color
     exception, matching macOS"). The track is tokenized; the knob is a fixed
     white-with-shadow, a deliberate platform-convention value like the green. */
  input[type='checkbox'] {
    appearance: none;
    -webkit-appearance: none;
    position: relative;
    flex-shrink: 0;
    width: 26px;
    height: 16px;
    border-radius: 999px;
    background: var(--v4-control-bg);
    cursor: pointer;
    transition: background-color 0.15s ease;
  }

  input[type='checkbox']::after {
    content: '';
    position: absolute;
    top: 2px;
    left: 2px;
    width: 12px;
    height: 12px;
    border-radius: 50%;
    background: #fff;
    box-shadow: 0 1px 2px rgba(0, 0, 0, 0.25);
    transition: transform 0.15s ease;
  }

  input[type='checkbox']:checked {
    background: var(--v4-ok);
  }

  input[type='checkbox']:checked::after {
    transform: translateX(10px);
  }

  input[type='checkbox']:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  input[type='checkbox']:focus-visible {
    outline: 1.5px solid var(--v4-text-2);
    outline-offset: 2px;
  }

  input:not([type='checkbox']),
  select {
    height: 30px;
    border: 1px solid var(--v4-hairline);
    border-radius: 6px;
    background: var(--v4-inset);
    color: var(--v4-text-1);
    font: inherit;
    font-size: var(--text-base);
  }

  .gated-row em {
    padding: 3px 7px;
    border: 1px solid var(--v4-hairline);
    border-radius: 999px;
    color: var(--v4-text-3);
    font-size: var(--text-base);
    font-style: normal;
  }

  /* Trailing row affordance (Change… / Manage) — quiet pill matching the V4
     control language. */
  .row-button {
    justify-self: end;
    height: 30px;
    padding: 0 12px;
    border: 1px solid var(--v4-hairline);
    border-radius: 6px;
    background: var(--v4-inset);
    color: var(--v4-text-1);
    font: inherit;
    font-size: var(--text-base);
    cursor: pointer;
    white-space: nowrap;
  }

  .row-button:hover {
    border-color: var(--v4-text-3);
  }

  .row-button:focus-visible {
    outline: 1.5px solid var(--v4-text-2);
    outline-offset: 2px;
  }

  .platform-row {
    align-items: start;
  }

  .platforms {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    justify-content: flex-end;
  }

  .platforms button {
    height: 26px;
    border: 1px solid var(--v4-hairline);
    border-radius: 999px;
    background: transparent;
    color: var(--v4-text-2);
    font: inherit;
    font-size: var(--text-base);
  }

  .platforms button.active {
    background: var(--v4-control-bg);
    color: var(--v4-text-1);
  }
</style>
