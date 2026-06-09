import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';

// Source-contract regression guard for the Moderation admin gate.
//
// Bug fixed here: the Moderation nav row (DesktopApp) and the ModerationPanel
// self-gate both gated on `desktop_alt_enabled`, which is the GA gate (true for
// EVERY signed-in user), not the `@getindigo.ai` admin gate. The result was the
// reviewer surface showing for normal HQ users (the server's 403 prevented an
// actual data leak, but the whole admin UI was visible). The fix adds a
// dedicated `desktop_alt_is_admin` command (→ feature_gate::is_indigo_user) and
// points both UX gates at it. These assertions ensure the gates never regress
// back to the GA gate.

const read = (rel: string) => readFileSync(fileURLToPath(new URL(rel, import.meta.url)), 'utf8');
const desktopApp = read('../../src/desktop-alt/DesktopApp.svelte');
const moderationPanel = read('../../src/desktop-alt/panels/ModerationPanel.svelte');
const desktopAltRs = read('../../src-tauri/src/commands/desktop_alt.rs');
const mainRs = read('../../src-tauri/src/main.rs');

describe('desktop-alt Moderation admin gate', () => {
  it('DesktopApp resolves the Moderation nav gate via the admin command, not the GA gate', () => {
    expect(desktopApp).toMatch(/invoke<boolean>\('desktop_alt_is_admin'\)/);
    expect(desktopApp).not.toMatch(/invoke<boolean>\('desktop_alt_enabled'\)/);
  });

  it('ModerationPanel self-gates via the admin command, not the GA gate', () => {
    expect(moderationPanel).toMatch(/invoke<boolean>\('desktop_alt_is_admin'\)/);
    expect(moderationPanel).not.toMatch(/invoke<boolean>\('desktop_alt_enabled'\)/);
  });

  it('the admin command maps to the @getindigo.ai gate, distinct from the GA gate', () => {
    // desktop_alt_is_admin → is_indigo_user (@getindigo.ai)
    expect(desktopAltRs).toMatch(
      /fn desktop_alt_is_admin\(\)[\s\S]*?feature_gate::is_indigo_user\(\)/,
    );
    // desktop_alt_enabled stays the GA gate (the two must not be swapped)
    expect(desktopAltRs).toMatch(
      /fn desktop_alt_enabled\(\)[\s\S]*?feature_gate::desktop_features_enabled\(\)/,
    );
  });

  it('registers the admin command in the Tauri invoke handler', () => {
    expect(mainRs).toMatch(/commands::desktop_alt::desktop_alt_is_admin/);
  });
});
