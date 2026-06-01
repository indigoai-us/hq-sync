#!/usr/bin/env bash
#
# Verify the meeting-detect-notify feature is actually live in the
# deployed /Applications/HQ Sync.app — not just "processes are running"
# (which is misleading because macOS keeps detached processes alive on
# inode handles after their .app is overwritten by an auto-update).
#
# Run this before claiming the feature is testable. If anything fails,
# the answer to "can I test meeting detection?" is NO regardless of
# what `ps` shows.
#
# Exit codes:
#   0 = all checks pass, feature is wired and the deployed bundle has it
#   1 = bundle missing or one of the integrity checks failed
#   2 = bundle is there but a *running* process predates the bundle on
#       disk (the ghost-process trap that bit me on 2026-05-25)

set -uo pipefail

APP="${1:-/Applications/HQ Sync.app}"
PASS=()
FAIL=()
WARN=()

red()    { printf '\033[31m%s\033[0m\n' "$*"; }
green()  { printf '\033[32m%s\033[0m\n' "$*"; }
yellow() { printf '\033[33m%s\033[0m\n' "$*"; }

check() {
  local label="$1"; shift
  if "$@" >/dev/null 2>&1; then
    PASS+=("$label")
    green "  ✓ $label"
  else
    FAIL+=("$label")
    red   "  ✗ $label"
  fi
}

warn_check() {
  local label="$1"; shift
  if "$@" >/dev/null 2>&1; then
    PASS+=("$label")
    green "  ✓ $label"
  else
    WARN+=("$label")
    yellow "  ⚠ $label"
  fi
}

echo "Inspecting: $APP"
echo ""

# ── Bundle existence ─────────────────────────────────────────────────────
if [ ! -d "$APP" ]; then
  red "✗ Bundle not found at $APP"
  exit 1
fi

# Print bundle metadata up front — useful even when checks fail.
echo "Bundle metadata:"
echo "  on-disk mtime: $(stat -f '%Sm' "$APP")"
codesign -dvv "$APP" 2>&1 | grep -E "^(Identifier|Authority|TeamIdentifier|CDHash)" | sed 's/^/  /' || true
echo ""

# ── Required SDK assets ──────────────────────────────────────────────────
echo "SDK assets (must be present for meeting detection to work):"

BRIDGE="$APP/Contents/Resources/recall-sdk-bridge/bridge.mjs"
check "bridge.mjs at $BRIDGE" \
  test -f "$BRIDGE"

SDK_EXE="$APP/Contents/Resources/recall-sdk-bridge/node_modules/@recallai/desktop-sdk/desktop_sdk_macos_exe"
check "Recall SDK helper at desktop_sdk_macos_exe" \
  test -f "$SDK_EXE"

WRAPPER="$APP/Contents/MacOS/recall-desktop-sdk"
check "Tauri sidecar wrapper at $WRAPPER" \
  test -f "$WRAPPER"

GST="$APP/Contents/Resources/recall-sdk-bridge/node_modules/@recallai/desktop-sdk/Frameworks/GStreamer.framework"
check "GStreamer.framework bundled" \
  test -d "$GST"

# Framework symlinks (the post-build fix-recall-framework-symlinks.sh
# step). If these are missing, the SDK SIGABRTs at first dlopen on
# libgstaudio-1.0.0.dylib.
check "GStreamer.framework symlink: Versions/Current -> 1.0" \
  test -L "$GST/Versions/Current"
check "GStreamer.framework symlink: top-level GStreamer mach-O" \
  test -L "$GST/GStreamer"

echo ""

# ── Rust code symbols (proves the feature is compiled in) ────────────────
echo "Rust code (proves the @getindigo.ai gate + permissions plumbing is compiled in):"

MENUBAR="$APP/Contents/MacOS/hq-sync-menubar"
if [ ! -f "$MENUBAR" ]; then
  FAIL+=("hq-sync-menubar binary missing")
  red "  ✗ hq-sync-menubar binary missing"
else
  # `strings` finds the symbol names embedded as panic/format paths even
  # in stripped release binaries — Tauri's release profile keeps the
  # symbol table by default.
  # Look for distinctive log-message substrings that survive `strip` in
  # release builds (function symbol names DO get stripped — looking for
  # the function name itself misses real positives, as we hit on the
  # first run of this script). These strings are baked into the binary
  # via the `log()` macro and only appear in their respective modules.
  check "@getindigo.ai gate compiled in (meeting_detect: log signature)" \
    bash -c "strings '$MENUBAR' | grep -q 'start_recall_sdk: user not in @getindigo.ai allowlist'"
  check "Permission registration compiled in (CGRequestScreenCaptureAccess log)" \
    bash -c "strings '$MENUBAR' | grep -q 'CGRequestScreenCaptureAccess'"
  check "Recall SDK lifecycle compiled in (start_recall_sdk: initialising log)" \
    bash -c "strings '$MENUBAR' | grep -q 'start_recall_sdk: initialising'"
