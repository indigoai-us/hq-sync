# HQ Sync

Menubar sync agent for HQ. Built with Tauri 2 + Svelte.

## Development

```bash
npm install
npm run tauri dev
```

## Indigo Desktop View

Indigo dogfood users see an **Open desktop view** icon in the popover header. It opens a separate decorated Tauri window with Sync, Meetings, and per-company Board / Activity / Deployments / Secrets panels. The surface is hidden from non-`@getindigo.ai` users in the UI and rejected again by the backend command gate.

Implementation notes: [`docs/desktop-alt.md`](docs/desktop-alt.md)

## Build

```bash
npm run tauri build
```

## Testing

Classic popover release testing still uses the manual checklist and Loom proof below. The Indigo desktop view also has a Vitest E2E harness for gate visibility, window lifecycle, smoke pages, and metadata-only secrets.

### Manual Testing

Manual testing is done via a structured checklist covering the 7 user journeys defined in the PRD:

| Journey | Description |
|---------|-------------|
| UJ-001  | First install to first sync in <5 min, zero terminal |
| UJ-002  | Returning user — expired token silent refresh |
| UJ-003  | Sync conflict — resolve in popover modal, no terminal |
| UJ-004  | Retether — user changes HQ path via Settings |
| UJ-005  | Auto-update — new version installed silently |
| UJ-006  | Auto-provisioning + Personal HQ — first sync auto-creates `person` entity in HQ-Cloud and provisions the `personal` company bucket |
| UJ-007  | Telemetry opt-in round-trip — toggle in Settings persists to `~/.hq/menubar.json`, vault `/v1/usage/opt-in` reflects the change, next sync respects new state |

Full checklist with step-by-step instructions, expected outcomes, and pass/fail checkboxes: **[`tests/MANUAL_TESTING.md`](tests/MANUAL_TESTING.md)**

### Unit Tests

```bash
# Rust unit tests
cargo test --manifest-path=src-tauri/Cargo.toml

# Frontend unit/story tests
npm test

# Desktop-alt scripted or live E2E
npm run test:e2e:desktop-alt
```

### Release Testing Protocol

Before each release (v1.0.0 and every minor/patch):

1. Run through the full manual checklist on a **fresh macOS VM**
2. Record a **Loom video** walking through all test scenarios
3. Publish the Loom video link in the **GitHub Release notes**
4. Verify performance budgets pass (see `tests/PERF.md`)
5. Verify code signing: `spctl -a -vv "HQ Sync.app"`

### Desktop-Alt E2E

`npm run test:e2e:desktop-alt` defaults to a scripted source-contract harness, so it can run in CI without booting a signed app. For live Tauri-driver coverage, set `HQ_SYNC_DESKTOP_ALT_LIVE=1` with `HQ_SYNC_DESKTOP_ALT_APP` or `HQ_SYNC_DESKTOP_ALT_APP_PATH`. `HQ_SYNC_DESKTOP_ALT_WEBDRIVER_URL` defaults to `http://127.0.0.1:4444`.
