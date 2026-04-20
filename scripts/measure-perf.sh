#!/usr/bin/env bash
# measure-perf.sh — Performance budget verification for HQ Sync menubar app
# Usage: scripts/measure-perf.sh [--build]
#   --build   Run a full release build before measuring (default: use existing .app)
#
# Automated checks:
#   - Bundle size < 15 MB
#
# Manual checks (instructions printed):
#   - Idle resident memory < 50 MB
#   - Popover open latency < 100 ms

set -euo pipefail

BUDGET_BUNDLE_MB=15
APP_NAME="HQ Sync.app"
REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BUNDLE_DIR="$REPO_ROOT/src-tauri/target/release/bundle/macos"
APP_PATH="$BUNDLE_DIR/$APP_NAME"
DO_BUILD=false

for arg in "$@"; do
  case "$arg" in
    --build) DO_BUILD=true ;;
    *)
      echo "Unknown argument: $arg"
      echo "Usage: $0 [--build]"
      exit 1
      ;;
  esac
done

echo "======================================"
echo " HQ Sync — Performance Budget Check"
echo "======================================"
echo ""

# --- Bundle size (automated) ---

if $DO_BUILD; then
  echo "[build] Running release build..."
  (cd "$REPO_ROOT/src-tauri" && cargo tauri build 2>&1)
  echo ""
fi

if [ ! -d "$APP_PATH" ]; then
  echo "ERROR: App bundle not found at:"
  echo "  $APP_PATH"
  echo ""
  echo "Run with --build to build first, or build manually:"
  echo "  cd src-tauri && cargo tauri build"
  exit 1
fi

# Measure bundle size in bytes, convert to MB for comparison
BUNDLE_KB=$(du -sk "$APP_PATH" | awk '{print $1}')
BUNDLE_MB=$((BUNDLE_KB / 1024))
BUNDLE_HUMAN=$(du -sh "$APP_PATH" | awk '{print $1}')

echo "--- Bundle Size ---"
echo "  Path:    $APP_PATH"
echo "  Size:    $BUNDLE_HUMAN ($BUNDLE_MB MB)"
echo "  Budget:  < ${BUDGET_BUNDLE_MB} MB"

# Compare using integer KB to avoid floating-point issues
BUDGET_KB=$((BUDGET_BUNDLE_MB * 1024))
if [ "$BUNDLE_KB" -lt "$BUDGET_KB" ]; then
  echo "  Result:  PASS"
  BUNDLE_PASS=true
else
  echo "  Result:  FAIL — bundle exceeds ${BUDGET_BUNDLE_MB} MB budget"
  BUNDLE_PASS=false
fi
echo ""

# --- Manual checks ---

echo "--- Idle Resident Memory (manual) ---"
echo "  Budget: < 50 MB"
echo "  Steps:"
echo "    1. Launch: open \"$APP_PATH\""
echo "    2. Close the popover (click away from tray icon)"
echo "    3. Wait 10 minutes with no interaction"
echo "    4. Open Activity Monitor > filter 'HQ Sync'"
echo "    5. Read 'Real Memory' column — must be < 50 MB"
echo ""

echo "--- Popover Open Latency (manual) ---"
echo "  Budget: < 100 ms"
echo "  Steps:"
echo "    1. Launch the app"
echo "    2. Add instrumentation: performance.now() at tray-click"
echo "       and in Svelte onMount, log the delta"
echo "    3. Click tray icon 5 times, record each delta"
echo "    4. Median must be < 100 ms"
echo ""

# --- Summary ---

echo "======================================"
echo " Summary"
echo "======================================"
echo "  Bundle size:       $(if $BUNDLE_PASS; then echo 'PASS'; else echo 'FAIL'; fi)"
echo "  Idle memory:       MANUAL — see instructions above"
echo "  Popover latency:   MANUAL — see instructions above"
echo ""

if $BUNDLE_PASS; then
  echo "All automated checks passed."
  echo "Complete manual checks and record results in tests/PERF.md."
  exit 0
else
  echo "AUTOMATED CHECK FAILED — release blocked."
  exit 1
fi
