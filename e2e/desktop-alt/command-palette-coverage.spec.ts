import { describe, expect, it } from 'vitest';
import { readRepoFile } from './harness';

// The ⌘K command palette is the keyboard-first navigation surface for the whole
// desktop window. SPEC requires it to reach EVERY routable sub-surface, not just
// the top-level destinations: a user typing "marketplace" or "notifications"
// must be able to jump straight there. Company sub-sections were already
// enumerated (COMPANY_SECTIONS); this locks down the matching coverage for the
// Library sub-tabs and the Settings sub-sections so the palette can't silently
// regress back to "Go to Library" / "Go to Settings" landing pages only.

describe('desktop-alt command palette coverage', () => {
  const desktopApp = readRepoFile('src/desktop-alt/DesktopApp.svelte');
  const route = readRepoFile('src/desktop-alt/route.ts');

  it('imports the Library and Settings section tables + their defaults', () => {
    expect(desktopApp).toContain('LIBRARY_SECTIONS');
    expect(desktopApp).toContain('SETTINGS_SECTIONS');
    expect(desktopApp).toContain('DEFAULT_LIBRARY_TAB');
    expect(desktopApp).toContain('DEFAULT_SETTINGS_TAB');
  });

  it('enumerates every Library sub-tab into the palette (minus the default landing tab)', () => {
    // The builder filters out the default tab (it is already reachable via the
    // top-level "Go to Library" row) and maps the rest to navigate({tab}).
    expect(desktopApp).toContain(
      "LIBRARY_SECTIONS.filter((section) => section.id !== DEFAULT_LIBRARY_TAB)",
    );
    expect(desktopApp).toContain('command-go-library-${section.id}');
    expect(desktopApp).toContain("navigate({ kind: 'library', tab: section.id })");
  });

  it('enumerates every Settings sub-section into the palette (minus the default landing tab)', () => {
    expect(desktopApp).toContain(
      "SETTINGS_SECTIONS.filter((section) => section.id !== DEFAULT_SETTINGS_TAB)",
    );
    expect(desktopApp).toContain('command-go-settings-${section.id}');
    expect(desktopApp).toContain("navigate({ kind: 'settings', tab: section.id })");
  });

  it('the section tables it enumerates carry every routable tab', () => {
    // Guards against the tables being trimmed without the palette noticing —
    // these are the SPEC-ordered sub-surfaces the palette promises to reach.
    for (const tab of ['skills', 'workers', 'installed', 'marketplace', 'profile']) {
      expect(route).toContain(`id: '${tab}'`);
    }
    for (const tab of ['sync', 'notifications', 'updates', 'general', 'meetings']) {
      expect(route).toContain(`id: '${tab}'`);
    }
  });
});
