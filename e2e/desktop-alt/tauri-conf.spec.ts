import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';

// Source-contract regression guard for the desktop-alt window declaration in
// src-tauri/tauri.conf.json. The scripted E2E harness mocks window behaviour and
// never boots a real Tauri app, so an invalid tauri.conf.json (e.g. a bad
// `titleBarStyle` enum casing) passes every other gate but fails `tauri dev` at
// launch. Regression for: `titleBarStyle: "overlay"` (must be PascalCase
// "Overlay") which broke the cold dev build after US-002.

const confPath = fileURLToPath(new URL('../../src-tauri/tauri.conf.json', import.meta.url));
const conf = JSON.parse(readFileSync(confPath, 'utf8'));

// Valid values for the macOS title bar style in Tauri 2's tauri.conf.json schema.
const VALID_TITLE_BAR_STYLES = ['Visible', 'Transparent', 'Overlay'];

describe('tauri.conf.json desktop-alt window declaration', () => {
  const windows = conf.app?.windows ?? [];
  const desktopAlt = windows.find((w: { label?: string }) => w.label === 'desktop-alt');

  it('declares the desktop-alt window', () => {
    expect(desktopAlt, 'desktop-alt window must exist in tauri.conf.json').toBeDefined();
  });

  it('uses a schema-valid titleBarStyle for every window (PascalCase enum)', () => {
    for (const w of windows) {
      if (w.titleBarStyle !== undefined) {
        expect(
          VALID_TITLE_BAR_STYLES,
          `window "${w.label ?? '(main)'}" has invalid titleBarStyle "${w.titleBarStyle}" — Tauri 2 requires one of ${VALID_TITLE_BAR_STYLES.join(', ')}`,
        ).toContain(w.titleBarStyle);
      }
    }
  });

  it('keeps the desktop-alt window hidden + lazily created (popover stays default)', () => {
    expect(desktopAlt.visible).toBe(false);
    expect(desktopAlt.create).toBe(false);
  });

  it('keeps the desktop-alt window decorated at the expected size', () => {
    expect(desktopAlt.decorations).toBe(true);
    expect(desktopAlt.width).toBe(1180);
    expect(desktopAlt.height).toBe(760);
  });
});
