#!/bin/bash
# Inside-out signing for the HQ Sync .app bundle.
#
# `codesign --deep` is deprecated and unreliable — it traverses the bundle
# in an order that doesn't respect dylib dependency edges, which mangles
# load commands in the Recall Desktop SDK's GStreamer dylibs (we hit a
# SIGKILL loop on libgstaudio-1.0.0.dylib during the meeting-detect-notify
# project — see the project journal). The fix is the canonical inside-out
# pass: sign each dylib first, then frameworks, then nested executables,
# then the main binary, then the bundle as a whole. Apple's own docs and
# Tauri's distribution guide both recommend this order.
#
# Usage: ./scripts/sign-bundle.sh <path-to-HQ Sync.app> [identity]
#
# Identity defaults to the "HQ Installer Dev" self-signed cert that lives
# in the local keychain (sha1: EAAEC4A1A7AF86CE8B54B8D657669D3F54D572D9).
# Pass a Developer ID Application identity for distribution builds.

set -euo pipefail

APP="${1:-}"
IDENTITY="${2:-HQ Installer Dev}"

if [ -z "$APP" ] || [ ! -d "$APP" ]; then
  echo "Usage: $0 <path-to-HQ Sync.app> [identity]" >&2
  exit 1
fi

if ! security find-identity -v -p codesigning | grep -q "$IDENTITY"; then
  echo "ERROR: code-signing identity '$IDENTITY' not found in the login keychain." >&2
  echo "       Available identities:" >&2
  security find-identity -v -p codesigning >&2 || true
  exit 1
fi

echo "Inside-out signing $APP with identity '$IDENTITY'…"

# Shared codesign flags.
#
# Hardened runtime (`--options runtime`) defaults OFF for the local dev cert:
# self-signed certs have no Team ID, and hardened runtime then refuses to
# load dylibs whose own (Team-ID-less) signatures don't match the host
# process — every GStreamer dylib in the Recall SDK fails with "mapping
# process and mapped file (non-platform) have different Team IDs" and the
# SDK SIGABRT-loops at boot. Set `HARDENED_RUNTIME=1` to opt back in when
# signing with a real Developer ID Application cert (which provides a Team
# ID); the accompanying entitlements file should also set
# `com.apple.security.cs.disable-library-validation` if the SDK still hits
# the same wall on Apple's signing infrastructure.
SIGN_FLAGS=(--force --sign "$IDENTITY")
if [ "${HARDENED_RUNTIME:-0}" = "1" ]; then
  SIGN_FLAGS+=(--options runtime)
fi

# `--timestamp` is skipped for the local dev cert (Apple's timestamp server
# rejects self-signed certs) — set TIMESTAMP=1 with a real Developer ID
# Application identity.
if [ "${TIMESTAMP:-0}" = "1" ]; then
  SIGN_FLAGS+=(--timestamp)
fi

# Optional entitlements file — pass HQ_SIGN_ENTITLEMENTS=path/to.plist to
# attach an entitlements file (needed when signing with a Developer ID
# cert that requires the hardened runtime + specific entitlements like
# Apple Events / sandboxed network access / disable-library-validation
# for unsigned-by-Apple SDK dylibs).
if [ -n "${HQ_SIGN_ENTITLEMENTS:-}" ] && [ -f "$HQ_SIGN_ENTITLEMENTS" ]; then
  SIGN_FLAGS+=(--entitlements "$HQ_SIGN_ENTITLEMENTS")
fi

sign_file() {
  local target="$1"
  local out
  # Capture combined output instead of discarding stderr: under
  # `set -euo pipefail` a codesign failure on the very first dylib aborts the
  # whole step with NO diagnostic if stderr is sent to /dev/null. Stay quiet on
  # success (codesign chatters "replacing existing signature" on every file),
  # but on failure surface the real error AND the identities codesign can
  # actually see (the default keychain search list) so CI failures are
  # debuggable in one run.
  if ! out=$(codesign "${SIGN_FLAGS[@]}" "$target" 2>&1); then
    echo "ERROR: codesign failed for: $target" >&2
    echo "  codesign output: ${out:-<none>}" >&2
    echo "  identities visible to codesign (default search list):" >&2
    security find-identity -v -p codesigning >&2 || true
    return 1
  fi
}

SDK_DIR="$APP/Contents/Resources/recall-sdk-bridge/node_modules/@recallai/desktop-sdk"
GST_FRAMEWORK="$SDK_DIR/Frameworks/GStreamer.framework"

