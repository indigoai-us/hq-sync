import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { describe, expect, it } from 'vitest';

// The macOS menu-bar "HQ" item is provided by a SEPARATE native AppKit helper
// process (src-tauri/helper/hq-tray-helper.swift), spawned by the main app.
//
// WHY: on macOS Tahoe, Tauri's tao runtime parks any in-process NSStatusItem
// off-screen — verified on-device across v0.7.25, v0.7.33, beta.14, and a
// hand-written in-process native item, all at x=1693 (off the visible strip),
// while a bare AppKit process places its item at x≈1237. The helper is that
// clean process. Source-contract assertions lock the whole wiring chain
// (compile → bundle → SIGN → spawn → poll) so a dropped link — most dangerously
// the signing phase, whose absence fails notarization / yields no icon — fails
// fast in CI instead of shipping a release with no menu-bar icon again.

const read = (p: string) => readFileSync(resolve(process.cwd(), p), 'utf8');

describe('macOS menu-bar helper process (HQ status item)', () => {
  it('ships a native Swift helper that shows the "HQ" item + relays clicks', () => {
    const swift = read('src-tauri/helper/hq-tray-helper.swift');
    expect(swift).toContain('NSStatusBar.system.statusItem');
    expect(swift).toContain('"HQ"');
    // Relays actions to the main app via the command file it polls.
    expect(swift).toContain('.hq/.tray-cmd');
    expect(swift).toContain('writeCommand("show")');
    expect(swift).toContain('writeCommand("quit")');
    // Self-exits when the main app (argv[1] PID) dies — no orphan icon.
    expect(swift).toContain('kill(hqPid, 0)');
  });

  it('build.rs compiles the helper on macOS and fails loud if swiftc breaks', () => {
    const build = read('src-tauri/build.rs');
    expect(build).toContain('CARGO_CFG_TARGET_OS');
    expect(build).toContain('helper/hq-tray-helper.swift');
    expect(build).toContain('swiftc');
    // assert!(status.success(...)) — a silent drop would ship no icon.
    expect(build).toMatch(/assert!\(\s*status\.success\(\)/);
  });

  it('bundles the compiled helper into Contents/Resources', () => {
    const conf = read('src-tauri/tauri.conf.json');
    expect(conf).toContain('"helper/hq-tray-helper": "hq-tray-helper"');
  });

  it('SIGNS the helper before sealing the bundle (notarization-critical)', () => {
    const sign = read('scripts/sign-bundle.sh');
    expect(sign).toContain('Contents/Resources/hq-tray-helper');
    // Signed in Phase 5b — before Phase 8 seals the .app.
    expect(sign).toMatch(/Phase 5b[\s\S]*sign_file "\$APP\/Contents\/Resources\/hq-tray-helper"/);
  });

  it('main.rs spawns the helper on macOS and skips the off-screen tao tray', () => {
    const main = read('src-tauri/src/main.rs');
    expect(main).toContain('tray_helper::spawn_and_poll');
    const tray = read('src-tauri/src/tray.rs');
    // On macOS the tao tray is NOT built (it lands off-screen); only non-macOS
    // builds the tao TrayIcon.
    expect(tray).toContain('#[cfg(not(target_os = "macos"))]');
    expect(tray).toContain('menu-bar item provided by native helper');
  });

  it('tray_helper polls the command file and dispatches show/sync/quit', () => {
    const helper = read('src-tauri/src/tray_helper.rs');
    expect(helper).toContain('.tray-cmd');
    expect(helper).toContain('"show" => show_popover');
    expect(helper).toContain('"quit" => app.exit(0)');
    // The popover is repositioned on-screen (the off-screen tao rect dragged it
    // off the right edge) and the spurious auto-hide is suppressed.
    expect(helper).toContain('suppress_blur_hide_briefly');
    expect(helper).toContain('set_position');
  });
});
