// HQ Sync menu-bar helper — a TINY standalone native AppKit process whose only
// job is to show the "HQ" menu-bar status item and relay clicks to the main app.
//
// WHY A SEPARATE PROCESS: on macOS Tahoe the main app's Tauri/tao runtime parks
// any NSStatusItem off-screen (verified on-device across every app version and
// both the tao tray and a native in-process item — while a clean AppKit process
// places its item correctly at a normal slot). This helper is that clean
// AppKit process.
//
// IPC is deliberately trivial + robust: the helper writes a one-word command to
// ~/.hq/.tray-cmd and the main app polls it (no sockets, no signals, no
// entitlements). The helper exits itself when the main app's PID (argv[1]) dies.
//
// Interaction matches a normal menu-bar app:
//   • LEFT-click  → open the popover (write "show" + activate the main app so
//                   the popover reliably comes to the front — without activation
//                   a background-launched app shows the window behind everything
//                   / lets the click-away handler swallow it).
//   • RIGHT-click → context menu (Sync Now / Quit).
//
// Build: swiftc -O hq-tray-helper.swift -o hq-tray-helper

import Cocoa
import Foundation

let hqPid: Int32 = CommandLine.arguments.count > 1 ? (Int32(CommandLine.arguments[1]) ?? 0) : 0
let cmdURL = FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent(".hq/.tray-cmd")

func writeCommand(_ s: String) {
    try? FileManager.default.createDirectory(
        at: cmdURL.deletingLastPathComponent(), withIntermediateDirectories: true)
    try? s.write(to: cmdURL, atomically: true, encoding: .utf8)
}

// Bring the main HQ app to the foreground so the popover it shows becomes the
// key window. The main app is a background "accessory" app; without this its
// freshly-shown popover renders behind other windows and the click-away handler
// hides it. Cross-process activation works because the helper is a normal app.
func activateHQ() {
    guard hqPid > 0, let app = NSRunningApplication(processIdentifier: hqPid) else { return }
    app.activate(options: [.activateIgnoringOtherApps])
}

final class TrayController: NSObject {
    let item = NSStatusBar.system.statusItem(withLength: NSStatusItem.variableLength)
    let menu = NSMenu()

    override init() {
        super.init()
        item.button?.title = "HQ"
        item.button?.toolTip = "HQ Sync"

        // Right-click context menu (NOT set as item.menu — that would make a
        // plain left-click open the menu instead of the popover).
        let sync = NSMenuItem(title: "Sync Now", action: #selector(syncNow), keyEquivalent: "")
        sync.target = self
        menu.addItem(sync)
        menu.addItem(.separator())
        let quit = NSMenuItem(title: "Quit HQ Sync", action: #selector(quitHQ), keyEquivalent: "q")
        quit.target = self
        menu.addItem(quit)

        // Handle the click ourselves so left vs. right can diverge.
        item.button?.target = self
        item.button?.action = #selector(statusItemClicked)
        item.button?.sendAction(on: [.leftMouseUp, .rightMouseUp])
    }

    @objc func statusItemClicked() {
        let event = NSApp.currentEvent
        let isRight =
            event?.type == .rightMouseUp || event?.modifierFlags.contains(.control) == true
        if isRight {
            // Show the context menu, then detach it so the next left-click again
            // triggers the action rather than re-opening the menu.
            item.menu = menu
            item.button?.performClick(nil)
            item.menu = nil
        } else {
            writeCommand("show")
            activateHQ()
        }
    }

    @objc func syncNow() { writeCommand("sync") }
    @objc func quitHQ() {
        writeCommand("quit")
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.3) { NSApp.terminate(nil) }
    }
}

let app = NSApplication.shared
app.setActivationPolicy(.accessory)
let controller = TrayController()

// Exit when the main HQ app dies so we never leave an orphan "HQ" in the bar.
if hqPid > 0 {
    Timer.scheduledTimer(withTimeInterval: 2.0, repeats: true) { _ in
        if kill(hqPid, 0) != 0 { NSApp.terminate(nil) }
    }
}

app.run()
