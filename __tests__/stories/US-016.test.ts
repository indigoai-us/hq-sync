import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { describe, expect, it } from 'vitest';

const desktopApp = readFileSync(
  resolve(process.cwd(), 'src/desktop-alt/DesktopApp.svelte'),
  'utf8',
);
const commandPalette = readFileSync(
  resolve(process.cwd(), 'src/desktop-alt/components/CommandPalette.svelte'),
  'utf8',
);
const statusBar = readFileSync(
  resolve(process.cwd(), 'src/desktop-alt/DesktopStatusBar.svelte'),
  'utf8',
);
const banner = readFileSync(
  resolve(process.cwd(), 'src/components/BannerNotification.svelte'),
  'utf8',
);
const homePage = readFileSync(
  resolve(process.cwd(), 'src/desktop-alt/pages/HomePage.svelte'),
  'utf8',
);
const companiesPage = readFileSync(
  resolve(process.cwd(), 'src/desktop-alt/pages/CompaniesPage.svelte'),
  'utf8',
);
const meetingsPage = readFileSync(
  resolve(process.cwd(), 'src/desktop-alt/pages/MeetingsPage.svelte'),
  'utf8',
);
const liveNowCard = readFileSync(
  resolve(process.cwd(), 'src/desktop-alt/components/LiveNowCard.svelte'),
  'utf8',
);
const messagesShell = readFileSync(
  resolve(process.cwd(), 'src/components/messaging/MessagesShell.svelte'),
  'utf8',
);
const marketplacePanel = readFileSync(
  resolve(process.cwd(), 'src/desktop-alt/panels/MarketplacePanel.svelte'),
  'utf8',
);

function normalize(source: string): string {
  return source.replace(/\s+/g, ' ');
}

describe('US-016: V4 connective tissue stays complete', () => {
  it('renders first-load skeletons and empty states across the main V4 surfaces', () => {
    expect(homePage).toContain('class="home-skeleton"');
    expect(homePage).toContain('aria-busy="true"');
    expect(companiesPage).toContain('class="companies-skeleton"');
    expect(companiesPage).toContain('No companies connected yet');
    expect(messagesShell).toContain('Loading conversations');
    expect(messagesShell).toContain('class="pane-empty"');
    expect(liveNowCard).toContain('No active meeting window has been detected.');
    expect(meetingsPage).toContain('No calendars connected yet');
    expect(marketplacePanel).toContain('class="grid-skeleton"');
    expect(marketplacePanel).toContain('data-testid="marketplace-empty"');
  });

  it('keeps banner notifications source-agnostic with click, action, and dismiss affordances', () => {
    expect(banner).toContain('kind: string;');
    expect(banner).toContain('actionLabel?: string | null;');
    expect(banner).toContain('actionId?: string | null;');
    expect(banner).toContain('clickActionId: string;');
    expect(banner).toContain("listen<BannerPayload>('banner:event'");
    expect(banner).toContain("invoke('banner_window_ready')");
    expect(banner).toContain("invoke('banner_action', { action: actionId, payload })");
    expect(banner).toContain("invoke('dismiss_banner')");
    expect(banner).toContain('data-kind={payload.kind}');
    expect(banner).toContain('{payload.actionLabel}');
  });

  it('groups cmd-K into ACTIONS and NAVIGATE and includes company section destinations', () => {
    const app = normalize(desktopApp);
    const palette = normalize(commandPalette);

    expect(desktopApp).toContain('COMPANY_SECTIONS');
    expect(desktopApp).toContain('companies.flatMap((company, index) => [');
    expect(desktopApp).toContain('label: `Go to ${company.displayName} ${section.label}`');
    expect(desktopApp).toContain("action: () => navigate({ kind: 'company', slug: company.slug, tab: section.id })");

    expect(commandPalette).toContain("label: 'ACTIONS'");
    expect(commandPalette).toContain("label: 'NAVIGATE'");
    expect(commandPalette).toContain("return command.id.startsWith('command-go-') ? 'navigate' : 'actions';");
    expect(palette).toContain('{#each commandSections as section (section.id)}');
    expect(palette).toContain('<div class="command-section-title">{section.label}</div>');
    expect(app).toContain("label: 'Sync now'");
    expect(app).toContain("label: 'Go to Companies'");
  });

  it('keeps the V4 status bar tied to sync summary, watching count, next meeting, and version', () => {
    expect(statusBar).toContain('filesProgressed?: number;');
    expect(statusBar).toContain('totalFiles?: number;');
    expect(statusBar).toContain('workspaceCount?: number;');
    expect(statusBar).toContain('nextMeetingLabel?: string | null;');
    expect(statusBar).toContain('{filesProgressed}/{totalFiles} files');
    expect(statusBar).toContain('watching <span class="mono">{workspaceCount}</span>');
    expect(statusBar).toContain('next <span class="mono">{nextMeetingLabel}</span>');
    expect(statusBar).toContain('v{version}');
    expect(desktopApp).toContain('<DesktopStatusBar');
    expect(desktopApp).toContain('workspaceCount={companies.length}');
    expect(desktopApp).toContain('{nextMeetingLabel}');
  });
});
