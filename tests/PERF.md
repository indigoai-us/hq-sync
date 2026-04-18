# HQ Sync Menubar — Performance Budget Verification

> **Hard gate:** Any budget miss blocks release. Re-run before every release.

## Performance Budgets

| # | Metric | Budget | Measurement Method | Tool |
|---|--------|--------|--------------------|------|
| 1 | Idle resident memory | < 50 MB | Activity Monitor after 10 min idle, popover closed | Activity Monitor (macOS) |
| 2 | Bundle size | < 15 MB | `du -sh "HQ Sync.app"` on release build | `scripts/measure-perf.sh` (automated) |
| 3 | Popover open latency | < 100 ms | `performance.now()` delta from tray click to popover `onMount` | Browser DevTools / Svelte instrumentation |

---

## Measurement Instructions

### 1. Idle Resident Memory (< 50 MB)

1. Build a release binary: `cargo tauri build`
2. Launch **HQ Sync.app** from `src-tauri/target/release/bundle/macos/HQ Sync.app`
3. Ensure the popover is **closed** (click away from the tray icon)
4. Wait **10 minutes** with no interaction
5. Open **Activity Monitor** > filter for `HQ Sync`
6. Read the **Real Memory** (resident) column
7. Record the value below — must be < 50 MB to pass

### 2. Bundle Size (< 15 MB)

Automated via `scripts/measure-perf.sh`. To measure manually:

1. Build a release binary: `cargo tauri build`
2. Run: `du -sh src-tauri/target/release/bundle/macos/"HQ Sync.app"`
3. Record the value below — must be < 15 MB to pass

### 3. Popover Open Latency (< 100 ms)

1. Build and launch a debug or release binary
2. Open the Svelte DevTools or add temporary instrumentation:
   - Record `performance.now()` when the Svelte side receives the `tray-click` event from Rust
   - Record `performance.now()` in the popover's Svelte `onMount`
   - Delta = popover open latency
3. Click the tray icon 5 times, record each delta
4. Use the **median** of the 5 measurements
5. Record below — median must be < 100 ms to pass

---

## Latest Results

| Date | Version | Metric | Measured | Budget | Pass/Fail | Notes |
|------|---------|--------|----------|--------|-----------|-------|
| _YYYY-MM-DD_ | _0.x.x_ | Idle memory | _XX MB_ | < 50 MB | _PASS/FAIL_ | |
| _YYYY-MM-DD_ | _0.x.x_ | Bundle size | _XX MB_ | < 15 MB | _PASS/FAIL_ | |
| _YYYY-MM-DD_ | _0.x.x_ | Popover latency | _XX ms_ | < 100 ms | _PASS/FAIL_ | |

---

## Notes

- Bundle size is the only fully automated check (see `scripts/measure-perf.sh`).
- Memory and popover latency require a running app and manual observation.
- All three budgets must pass before tagging a release.
