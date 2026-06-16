import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { describe, expect, it } from 'vitest';

// Regression: the custom "update available" banner (commands/banner.rs,
// shown while the popover is closed) re-emits `notification:banner-action`,
// which App.svelte routes by `kind`. The "Update now" chip (action === 'update')
// used to call a bare `invoke('install_update')`: the popover never opened and
// `updateInstalling` never flipped, so from the user's seat "nothing happened"
// — the download ran invisibly until the app abruptly restarted.
//
// The fix routes that chip through the SAME guarded path the in-app Install
// button uses: reveal the popover (`show_main_window`) so the user lands on the
// in-app update banner, then `handleInstallUpdate()` (which dedupes, flips
// `updateInstalling` → "Installing…", and resets on error).
const app = readFileSync(resolve(process.cwd(), 'src/App.svelte'), 'utf8');
const norm = (s: string): string => s.replace(/\s+/g, ' ');

describe('update banner "Update now" chip opens the popover and starts a guarded install', () => {
  const a = norm(app);

  it('routes kind=update/action=update to show_main_window THEN handleInstallUpdate (not a bare install_update)', () => {
    // The two calls must be adjacent and in this order: reveal first, then
    // install via the guarded helper so the popover shows "Installing…".
    expect(a).toContain("await invoke('show_main_window'); await handleInstallUpdate();");
  });

  it('keeps the body-click (action=open) revealing the popover without forcing an install', () => {
    expect(a).toContain("else if (action === 'open') { await invoke('show_main_window'); }");
  });

  it('handleInstallUpdate is the guarded path that flips the banner to Installing…', () => {
    // Dedupe guard + the reactive flag the in-app banner reads to render
    // "Installing…" instead of "Install".
    expect(a).toContain('async function handleInstallUpdate() { if (updateInstalling) return; updateInstalling = true;');
  });
});
