<script lang="ts">
  import { invoke } from '@tauri-apps/api/core';
  import type { SettingsTab } from '../route';
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
  }

  const platforms: Platform[] = ['zoom', 'meet', 'teams', 'slack', 'webex'];

  let loading = $state(true);
  let saved = $state(false);
  let error = $state<string | null>(null);
  let isIndigoUser = $state(false);
  let availableChannels = $state<Channel[]>(['stable']);

  let hqPath = $state<string | null>(null);
  let syncOnLaunch = $state(false);
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

  const displayedChannel = $derived<Channel>(
    releaseChannel ?? (availableChannels.includes('beta') ? 'beta' : 'stable'),
  );
  const hqPathLabel = $derived(hqPath ? hqPath.replace(/^\/Users\/[^/]+/, '~') : 'HQ folder not set');

  $effect(() => {
    void loadSettings();
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
      const [settings, indigoUser, channels] = await Promise.all([
        invoke<SettingsWire>('get_settings'),
        invoke<boolean>('meetings_feature_enabled').catch(() => false),
        invoke<string[]>('available_channels').catch(() => ['stable']),
      ]);
      hqPath = settings.hqPath;
      syncOnLaunch = settings.syncOnLaunch ?? false;
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
      isIndigoUser = indigoUser;
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
        },
      });
      saved = true;
      window.setTimeout(() => (saved = false), 1000);
    } catch (err) {
      error = String(err);
    }
  }
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
        </div>
        <label class="setting-row"><span><strong>Sync on launch</strong><small>Run a sync when the app starts.</small></span><input type="checkbox" bind:checked={syncOnLaunch} onchange={saveSettings} /></label>
        <label class="setting-row"><span><strong>Auto-sync</strong><small>Sync every few minutes in the background.</small></span><input type="checkbox" bind:checked={realtimeSync} onchange={saveSettings} /></label>
        <label class="setting-row"><span><strong>Instant sync</strong><small>Push local edits within seconds when eligible.</small></span><input type="checkbox" bind:checked={instantSync} onchange={saveSettings} /></label>
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
        <label class="setting-row gated-row"><span><strong>HQ core staging channel</strong><small>@getindigo.ai only. Changes rescue and drift targets.</small></span><input type="checkbox" disabled={!isIndigoUser} bind:checked={stagingChannel} onchange={saveSettings} /><em>Gated</em></label>
        <label class="setting-row gated-row"><span><strong>Release channel</strong><small>@getindigo.ai only. Stable is enforced for everyone else.</small></span><select disabled={availableChannels.length <= 1} bind:value={releaseChannel} onchange={saveSettings}><option value={null}>Default ({displayedChannel})</option>{#each availableChannels as channel (channel)}<option value={channel}>{channel}</option>{/each}</select><em>Gated</em></label>
      </div>
    </section>

    <section id="general" class="settings-section">
      <h2>General</h2>
      <div class="settings-card">
        <label class="setting-row"><span><strong>Start at login</strong><small>Open HQ Sync when macOS starts.</small></span><input type="checkbox" bind:checked={startAtLogin} onchange={saveSettings} /></label>
      </div>
    </section>

    <section id="meetings" class="settings-section">
      <h2>Meetings</h2>
      <div class="settings-card">
        <label class="setting-row gated-row"><span><strong>Meeting detection</strong><small>Detect active meeting apps and surface recording actions.</small></span><input type="checkbox" disabled={!isIndigoUser} bind:checked={meetingDetectEnabled} onchange={saveSettings} /><em>Gated</em></label>
        <div class="setting-row platform-row">
          <span><strong>Platforms</strong><small>Choose which meeting apps are watched.</small></span>
          <div class="platforms">
            {#each platforms as platform (platform)}
              <button type="button" class:active={meetingDetectPlatforms.includes(platform)} onclick={() => togglePlatform(platform)}>{platform}</button>
            {/each}
          </div>
        </div>
        <label class="setting-row"><span><strong>Default recording company</strong><small>Personal unless a company id is set.</small></span><input value={defaultRecordingCompanyUid ?? ''} placeholder="Personal" oninput={(event) => (defaultRecordingCompanyUid = event.currentTarget.value || null)} onchange={saveSettings} /></label>
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
    accent-color: var(--v4-text-1);
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
