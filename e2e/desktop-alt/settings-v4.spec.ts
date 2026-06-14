import { describe, expect, it } from 'vitest';
import { readRepoFile } from './harness';

describe('desktop-alt V4 settings and first-run (US-013)', () => {
  const page = readRepoFile('src/desktop-alt/pages/SettingsPage.svelte');
  const settings = readRepoFile('src/components/Settings.svelte');
  const firstRun = readRepoFile('src/components/FirstRunWelcome.svelte');

  it('groups every menubar setting knob and persists through get_settings/save_settings', () => {
    expect(page).toContain("invoke<SettingsWire>('get_settings')");
    expect(page).toContain("await invoke('save_settings', {");
    for (const key of [
      'hqPath',
      'syncOnLaunch',
      'notifications',
      'startAtLogin',
      'realtimeSync',
      'personalSyncEnabled',
      'instantSync',
      'shareNotifications',
      'dmNotifications',
      'cliAutoUpdate',
      'stagingChannel',
      'releaseChannel',
      'meetingDetectNotify',
      'defaultRecordingCompanyUid',
      // Usage telemetry is opt-in; the toggle must round-trip through
      // get_settings/save_settings in BOTH the desktop-alt page and the
      // classic popover so a save never drops it (and the data-loss merge in
      // save_settings preserves machineId/firstRunCompleted alongside it).
      'telemetryEnabled',
    ]) {
      expect(page).toContain(key);
      expect(settings).toContain(key);
    }
  });

  it('applies live backend side-effects, not just persistence, for daemon + autostart toggles', () => {
    // Persisting menubar.json is not enough — Auto-sync must start/stop the
    // daemon, Instant-sync must bounce it, and Start-at-login must reconcile the
    // macOS LaunchAgent. Without these the running process keeps its old
    // behavior until the next launch (the desktop window's original bug). These
    // mirror the classic popover Settings so both surfaces behave identically.
    expect(page).toContain("invoke('start_daemon')");
    expect(page).toContain("invoke('stop_daemon')");
    expect(page).toContain("invoke('set_autostart_enabled', { enabled: startAtLogin })");
    // The three toggles must route through their effect handlers, not the bare
    // saveSettings persistence path.
    expect(page).toContain('onchange={applyRealtimeSync}');
    expect(page).toContain('onchange={applyInstantSync}');
    expect(page).toContain('onchange={applyStartAtLogin}');
    // And the same three live effects must exist in the classic popover too.
    expect(settings).toContain("invoke('start_daemon')");
    expect(settings).toContain("invoke('stop_daemon')");
    expect(settings).toContain('set_autostart_enabled');
  });

  it('reaches parity with the classic Settings (folder re-tether, memberships dropdown, permissions, platform gating)', () => {
    // The desktop Settings page was missing several controls the classic popover
    // has, leaving features unreachable from the desktop window.
    // (1) HQ folder re-tether — a "Change…" button calling pick_folder.
    expect(page).toContain("invoke<string | null>('pick_folder')");
    expect(page).toContain('Change…');
    // (2) Real membership dropdown for default-recording-company, not a free-text
    //     UID field that saved unvalidated input only on blur.
    expect(page).toContain("invoke<CompanyMembership[]>('meetings_list_memberships')");
    expect(page).toContain('<option value="">Personal</option>');
    expect(page).toContain('{m.companyName?.trim()');
    // No raw UID leaked as the option label.
    expect(page).not.toContain('{m.companyUid}</option>');
    // (3) Meeting permissions wizard reachable from the desktop window.
    expect(page).toContain("invoke('open_meeting_permissions_window')");
    expect(page).toContain('permissionState.meetingPermissions');
    // (4) Platform toggles only shown when detection is enabled (otherwise they
    //     looked actionable but changed nothing).
    expect(page).toContain('{#if meetingDetectEnabled}');
  });

  it('renders the section anchors, scroll behavior, and gated annotations', () => {
    // SettingsPage renders every section inline (one scroll) and scrolls the
    // active one into view; the secondary sidebar provides the section index.
    for (const [id, label] of [
      ['sync', 'Sync'],
      ['notifications', 'Notifications'],
      ['updates', 'Updates'],
      ['general', 'General'],
      ['meetings', 'Meetings'],
    ]) {
      expect(page).toContain(`id="${id}"`);
      expect(page).toContain(`<h2>${label}</h2>`);
    }
    expect(page).toContain("scrollIntoView({ behavior: 'smooth', block: 'start' })");
    expect(page).toContain('class="setting-row gated-row"');
    expect(page).toContain('@getindigo.ai only');
    expect(page).toContain('<em>Gated</em>');
  });

  it('styles toggles as macOS green-fill pills (the one sanctioned control color)', () => {
    // SPEC §5/§6: setting toggles use a 26×16 pill, green when on — not the
    // default native checkbox. Locks the appearance:none pill + green fill.
    expect(page).toContain("input[type='checkbox']");
    expect(page).toContain('appearance: none');
    expect(page).toContain('var(--v4-ok)');
  });

  it('restyles first-run and keeps the one-time auto-sync notice explicit', () => {
    expect(firstRun).toContain('data-testid="v4-first-run-card"');
    expect(firstRun).toContain('FIRST RUN');
    expect(firstRun).toContain('One-time auto-sync notice');
    expect(firstRun).toContain('Auto-sync is on for this first pass');
    expect(firstRun).toContain('var(--v4-surface');
    expect(firstRun).toContain('var(--v4-hairline');
  });
});
