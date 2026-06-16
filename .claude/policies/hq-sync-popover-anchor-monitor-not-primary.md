---
id: hq-sync-popover-anchor-monitor-not-primary
title: Anchor the macOS popover on the monitor the menu-bar icon was clicked, never primary_monitor()
scope: repo
trigger: positioning the menu-bar popover on macOS (show_popover_window / tray anchor / multi-monitor placement)
when: show_popover || primary_monitor || available_monitors || tray_anchor || set_position || popover || monitor || multi-monitor || menu bar
on: [PreToolUse, PostToolUse, UserPromptSubmit, AssistantIntent]
enforcement: hard
public: false
version: 1
created: 2026-06-15
updated: 2026-06-15
source: user-correction
---

## Rule

When positioning the menu-bar popover on macOS, place it on the monitor whose
on-screen span CONTAINS the icon's reported anchor — NEVER on
`window.primary_monitor()`. macOS gives every display its own menu bar, and the
native helper reports the icon's horizontal centre in Cocoa screen POINTS that
span ALL displays, so a click on a secondary monitor reports an anchor outside
the primary's range. Forcing `primary_monitor()` and clamping the popover's X to
its width drags the window back onto display 1 — the popover stops following the
click.

ALWAYS:

- Enumerate `available_monitors()` and pick the one whose horizontal span
  contains the anchor. Derive each monitor's points span from its OWN scale
  (`work_x / scale … (work_x + work_w) / scale`) so mixed-DPI rigs map correctly
  instead of assuming one global points↔px factor.
- Position within THAT monitor's `work_area()`: top edge at `work_y` (just below
  its own menu bar), X centred under the anchor and clamped to
  `[work_x, work_x + work_w - win_w]`.
- Fall back to the primary monitor's corner ONLY when no monitor's span contains
  the anchor (anchor never reported, or stale and off every display).

Keep the placement math in a pure function (`position_popover_under_anchor`)
with unit tests over synthetic multi-display layouts — there is no live AppKit
display in `cargo test`, so a runtime-only fix ships unverified.

## Rationale

Real bug: with two monitors, clicking the "HQ" menu-bar icon on the second
display always opened the popover on the first. `show_popover_window` used
`window.primary_monitor()` and clamped the popover X to the primary's width, so
the global anchor X — already correct, reported by the Swift helper in
`hq-tray-helper.swift` as `item.button?.window?.frame.midX` — was discarded. The
old code comment even asserted "the menu bar lives on the primary monitor," which
is false on macOS: with "Displays have separate Spaces" (the default) every
display carries its own menu bar. Fix: select the monitor containing the anchor
and position within its work area. Companion to
`hq-sync-window-ops-from-bg-thread-deadlock` — both are macOS menu-bar popover
correctness rules living in `src-tauri/src/tray.rs`.
