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
  it('renders the Indigo desktop-alt toggle in the header next to the settings gear', () => {
    const gate = getDesktopAltGate();
    const compactGate = normalize(gate);
    const toggleIndex = gate.indexOf('data-testid="desktop-alt-toggle"');
    const settingsIndex = gate.indexOf('aria-label="Settings"');

    expect(toggleIndex).toBeGreaterThanOrEqual(0);
    expect(settingsIndex).toBeGreaterThan(toggleIndex);
    expect(compactGate).toContain('class="header-icon-button desktop-alt-toggle"');
    expect(compactGate).toContain('class="header-icon-button header-settings-toggle"');
    expect(compactGate).toContain('title="Open desktop view"');
    expect(compactGate).toContain('aria-label="Open desktop view (Indigo dogfood)"');
    expect(compactGate).toMatch(/<rect\b[^>]+width="12"[^>]+height="11"/);
    expect(compactGate).toContain('intentionally distinct from the Settings gear');
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

  it('gates the two-row header layout to the desktop-alt header state', () => {
    const compactSource = normalize(popoverSource);

    expect(compactSource).toContain(
      '<header class="popover-header" class:has-desktop-alt-controls={desktopAltEnabled} data-tauri-drag-region>',
    );
    expect(compactSource).toContain('.popover-header.has-desktop-alt-controls { flex-wrap: wrap;');
    expect(compactSource).toContain('.popover-header.has-desktop-alt-controls .header-text { order: 2; flex: 1 0 100%;');
    expect(compactSource).toContain('.popover-header.has-desktop-alt-controls .header-sync { margin-left: auto;');
  });
});
