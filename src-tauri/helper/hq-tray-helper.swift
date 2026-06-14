// HQ Sync menu-bar helper — a TINY standalone native AppKit process whose only
// job is to show the "HQ" menu-bar status item and relay clicks to the main app.
//
// WHY A SEPARATE PROCESS: on macOS Tahoe the main app's Tauri/tao runtime parks
// any NSStatusItem off-screen (verified on-device across every app version and
// both the tao tray and a native item — while a clean AppKit process places its
// item correctly at a normal slot). This helper is that clean AppKit process.
//
// IPC is deliberately trivial + robust: the helper writes a one-word command to
// ~/.hq/.tray-cmd and the main app polls it (no sockets, no signals, no
// entitlements). The helper exits itself when the main app's PID (argv[1]) dies.
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

final class TrayController: NSObject {
    let item = NSStatusBar.system.statusItem(withLength: NSStatusItem.variableLength)

    override init() {
        super.init()
        item.button?.title = "HQ"
        item.button?.toolTip = "HQ Sync"
        let menu = NSMenu()
        let open = NSMenuItem(title: "Open HQ Sync", action: #selector(openHQ), keyEquivalent: "")
        open.target = self
        menu.addItem(open)
        let sync = NSMenuItem(title: "Sync Now", action: #selector(syncNow), keyEquivalent: "")
        sync.target = self
        menu.addItem(sync)
        menu.addItem(.separator())
        let quit = NSMenuItem(title: "Quit HQ Sync", action: #selector(quitHQ), keyEquivalent: "q")
        quit.target = self
        menu.addItem(quit)
        item.menu = menu
    }

    @objc func openHQ() { writeCommand("show") }
    @objc func syncNow() { writeCommand("sync") }
    @objc func quitHQ() {
        writeCommand("quit")
        // Give the main app a beat to read the command, then exit the helper.
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
