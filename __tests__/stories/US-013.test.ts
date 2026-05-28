import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { describe, expect, it } from 'vitest';

const desktopApp = readFileSync(
  resolve(process.cwd(), 'src/desktop-alt/DesktopApp.svelte'),
  'utf8',
);
const statusBar = readFileSync(
  resolve(process.cwd(), 'src/desktop-alt/DesktopStatusBar.svelte'),
  'utf8',
);
const commandPalette = readFileSync(
  resolve(process.cwd(), 'src/desktop-alt/components/CommandPalette.svelte'),
  'utf8',
);

function normalize(source: string): string {
  return source.replace(/\s+/g, ' ');
}

describe('US-013: desktop status bar and command palette', () => {
  it('renders status-bar regions from sync state, progress, meeting cache, network placeholder, VPN, and build version', () => {
    const app = normalize(desktopApp);
    const bar = normalize(statusBar);

    expect(app).toContain('version={__APP_VERSION__}');
    expect(app).toContain('const effectiveTotalFiles = $derived(syncPlanTotalFiles > 0 ? syncPlanTotalFiles : syncTotalFiles)');
    expect(app).toContain('filesProgressed={syncFilesProgressed}');
    expect(app).toContain('totalFiles={effectiveTotalFiles}');
    expect(app).toContain('loadMeetingsCache<MeetingEvent, ScheduledBot, GoogleAccount, GoogleCalendar>()');
    expect(app).toContain('return `${company} · in ${minutes}m`;');
    expect(bar).toContain('Connected');
    expect(bar).toContain("if (state === 'syncing') return 'syncing';");
    expect(bar).toContain("if (state === 'error' || state === 'auth-error') return 'error';");
    expect(bar).toContain("if (state === 'conflict' || state === 'setup-needed') return 'conflict';");
    expect(bar).toContain("return 'idle';");
    expect(bar).toContain('`Syncing ${progress?.company ?? \'workspace\'} · ${syncPercent}%`');
    expect(bar).toContain('class="sparkbars"');
    expect(bar).toContain('indigo-vpn');
    expect(bar).toContain('v{version}');
  });

  it('opens a keyboard-navigable command palette on cmd/ctrl-K and wires required actions', () => {
    const app = normalize(desktopApp);
    const palette = normalize(commandPalette);

    expect(app).toContain("event.key.toLowerCase() === 'k'");
    expect(app).toContain('commandPaletteOpen = true');
    expect(app).toContain("label: 'Sync now'");
    expect(app).toContain('action: handleSyncAll');
    expect(app).toContain("label: 'Open settings'");
    expect(app).toContain('action: handleOpenSettings');
    expect(app).toContain("label: 'Go to Sync'");
    expect(app).toContain("action: () => navigate({ kind: 'sync' })");
    expect(app).toContain("label: 'Go to Meetings'");
    expect(app).toContain("action: () => navigate({ kind: 'meetings' })");
    expect(app).toContain('label: `Go to ${company.displayName}`');
    expect(app).toContain("action: () => navigate({ kind: 'company', slug: company.slug })");
    expect(app).toContain('<CommandPalette commands={commandItems} onclose={() => (commandPaletteOpen = false)} />');
    expect(palette).toContain('role="dialog"');
    expect(palette).toContain("if (event.key === 'ArrowDown')");
    expect(palette).toContain("if (event.key === 'ArrowUp')");
    expect(palette).toContain("if (event.key === 'Enter')");
    expect(palette).toContain("if (event.key === 'Escape')");
    expect(palette).toContain('function fuzzyMatch');
  });
});
