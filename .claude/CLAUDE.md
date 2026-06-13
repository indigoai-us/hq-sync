# HQ Sync Menubar

macOS menu bar app wrapping `hq sync` for non-technical users. Tauri 2 + Svelte 5 + vanilla CSS.

## Architecture

**Frontend:** Svelte 5 with runes (`$state`, `$effect`). No component library — vanilla CSS in `src/styles/popover.css` owns the canonical Liquid Glass tokens for the classic popover, and `src/desktop-alt/styles/desktop-alt.css` imports those tokens for both the V4 desktop window and standalone Messages window. Views: `SignInPrompt` (OAuth), `Popover` (main sync UI with per-workspace rows, real per-file progress bar, **Stop** button mid-sync, Connect diagnostics drawer, and a footer HQ-version row whose action pills — drift count + Restore vX.Y.Z + update-done chip — wrap to a second line below the version label so they never overlap it, fixed v0.2.2), `Settings` (preferences + folder re-tether), and the desktop-alt shell under `src/desktop-alt/` (Home, Companies, Messages, Meetings, Library, Settings, safety flows, V4 sidebars, status bar, command palette). Conflict resolution via `ConflictModal` + `ConflictRow` components. First-launch onboarding via `FirstRunWelcome` (3-slide welcome carousel shown once on a brand-new install's first run) and `AutoSyncNotice` (one-time in-app card shown to updating users explaining auto-sync is on + how to switch to manual) — both orchestrated by `App.svelte` off the `first_run` backend classification. New-file notification via `NewFilesBadge` (in-popover count) + `NewFilesDetail` (secondary window with file list and attribution). Recent Changes via `ActivityLog` (secondary window listing the session's per-file syncs, each attributed as "{author} added/updated" — author email + verb — falling back to the company slug when no author). DM notifications open `DmDetail` (secondary window showing the full two-way **conversation thread** — received vs. sent bubbles, scrollable, with the live message at the bottom — plus a reply composer: textarea + **Send** / ⌘↵; sent replies append optimistically); share notifications open `ShareDetail`.

**Backend:** Tauri 2 Rust commands in `src-tauri/src/commands/`. ~110 registered commands in `main.rs`.

**State flow:** Svelte frontend calls Tauri commands via `invoke()`. Rust backend emits typed events (`sync:progress`, `sync:conflict`, `sync:error`, `sync:complete`, `sync:new-files`) that Svelte listens to via `listen()`.

## Key Modules

