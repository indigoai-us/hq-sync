#!/usr/bin/env bash
# Cross-language parity check: asserts TS DEFAULT_IGNORES (hq-cloud/src/ignore.ts)
# and Rust DEFAULT_IGNORES (hq-sync/src-tauri/src/util/ignore.rs) are byte-for-byte
# identical. Run from any directory — paths are derived from SCRIPT_DIR.
#
# hq-sync and hq are expected to be sibling directories; the defaults below
# assume that structure. Override via env vars if needed:
#   HQ_CLOUD_DIR=/path/to/hq/packages/hq-cloud bash scripts/check-default-ignores-parity.sh
#
# Requires: hq-cloud/node_modules already installed (pnpm install --frozen-lockfile).
# If not installed, the script prints a clear error and exits 1.
#
# NOTE: This script only passes once the hq and hq-sync branches containing
# step-4 changes are merged (or when HQ_CLOUD_DIR / HQ_SYNC_DIR env vars are
# set to point at the worktrees directly).
set -euo pipefail

SCRIPT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
HQ_SYNC_DIR="${HQ_SYNC_DIR:-$(cd "$SCRIPT_DIR/.." && pwd)}"
# hq and hq-sync are siblings — walk up from scripts/ to find the peer hq repo.
HQ_CLOUD_DIR="${HQ_CLOUD_DIR:-$(cd "$SCRIPT_DIR/../../hq/packages/hq-cloud" 2>/dev/null && pwd || true)}"

if [[ -z "$HQ_CLOUD_DIR" || ! -d "$HQ_CLOUD_DIR" ]]; then
  echo "ERROR: Cannot locate hq-cloud. Set HQ_CLOUD_DIR=/path/to/hq/packages/hq-cloud." >&2
  exit 1
fi

# 1) Emit TS DEFAULT_IGNORES as newline-joined stdout. hq-cloud is
#    "type": "module", so dist/ignore.js is ESM; use dynamic import().
if [[ ! -x "$HQ_CLOUD_DIR/node_modules/.bin/tsc" ]]; then
  echo "ERROR: $HQ_CLOUD_DIR/node_modules/.bin/tsc not found." >&2
  echo "       Run: pnpm install --frozen-lockfile in $HQ_CLOUD_DIR (or its workspace root) first." >&2
  exit 1
fi
(cd "$HQ_CLOUD_DIR" && pnpm -s run build >/dev/null)
node --input-type=module -e "
  const mod = await import('file://${HQ_CLOUD_DIR}/dist/ignore.js');
  if (!Array.isArray(mod.DEFAULT_IGNORES)) {
    process.stderr.write('DEFAULT_IGNORES is not exported as an array\n');
    process.exit(2);
  }
  process.stdout.write(mod.DEFAULT_IGNORES.join('\n') + '\n');
" > /tmp/ts-default-ignores.txt
# 2) Emit Rust DEFAULT_IGNORES via the tiny bin target.
(cd "$HQ_SYNC_DIR/src-tauri" && cargo run --quiet --bin emit-default-ignores) \
  > /tmp/rust-default-ignores.txt
# 3) Exact diff.
diff -u /tmp/ts-default-ignores.txt /tmp/rust-default-ignores.txt
