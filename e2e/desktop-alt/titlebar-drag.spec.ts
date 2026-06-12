import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';

// Source-contract regression guard for the desktop-alt title bar (V4TitleBar
// since US-002).
//
// Two bugs were fixed here and must not regress (the scripted E2E harness never
// boots a real Tauri window, so these are asserted against source):
//
// 1. Drag — the title bar is a single drag region (data-tauri-drag-region on the
//    <header>). Its child elements would otherwise swallow the drag so only the
//    bare edge moved the window; the non-interactive status cluster falls
//    through via pointer-events:none and only the contextual action button
//    (Sync Now / Cancel / Retry) stays interactive.
// 2. Shadows — the desktop-alt window shows the real macOS traffic lights
//    (TitleBarStyle::Overlay). A redundant set of fake CSS dots (.titlebar-traffic)
//    rendered underneath them, offset and misaligned, reading as "button shadows".
//    Those fake dots were removed and must never come back.

const titleBarPath = fileURLToPath(
  new URL('../../src/desktop-alt/v4/V4TitleBar.svelte', import.meta.url),
);
const appPath = fileURLToPath(new URL('../../src/desktop-alt/DesktopApp.svelte', import.meta.url));
const cssPath = fileURLToPath(new URL('../../src/desktop-alt/styles/desktop-alt.css', import.meta.url));
const capPath = fileURLToPath(new URL('../../src-tauri/capabilities/desktop-alt.json', import.meta.url));
const builderPath = fileURLToPath(new URL('../../src-tauri/src/commands/desktop_alt.rs', import.meta.url));
const titleBar = readFileSync(titleBarPath, 'utf8');
const app = readFileSync(appPath, 'utf8');
const css = readFileSync(cssPath, 'utf8');
const cap = JSON.parse(readFileSync(capPath, 'utf8'));
const builder = readFileSync(builderPath, 'utf8');

describe('desktop-alt title bar (V4)', () => {
  it('mounts V4TitleBar as the shell title bar', () => {
    expect(app).toContain('<V4TitleBar');
  });

  it('marks the title bar header as a Tauri drag region', () => {
    expect(titleBar).toMatch(/<header class="v4-titlebar" data-tauri-drag-region/);
  });

  it('lets the non-interactive status cluster fall through to the drag region', () => {
    expect(titleBar).toMatch(/\.v4-status\s*\{[\s\S]*?pointer-events: none;/);
  });

  it('keeps exactly one interactive primary action in the title bar', () => {
    // The contextual action button is the only interactive element; it never
    // sets pointer-events:none so it opts out of the drag region by default.
    expect(titleBar).toMatch(/<button type="button" class="v4-action"/);
    expect(titleBar).not.toMatch(/\.v4-action\s*\{[\s\S]*?pointer-events: none/);
  });

  it('does not render fake CSS traffic-light dots (the real macOS overlay owns that space)', () => {
    for (const source of [titleBar, app, css]) {
      expect(source).not.toContain('titlebar-traffic');
    }
  });

  it('reserves a left inset for the real macOS overlay traffic lights', () => {
    expect(titleBar).toMatch(/\.v4-titlebar\s*\{[\s\S]*?padding: 0 14px 0 78px;/);
  });

  it('grants the desktop-alt window the start-dragging permission (the web drag region is inert without it)', () => {
    expect(cap.permissions).toContain('core:window:allow-start-dragging');
  });

  it('does not paint a native window title over the custom titlebar (Overlay style)', () => {
    // The window title must be blank — a non-empty title renders in the Overlay
    // title bar on top of the status text ("HQ" overlapping "All synced").
    expect(builder).toMatch(/\.title\(""\)/);
    expect(builder).not.toMatch(/\.title\("HQ"\)/);
  });
});
