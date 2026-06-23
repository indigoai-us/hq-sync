import { describe, expect, it } from 'vitest';
import { readRepoFile } from './harness';

// Locks the "no onboarding pages, just pop open + sync on first run" behavior
// and the sync-on-launch default. A regression that re-introduces a welcome
// carousel / auto-sync notice, or flips the sync-on-launch default back to
// off, should fail here.
describe('first-run pops the app open instead of onboarding pages', () => {
  const app = readRepoFile('src/App.svelte');
  const settings = readRepoFile('src/components/Settings.svelte');
  const settingsPage = readRepoFile('src/desktop-alt/pages/SettingsPage.svelte');
  const settingsRust = readRepoFile('src-tauri/src/commands/settings.rs');

  it('has no onboarding components left in the tree', () => {
    expect(() => readRepoFile('src/components/FirstRunWelcome.svelte')).toThrow();
    expect(() => readRepoFile('src/components/AutoSyncNotice.svelte')).toThrow();
  });

  it('App.svelte does not import or render the onboarding overlays', () => {
    expect(app).not.toContain('FirstRunWelcome');
    expect(app).not.toContain('AutoSyncNotice');
    expect(app).not.toContain('showWelcome');
    expect(app).not.toContain('showAutoSyncNotice');
  });

  it('first run pops the popover open, starts a sync, and marks first-run complete', () => {
    // The runOnboarding path must force the window open, kick a sync, and
    // persist completion so it never repeats — with no carousel in between.
    expect(app).toContain("invoke<boolean>('is_first_run')");
    expect(app).toContain("invoke('show_main_window')");
    expect(app).toContain('void handleSyncNow();');
    expect(app).toContain("invoke('mark_first_run_complete')");
  });

  it('sync-on-launch defaults ON in both Settings surfaces', () => {
    expect(settings).toContain('settings.syncOnLaunch ?? true');
    expect(settingsPage).toContain('settings.syncOnLaunch ?? true');
    // And no surface silently falls back to OFF.
    expect(settings).not.toContain('settings.syncOnLaunch ?? false');
    expect(settingsPage).not.toContain('settings.syncOnLaunch ?? false');
  });

  it('sync-on-launch defaults ON in the Rust get_settings defaults', () => {
    // Fresh-install (no file) branch and the per-field default both resolve ON.
    expect(settingsRust).toContain('sync_on_launch: Some(true)');
    expect(settingsRust).toContain('prefs.sync_on_launch.unwrap_or(true)');
    expect(settingsRust).not.toContain('prefs.sync_on_launch.unwrap_or(false)');
  });
});
