#!/usr/bin/env bash
# vault-tool.sh
#
# Pull / push an HQ vault to/from S3 with hq-symlink: protocol handling.
#
# Mirrors what the menubar app's Rust client does in apps/hq-cloud/src/s3.ts:
#   - Symlink body wire format:   "hq-symlink:<target>"  (NO trailing newline)
#   - Symlink S3 metadata header: x-amz-meta-hq-symlink-target = "1"
#   - Pull recognizes the header (preferred) AND the body sniff (fallback).
#   - No prefix prepended to keys; bucket root holds the whole tree.
#
# Two subcommands:
#
#   pull --bucket <name> --local <dir> [--profile P] [--region R] [--dry-run]
#     1. aws s3 sync s3://<bucket>/ <local>/      (all keys as flat files)
#     2. Walk <local>/, find files whose first 11 bytes == "hq-symlink:"
#     3. Replace each with a real symlink to the marker payload.
#     Note: standalone download has no metadata-header awareness — the body
#     sniff is the canonical detection here (matches the menubar app's
#     body-first preference).
#
#   push --local <dir> --bucket <name> [--profile P] [--region R]
#        [--dry-run] [--no-delete] [--include-ignored]
#     1. Build staging tree from <local>/: copy/hardlink regular files,
#        serialize each symlink as a "hq-symlink:<target>" marker file,
#        skip ignored paths (hardcoded denylist matching hq-cloud/ignore.ts).
#     2. aws s3 sync <staging>/ s3://<bucket>/ --delete  (or --no-delete).
#     3. For each known symlink marker, overwrite with a put-object that
#        sets x-amz-meta-hq-symlink-target="1" so the menubar app
#        materializes it as a symlink on the next pull.
#
#   resolve <prs_*>                      → "hq-vault-<lower-prs-id>"
#
# Cross-operator note: this script uses your AWS profile's S3 perms
# directly. The real menubar app authenticates via vault-service STS
# (companies vended membership-gated, persons self-ownership-gated).
# Use the AWS-direct path only for operator/admin maintenance.
#
# macOS caveat: HFS+/APFS default mounts are case-insensitive. If a
# vault has both `AGENTS.md` and `agents.md` as separate S3 keys, the
# pull collapses them into a single local file (the second download
# overwrites the first). Round-trip then produces a spurious 1-file
# upload + 1-file delete in the case-mismatched key. Linux operator
# boxes (or a case-sensitive disk image) avoid this entirely.

set -euo pipefail

# ---------- shared state ----------
SCRIPT_NAME="$(basename "$0")"
SUBCMD="${1:-}"
[ -n "$SUBCMD" ] || { echo "usage: $SCRIPT_NAME <pull|push|resolve> [opts...]" >&2; exit 2; }
shift || true

PROFILE="${HQ_VAULT_AWS_PROFILE:-}"
REGION="${HQ_VAULT_AWS_REGION:-us-east-1}"
DRY_RUN=0
DO_DELETE=1
INCLUDE_IGNORED=0
LOCAL_DIR=""
BUCKET=""

usage() {
  sed -n '2,40p' "$0"
  exit 2
}

aws_args() {
  local args=()
  [ -n "$PROFILE" ] && args+=(--profile "$PROFILE")
  args+=(--region "$REGION")
  printf '%s\n' "${args[@]}"
}

# Build the aws CLI invocation prefix as a bash array.
aws_prefix() {
  local arr=()
  [ -n "$PROFILE" ] && arr+=(--profile "$PROFILE")
  arr+=(--region "$REGION")
  AWS_PREFIX=("${arr[@]}")
}

# Minimal ignore set for round-trip operator use. The menubar client has a
# much larger denylist in apps/hq-cloud/src/ignore.ts that filters what
# gets uploaded from a real HQ install — but on a vault snapshot (whatever
# is in S3) the snapshot IS the source of truth, so additional filtering
# would corrupt round-trips by deleting "ignored" paths the menubar
# uploaded previously. Only filter things that should never have been in
# S3 in the first place and that would cause damage if accidentally
# round-tripped (build outputs, .git internals, credentials).
#
# Pass --include-ignored on push to disable this filter entirely.
IGNORE_DIRS=(
  .git
  node_modules
  target
  dist
  build
  .next
  .svelte-kit
  .turbo
  __pycache__
  .venv
  venv
)

