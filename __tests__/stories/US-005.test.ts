import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';

const desktopApp = readFileSync(resolve(process.cwd(), 'src/desktop-alt/DesktopApp.svelte'), 'utf8');
const syncPage = readFileSync(resolve(process.cwd(), 'src/desktop-alt/pages/SyncPage.svelte'), 'utf8');
const heroStatus = readFileSync(
  resolve(process.cwd(), 'src/desktop-alt/components/HeroStatus.svelte'),
  'utf8',
);
const sourcesList = readFileSync(
  resolve(process.cwd(), 'src/desktop-alt/components/SourcesList.svelte'),
  'utf8',
);
const attentionPanel = readFileSync(
  resolve(process.cwd(), 'src/desktop-alt/components/AttentionPanel.svelte'),
  'utf8',
);
const syncModel = readFileSync(resolve(process.cwd(), 'src/desktop-alt/lib/sync-model.ts'), 'utf8');

function normalize(source: string): string {
  return source.replace(/\s+/g, ' ');
}

describe('US-005: Alt Sync page wires to real sync state and events', () => {
  it('subscribes DesktopApp to the sync events that drive the popover', () => {
    for (const eventName of [
      'sync:progress',
      'sync:complete',
      'sync:all-complete',
      'sync:plan',
      'sync:totals',
      'sync:fanout-plan',
      'sync:error',
      'sync:personal-first-push-progress',
      'sync:personal-first-push-complete',
    ]) {
      expect(desktopApp).toContain(`'${eventName}'`);
    }

    expect(desktopApp).toContain("invoke<WorkspacesResult>('list_syncable_workspaces')");
    expect(desktopApp).toContain("invoke<ActivityEntry[]>('get_activity_log')");
  });

  it('wires quick actions to real Tauri commands and keeps Add source as Coming soon', () => {
    const app = normalize(desktopApp);
    const hero = normalize(heroStatus);

    expect(app).toContain("await invoke('start_sync')");
    expect(app).toContain("await invoke('open_settings_window')");
    expect(hero).toContain('title="Coming soon"');
    expect(hero).toContain('Sync all');
    expect(hero).toContain('Settings');
    expect(hero).toContain('Add source');
  });

  it('renders sources, attention, and recent activity without demo fixtures', () => {
    const combined = normalize(`${syncPage}\n${sourcesList}\n${attentionPanel}\n${syncModel}`);

    expect(combined).toContain('No syncable workspaces found.');
    expect(combined).toContain('No sync events yet');
    expect(combined).toContain('Reauth');
    expect(combined).toContain('Paused');
    expect(combined).toContain('Up to date');
    expect(combined).not.toMatch(/Acme|Volta|Globex|Indigo demo|prototype/i);
  });
});
