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
    // Relays actions to the main app via the command file it polls. The "show"
    // command carries the icon's on-screen x so the popover anchors under it.
    expect(swift).toContain('.hq/.tray-cmd');
    expect(swift).toContain('writeCommand("show ');
    expect(swift).toContain('writeCommand("quit")');
    // Self-exits when the main app (argv[1] PID) dies — no orphan icon.
    expect(swift).toContain('kill(hqPid, 0)');
  });

  it('opens the popover on LEFT-click (menu on right-click) and activates the app', () => {
    const swift = read('src-tauri/helper/hq-tray-helper.swift');
    // Left-click handled via the button action (NOT item.menu, which would make
    // a plain click open the menu instead of the popover).
    expect(swift).toContain('item.button?.action = #selector(statusItemClicked)');
    expect(swift).toContain('sendAction(on: [.leftMouseUp, .rightMouseUp])');
    expect(swift).toContain('.rightMouseUp');
    // Left-click activates the main app so the background-launched popover
    // reliably comes to the front (the bug the user hit: click did nothing).
    expect(swift).toContain('func activateHQ()');
    expect(swift).toContain('NSRunningApplication(processIdentifier: hqPid)');
    expect(swift).toContain('activateHQ()');
  });

  it('reports the icon position so the popover anchors under it (not top-right)', () => {
    const swift = read('src-tauri/helper/hq-tray-helper.swift');
    // The helper reads the status button window's centre and ships it with "show".
    expect(swift).toContain('frame.midX');
    const helper = read('src-tauri/src/tray_helper.rs');
    // The poller parses "show <x>" and records the anchor before toggling.
    expect(helper).toContain('strip_prefix("show")');
    expect(helper).toContain('set_tray_anchor_x');
    const tray = read('src-tauri/src/tray.rs');
    // The positioner uses the anchor (centre − half width) to place the popover
    // on the monitor the icon was clicked on, and only falls back to the primary
    // corner when the icon position is unknown / off every display.
    expect(tray).toContain('pub fn set_tray_anchor_x');
    expect(tray).toContain('tray_anchor_x_points()');
    // Picks the monitor whose span contains the anchor (multi-monitor fix) —
    // never hard-codes the primary display.
    expect(tray).toContain('position_popover_under_anchor');
    expect(tray).toMatch(/available_monitors\(\)/);
    expect(tray).toMatch(/center_px - win_w \/ 2/);
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
    // Menu-bar click toggles the popover (show if hidden, hide if up) via the
    // shared window-management helper in tray.rs — the "show" command carries
    // the icon anchor, so it's matched by prefix.
    expect(helper).toContain('strip_prefix("show")');
    expect(helper).toContain('toggle_popover_window');
    expect(helper).toContain('"quit" => app.exit(0)');
  });

  it('popover toggle shows on-screen, suppresses auto-hide, and is single-window', () => {
    const tray = read('src-tauri/src/tray.rs');
    // The popover is repositioned on-screen (the off-screen tao rect dragged it
    // off the right edge) and the spurious auto-hide is suppressed.
    expect(tray).toContain('pub fn show_popover_window');
    expect(tray).toContain('suppress_blur_hide_briefly');
    expect(tray).toContain('set_position');
    // Toggle: hide if already visible, else show.
    expect(tray).toContain('pub fn toggle_popover_window');
    expect(tray).toMatch(/is_visible\(\)\.unwrap_or\(false\)[\s\S]*?\.hide\(\)/);
    // Single-window: showing the popover hides the desktop window.
    expect(tray).toContain('pub fn hide_desktop_alt');
    expect(tray).toContain('get_webview_window("desktop-alt")');
  });

  it('the two global shortcuts toggle their window (single-window enforced)', () => {
    const main = read('src-tauri/src/main.rs');
    // Both shortcuts marshal their window ops onto the main thread (the
    // is_visible toggle query deadlocks AppKit off-main) and toggle.
    expect(main).toContain('run_on_main_thread');
    expect(main).toContain('tray::toggle_popover_window(&app_main)');
    // Opt+Shift+O toggles the desktop window (hide if visible, else open).
    expect(main).toContain('tray::hide_desktop_alt(&app_main)');
    expect(main).toMatch(/desktop_visible[\s\S]*?open_desktop_alt_window_inner/);
    // Opening the desktop hides the popover (single-window) at the canonical path.
    const desktop = read('src-tauri/src/commands/desktop_alt.rs');
    expect(desktop).toMatch(/get_webview_window\("main"\)[\s\S]*?\.hide\(\)/);
  });

  it('marshals the menu-bar click toggle onto the main thread (no poll-thread deadlock)', () => {
    const helper = read('src-tauri/src/tray_helper.rs');
    // The poll thread must NOT call window ops directly — it marshals them.
    expect(helper).toMatch(/run_on_main_thread\([\s\S]*?toggle_popover_window/);
  });
});