fi

echo ""

# ── Info.plist usage descriptions ────────────────────────────────────────
# Apple kills the app on the spot if a TCC-controlled API is called without
# the matching usage-description key in Info.plist. Hit this on 2026-05-25
# when AVCaptureDevice.requestAccess(audio) crashed with "This app has
# crashed because it attempted to access privacy-sensitive data without a
# usage description." — so the verifier checks every key our Rust code
# directly relies on. If we add a new TCC call, append the matching key here.
echo "Info.plist usage descriptions (Apple kills the app instantly if these are missing):"

PLIST="$APP/Contents/Info.plist"
plist_has() {
  /usr/libexec/PlistBuddy -c "Print :$1" "$PLIST" >/dev/null 2>&1
}
check "NSMicrophoneUsageDescription (required by AVCaptureDevice.requestAccess(audio))" \
  plist_has NSMicrophoneUsageDescription

echo ""

# ── Code signing integrity ───────────────────────────────────────────────
echo "Code signing:"
check "codesign --verify --deep --strict passes" \
  codesign --verify --deep --strict "$APP"

# Warn-only: hardened runtime tells us whether the build will pass
# notarization. Local dev-cert builds skip it on purpose.
warn_check "hardened runtime enabled (required for distribution)" \
  bash -c "codesign -d --entitlements - '$APP' 2>&1 | grep -q 'com.apple.security.app-sandbox\\|runtime'" 2>/dev/null

echo ""

# ── Ghost-process check (the trap from 2026-05-25) ───────────────────────
# Only meaningful for a DEPLOYED bundle — running this against a fresh
# build in src-tauri/target/ would false-fire because the running PIDs
# legitimately don't reference that build directory.
echo "Process / bundle alignment:"

if [[ "$APP" == /Applications/* ]]; then
  MENUBAR_PIDS=$(pgrep -f 'HQ Sync.app/Contents/MacOS/hq-sync-menubar' 2>/dev/null || true)
  GHOST=0
  if [ -n "$MENUBAR_PIDS" ]; then
    BUNDLE_MTIME=$(stat -f '%m' "$APP")
    for pid in $MENUBAR_PIDS; do
      PROC_START=$(ps -p "$pid" -o lstart= 2>/dev/null | xargs -I{} date -j -f '%a %b %e %T %Y' '{}' '+%s' 2>/dev/null || echo 0)
      if [ "$PROC_START" -gt 0 ] && [ "$PROC_START" -lt "$BUNDLE_MTIME" ]; then
        red "  ✗ PID $pid started BEFORE the on-disk bundle (mtime=$BUNDLE_MTIME, started=$PROC_START)"
        red "    This is a GHOST process from a previous bundle that got auto-updated."
        red "    Kill it (kill -TERM $pid) and relaunch from /Applications."
        GHOST=1
      fi
    done
    if [ "$GHOST" -eq 0 ]; then
      green "  ✓ Running hq-sync-menubar PIDs match the on-disk bundle"
    fi
  else
    yellow "  ⚠ HQ Sync is not running. Launch it before testing."
  fi
else
  GHOST=0
  yellow "  (skipped — bundle is not at /Applications; ghost-process check is deploy-target-only)"
fi

echo ""

# ── Summary ──────────────────────────────────────────────────────────────
echo "Summary:"
echo "  ${#PASS[@]} passed, ${#WARN[@]} warnings, ${#FAIL[@]} failed"
if [ "$GHOST" -eq 1 ]; then
  echo ""
  red "RESULT: Ghost processes detected — see above."
  exit 2
fi
if [ "${#FAIL[@]}" -gt 0 ]; then
  echo ""
  red "RESULT: Feature is NOT testable on this bundle. Failed checks:"
  for f in "${FAIL[@]}"; do echo "    - $f" >&2; done
  exit 1
fi
echo ""
green "RESULT: Bundle has the meeting-detect-notify feature wired."
echo "  Next: ensure TCC grants exist for accessibility / screen-capture /"
echo "  microphone / system-audio / full-disk-access — see"
echo "  src-tauri/src/commands/permissions.rs."
