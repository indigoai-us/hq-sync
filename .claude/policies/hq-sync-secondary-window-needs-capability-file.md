---
id: hq-sync-secondary-window-needs-capability-file
title: Every secondary Tauri window needs its own capability file
scope: repo
trigger: Adding a new secondary window in hq-sync (any `WebviewWindowBuilder::new(app, "<label>", ...)` call, or any window opened from a `#[tauri::command]` other than `main`)
enforcement: hard
version: 2
created: 2026-05-21
updated: 2026-05-29
public: false
source: user-correction
learned_from: session/2026-05-21-hq-sync-v0185
---

## Rule

ALWAYS create a capability file at `src-tauri/capabilities/{window-label}.json` for every secondary Tauri 2 window before shipping it. The file MUST include at minimum:

```json
{
  "identifier": "{window-label}-capability",
  "windows": ["{window-label}"],
  "permissions": ["core:default", "core:event:default"]
}
```

The `windows` array must contain the exact label passed to `WebviewWindowBuilder::new(app, "<label>", ...)`. Without this file, every `invoke()` call from inside that window silently fails at the Tauri capability check — no error is surfaced to the webview, no log is written, the Rust command simply never runs.

Existing canonical examples in `src-tauri/capabilities/`:

- `default.json` — main window
- `new-files-detail.json` — new-files secondary window
- `meetings-window.json` — meetings secondary window
- `drift-detail.json` — drift secondary window
- `share-detail.json` — share secondary window
- `dm-detail.json` — DM secondary window

## Rationale

In Tauri 2, ACL capabilities default-deny. The main window inherits `default.json` only because that file lists `"windows": ["main"]`. Any other window label has zero permissions until a capability file explicitly grants them.

The failure mode is silent and indistinguishable from a bug elsewhere in the stack: the secondary window's `detail_window_ready` (or equivalent handshake) command never reaches the Rust handler, so the parent state thinks the window is still mounting. The window stays stuck on its loading view forever. There is no panic, no console error, no Sentry breadcrumb — Tauri silently drops the invoke.

This cost a multi-hour Rust-side log-instrumentation round-trip during v0.1.85 (drift-detail window). The capability file MUST be added in the same commit as the `WebviewWindowBuilder` call, not as a follow-up.

**Recurred on 0.2.0-beta.3 (dm-detail window):** "Open details" on a rich DM notification did nothing — the `dm-detail` window shipped without its capability file. Tell-tale that confirms this failure mode: an action that runs in the **main** window works (DM "Copy prompt" → `navigator.clipboard` in the main webview) while the sibling action that opens/drives the **new** window fails. The recurrence happened in a cross-repo session that never loaded this hq-sync repo policy — so when touching hq-sync windows, load `repos/public/hq-sync/.claude/policies/` even mid-task in a multi-repo session.

## How to comply

When adding a new secondary window:

1. Pick the window label (e.g. `drift-detail`).
2. Create `src-tauri/capabilities/{label}.json` with `core:default` + `core:event:default` minimum.
3. Add any additional permissions the window actually needs (e.g. `core:webview:allow-set-webview-focus`, plugin permissions for `dialog`, `fs`, etc.).
4. Verify the window's `invoke()` calls reach Rust by running the dev build and confirming a tracing log fires on the first command.

## Exceptions

None. Even short-lived debug windows need a capability file — silent failure is too costly to skip.
