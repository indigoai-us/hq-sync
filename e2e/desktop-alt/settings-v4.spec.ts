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
    ]) {
      expect(page).toContain(key);
      expect(settings).toContain(key);
    }
  });

  it('renders the V4 section index with scroll anchors and gated annotations', () => {
    expect(page).toContain("id: 'sync', label: 'Sync'");
    expect(page).toContain("id: 'notifications', label: 'Notifications'");
    expect(page).toContain("id: 'updates', label: 'Updates'");
    expect(page).toContain("id: 'general', label: 'General'");
    expect(page).toContain("id: 'meetings', label: 'Meetings'");
    expect(page).toContain('scrollIntoView({ block: \'start\', behavior: \'smooth\' })');
    expect(page).toContain('class="setting-row gated-row"');
    expect(page).toContain('@getindigo.ai only');
    expect(page).toContain('<em>Gated</em>');
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
