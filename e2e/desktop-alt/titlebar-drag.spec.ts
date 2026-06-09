import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';

// Source-contract regression guard for the desktop-alt title bar.
//
// Two bugs were fixed here and must not regress (the scripted E2E harness never
// boots a real Tauri window, so these are asserted against source):
//
// 1. Drag — the title bar is a single drag region (data-tauri-drag-region on the
//    <header>). Its child elements would otherwise swallow the drag so only the
//    bare edge moved the window; every non-interactive child now falls through
//    via pointer-events:none and only the Sync Now button opts back in.
// 2. Shadows — the desktop-alt window shows the real macOS traffic lights
//    (TitleBarStyle::Overlay). A redundant set of fake CSS dots (.titlebar-traffic)
//    rendered underneath them, offset and misaligned, reading as "button shadows".
//    Those fake dots were removed.

const appPath = fileURLToPath(new URL('../../src/desktop-alt/DesktopApp.svelte', import.meta.url));
const cssPath = fileURLToPath(new URL('../../src/desktop-alt/styles/desktop-alt.css', import.meta.url));
const capPath = fileURLToPath(new URL('../../src-tauri/capabilities/desktop-alt.json', import.meta.url));
const builderPath = fileURLToPath(new URL('../../src-tauri/src/commands/desktop_alt.rs', import.meta.url));
const app = readFileSync(appPath, 'utf8');
const css = readFileSync(cssPath, 'utf8');
const cap = JSON.parse(readFileSync(capPath, 'utf8'));
const builder = readFileSync(builderPath, 'utf8');

describe('desktop-alt title bar', () => {
  it('marks the title bar header as a Tauri drag region', () => {
    expect(app).toMatch(/<header class="desktop-titlebar" data-tauri-drag-region/);
  });

  it('lets non-interactive title bar elements fall through to the drag region', () => {
    // Containers and their children disable pointer events so clicks reach the
    // header's drag region instead of being swallowed.
    expect(css).toMatch(/\.desktop-titlebar > \*[\s\S]*?pointer-events: none;/);
  });

  it('keeps the Sync Now button interactive (opts back into pointer events)', () => {
    expect(css).toMatch(/\.desktop-titlebar \.titlebar-sync-now\s*\{\s*pointer-events: auto;/);
  });

  it('does not render fake CSS traffic-light dots (the real macOS overlay owns that space)', () => {
    expect(app).not.toContain('titlebar-traffic');
    expect(css).not.toContain('titlebar-traffic');
  });

  it('reserves a left inset for the real macOS overlay traffic lights', () => {
    expect(css).toMatch(/\.desktop-titlebar\s*\{[\s\S]*?padding: 0 14px 0 78px;/);
  });

  it('grants the desktop-alt window the start-dragging permission (the web drag region is inert without it)', () => {
    expect(cap.permissions).toContain('core:window:allow-start-dragging');
  });

  it('does not paint a native window title over the custom titlebar (Overlay style)', () => {
    // The window title must be blank — a non-empty title renders in the Overlay
    // title bar on top of the verdict text ("HQ" overlapping "All synced").
    expect(builder).toMatch(/\.title\(""\)/);
    expect(builder).not.toMatch(/\.title\("HQ"\)/);
  });
});
