import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { describe, expect, it } from 'vitest';

const desktopApp = readFileSync(
  resolve(process.cwd(), 'src/desktop-alt/DesktopApp.svelte'),
  'utf8',
);
const trayApp = readFileSync(resolve(process.cwd(), 'src/App.svelte'), 'utf8');
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

describe('US-013: Status bar + global ⌘K command surface', () => {
  it("renders the three-region status bar from real sync progress, upcoming meeting cache, and build version", () => {
    const app = normalize(desktopApp);
    const tray = normalize(trayApp);
    const bar = normalize(statusBar);

    expect(app).toContain('version={__APP_VERSION__}');
    expect(tray).toContain(
      'const effectiveTotalFiles = $derived( syncPlanTotalFiles > 0 ? syncPlanTotalFiles : syncTotalFiles );',
    );
    expect(app).toContain('const effectiveTotalFiles = $derived(syncPlanTotalFiles > 0 ? syncPlanTotalFiles : syncTotalFiles)');
    expect(app).toContain('filesProgressed={syncFilesProgressed}');
    expect(app).toContain('totalFiles={effectiveTotalFiles}');

    expect(bar).toContain('<div class="status-left">');
    expect(bar).toContain('<div class="status-right">');
    expect(app).toContain('loadMeetingsCache<MeetingEvent, ScheduledBot, GoogleAccount, GoogleCalendar>()');
    expect(app).toContain('.filter((event) => isToday(event, now))');
    expect(app).toContain('.filter((event) => (eventStart(event)?.getTime() ?? 0) >= now.getTime())');
    expect(app).toContain('return `${company} · in ${minutes}m`;');
    expect(bar).toContain('{#if nextMeetingLabel}');
    expect(bar).toContain('Connected');
    expect(bar).toContain("if (state === 'syncing') return 'syncing';");
    expect(bar).toContain("if (state === 'error' || state === 'auth-error') return 'error';");
    expect(bar).toContain("if (state === 'conflict' || state === 'setup-needed') return 'conflict';");
    expect(bar).toContain("return 'idle';");
    expect(bar).toContain('Math.round((filesProgressed / totalFiles) * 100)');
    expect(bar).toContain('`Syncing ${progress?.company ?? \'workspace\'} · ${syncPercent}%`');
    expect(bar).toContain('class="sparkbars"');
    expect(bar).toContain('indigo-vpn');
    expect(bar).toContain('v{version}');
  });

  it('opens a modal command palette on cmd/ctrl-K with Sync now as the first row', () => {
    const app = normalize(desktopApp);
    const palette = normalize(commandPalette);

    expect(app).toContain("event.key.toLowerCase() === 'k'");
    expect(app).toContain('commandPaletteOpen = true');
    expect(app).toMatch(/const commandItems = \$derived<CommandPaletteItem\[]>\(\[\s*\{\s*id: 'command-sync-now',\s*label: 'Sync now'/);
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
    expect(palette).toContain('aria-modal="true"');
    expect(palette).toContain('bind:value={query}');
    expect(palette).toContain('role="listbox"');
    expect(palette).toContain('role="option"');
    expect(palette).toContain('button');
    expect(palette).toContain('function fuzzyMatch');
  });

  it('fuzzy-filters meet to Go to Meetings, executes Enter, switches the main pane, and closes on Enter or Esc', () => {
    const app = normalize(desktopApp);
    const palette = normalize(commandPalette);

    expect(palette).toContain('fuzzyMatch(`${command.label} ${command.detail} ${command.shortcut ?? \'\'}`, query)');
    expect(app).toMatch(/label: 'Go to Meetings'[\s\S]*detail: 'Show calendar and recordings'[\s\S]*action: \(\) => navigate\(\{ kind: 'meetings' \}\)/);
    expect(app).toContain("{:else if route.kind === 'meetings'}");
    expect(app).toContain('<MeetingsPage />');

    expect(palette).toContain("if (event.key === 'ArrowDown')");
    expect(palette).toContain("if (event.key === 'ArrowUp')");
    expect(palette).toContain("if (event.key === 'Enter')");
    expect(palette).toContain('void execute(filteredCommands[highlightedIndex])');
    expect(palette).toContain('await command.action();');
    expect(palette).toContain('onclose();');
    expect(palette).toContain("if (event.key === 'Escape')");
    expect(palette).toContain('onkeydown={handleKeydown}');
    expect(palette).toContain('bind:this={inputEl}');
    expect(palette).toContain('onclick={() => void execute(command)}');
  });
});
