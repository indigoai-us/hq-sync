---
id: hq-sync-verify-ui-fixes-in-browser-preview-harness
title: Verify hq-sync UI fixes in the browser preview harness
scope: repo
trigger: making or verifying a UI/UX change to the hq-sync Tauri menubar popover/footer
enforcement: soft
public: false
version: 1
created: 2026-05-29
updated: 2026-05-29
source: session-learning
---

## Rule

ALWAYS: verify hq-sync (Tauri menubar) UI fixes in the browser preview harness. Run `npm run dev:preview` (port 1422) — it serves `/dev-harness/index.html?view=popover&theme=dark` with mocked Tauri APIs. Edit `dev-harness/fixtures.ts` to reproduce a specific footer/popover state, then screenshot at 320x440.

The real app is served at `/`; the harness only at `/dev-harness/`.

## Rationale

The preview harness mocks the Tauri APIs so the Svelte popover can be driven and screenshotted in a plain browser without launching the full menubar app — a far faster feedback loop for visual fixes. Driving a specific footer/popover state requires editing `dev-harness/fixtures.ts` to inject the fixture, then screenshotting at the popover's native 320x440 dimensions. Captured after a session that iterated on a footer/popover state through this harness.
