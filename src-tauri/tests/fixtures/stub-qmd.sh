#!/bin/sh
# Stub qmd binary used by the embeddings handoff integration test.
#
# Echoes three progress-style lines to stdout, then exits.
#
# Exit code is controlled by the first positional arg when the test wants
# to exercise the error path:
#   $1 = exit code (default 0)
#
# Real qmd also reads `embed` as its first CLI arg, but the stub ignores
# everything — its entire job is to emit stdout and exit. POSIX-only: no
# bash-isms, so it runs under `/bin/sh` on every macOS and Linux the test
# harness might execute on.

exit_code=0
# The real command-line from our code is `qmd embed`, so $1 is usually
# "embed". If the test wants a non-zero exit, it invokes the stub with a
# numeric first arg; we detect "looks like a number" and treat it as
# the exit code.
case "$1" in
  ''|*[!0-9]*)
    # Not a number — ignore and use default 0
    ;;
  *)
    exit_code="$1"
    ;;
esac

echo "Loading qmd model…"
echo "Indexing chunk 1/3"
echo "Indexing chunk 2/3"

exit "$exit_code"
