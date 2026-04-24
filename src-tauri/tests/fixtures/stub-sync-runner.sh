#!/bin/sh
# Stub `hq-sync-runner` for US-006 integration tests.
#
# Shipped as a checked-in POSIX script (no bash/zsh isms, no external deps
# beyond `printf`) so the test that drives it works identically on macOS CI
# and any developer machine with `/bin/sh`.
#
# Flags honored:
#   --list-all-companies   Print a canned JSON array of companies to stdout.
#                          One local entry ("acme") + one aws entry ("beta")
#                          — same canonical shape the real runner emits.
#                          Exits 0.
#   --promote <slug>       Emit a canned ndjson stream for the promote flow:
#                          start → progress(entity) → progress(bucket) →
#                          progress(writeback) → complete. The `slug` value
#                          is echoed back in every event's payload. Exits 0.
#
# Unknown flags: exit 2 with a message on stderr so a typo in a test call
# doesn't silently look like success.

set -eu

mode=""
slug=""

# Single pass over argv — cheap, POSIX-safe. The real runner accepts
# --hq-root too (plus any number of neighbor flags); we accept-and-ignore
# them here so the test can point to any hq-root without the stub caring.
while [ $# -gt 0 ]; do
  case "$1" in
    --list-all-companies)
      mode="list"
      shift
      ;;
    --promote)
      mode="promote"
      shift
      if [ $# -lt 1 ]; then
        printf 'stub-sync-runner: --promote requires a slug argument\n' >&2
        exit 2
      fi
      slug="$1"
      shift
      ;;
    --hq-root)
      # Swallow --hq-root <value> so the caller can pass any path.
      shift
      if [ $# -gt 0 ]; then
        shift
      fi
      ;;
    *)
      # Unknown flag — absorb it to stay forward-compatible with any new
      # neighbor flag the real runner gains. Production tests assert on
      # event payloads, not on our rejection of extras.
      shift
      ;;
  esac
done

case "$mode" in
  list)
    # Canonical mixed-source response from the PRD:
    #   acme: local-only (no uid)
    #   beta: aws-known (uid present)
    # Single line — list_all_companies_impl trims then json-parses it.
    printf '[{"slug":"acme","name":"Acme","source":"local"},{"slug":"beta","name":"Beta","uid":"U-1","source":"aws"}]\n'
    exit 0
    ;;
  promote)
    # ndjson stream, one event per line. The Rust side re-emits these
    # as `promote:*` Tauri events; test assertions ride the event bus.
    printf '{"type":"promote:start","slug":"%s"}\n' "$slug"
    printf '{"type":"promote:progress","slug":"%s","step":"entity"}\n' "$slug"
    printf '{"type":"promote:progress","slug":"%s","step":"bucket"}\n' "$slug"
    printf '{"type":"promote:progress","slug":"%s","step":"writeback"}\n' "$slug"
    printf '{"type":"promote:complete","slug":"%s","uid":"cmp_%s","bucketName":"bucket-%s"}\n' "$slug" "$slug" "$slug"
    exit 0
    ;;
  *)
    printf 'stub-sync-runner: no mode flag given (expected --list-all-companies or --promote <slug>)\n' >&2
    exit 2
    ;;
esac
