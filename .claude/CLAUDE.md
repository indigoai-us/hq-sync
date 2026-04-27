# HQ Sync Menubar

macOS menu bar app wrapping `hq sync` for non-technical users. Tauri 2 + Svelte 5 + vanilla CSS.

## Architecture

**Frontend:** Svelte 5 with runes (`$state`, `$effect`). No component library — vanilla CSS in `src/styles/popover.css` (Liquid Glass styling adopted v0.1.23). Views: `SignInPrompt` (OAuth), `Popover` (main sync UI with per-workspace rows, real per-file progress bar, **Stop** button mid-sync, and Connect diagnostics drawer), `Settings` (preferences + folder re-tether). Conflict resolution via `ConflictModal` + `ConflictRow` components.

**Backend:** Tauri 2 Rust commands in `src-tauri/src/commands/`. 27 registered commands in `main.rs`.

**State flow:** Svelte frontend calls Tauri commands via `invoke()`. Rust backend emits typed events (`sync:progress`, `sync:conflict`, `sync:error`, `sync:complete`) that Svelte listens to via `listen()`.

## Key Modules

| Module | Purpose |
|--------|---------|
| `commands/sync.rs` | Spawns `hq sync --json`, streams ndjson events, 10-min timeout. Includes a "Preparing sync…" pre-pass that walks the tree to compute real `filesTotal` before transfers start (so progress isn't fake) |
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
| `commands/settings.rs` | Settings persistence |
| `commands/autostart.rs` | Login-item autostart |
| `tray.rs` | System tray with 4 visual states (idle/syncing/error/conflict) |
| `updater.rs` | Auto-update checker (10s delay, then every 6h) |
| `events.rs` | Typed sync event structs (ndjson discriminated union) |
| `sentry_scrub.rs` | Sentry event scrubber — strips Cognito tokens and home-dir paths before send |
| `util/paths.rs` | HQ folder resolver (4-tier — see below). Also provides `resolve_bin` + `child_path` for finding `hq` and node-shebang interpreters under launchd's minimal PATH |
| `util/ignore.rs` | Sync ignore rules — excludes `settings/`, `data/`, `workers/`, `.git/`, etc. from cloud sync (privacy class) |
| `util/journal.rs` | Append-only sync journal at `~/.hq/sync-journal.log` (used by Connect diagnostics) |
| `util/logfile.rs` | Persistent diagnostic log for the sync pipeline at `~/.hq/sync-debug.log` (rotated at 10MB) |

## Config Files (User Machine)

| File | Written By | Purpose |
|------|-----------|---------|
| `~/.hq/config.json` | hq-installer | Company UID, slug, person, bucket, vault URL, HQ folder path |
| `~/.hq/menubar.json` | This app | HQ path override, syncOnLaunch, notifications, startAtLogin, autostartDaemon |
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
{"type":"progress","phase":"uploading","filesComplete":3,"filesTotal":10}
{"type":"conflict","path":"file.txt","localHash":"aaa","remoteHash":"bbb","canAutoResolve":true}
{"type":"error","code":"NET_FAIL","message":"Connection reset"}
{"type":"complete","filesChanged":7,"bytesTransferred":204800,"journalPath":"/tmp/j.log"}
```

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

4 embedded PNG icons (`src-tauri/icons/tray-*.png`), cached via `OnceLock` after first decode. `icon_as_template(true)` for macOS dark/light mode adaptation.

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

Manual testing only in V1 (documented policy deviation). Checklist at `tests/MANUAL_TESTING.md`. Automated e2e planned for V2. Rust unit tests cover serialization, config parsing, process management.

## Gotchas

- `tauri_plugin_updater::Update` is not `Clone` -- must call `updater.check()` again in `install_update`. This is a plugin constraint, not redundant.
- OAuth uses loopback port **53682** -- must match Cognito app client redirect URIs exactly.
- `hq sync --json` double-binds the HQ folder path (both `HQ_ROOT` env var and `--hq-path` CLI flag) for defense-in-depth.
- Tray icons must be `@2x` PNGs for Retina. `icon_as_template(true)` is required for macOS menu bar dark/light adaptation.
- `nix::sys::signal::kill(pid, None)` (kill-0) can false-positive on PID reuse -- acceptable for V2 prep scope.
