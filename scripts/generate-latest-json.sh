#!/bin/bash
# Generate latest.json for Tauri auto-updater
# Usage: ./scripts/generate-latest-json.sh <version> <signature> <download-url> [output-path]
#
# This script is for local testing. In CI, latest.json is generated
# automatically by .github/workflows/release.yml.

set -euo pipefail

VERSION="${1:?Usage: generate-latest-json.sh <version> <signature> <download-url> [output-path]}"
SIGNATURE="${2:?Missing signature argument}"
DOWNLOAD_URL="${3:?Missing download URL argument}"
OUTPUT="${4:-latest.json}"
PUB_DATE=$(date -u +"%Y-%m-%dT%H:%M:%SZ")

cat > "$OUTPUT" <<EOF
{
  "version": "${VERSION}",
  "notes": "See https://github.com/indigoai-us/hq-sync/releases/tag/v${VERSION}",
  "pub_date": "${PUB_DATE}",
  "platforms": {
    "darwin-universal": {
      "signature": "${SIGNATURE}",
      "url": "${DOWNLOAD_URL}"
    },
    "darwin-aarch64": {
      "signature": "${SIGNATURE}",
      "url": "${DOWNLOAD_URL}"
    },
    "darwin-x86_64": {
      "signature": "${SIGNATURE}",
      "url": "${DOWNLOAD_URL}"
    }
  }
}
EOF

echo "Generated ${OUTPUT} for version ${VERSION}"
