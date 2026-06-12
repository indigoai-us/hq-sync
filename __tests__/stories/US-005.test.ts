import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';

// The V4 Home surface (HomePage + home-model + NeedsYouCard + ActivityDigest)
// superseded the SyncPage/HeroStatus/SourcesList sources-table in US-003 of
// the V4 redesign — same coverage (real sync state + events, no demo
// fixtures), new contracts.
const desktopApp = readFileSync(resolve(process.cwd(), 'src/desktop-alt/DesktopApp.svelte'), 'utf8');
const homePage = readFileSync(
  resolve(process.cwd(), 'src/desktop-alt/pages/HomePage.svelte'),
  'utf8',
);
const homeModel = readFileSync(resolve(process.cwd(), 'src/desktop-alt/v4/home-model.ts'), 'utf8');
const activityDigest = readFileSync(
  resolve(process.cwd(), 'src/desktop-alt/v4/ActivityDigest.svelte'),
  'utf8',
);
const syncModel = readFileSync(resolve(process.cwd(), 'src/desktop-alt/lib/sync-model.ts'), 'utf8');
const appShell = readFileSync(resolve(process.cwd(), 'src/App.svelte'), 'utf8');
const cognitoCommands = readFileSync(
  resolve(process.cwd(), 'src-tauri/src/commands/cognito.rs'),
  'utf8',
);
const featureGate = readFileSync(
  resolve(process.cwd(), 'src-tauri/src/util/feature_gate.rs'),
  'utf8',
);

function normalize(source: string): string {
  return source.replace(/\s+/g, ' ');
}

describe('US-005: Alt Home surface wires to real sync state and events', () => {
  it('subscribes DesktopApp to the sync events that drive the popover', () => {
    for (const eventName of [
      'sync:progress',
      'sync:complete',
      'sync:all-complete',
      'sync:plan',
      'sync:totals',
      'sync:fanout-plan',
      'sync:error',
      'sync:conflict',
      'sync:personal-first-push-progress',
      'sync:personal-first-push-complete',
    ]) {
      expect(desktopApp).toContain(`'${eventName}'`);
    }

    expect(desktopApp).toContain("invoke<WorkspacesResult>('list_syncable_workspaces')");
    expect(desktopApp).toContain("invoke<ActivityEntry[]>('get_activity_log')");
  });

  it('wires the Home inline actions to real Tauri commands', () => {
    const app = normalize(desktopApp);

    expect(app).toContain("await invoke('start_sync')");
    expect(app).toContain("await invoke('cancel_sync')");
    expect(app).toContain("function handleOpenSettings(tab?: SettingsTab) { navigate({ kind: 'settings', tab }); }");
    expect(app).toContain("await invoke('resolve_conflict', { path, strategy })");
    expect(app).toContain("invoke('open_in_editor', { path })");
    expect(app).toContain("await invoke('restore_from_upstream', {");
    expect(app).toContain("invoke('open_drift_detail', { report })");
    expect(app).toContain("await invoke('refresh_tokens')");
    expect(app).toContain("invoke('open_activity_log')");
  });

  it('renders health, needs-you, and digest from real state without demo fixtures', () => {
    const combined = normalize(`${homePage}\n${activityDigest}\n${homeModel}\n${syncModel}`);

    expect(combined).toContain('Needs you');
    expect(combined).toContain('Sync in progress');
    expect(combined).toContain('Today across your companies');
    expect(combined).toContain('Nothing yet today');
    expect(combined).toContain('Technical details');
    expect(combined).not.toMatch(/Acme|Volta|Globex|Indigo demo|prototype/i);
  });

  it('keeps auth success and token writes connected to the desktop-alt gate refresh', () => {
    const app = normalize(appShell);
    const cognito = normalize(cognitoCommands);
    const gate = normalize(featureGate);

    expect(app).toContain("desktopAltEnabled = await invoke<boolean>('desktop_alt_enabled')");
    expect(app).toContain('function handleAuthSuccess(auth: { authenticated: boolean; expiresAt: string })');
    expect(app).toMatch(/function handleAuthSuccess[\s\S]*void refreshDesktopAltEnabled\(\);/);
    expect(app).toMatch(
      /if \(authenticated\) \{ await refreshDesktopAltEnabled\(\); void runOnboarding\(\); \}/,
    );

    expect(cognito).toMatch(/pub async fn set_tokens[\s\S]*clear_cached_gate\(\);/);
    expect(gate).toContain('pub fn clear_cached_gate()');
    expect(gate).toMatch(/pub fn clear_cached_gate\(\) \{[\s\S]*\*guard = None;/);
  });

  it('pins debugger event regressions in DesktopApp handlers', () => {
    const app = normalize(desktopApp);

    expect(app).toMatch(
      /sync:personal-first-push-progress[\s\S]*company: 'personal'[\s\S]*updateWorkspaceStats\('personal'/,
    );
    expect(app).toMatch(
      /sync:complete[\s\S]*aborted: stats\.aborted \|\| event\.payload\.aborted[\s\S]*syncState = 'conflict'/,
    );
    expect(app).toMatch(
      /sync:error[\s\S]*if \(event\.payload\.company\)[\s\S]*errorMessage: event\.payload\.message/,
    );
  });
});
