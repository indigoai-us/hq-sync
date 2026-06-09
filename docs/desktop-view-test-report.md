# Desktop View — Smoke / Stress Test Report

Date: 2026-06-08 · Tester: Claude (driven session) · Build: dev (`npm run tauri dev`, v0.7.1 source)
Signed in as: corey@getindigo.ai (admin) · Screens driven via keyboard hotkeys + native `screencapture`.

## Summary

The expanded desktop view is in good shape. All primary screens render and navigate correctly. One
real **app bug was found and fixed** (Moderation visible to non-admins). The other notable issues are
**upstream data/API problems** (a corrupted HQ-core registry, a cross-tenant deployments API) that the
desktop app surfaces but does not cause — it handles the bad data gracefully.

Baseline test suites green: **407 unit + 100 desktop-alt e2e** (incl. 4 new gate tests).

## Screens verified (render + navigation)

| Screen | Hotkey | Result | Notes |
|---|---|---|---|
| Sync | ⌘1 | ✅ PASS | 23 sources "OK / Up to date", recent-activity feed, "watching 12 workspaces". Titlebar fixes confirmed live (drag, no traffic-light shadows, no "HQ" title overlap). |
| Meetings | ⌘2 | ✅ PASS | Correct empty states ("No calendars connected yet"). Minor: red "Could not refresh meetings" banner shown alongside the no-calendars empty state is redundant. |
| Skills | ⌘3 | ✅ PASS | "206 skills available", scope filter + search, clean card grid. Library-split feature works. |
| Workers | ⌘4 | ⚠️ EMPTY | "0 workers available" — **not an app bug**; caused by a corrupted `core/workers/registry.yaml` (see F2). App degrades gracefully. |
| Marketplace | ⌘5 | ✅ PASS | 3 listings (hq-pack-review / -quality-gate / -tdd by @indigo), search box. (Initial capture caught the loading skeleton — it resolves on load.) |
| Profile | ⌘6 | ✅ PASS | Handle @corey, bio, avatar upload, social links, tip link, Save + preview. (Also just slow on first paint.) |
| Moderation | (admin) | ✅ PASS | Renders for admin: Packs/Requests, review queue, Yank-listing form. Gating fixed (F1). |
| Company → Board | — | ✅ PASS | Indigo: 4 objectives "On Track", 6 in-flight projects, goals + KR cards. |
| Company → Activity/Deployments/Secrets/Library | — | ◑ PARTIAL | Tab-click coordinates were unreliable in this driven session; Deployments scoping confirmed via logs (F3); Secrets metadata-only guarded by `secrets-never-leak.spec.ts`. Recommend a focused re-run. |

## External links — reachability (Phase 4)

All link targets return HTTP 200 (no dead/404 links):

| Link | URL | Status |
|---|---|---|
| Company console | https://hq.getindigo.ai/indigo | 200 |
| Company invite | https://hq.getindigo.ai/indigo/invite | 200 |
| Meetings integrations | https://hq.getindigo.ai/integrations | 200 |
| Open calendar | https://calendar.google.com | 200 |
| Indigo site | https://getindigo.ai | 200 |

(Reachability only — full logged-in visual confirmation of each landing page is a recommended follow-up.)

## Findings

### F1 — Moderation visible to all signed-in users (FIXED ✅, in this repo)
- **Severity:** High (UX leak of an admin surface; no data leak — server 403 protects).
- **Root cause:** `DesktopApp.svelte` and `ModerationPanel.svelte` gated the Moderation nav row + panel on
  `desktop_alt_enabled`, which is the **GA gate** (`desktop_features_enabled` → any signed-in user), not the
  `@getindigo.ai` admin gate. `is_indigo_user()` existed but no command exposed it.
- **Fix:** Added `desktop_alt_is_admin` command (→ `feature_gate::is_indigo_user`), registered it, and pointed
  both UX gates at it. Regression test: `e2e/desktop-alt/moderation-admin-gate.spec.ts`.

### F2 — Workers library shows 0 (UPSTREAM — HQ-core generator)
- **Severity:** Medium (broken Workers tab; also breaks worker routing across the whole HQ system).
- **Root cause:** `core/workers/registry.yaml` in the HQ tree is corrupted — 25 of 97 entries have garbage
  `id:` values (unescaped sentence fragments with quotes/colons) and empty `path:` fields. The invalid YAML
  breaks the whole-document parse, so the desktop app (correctly, leniently) yields 0 workers. Generator:
  `core/scripts/generate-workers-registry.sh`.
- **Action:** Flagged as a separate HQ-core task (not a hq-sync fix). The app's graceful degradation is correct.

### F3 — Deployments not company-scoped / cross-tenant bleed (UPSTREAM — hq-deploy API)
- **Severity:** High (multi-tenant isolation: every company's Deployments tab shows all ~309 deployments).
- **Evidence:** `GET https://api.indigo-hq.com/api/apps` with `x-org-slug=amass|moonflow|personal|indigo|hpo`
  all return identical 110831-byte payloads / 309 apps. The client filter
  `deployment_matches_selected_slug` (desktop_alt.rs:1056) does `.unwrap_or(true)`, so rows with no `orgSlug`
  are included for every company — but the primary cause is the API ignoring `x-org-slug` (no server-side scoping).
- **Action:** Flagged as a separate hq-deploy task. A defensive hq-sync change (don't include un-attributed rows)
  is possible but risks zeroing the panel if the API never returns `orgSlug`; server-side scoping is the real fix.

### F4 — Malformed PRD skipped (minor, handled)
- `companies/indigo/projects/hq-creator-marketplace/prd.json` is unparseable (`invalid type: integer 1, expected
  a string at line 20 col 19`). The Projects reader logs and skips it gracefully. Worth correcting the file.

### F5 — Meetings refresh banner redundant (minor, cosmetic)
- "Could not refresh meetings — showing the last cached view" appears together with the "No calendars connected
  yet" empty state. When the real condition is "no calendars connected," the error banner reads as a false failure.

## Not completed this pass (recommended follow-ups)
- **Phase 3 full mutations** (marketplace publish→install→moderate→yank, project status / story-passes writes,
  meeting-bot invite/cancel/record) — deferred: interactive WKWebView clicks were unreliable in this session.
- **Company Activity/Deployments/Secrets/Library** visual confirmation — re-run with reliable tab targeting.
- **Logged-in link landing pages** — confirm each 200 actually renders the right page (not a login loop).

## Environment notes
- Installed app was moved out of `/Applications` during testing (LaunchServices kept relaunching it over the dev
  build) and restored afterward. Current installed version observed: v0.7.8.
