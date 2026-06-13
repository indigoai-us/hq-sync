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
