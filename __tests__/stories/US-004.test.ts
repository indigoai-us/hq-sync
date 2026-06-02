import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';

const popoverSource = readFileSync(
  resolve(process.cwd(), 'src/components/Popover.svelte'),
  'utf8',
);

function normalize(source: string): string {
  return source.replace(/\s+/g, ' ');
}

function getDesktopAltGate(source = popoverSource): string {
  const start = source.indexOf('{#if desktopAltEnabled}');
  expect(start).toBeGreaterThanOrEqual(0);

  const end = source.indexOf('{/if}', start);
  expect(end).toBeGreaterThan(start);

  return source.slice(start, end);
}

function getDesktopAltButton(source = popoverSource): string {
  const gate = getDesktopAltGate(source);
  const start = gate.indexOf('<button');
  expect(start).toBeGreaterThanOrEqual(0);

  const end = gate.indexOf('</button>', start);
  expect(end).toBeGreaterThan(start);

  return gate.slice(start, end);
}

describe('US-004: Toggle icon button in the classic popover header', () => {
  it('renders the Indigo desktop-alt toggle in the header action cluster, with Settings decluttered to the footer', () => {
    const gate = getDesktopAltGate();
    const compactGate = normalize(gate);
    const compactSource = normalize(popoverSource);
    const toggleIndex = gate.indexOf('data-testid="desktop-alt-toggle"');

    // The desktop-alt toggle lives in the identity-gated header action cluster.
    expect(toggleIndex).toBeGreaterThanOrEqual(0);
    expect(compactGate).toContain('class="header-icon-button desktop-alt-toggle"');
    expect(compactGate).toContain('title="Open desktop view"');
    expect(compactGate).toContain('aria-label="Open desktop view (Indigo dogfood)"');
    // "Open in window" glyph (replaced the old window-rect icon): the pop-out arrow.
    expect(compactGate).toContain('d="M13.5 2.5L7.5 8.5"');
    // Settings was decluttered OUT of the header (no header settings toggle) and
    // now lives as a single footer entry instead.
    expect(gate).not.toContain('aria-label="Settings"');
    expect(compactSource).not.toContain('header-settings-toggle');
    expect(compactSource).toContain('<button class="footer-action" onclick={onsettings}>');
  });

  it('wires the toggle click to invoke open_desktop_alt_window exactly once', () => {
    const button = normalize(getDesktopAltButton());
    const handler = normalize(
      popoverSource.slice(
        popoverSource.indexOf('async function openDesktopAltWindow()'),
        popoverSource.indexOf('$effect(() => {', popoverSource.indexOf('async function openDesktopAltWindow()')),
      ),
    );
    const invokeMatches = handler.match(/invoke\('open_desktop_alt_window'\)/g) ?? [];

    expect(button).toContain('onclick={openDesktopAltWindow}');
    expect(invokeMatches).toHaveLength(1);
    expect(handler).toContain("await invoke('open_desktop_alt_window')");
  });

  it('keeps the desktop-alt toggle absent from the non-Indigo DOM path', () => {
    const gate = getDesktopAltGate();
    const sourceWithoutGate = popoverSource.replace(gate, '');

    expect(sourceWithoutGate).not.toContain('data-testid="desktop-alt-toggle"');
    expect(sourceWithoutGate).not.toContain('Open desktop view (Indigo dogfood)');
    expect(normalize(popoverSource)).toContain('desktopAltEnabled = false');
  });

  it('surfaces failures as an inline header error and auto-dismisses within 5s', () => {
    const compactSource = normalize(popoverSource);

    expect(compactSource).toContain("console.error('open_desktop_alt_window failed:', e)");
    expect(compactSource).toContain("showDesktopAltError('Could not open desktop view.')");
    expect(compactSource).toContain('<p class="header-inline-error" role="status">{desktopAltError}</p>');
    expect(compactSource).toContain('desktopAltErrorTimer = setTimeout(() => {');
    expect(compactSource).toContain("desktopAltError = ''");
    expect(compactSource).toMatch(/},\s*5000\)/);
    expect(compactSource).toContain('clearDesktopAltErrorTimer()');
  });

  it('keeps the header on a single row, with the action cluster pushed to the edge instead of a two-row wrap', () => {
    const compactSource = normalize(popoverSource);

    // Single-row header: the old `class:has-desktop-alt-controls` two-row wrap was
    // removed when Settings moved to the footer (one settings entry).
    expect(compactSource).toContain('<header class="popover-header" data-tauri-drag-region>');
    expect(compactSource).not.toContain('has-desktop-alt-controls');
    // The draggable `.header-wordmark` left anchor (a quiet "HQ Sync" label that
    // replaced the removed HQ badge + workspace name/path) is also the flex
    // spacer: it soaks the spare width and pushes the right-aligned action
    // cluster to the edge on one line. The identity-gated controls sit in
    // `.header-actions`; the old empty `.header-spacer` div is gone.
    expect(compactSource).toContain('<div class="header-wordmark" data-tauri-drag-region>');
    expect(compactSource).not.toContain('class="header-spacer"');
    expect(compactSource).not.toContain('<div class="header-text">');
    expect(compactSource).toContain('<div class="header-actions">');
  });
});