# ─── 1. Sign every Mach-O inside the GStreamer framework ────────────────────
# Sign by CONTENT, not by name. The framework ships Mach-O binaries that do
# NOT end in .dylib — notably the `GStreamer` umbrella binary, which exists at
# BOTH Versions/1.0/GStreamer AND Versions/1.0/lib/GStreamer (two distinct
# inodes, byte-identical, each a fat x86_64+arm64 binary). A `*.dylib` glob
# misses both. Apple's notary service rejects ANY unsigned nested Mach-O,
# while `codesign --verify --deep` (Phase 9) does NOT — it only seals them as
# hashed nested resources. That gap let the unsigned Versions/1.0/lib/GStreamer
# pass the Sign step's verify and then fail the Notarize step ~8 min later
# (dry-run run 26725856981). Classify each file with `file` and sign anything
# Mach-O; skip the rest so `set -e` doesn't abort on Resources/headers/.py.
#
# Order within this phase doesn't matter — each file is signed in isolation
# (no --deep). Leaf-first ordering only matters when SEALING directories (the
# framework in Phase 3, the .app in Phase 8). find -print0 / read -d '' keeps
# paths with spaces intact and avoids ARG_MAX with the ~400 SDK binaries.
if [ -d "$GST_FRAMEWORK" ]; then
  echo "  Phase 1: sign every Mach-O in GStreamer.framework"
  macho_count=0
  while IFS= read -r -d '' f; do
    desc=$(file -b "$f" 2>/dev/null) || true
    case "$desc" in
      *Mach-O*)
        sign_file "$f"
        macho_count=$((macho_count + 1))
        ;;
    esac
  done < <(find "$GST_FRAMEWORK" -type f -print0)
  echo "    ($macho_count Mach-O binaries signed)"
fi

# ─── 2. Assert the umbrella Mach-O binaries are signed ──────────────────────
# Belt-and-suspenders for the exact regression that broke notarization: the
# `GStreamer` umbrella binary at BOTH Versions/1.0/GStreamer and
# Versions/1.0/lib/GStreamer must carry a signature after Phase 1. Verify it
# HERE (sign time) so a future glob/scan regression fails fast and loud,
# instead of silently shipping an unsigned Mach-O that only Apple's notary
# rejects — 8+ minutes downstream, with a far less obvious error.
for umbrella in \
  "$GST_FRAMEWORK/Versions/1.0/GStreamer" \
  "$GST_FRAMEWORK/Versions/1.0/lib/GStreamer"; do
  [ -f "$umbrella" ] || continue
  if ! codesign --verify --strict "$umbrella" 2>/dev/null; then
    echo "ERROR: framework umbrella Mach-O is unsigned after Phase 1: $umbrella" >&2
    echo "       (Phase 1 must sign every Mach-O by content — notary rejects unsigned nested Mach-O)" >&2
    exit 1
  fi
done
echo "  Phase 2: framework umbrella binaries verified signed"

# ─── 3. Sign the framework directory itself ────────────────────────────────
# This seals the framework — creates Versions/1.0/_CodeSignature with
# hashes of everything we just signed.
if [ -d "$GST_FRAMEWORK" ]; then
  echo "  Phase 3: seal GStreamer.framework"
  sign_file "$GST_FRAMEWORK"
fi

# ─── 4. Sign top-level Recall dylibs (libui_recorder, liblibbot_desktop_rs) ─
echo "  Phase 4: top-level Recall dylibs"
if [ -d "$SDK_DIR/Frameworks" ]; then
  find "$SDK_DIR/Frameworks" -maxdepth 1 -type f -name "*.dylib" -print0 \
    | while IFS= read -r -d '' dylib; do
        sign_file "$dylib"
      done
fi

# ─── 5. Sign the SDK's child binary (desktop_sdk_macos_exe) ─────────────────
echo "  Phase 5: SDK child binary"
if [ -f "$SDK_DIR/desktop_sdk_macos_exe" ]; then
  sign_file "$SDK_DIR/desktop_sdk_macos_exe"
fi

# ─── 6. Sign sidecar bash wrapper(s) ────────────────────────────────────────
# Tauri's bundle.externalBin places the bash wrapper(s) into Contents/MacOS/
# as `recall-desktop-sdk`. Bash scripts aren't strictly required to be
# signed, but codesign --strict --deep complains if they aren't when the
# parent .app is sealed — so do it anyway.
echo "  Phase 6: Contents/MacOS sidecars"
for sidecar in "$APP/Contents/MacOS/recall-desktop-sdk"*; do
  [ -e "$sidecar" ] || continue
  # Skip the main binary — that's the app exe, signed in Phase 7.
  [ "$(basename "$sidecar")" = "hq-sync-menubar" ] && continue
  sign_file "$sidecar"
done

# ─── 7. Sign the main app binary ────────────────────────────────────────────
echo "  Phase 7: main app binary"
if [ -f "$APP/Contents/MacOS/hq-sync-menubar" ]; then
  sign_file "$APP/Contents/MacOS/hq-sync-menubar"
fi

# ─── 8. Seal the .app bundle ────────────────────────────────────────────────
# Last step — generates the top-level _CodeSignature/ that macOS uses for
# TCC bundle-identity attribution. This is what determines whether
# permission grants stick across rebuilds (provided the bundle ID and
# identity stay constant).
echo "  Phase 8: seal .app bundle"
sign_file "$APP"

# ─── 9. Verify ──────────────────────────────────────────────────────────────
echo ""
echo "Verifying signature integrity (codesign --verify --deep)…"
codesign --verify --deep --strict --verbose=2 "$APP" 2>&1 | tail -5

echo ""
echo "Bundle identity:"
codesign -dvv "$APP" 2>&1 | grep -E "(Identifier|Authority|TeamIdentifier)" || true

echo ""
echo "✓ Signing complete: $APP"
