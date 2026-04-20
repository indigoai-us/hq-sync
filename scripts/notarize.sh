#!/usr/bin/env bash
#
# notarize.sh — Notarize a macOS .app bundle using Apple's notarytool.
#
# Usage:
#   ./scripts/notarize.sh <path-to-app>
#
# Required environment variables:
#   APPLE_API_KEY       — App Store Connect API key ID
#   APPLE_API_ISSUER    — App Store Connect API issuer UUID
#   APPLE_API_KEY_PATH  — Path to the .p8 API key file
#
# The script will:
#   1. Create a ZIP of the .app for submission
#   2. Submit to Apple's notary service
#   3. Wait for notarization to complete
#   4. Staple the notarization ticket to the .app
#   5. Verify with spctl

set -euo pipefail

APP_PATH="${1:?Usage: notarize.sh <path-to.app>}"

# Validate inputs
if [ ! -d "$APP_PATH" ]; then
  echo "Error: '$APP_PATH' is not a directory"
  exit 1
fi

for var in APPLE_API_KEY APPLE_API_ISSUER APPLE_API_KEY_PATH; do
  if [ -z "${!var:-}" ]; then
    echo "Error: $var is not set"
    exit 1
  fi
done

if [ ! -f "$APPLE_API_KEY_PATH" ]; then
  echo "Error: API key file not found at '$APPLE_API_KEY_PATH'"
  exit 1
fi

APP_NAME=$(basename "$APP_PATH")
ZIP_PATH=$(mktemp -t notarize-XXXXXX).zip

echo "==> Creating ZIP for notarization submission..."
ditto -c -k --keepParent "$APP_PATH" "$ZIP_PATH"

echo "==> Submitting '$APP_NAME' to Apple notary service..."
xcrun notarytool submit "$ZIP_PATH" \
  --key "$APPLE_API_KEY_PATH" \
  --key-id "$APPLE_API_KEY" \
  --issuer "$APPLE_API_ISSUER" \
  --wait \
  --timeout 1800

echo "==> Stapling notarization ticket..."
xcrun stapler staple "$APP_PATH"

echo "==> Verifying notarization..."
spctl -a -vv "$APP_PATH"

# Cleanup
rm -f "$ZIP_PATH"

echo "==> Notarization complete for '$APP_NAME'"