| Module | Purpose |
|--------|---------|
| `commands/sync.rs` | Spawns `hq sync --json`, streams ndjson events, 10-min timeout. Includes a "Preparing sync…" pre-pass that walks the tree to compute real `filesTotal` before transfers start (so progress isn't fake) |
| `commands/first_run.rs` | First-run / first-update onboarding classification + persisted flags. `classify_launch` runs at the very top of `main.rs` `.setup()` — BEFORE `config::ensure_machine_id` writes `machineId` — and caches a `LaunchKind` (`FirstRun` / `ExistingUpdate` / `Normal`) in managed state. Tiebreaker between a brand-new install and a legacy update is the *pre-write* `machineId` (existing users have it, fresh installs don't; `firstRunCompleted` alone can't tell them apart). Exposes `is_first_run`, `should_show_auto_sync_notice` (gated to `ExistingUpdate` + notice-not-shown + `realtimeSync` still on — never overrides an explicit opt-out), `mark_first_run_complete` (writes `firstRunCompleted` + `autoSyncNoticeShown` + `realtimeSync`/`personalSyncEnabled` true), `mark_auto_sync_notice_shown` (writes `autoSyncNoticeShown` + `firstRunCompleted`, never touches `realtimeSync`). All writes use the same untyped-merge + atomic-rename algorithm as `config::ensure_machine_id` so unknown/future top-level keys survive |
| `commands/auth.rs` | Cognito token state + silent refresh |
| `commands/cognito.rs` | Cognito client wrapper (refresh, sign-out, hosted-UI URL builder) |
| `commands/oauth.rs` | PKCE OAuth flow on loopback port 53682 |
| `commands/config.rs` | Reads `~/.hq/config.json` + `~/.hq/menubar.json` |
| `commands/status.rs` | Live status surface for the popover (last sync, current state, error count) |
| `commands/workspaces.rs` | Manifest-driven workspace list — reads `companies/manifest.yaml`, unions with cloud memberships, exposes per-row Connect state |
| `commands/folder_picker.rs` | Native folder picker for the Settings re-tether flow |
| `commands/personal.rs` | Auto-provisions the `personal` company row + bucket on first sync if missing |
| `commands/provision.rs` | Auto-provisions the user's `person` entity in HQ-Cloud on first sync (UJ-006) |
| `commands/first_push.rs` | First-push protection — companies that have never synced are pre-walked and validated against ignore rules before any upload |
| `commands/prewarm.rs` | Warms the vault client + manifest cache on app launch so the first popover open is <100ms |
| `commands/vault_client.rs` | HTTPS client to hq-ops `vault` endpoints (signed S3 URLs, telemetry opt-in, person provisioning) |
| `commands/telemetry.rs` | Per-sync telemetry collector — scans the HQ tree, diffs against `~/.hq/telemetry-cursor.json`, POSTs to `/v1/usage` (gated on `telemetryEnabled` in menubar.json + server-side opt-in) |
| `commands/daemon.rs` | Feature-flagged V2 daemon lifecycle (`autostartDaemon` in menubar.json) |
| `commands/process.rs` | Generic subprocess lifecycle with SIGTERM->SIGKILL |
| `commands/conflicts.rs` | Conflict resolution + open-in-editor |
| `commands/new_files.rs` | New files detail window — creates/focuses a secondary Tauri window showing file list with attribution. Uses managed `PendingNewFiles` state + ready handshake pattern |
| `commands/activity.rs` | Session activity log (Recent Changes window) — in-memory append-only `Vec<ActivityEntry>` in managed state, one entry per `progress` event. Each entry carries `direction`, `author`, and `is_new`. `record_new_files()` reconciles the per-company `new-files` event onto the matching download rows (flips `is_new` so the UI renders "added" vs "updated", back-fills `author` from `addedBy` where the progress event had none). Renders as "{author} added/updated", falling back to the company slug when no author |
| `commands/share_notify.rs` | "Shared with me" notification client — polls `GET /v1/files/shared-with-me` on an **independent interval timer** (`setup_share_notify_poller`, one timer that also drives DM polling), surfaces a native banner, opens `ShareDetail`. Decoupled from sync events on purpose (a stalled sync must not stop notifications) |
| `commands/dm_notify.rs` | User-to-user DM notification client, layered on the same poll timer as `share_notify`. Polls `GET /v1/notify/inbox`, surfaces a banner; **every DM is clickable** — a body-click maps to the `"open"` action (opens `DmDetail`), while `"copy"` (write the agent prompt to the clipboard) stays an explicit action button for DMs that carry a `prompt`. Outbound `send_dm` (`POST /v1/notify/dm` via the sender's `fromPersonUid` → recipient's `toPersonUid`) powers the `DmDetail` reply composer — the app is no longer receive-only. `fetch_dm_thread` (`GET /v1/notify/thread?withPersonUid=`) loads the full two-way conversation history (server-side pair-keyed mirror; messages `direction`-tagged in/out) so `DmDetail` renders a thread, not just the single triggering DM. Cursor `~/.hq/dm-cursor.json`; gated by `dmNotifications` in menubar.json (default on); log codes `DM_NOTIFY_*` in `~/.hq/logs/hq-sync.log`. All DM sends take the guarded blocking-send path (`BlockingNotifyGuard` caps blocking sends at ~1 core) |
| `commands/dm_mqtt.rs` | **Instant DM delivery** (GA in 0.3.0, all signed-in users). MQTT-over-WSS receiver: fetches scoped STS creds from `POST /v1/realtime/credentials`, SigV4-presigns the AWS IoT Core endpoint (sign WITHOUT the session token, append `X-Amz-Security-Token` after — IoT rejects a token in the signed query with 403), subscribes to its own `hq/{personUid}/dm`, and on any wake (and on connect/reconnect for offline catch-up) calls `dm_notify::poll_dm_once` — the MQTT message is only a wake signal, so dedupe/cursor/notification all reuse the poll path. Spawned from `main.rs` `.setup()`; capped exponential backoff; on any failure falls back silently to the 60s poll (no regression). Log codes `DM_MQTT_*`. |
| `commands/notifications.rs` | OS notification permission state + request (`notification_permission_state` / `notification_request_permission`) |
| `commands/desktop_alt.rs` | GA desktop window gate, open/focus command, and read-only company panel commands. Uses `feature_gate::desktop_features_enabled()` for UI eligibility and backend enforcement. Board/Activity call the vault API, Deployments calls hq-deploy with `x-org-slug`, and Secrets returns metadata-only `{key, upd, rot}` rows with no plaintext fields |
| `commands/settings.rs` | Settings persistence |
| `commands/autostart.rs` | Login-item autostart. `ensure_autostart_on_launch()` (called from `main.rs` `.setup()`, macOS-gated) idempotently reconciles the LaunchAgent plist with the effective `startAtLogin` pref on every launch — **default-on** (a fresh install autostarts without opening Settings), honouring an explicit `"startAtLogin": false` opt-out (stale plist removed). Mirrors the `daemon.rs` `realtime_sync` default-on convention |
| `tray.rs` | System tray with 4 visual states (idle/syncing/error/conflict) |
| `updater.rs` | Auto-update checker (10s delay, then every 6h). **Channel-aware**: resolves a per-user endpoint via `util/release_channel.rs` from `MenubarPrefs.release_channel` × `util/feature_gate::is_indigo_user`. Non-`@getindigo.ai` users are coerced to Stable regardless of stored preference (defense-in-depth). Exposes `available_channels` command for the Settings picker. |
| `events.rs` | Typed sync event structs (ndjson discriminated union) |
| `sentry_scrub.rs` | Sentry event scrubber — strips Cognito tokens and home-dir paths before send |
| `util/paths.rs` | HQ folder resolver (4-tier — see below). Also provides `resolve_bin` + `child_path` for finding `hq` and node-shebang interpreters under launchd's minimal PATH |
| `util/ignore.rs` | Sync ignore rules — excludes `settings/`, `data/`, `workers/`, `.git/`, etc. from cloud sync (privacy class) |
| `util/journal.rs` | Append-only sync journal at `~/.hq/sync-journal.log` (used by Connect diagnostics) |
| `util/logfile.rs` | Persistent diagnostic log for the sync pipeline at `~/.hq/sync-debug.log` (rotated at 10MB) |

## Desktop Alt UX (GA — public since 0.7.0)

The desktop-alt UX is a second Tauri window labeled `desktop-alt`, declared hidden in `src-tauri/tauri.conf.json` with `create: false` and opened only by `open_desktop_alt_window`. The classic menubar popover remains the default UI; all signed-in users get a header icon button with title `Open desktop view` (graduated from the Indigo dogfood to GA in 0.7.0).

Access is defense-in-depth:

1. `App.svelte` invokes `desktop_alt_enabled`.
2. `Popover.svelte` only renders `data-testid="desktop-alt-toggle"` when that result is true.
3. `open_desktop_alt_window` calls the same backend gate again and rejects signed-out callers — the window graduated to GA via `feature_gate::desktop_features_enabled`; pre-release update channels stay Indigo-only via `is_indigo_user`.

Frontend files live under `src/desktop-alt/`. `DesktopApp.svelte` owns the route, V4 chrome, command-K palette, status bar, sync event listeners, workspace loading, and meetings cache hydration. `route.ts` maps the sidebar rows and command-number hotkeys. The company work-system views read local goals/projects where possible; Activity, Deployments, and Secrets call `get_company_activity`, `get_company_deployments`, and `get_company_secrets`.

Important constraints:

- Any new desktop-alt invoke path must be allowed by `src-tauri/capabilities/desktop-alt.json`.
- Secrets are read-only metadata. Do not add `value`, `secret`, or reveal-mode fields to `SecretEnv` / `SecretItem`; `e2e/desktop-alt/secrets-never-leak.spec.ts` exists to lock this down.
- V4 implementation notes live in `docs/design/v4/IMPLEMENTATION-NOTES.md`; the visual/source-of-truth spec lives in `docs/design/v4/SPEC.md`.

## Config Files (User Machine)

| File | Written By | Purpose |
|------|-----------|---------|
| `~/.hq/config.json` | hq-installer | Company UID, slug, person, bucket, vault URL, HQ folder path |
| `~/.hq/menubar.json` | This app | HQ path override, syncOnLaunch, notifications, startAtLogin, autostartDaemon, realtimeSync, personalSyncEnabled, instantSync, driftStagingRepo, shareNotifications, releaseChannel, machineId, firstRunCompleted, autoSyncNoticeShown, cliAutoUpdate, cliUpdateDismissedVersion (the hq-CLI version the user dismissed the "update available" notice for — sticky until a newer version publishes) |
| `~/.hq/cognito-tokens.json` | hq-installer / this app | Cognito access + refresh + id tokens |

## HQ Folder Path Resolution

Priority order (in `util/paths.rs::resolve_hq_folder`):

1. **`menubar.json` -> `hqPath`** — user override via Settings, OR canonical path written by hq-installer ≥0.1.28 at end of install wizard
2. **`config.json` -> `hqFolderPath`** — legacy path from older hq-installer flows
3. **Discovery via `core.yaml` signature** — scans candidate locations (`~/HQ`, `~/hq`, `~/Documents/HQ`, `~/Documents/hq`, `~/Desktop/HQ`, `~/Desktop/hq`) for a folder containing a valid `core.yaml` with `version` + `hqVersion` fields. First match wins
4. **`~/HQ`** — hardcoded last-resort default

### Why core.yaml is the discovery signature

- It exists at the root of every hq-core install (locked file)
- It has a verifiable schema (`version: 1` + `hqVersion: "12.0.0"`), not just a presence check — random folders won't false-match
- It's not present anywhere else in an HQ tree (unlike `companies/manifest.yaml`, which exists in many sub-locations and would cause false matches deep in the tree)

### Why this exists

The installer wizard lets the user pick any folder for their HQ install. Prior to hq-installer v0.1.28, it didn't communicate that path to HQ Sync, so HQ Sync's old fallback was a hardcoded `~/HQ` — a user who picked anything else (or whose `~/HQ` got moved) saw "0 files synced" forever. The v0.1.28 paired release fixed this:

- **hq-installer v0.1.28** writes `hqPath` to `~/.hq/menubar.json` after extraction, restoring Priority 1 as the canonical path for new installs
- **hq-sync v0.1.28** added Priority 3 (discovery) as a safety net for installs that already happened under the old flow

Discovery is the safety net, not the primary mechanism — once a user runs the v0.1.28+ installer, Priority 1 is always populated.

## Workspaces & Connect Flow

The popover renders a row per workspace. Workspaces are computed in `commands/workspaces.rs` as the **union** of:

1. **Manifest companies** — every company present in `companies/manifest.yaml` on disk (always includes `personal`, even if not yet provisioned in HQ-Cloud)
2. **Cloud memberships** — companies the signed-in user belongs to according to hq-ops `/v1/users/me/memberships`

Each row carries a `connectState` (`connected | needs_connect | provisioning | error`) and exposes a per-row **Connect** button when the company exists in the manifest but has no S3 vault yet. Replaced the older "No companies yet" empty-state dead-end (v0.1.21) — there is now always at least one row (`personal`) to act on.

The `personal` row is special-cased: if it's missing from the manifest at sync time, `commands/personal.rs` auto-provisions the directory + bucket so first-time users always have a working sync target.

## First-Run Onboarding

Three launch kinds are classified once at `.setup()` (`commands/first_run.rs`) and drive `App.svelte`:

- **FirstRun** (brand-new install) — the popover pops open, the first cloud sync auto-starts, and `FirstRunWelcome` shows a 3-slide carousel. Dismiss → `mark_first_run_complete` persists `firstRunCompleted` + `autoSyncNoticeShown` (carousel users skip the separate notice) and makes "sync is on" explicit (`realtimeSync` + `personalSyncEnabled` true). Never repeats.
- **ExistingUpdate** (legacy user who updated to an onboarding-aware build) — `AutoSyncNotice` shows a one-time card explaining auto-sync is on and how to switch to manual. Notify-only: gated on `realtimeSync` still being on, so a user who explicitly opted out sees nothing. Dismiss → `mark_auto_sync_notice_shown` (does NOT touch `realtimeSync`).
- **Normal** — first-run sequence already completed; no onboarding UI.

Classification rationale (why it runs before `ensure_machine_id`) is documented at the top of `commands/first_run.rs`.

### Calm First-Sync Labeling

The first sync of a fresh HQ uploads the entire release-shipped `core/` scaffold (docs, hooks, knowledge, policies, scripts, skills, workers) — identical for every user, not the user's own content. `src/lib/progressLabel.ts` collapses any `core/…` path (`isCorePath` → `CORE_SETUP_LABEL` = "Setting up HQ core files…") so the live label reads as one-time setup rather than a flood of unfamiliar files. Wired into the classic popover live card (`Popover.svelte` `liveWorkspaceLine`) and the desktop-alt status line (`desktop-alt/lib/sync-model.ts` `currentSyncLabel`). Display-only — the honest file counter and what actually gets stored are unchanged.

## First-Push Protection

`commands/first_push.rs` runs before any company's first upload and rejects the push if any of these would be sent to S3:

| Excluded path | Reason |
|---|---|
| `**/settings/` | Credentials, OAuth tokens, vault refs |
| `**/data/` | Company datasets (added v0.1.x cloud-sync exclude) |
| `**/workers/` | Prompt libraries — same privacy class as settings |
| `**/.git/` | Git internals |
| Anything matched in `util/ignore.rs` | General sync ignore set |

Enforced via `util::ignore::tests::company_local_dirs_are_ignored`. A failed first-push protection check surfaces as a `sync:error` event with code `FIRST_PUSH_BLOCKED` and the offending path.

## Telemetry Collector

`commands/telemetry.rs` runs after each successful sync (best-effort, async, errors swallowed):

1. Read `~/.hq/telemetry-cursor.json` (last-sent state)
2. Walk the HQ tree, count files / sizes / company breakdown
3. Diff against cursor
4. Check opt-in: vault `/v1/usage/opt-in` (authoritative) → falls back to `telemetryEnabled` in `~/.hq/menubar.json` if vault is unreachable
5. If opted in, POST diff to `/v1/usage`
6. Update cursor

The cursor is per-machine (keyed by `machineId` in menubar.json) so re-installs don't double-count.

## Auto-Provisioning (UJ-006)

On first sync after a fresh install, `commands/provision.rs` and `commands/personal.rs` perform two background provisions:

1. **Person entity** — POSTs to vault `/v1/people` to create the user's `person` record in HQ-Cloud (idempotent — server returns existing if already created). Uses Cognito email from the access token as the lookup key.
2. **Personal company bucket** — if `companies/personal/` exists locally but has no `bucket` mapping in vault, requests S3 bucket creation + writes the bucket ref back to the local `companies/personal/settings/vault.json`.

Both are best-effort and don't block the sync. Failures log to the diagnostic log (`util/logfile.rs`) with a `PROVISION_*` code so Connect-diagnostics surfaces them.

## Sync Event Protocol

`hq sync --json` emits ndjson lines. Types defined in `events.rs`:

```
{"type":"progress","company":"indigo","path":"docs/a.md","bytes":42,"direction":"down","author":"user@example.com"}
{"type":"conflict","path":"file.txt","localHash":"aaa","remoteHash":"bbb","canAutoResolve":true}
{"type":"error","code":"NET_FAIL","message":"Connection reset"}
{"type":"complete","filesChanged":7,"bytesTransferred":204800,"journalPath":"/tmp/j.log"}
{"type":"new-files","company":"indigo","files":[{"path":"docs/new.md","bytes":1024,"addedBy":"user@example.com"}]}
```

On `progress`, `direction` (`"up"`/`"down"`, hq-cloud ≥5.29) and `author` (download-only, from S3 `created-by`, hq-cloud ≥5.31) are optional — older runners omit them. The activity log uses `author` to attribute each downloaded file ("{author} added/updated") and reconciles the per-company `new-files` event onto the matching download rows to flip the verb from "updated" to "added" and back-fill `addedBy` where `author` was absent (`commands/activity.rs::record_new_files`).

Parsed via `#[serde(tag = "type")]` discriminated union. Unknown types silently skipped.

## Process Management

- Singleton handle per process type (`hq-sync` for sync, `hq-sync-daemon` for daemon)
- `try_register_handle()` is TOCTOU-safe (atomic check-and-register)
- SIGTERM with 5s grace before SIGKILL
- 10-minute hard timeout on sync runs

## Daemon (V2 Prep)

Feature-flagged behind `autostartDaemon: true` in `~/.hq/menubar.json` (default: false). UI does NOT expose daemon controls in V1. Commands exist (`start_daemon`, `stop_daemon`, `daemon_status`) but are only reachable via Tauri devtools.

State files: `.hq-sync.pid`, `.hq-sync-daemon.json` in the HQ folder.

## Tray Icon

4 embedded PNG icons (`src-tauri/icons/tray-*.png`) are generated from the official HQ mark at `src-tauri/icons/source/HQ.svg` by `scripts/generate-tray-icons.py`. The generated canvases are 38x22 at @1x and 76x44 at @2x, monochrome black on transparent so macOS can template-invert them for light/dark menu bars. Runtime icons are cached via `OnceLock` after first decode. State swaps go through the `set_state_icon()` helper, which calls `set_icon()` then re-asserts `set_icon_as_template(true)` — macOS drops `isTemplate` on every `set_icon()`, so without the re-assert the template glyph would render as raw pixels after the first state change.

Left-click toggles popover window. Right-click shows context menu (Sync Now / Settings / Quit). Tray state auto-updates from sync event listeners.

## Build & Release

- **Dev:** `npm run tauri dev`
- **Build:** `npm run tauri build`
- **DMG:** `scripts/create-dmg.sh`
- **Notarize:** `scripts/notarize.sh`
- **CI:** `.github/workflows/release.yml` (code signing + notarization)
- **Auto-updater:** `latest.json` published to GitHub Releases, generated by `scripts/generate-latest-json.sh`

## Performance Budgets

Documented in `tests/PERF.md`:
- Idle memory: <50 MB
- Bundle size: <15 MB
- Popover open: <100 ms

## Testing

Classic popover release testing still uses `tests/MANUAL_TESTING.md` plus Loom proof. Rust unit tests cover serialization, config parsing, process management. Frontend and story tests run with `npm test`. Desktop-alt gate/window/page/secrets coverage runs with `npm run test:e2e:desktop-alt`; it uses a scripted source-contract harness by default and can switch to live `tauri-driver` with `HQ_SYNC_DESKTOP_ALT_LIVE=1` plus `HQ_SYNC_DESKTOP_ALT_APP` or `HQ_SYNC_DESKTOP_ALT_APP_PATH`.

## Gotchas

- `tauri_plugin_updater::Update` is not `Clone` -- must call `updater.check()` again in `install_update`. This is a plugin constraint, not redundant.
- OAuth uses loopback port **53682** -- must match Cognito app client redirect URIs exactly.
- `hq sync --json` double-binds the HQ folder path (both `HQ_ROOT` env var and `--hq-path` CLI flag) for defense-in-depth.
- Tray icons must be `@2x` PNGs for Retina. `icon_as_template(true)` is required for macOS menu bar dark/light adaptation, but it is **not sticky**: macOS resets `isTemplate` to NO on every `set_icon()`, so each runtime swap must re-assert `set_icon_as_template(true)`. Always swap through the `set_state_icon()` helper rather than calling `set_icon()` directly.
- `nix::sys::signal::kill(pid, None)` (kill-0) can false-positive on PID reuse -- acceptable for V2 prep scope.
- **Multi-window ready handshake:** Secondary windows (e.g. `new-files-detail`) use a managed-state + `detail_window_ready` command pattern instead of timed `emit_to` delays. The renderer calls `detail_window_ready` after mounting its `listen()` handler, which emits the data and shows the window. This avoids the race where `emit_to` fires before the webview's JS event listener is registered.
- `on_window_event` in main.rs is scoped to the `main` window label -- the detail window can close independently without quitting the app.
- New files state (`newFilesList`) in App.svelte accumulates across companies within a single sync run and resets when a new sync starts.
- The desktop-alt window is not a ready-handshake window. It self-loads from Tauri commands after mount, so failures usually come from missing capability permissions, command registration drift in `main.rs`, or the Indigo gate rejecting the caller.
