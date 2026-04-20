#!/usr/bin/env bash
#
# create-dmg.sh — Create a styled DMG installer for the HQ Sync app.
#
# Usage:
#   ./scripts/create-dmg.sh <path-to.app> <output.dmg>
#
# Example:
#   ./scripts/create-dmg.sh "target/release/bundle/macos/HQ Sync.app" HQ-Sync.dmg

set -euo pipefail

APP_PATH="${1:?Usage: create-dmg.sh <path-to.app> <output.dmg>}"
DMG_PATH="${2:?Usage: create-dmg.sh <path-to.app> <output.dmg>}"

if [ ! -d "$APP_PATH" ]; then
  echo "Error: '$APP_PATH' is not a directory"
  exit 1
fi

VOLUME_NAME="HQ Sync"
APP_NAME=$(basename "$APP_PATH")
STAGING_DIR=$(mktemp -d -t dmg-staging-XXXXXX)

echo "==> Preparing DMG staging area..."
cp -R "$APP_PATH" "$STAGING_DIR/"
ln -s /Applications "$STAGING_DIR/Applications"

# Remove existing DMG if present
rm -f "$DMG_PATH"

echo "==> Creating DMG..."
hdiutil create \
  -volname "$VOLUME_NAME" \
  -srcfolder "$STAGING_DIR" \
  -ov \
  -format UDZO \
  -fs HFS+ \
  -imagekey zlib-level=9 \
  "$DMG_PATH"

# Cleanup
rm -rf "$STAGING_DIR"

DMG_SIZE=$(du -h "$DMG_PATH" | cut -f1)
echo "==> DMG created: $DMG_PATH ($DMG_SIZE)"
