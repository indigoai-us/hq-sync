---
id: hq-sync-tray-icon-source-hq-svg
title: Use the official HQ SVG mark for tray icons
scope: repo
trigger: HQ Sync menu bar/tray icon work
when: tray || icon || menubar
on: [PreToolUse, PostToolUse, UserPromptSubmit, AssistantIntent]
enforcement: soft
version: 1
created: 2026-06-13
updated: 2026-06-13
public: false
source: session-learning
---

## Rule

For HQ Sync menu bar/tray icon work, use the official HQ SVG mark from src-tauri/icons/source/HQ.svg as the source of truth; do not replace it with hand-drawn text or placeholder glyphs.

## Rationale

Tray and menu bar icons are high-visibility brand surfaces. Keeping generated icon assets anchored to the canonical SVG prevents regressions where a placeholder or hand-drawn approximation ships instead of the HQ mark.
