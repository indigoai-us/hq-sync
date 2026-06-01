#!/bin/bash
# Repair the macOS framework symlink structure that Tauri's bundle.resources
# strips out when it copies @recallai/desktop-sdk into the .app.
#
# macOS frameworks rely on a specific symlink layout:
#   Frameworks/<Name>.framework/Versions/Current -> 1.0
#   Frameworks/<Name>.framework/<Name>         -> Versions/Current/<Name>
#   Frameworks/<Name>.framework/Resources      -> Versions/Current/Resources
#   Frameworks/<Name>.framework/Libraries      -> Versions/Current/Libraries
#
# When Tauri copies bundle.resources, it dereferences symlinks — the bundled
# framework ends up with real files at the top level and no Current symlink,
# which breaks @rpath resolution for libgstaudio-1.0.0.dylib etc.
#
# This script restores the symlinks. Run after `tauri build`.
#
# Usage: ./scripts/fix-recall-framework-symlinks.sh <path-to-.app>

set -euo pipefail

APP="${1:-}"
if [ -z "$APP" ] || [ ! -d "$APP" ]; then
  echo "Usage: $0 <path-to-HQ Sync.app>" >&2
  exit 1
fi

GST="$APP/Contents/Resources/recall-sdk-bridge/node_modules/@recallai/desktop-sdk/Frameworks/GStreamer.framework"
if [ ! -d "$GST" ]; then
  echo "GStreamer.framework not found at $GST" >&2
  exit 1
fi

cd "$GST"

# 1. Fix Versions/Current -> 1.0
if [ ! -L "Versions/Current" ]; then
  rm -rf "Versions/Current" 2>/dev/null || true
  ( cd Versions && ln -sfn "1.0" "Current" )
  echo "  + Versions/Current -> 1.0"
fi

# 2. Fix top-level GStreamer (real file -> symlink to Versions/Current/GStreamer)
if [ -f "GStreamer" ] && [ ! -L "GStreamer" ]; then
  rm -f "GStreamer"
  ln -sfn "Versions/Current/GStreamer" "GStreamer"
  echo "  + GStreamer -> Versions/Current/GStreamer"
fi

# 3. Fix top-level Resources / Libraries (if missing as symlinks)
for name in Resources Libraries; do
  if [ ! -L "$name" ]; then
    rm -rf "$name" 2>/dev/null || true
    if [ -d "Versions/Current/$name" ]; then
      ln -sfn "Versions/Current/$name" "$name"
      echo "  + $name -> Versions/Current/$name"
    fi
  fi
done

echo "GStreamer.framework symlinks restored in $APP"