is_ignored() {
  local rel="$1"
  local top="${rel%%/*}"
  local base="${rel##*/}"

  for d in "${IGNORE_DIRS[@]}"; do
    [ "$top" = "$d" ] && return 0
    case "$rel" in
      */$d|*/$d/*) return 0 ;;
    esac
  done

  case "$base" in
    .DS_Store|Thumbs.db) return 0 ;;
    *.pyc|*.class)       return 0 ;;
    .env|.env.*)         return 0 ;;
  esac
  return 1
}

# ---------- pull ----------
cmd_pull() {
  while [ $# -gt 0 ]; do
    case "$1" in
      --bucket)  BUCKET="$2"; shift 2 ;;
      --local)   LOCAL_DIR="$2"; shift 2 ;;
      --profile) PROFILE="$2"; shift 2 ;;
      --region)  REGION="$2"; shift 2 ;;
      --dry-run) DRY_RUN=1; shift ;;
      -h|--help) usage ;;
      *) echo "unknown arg: $1" >&2; usage ;;
    esac
  done
  [ -n "$BUCKET" ] || { echo "pull: --bucket required" >&2; exit 2; }
  [ -n "$LOCAL_DIR" ] || { echo "pull: --local required" >&2; exit 2; }

  aws_prefix
  mkdir -p "$LOCAL_DIR"

  echo "==> pull s3://$BUCKET/  ->  $LOCAL_DIR" >&2
  echo "==> phase 1/2: bulk download (aws s3 sync)" >&2
  if [ "$DRY_RUN" = "1" ]; then
    aws "${AWS_PREFIX[@]}" s3 sync "s3://$BUCKET/" "$LOCAL_DIR/" --dryrun
  else
    aws "${AWS_PREFIX[@]}" s3 sync "s3://$BUCKET/" "$LOCAL_DIR/"
  fi

  echo "==> phase 2/2: materialize hq-symlink: markers as real symlinks" >&2
  local count=0
  # Find files whose first 11 bytes are "hq-symlink:". Use `head -c 11`
  # to bound the read; on a binary or larger file this is essentially free.
  while IFS= read -r -d '' f; do
    local first11
    first11="$(head -c 11 "$f" 2>/dev/null || true)"
    [ "$first11" = "hq-symlink:" ] || continue

    # Whole marker (no trailing newline by spec, but tolerate one).
    local marker target
    marker="$(cat "$f")"
    target="${marker#hq-symlink:}"
    target="${target%$'\n'}"

    if [ -z "$target" ]; then
      echo "    skip $f (empty target)" >&2
      continue
    fi

    if [ "$DRY_RUN" = "1" ]; then
      echo "    would symlink: ${f#"$LOCAL_DIR/"} -> $target" >&2
    else
      rm -f "$f"
      ln -s "$target" "$f"
    fi
    count=$((count + 1))
  done < <(find "$LOCAL_DIR" -type f -print0)

  echo "==> done. ${count} hq-symlink: markers ${DRY_RUN:+would be }materialized as symlinks" >&2
}

# ---------- push ----------
cmd_push() {
  while [ $# -gt 0 ]; do
    case "$1" in
      --local)            LOCAL_DIR="$2"; shift 2 ;;
      --bucket)           BUCKET="$2"; shift 2 ;;
      --profile)          PROFILE="$2"; shift 2 ;;
      --region)           REGION="$2"; shift 2 ;;
      --dry-run)          DRY_RUN=1; shift ;;
      --no-delete)        DO_DELETE=0; shift ;;
      --include-ignored)  INCLUDE_IGNORED=1; shift ;;
      -h|--help)          usage ;;
      *) echo "unknown arg: $1" >&2; usage ;;
    esac
  done
  [ -n "$LOCAL_DIR" ] || { echo "push: --local required" >&2; exit 2; }
  [ -n "$BUCKET" ] || { echo "push: --bucket required" >&2; exit 2; }
  [ -d "$LOCAL_DIR" ] || { echo "push: --local '$LOCAL_DIR' is not a directory" >&2; exit 2; }

  aws_prefix
  STAGING="$(mktemp -d -t hq-vault-push-XXXXXX)"
  # Trap MUST reference an unconditional global (set -u trips if `$staging`
  # is local-scoped and not yet bound when the trap fires on early exit).
  trap 'rm -rf "${STAGING:-}"' EXIT
  local staging="$STAGING"

  echo "==> push $LOCAL_DIR  ->  s3://$BUCKET/" >&2
  echo "==> phase 1/3: staging tree at $staging (serialize symlinks, apply ignores)" >&2

  local n_files=0 n_symlinks=0 n_ignored=0
  # Walk every file + symlink under LOCAL_DIR.
  while IFS= read -r -d '' src; do
    local rel="${src#"$LOCAL_DIR/"}"
    [ "$rel" = "$src" ] && continue  # skip if substitution failed
    [ -z "$rel" ] && continue

    if [ "$INCLUDE_IGNORED" = "0" ] && is_ignored "$rel"; then
      n_ignored=$((n_ignored + 1))
      continue
    fi

    local dest="$staging/$rel"
    mkdir -p "$(dirname "$dest")"

    if [ -L "$src" ]; then
      # Serialize symlink as hq-symlink:<target> with no trailing newline.
      local target
      target="$(readlink "$src")"
      printf 'hq-symlink:%s' "$target" > "$dest"
      n_symlinks=$((n_symlinks + 1))
    elif [ -f "$src" ]; then
      cp -p "$src" "$dest"
      n_files=$((n_files + 1))
    fi
  done < <(find "$LOCAL_DIR" \( -type d \( -name node_modules -o -name .git -o -name target -o -name dist -o -name build \) -prune \) -o \( \( -type f -o -type l \) -print0 \))

  echo "    files: $n_files  symlinks: $n_symlinks  ignored: $n_ignored" >&2

  # Record symlink relpaths for the metadata-stamp phase.
  local symlist
  symlist="$staging.symlinks.txt"
  : > "$symlist"
  while IFS= read -r -d '' src; do
    local rel="${src#"$LOCAL_DIR/"}"
    if [ "$INCLUDE_IGNORED" = "0" ] && is_ignored "$rel"; then continue; fi
    printf '%s\n' "$rel"
  done < <(find "$LOCAL_DIR" -type l -print0) > "$symlist"

  echo "==> phase 2/3: aws s3 sync (delete=$DO_DELETE, dry-run=$DRY_RUN)" >&2
  # `--size-only` because staging files have mtime=now and S3 LastModified
  # reflects original upload time; mtime-based comparison treats every
  # local file as newer and re-uploads it unnecessarily. Symlink markers
  # and most files have content-stable sizes — same size => same content
  # is a safe heuristic for the round-trip case. False negatives (content
  # changed but size identical) are extremely rare; if you need byte-exact
  # comparison, drop staging entirely and use rsync against a separate
  # mount.
  local sync_args=("s3" "sync" "$staging/" "s3://$BUCKET/" "--size-only")
  [ "$DO_DELETE" = "1" ] && sync_args+=("--delete")
  [ "$DRY_RUN" = "1" ] && sync_args+=("--dryrun")
  aws "${AWS_PREFIX[@]}" "${sync_args[@]}"

  echo "==> phase 3/3: stamp symlink metadata header on $(wc -l < "$symlist" | tr -d ' ') marker objects" >&2
  if [ "$DRY_RUN" = "1" ]; then
    echo "    (dry-run; skipping put-object stamp)" >&2
  else
    while IFS= read -r rel; do
      [ -z "$rel" ] && continue
      local body_path="$staging/$rel"
      aws "${AWS_PREFIX[@]}" s3api put-object \
        --bucket "$BUCKET" \
        --key "$rel" \
        --body "$body_path" \
        --metadata "hq-symlink-target=1" \
        --content-type "text/plain" \
        >/dev/null
      echo "    stamped $rel" >&2
    done < "$symlist"
  fi

  echo "==> done." >&2
}

# ---------- resolve ----------
cmd_resolve() {
  local uid="${1:-}"
  [ -n "$uid" ] || { echo "usage: $SCRIPT_NAME resolve <prs_*|cmp_*>" >&2; exit 2; }
  case "$uid" in
    prs_*|cmp_*) ;;
    *) echo "resolve: expected prs_* or cmp_* UID" >&2; exit 2 ;;
  esac
  # Bucket convention: "hq-vault-<lowercased-uid-with-underscore-as-dash>"
  # Real bucket names in audit: "hq-vault-prs-01krgv66r22hnax643dkzb5pqq"
  # Pattern: prs_01ABC → prs-01abc
  local lower
  lower="$(printf '%s' "$uid" | tr '[:upper:]' '[:lower:]' | tr '_' '-')"
  echo "hq-vault-$lower"
}

case "$SUBCMD" in
  pull)    cmd_pull "$@" ;;
  push)    cmd_push "$@" ;;
  resolve) cmd_resolve "$@" ;;
  -h|--help) usage ;;
  *) echo "unknown subcommand: $SUBCMD" >&2; usage ;;
esac
