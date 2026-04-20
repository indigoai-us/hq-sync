# HQ Sync

Menubar sync agent for HQ. Built with Tauri 2 + Svelte.

## Development

```bash
npm install
npm run tauri dev
```

## Build

```bash
npm run tauri build
```

## Testing

> **Policy deviation:** V1 uses manual testing + Loom video instead of automated e2e tests. This is a documented exception from [`e2e-backpressure-required.md`](../../.claude/policies/e2e-backpressure-required.md), approved during PRD interview (QUALITY-2). Justification: dogfood-only cohort of 10 internal Indigo users with a direct feedback channel. V2 will add Playwright (for popover WebView) + AppleScript (for tray icon) automated e2e tests before any external customer rollout.

### Manual Testing

All testing is done via a structured manual checklist covering the 5 user journeys defined in the PRD:

| Journey | Description |
|---------|-------------|
| UJ-001  | First install to first sync in <5 min, zero terminal |
| UJ-002  | Returning user — expired token silent refresh |
| UJ-003  | Sync conflict — resolve in popover modal, no terminal |
| UJ-004  | Retether — user changes HQ path via Settings |
| UJ-005  | Auto-update — new version installed silently |

Full checklist with step-by-step instructions, expected outcomes, and pass/fail checkboxes: **[`tests/MANUAL_TESTING.md`](tests/MANUAL_TESTING.md)**

### Unit Tests

```bash
# Rust unit tests
cargo test --manifest-path=src-tauri/Cargo.toml

# Frontend (when added)
npm test
```

### Release Testing Protocol

Before each release (v1.0.0 and every minor/patch):

1. Run through the full manual checklist on a **fresh macOS VM**
2. Record a **Loom video** walking through all test scenarios
3. Publish the Loom video link in the **GitHub Release notes**
4. Verify performance budgets pass (see `tests/PERF.md`)
5. Verify code signing: `spctl -a -vv "HQ Sync.app"`

### V2 Automated E2E (Planned)

V2 will introduce automated e2e tests before any external rollout:

- **Playwright** — popover WebView interactions (sync button, conflict modal, settings pane)
- **AppleScript** — tray icon state verification, context menu actions
- **CI integration** — automated test suite in GitHub Actions on every PR
