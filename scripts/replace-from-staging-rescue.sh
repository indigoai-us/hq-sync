#!/usr/bin/env bash
# replace-from-staging-rescue.sh
#
# Variant of replace-from-staging.sh that:
#
#   1. Does NOT require the destination to be a git repo. The `.git/` check is
#      dropped and git status reporting at the end is replaced with a plain
#      file-count summary. (git is still required to fetch the source repo.)
#
#   2. Rescues drifts instead of clobbering them. Before the wipe, every file
#      in the wipe set that DIFFERS from staging (or exists only locally) is
#      MOVED into `personal/` so it survives as a layered override.
#
# Drift mapping (in order):
#
#   a. `.claude/CLAUDE.md`  ->  `personal/CLAUDE.md`           (hard-coded)
#   b. `.claude/<rest>`     ->  `personal/<rest>`              if `personal/<top-of-rest>/` exists
#   c. `core/<rest>`        ->  `personal/<rest>`              if `personal/<top-of-rest>/` exists
#   d. `<rest>`             ->  `personal/<rest>`              if `personal/<top-of-rest>/` exists
#   e. anything else        ->  `.hq-conflicts/rescue-<timestamp>/<original-relative-path>`
#
# If the rescue destination already exists, the moved file is suffixed with
# `.drift-<unix-ts>` so we never silently overwrite a prior override.
#
# Two operating modes (same as the parent script):
#
#   - Default ("preserve-list"): wipes every top-level entry except a hardcoded
#     preserve set (`.git`, `companies/` except `companies/_template/`,
#     `personal/`, `workspace/`, `repos/`, `.github/`, `.leak-scan/`,
#     `.hq-sync-journal.json`, `.hq/`, `.hq-conflicts/`),
#     plus any paths passed via --preserve.
#
#     `repos/` is always preserved: it holds user-owned git checkouts
#     (`repos/public/` + `repos/private/`) whose `.git/` directories would
#     shatter under a file-by-file rescue. hq-core never ships a `repos/`
#     tree, so the overlay can't restore it — the only safe handling is
#     to leave it alone entirely.
#
#   - `--paths`: wipes ONLY the explicit comma-separated list of top-level
#     entries and overlays only those.
#
# `--preserve-subpath <rel>` (repeatable) carves out individual files INSIDE
# the wipe set, copied to a mktemp shuttle pre-wipe and restored post-overlay.
#
# Drift truthiness — history-check gate (default ON):
#
#   A naive cmp-based drift detector flags every local-vs-staging difference.
#   That over-rescues: files the user never touched but that staging later
#   modified or deleted look identical to files the user actually edited.
#
#   With history-check enabled, every candidate drift's local blob SHA
#   (computed via `git hash-object`) is checked against the set of all blob
#   SHAs that ever appeared at that path across the staging repo's full
#   history. If the local content matches ANY past staging version at that
#   path, the user provably didn't author the change — the local copy is
#   just lag — and rescue is skipped. Only files that are genuinely
#   user-added or user-edited survive the gate.
#
#   Cost: the clone switches to `--filter=blob:none` (full commits/trees,
#   lazy blobs). ~15 MB for hq-core-staging vs. ~5 MB shallow. Indexing the
#   history adds ~1-3 s. Pass --no-history-check to disable.
#
# Sync-point provenance — `core/core.yaml`:
#
#   On a successful default-mode (full-replace) run, the script records the
#   staging commit it synced to under `core/core.yaml`'s
#   `replaced_from_staging:` key (source / ref / last_sync_sha / last_sync_at).
#   On the NEXT run, this is read before the clone and — if the source repo
#   matches and the SHA is reachable in the new clone — used as the
#   **history floor**: the index walks `git log <last_sync_sha>` instead of
#   `git log --all`, scoping it to "blobs staging knew about at our last
#   sync point". A local file whose content matches one of those blobs is
#   provably lag (user had it via prior sync). A local file whose content
#   only matches blobs from AFTER the last sync — i.e. blobs the user
#   couldn't have copied from a sync — is treated as user-authored even if
#   it happens to coincide with a recent staging commit.
#
# Usage:
#   replace-from-staging-rescue.sh [--ref REF] [--source OWNER/REPO]
#                                  [--paths PATH1,PATH2,...]
#                                  [--preserve PATH]...
#                                  [--preserve-subpath REL]...
#                                  [--hq-root DIR]
#                                  [--no-history-check]
#                                  [--dry-run] [--yes]
#
# Defaults:
#   --ref     main
#   --source  indigoai-us/hq-core-staging
#   --hq-root <script>/../../..    (assumes script lives at personal/skills/<skill>/)
#
# Requires: git (for source clone + hash-object), rsync, cmp, awk, grep, sort.

set -euo pipefail

REF="main"
SOURCE_REPO="indigoai-us/hq-core-staging"
DRY_RUN=0
ASSUME_YES=0
EXTRA_PRESERVE=()
PRESERVE_SUBPATHS=()
NARROW_PATHS_CSV=""
HQ_ROOT_OVERRIDE=""
HISTORY_CHECK=1
SKIPPED_BY_HISTORY=0

# Paths that are ALWAYS preserved across the wipe+overlay, regardless of
# mode or user flags. Each entry is shuttled to a mktemp area pre-wipe and
# restored post-overlay (same mechanism as --preserve-subpath), AND skipped
# by drift detection so the rescue scan never touches them.
#
# Why core/packages and packages: both hold user-curated packs (hq-pack-*)
# resolved through the npm/hq-cli install path; staging may also ship under
# `core/packages` with different content. Without this carve-out a
# full-replace would clobber the local pack tree.
# Why .claude/state: runtime session state — `.claude/state/active-session-*`
# changes every session; shuttling preserves it across the overlay.
CARVE_OUT_PATHS=( "core/packages" "packages" ".claude/state" )

usage() {
  sed -n '2,55p' "$0"
  exit 1
}

while [ $# -gt 0 ]; do
  case "$1" in
    --ref) REF="$2"; shift 2 ;;
    --source) SOURCE_REPO="$2"; shift 2 ;;
    --preserve) EXTRA_PRESERVE+=("$2"); shift 2 ;;
    --preserve-subpath) PRESERVE_SUBPATHS+=("$2"); shift 2 ;;
    --paths) NARROW_PATHS_CSV="$2"; shift 2 ;;
    --hq-root) HQ_ROOT_OVERRIDE="$2"; shift 2 ;;
    --no-history-check) HISTORY_CHECK=0; shift ;;
    --dry-run) DRY_RUN=1; shift ;;
    --yes|-y) ASSUME_YES=1; shift ;;
    -h|--help) usage ;;
    *) echo "unknown arg: $1" >&2; usage ;;
  esac
done

for p in "${EXTRA_PRESERVE[@]+"${EXTRA_PRESERVE[@]}"}"; do
  case "$p" in
    .git|companies|personal|workspace|repos|.github|.leak-scan|.hq-sync-journal.json|.hq|.hq-conflicts|.git/|companies/|personal/|workspace/|repos/|.github/|.leak-scan/|.hq/|.hq-conflicts/)
      echo "error: --preserve $p is redundant ('$p' is always preserved). Remove the flag." >&2
      exit 2
      ;;
  esac
done

for sp in "${PRESERVE_SUBPATHS[@]+"${PRESERVE_SUBPATHS[@]}"}"; do
  case "$sp" in
    /*|*..*)
      echo "error: --preserve-subpath $sp must be a relative path with no '..' segments." >&2
      exit 2
      ;;
  esac
done

# Append the always-preserved carve-outs to PRESERVE_SUBPATHS so the same
# backup/restore code path handles them. Dedup against any user-supplied
# entries (defensive — user may also have passed them explicitly).
for cp in "${CARVE_OUT_PATHS[@]}"; do
  already=0
  for sp in "${PRESERVE_SUBPATHS[@]+"${PRESERVE_SUBPATHS[@]}"}"; do
    if [ "$sp" = "$cp" ]; then already=1; break; fi
  done
  [ "$already" = "1" ] || PRESERVE_SUBPATHS+=("$cp")
done

NARROW_PATHS=()
if [ -n "$NARROW_PATHS_CSV" ]; then
  IFS=',' read -r -a _NARROW_RAW <<< "$NARROW_PATHS_CSV"
  for raw in "${_NARROW_RAW[@]}"; do
    name="${raw#"${raw%%[![:space:]]*}"}"
    name="${name%"${name##*[![:space:]]}"}"
    if [ -z "$name" ]; then continue; fi
    case "$name" in
      */*|..|.)
        echo "error: --paths entry '$name' must be a single top-level name (no slashes, no '.' or '..')." >&2
        exit 2
        ;;
    esac
    NARROW_PATHS+=("$name")
  done
  if [ "${#NARROW_PATHS[@]}" -eq 0 ]; then
    echo "error: --paths was passed but resolved to an empty list." >&2
    exit 2
  fi
  if [ "${#EXTRA_PRESERVE[@]}" -ne 0 ]; then
    echo "error: --paths and --preserve are mutually exclusive. Use --preserve-subpath for sub-paths inside the listed top-level entries." >&2
    exit 2
  fi
fi

# --- Resolve HQ root (no .git requirement) -----------------------------------
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
if [ -n "$HQ_ROOT_OVERRIDE" ]; then
  HQ_ROOT="$(cd "$HQ_ROOT_OVERRIDE" && pwd)"
else
  HQ_ROOT="$(cd "$SCRIPT_DIR/../../.." && pwd)"
fi

# Sanity: look for `companies/` and `personal/` (not `.git/`). Drift rescue
# needs personal/ to exist as the override target.
if [ ! -d "$HQ_ROOT/companies" ] || [ ! -d "$HQ_ROOT/personal" ]; then
  echo "error: $HQ_ROOT does not look like an HQ root (missing companies/ or personal/). Aborting." >&2
  echo "       pass --hq-root <dir> if the script is not at personal/skills/<skill>/." >&2
  exit 3
fi

# Per-run rescue bucket. Computed once so every drifted file in this run
# lands under the same `<hqRoot>/.hq-conflicts/rescue-<ts>/` dir — easy to
# diff, easy to delete, and historical runs accumulate as siblings so the
# user can audit past rescues without spelunking through personal/.
# Colons would be valid here but make paths awkward in shells/tab-completion;
# use hyphens to match other timestamped artifacts (sync conflict suffixes).
RESCUE_TS="$(date -u +%Y-%m-%dT%H-%M-%SZ)"
RESCUE_BUCKET=".hq-conflicts/rescue-$RESCUE_TS"

# --- Read prior sync-point metadata (must happen BEFORE the wipe) -----------
# If the user has run this script before in default mode, core/core.yaml
# carries the staging SHA we last synced to. We use that SHA later (after
# the clone) as the history-floor: scope the index walk to commits reachable
# from <last_sync_sha> instead of all branches. Only honored when the
# previously-recorded source matches the current --source.
PREV_SYNC_SHA=""
PREV_SYNC_SOURCE=""
PREV_SYNC_REF=""
PREV_SYNC_AT=""
if [ -f "$HQ_ROOT/core/core.yaml" ] && command -v yq >/dev/null 2>&1; then
  PREV_SYNC_SHA="$(yq -r '.replaced_from_staging.last_sync_sha // ""' "$HQ_ROOT/core/core.yaml" 2>/dev/null || true)"
  PREV_SYNC_SOURCE="$(yq -r '.replaced_from_staging.source // ""' "$HQ_ROOT/core/core.yaml" 2>/dev/null || true)"
  PREV_SYNC_REF="$(yq -r '.replaced_from_staging.ref // ""' "$HQ_ROOT/core/core.yaml" 2>/dev/null || true)"
  PREV_SYNC_AT="$(yq -r '.replaced_from_staging.last_sync_at // ""' "$HQ_ROOT/core/core.yaml" 2>/dev/null || true)"
fi

echo "==> HQ root:    $HQ_ROOT"
echo "==> Source:     https://github.com/$SOURCE_REPO @ $REF"
if [ -n "$PREV_SYNC_SHA" ]; then
  if [ "$PREV_SYNC_SOURCE" = "$SOURCE_REPO" ]; then
    echo "==> Prior sync: $PREV_SYNC_SHA from $PREV_SYNC_SOURCE@$PREV_SYNC_REF ($PREV_SYNC_AT) — will use as history floor"
  else
    echo "==> Prior sync: $PREV_SYNC_SHA from $PREV_SYNC_SOURCE (different source — ignoring as floor)"
  fi
fi
if [ "${#NARROW_PATHS[@]}" -ne 0 ]; then
  echo "==> Mode:       narrow (--paths)"
  echo "==> Wipe set:   ${NARROW_PATHS[*]}"
else
  echo "==> Mode:       preserve-list (default)"
  echo "==> Preserved:  .git, companies (except companies/_template), personal, workspace, repos, .github, .leak-scan, .hq-sync-journal.json, .hq, .hq-conflicts${EXTRA_PRESERVE[*]+, ${EXTRA_PRESERVE[*]}}"
fi
if [ "${#PRESERVE_SUBPATHS[@]}" -ne 0 ]; then
  echo "==> Preserved subpaths (backed up + restored across the overlay):"
  for sp in "${PRESERVE_SUBPATHS[@]}"; do
    # Mark the always-on carve-outs so it's obvious they're not from --preserve-subpath.
    is_carve=0
    for cp in "${CARVE_OUT_PATHS[@]}"; do
      if [ "$cp" = "$sp" ]; then is_carve=1; break; fi
    done
    if [ "$is_carve" = "1" ]; then
      echo "    - $sp  (always-on carve-out)"
    else
      echo "    - $sp"
    fi
  done
fi
echo "==> Drift policy: rescue to personal/ (fall back to $RESCUE_BUCKET/)"
if [ "$HISTORY_CHECK" = "1" ]; then
  echo "==> History gate: ON (skip drift if local matches any past staging blob at that path)"
else
  echo "==> History gate: OFF (--no-history-check; every diff rescued)"
fi
[ "$DRY_RUN" = "1" ] && echo "==> DRY RUN     (no destructive operations will run)"

if [ "$ASSUME_YES" != "1" ] && [ "$DRY_RUN" != "1" ]; then
  printf "\nThis will MOVE drift files into personal/, then DELETE the listed top-level\nentries, then unpack %s@%s on top.\nType 'yes' to proceed: " "$SOURCE_REPO" "$REF"
  read -r confirm
  [ "$confirm" = "yes" ] || { echo "Aborted."; exit 4; }
fi

TMPDIR="$(mktemp -d -t hq-replace-rescue-XXXXXX)"
trap 'rm -rf "$TMPDIR"' EXIT

# Build the clone URL. If GH_TOKEN is set in the environment, inject it as
# the basic-auth user so `git clone` can access private staging repos
# without an interactive credential prompt. This is the form the GitHub
# docs recommend for token-based git over HTTPS:
#   https://x-access-token:<token>@github.com/<owner>/<repo>.git
# The hq-sync Tauri caller resolves GH_TOKEN via `gh auth token` (same
# path the existing drift-classifier uses) before spawning this script.
if [ -n "${GH_TOKEN:-}" ]; then
  CLONE_URL="https://x-access-token:${GH_TOKEN}@github.com/$SOURCE_REPO.git"
  CLONE_URL_DISPLAY="https://x-access-token:***@github.com/$SOURCE_REPO.git"
else
  CLONE_URL="https://github.com/$SOURCE_REPO.git"
  CLONE_URL_DISPLAY="$CLONE_URL"
fi

echo ""
if [ "$HISTORY_CHECK" = "1" ]; then
  # Full commit/tree history (needed for the path → all-time-SHA index) but
  # lazy blob fetching. We never need blob contents for the index — only
  # blob SHAs from `git log --raw` — so blobs are never lazy-fetched in
  # practice. Checkout of HEAD does fetch HEAD-tree blobs, which we need
  # anyway for the cmp-based current-state comparison.
  echo "==> Cloning $CLONE_URL_DISPLAY @$REF (full history, blob:none filter) ..."
  git clone --filter=blob:none "$CLONE_URL" "$TMPDIR/src" >/dev/null 2>&1 || {
    echo "error: clone failed" >&2; exit 5
  }
  (cd "$TMPDIR/src" && git checkout "$REF" >/dev/null 2>&1) || {
    echo "error: could not check out ref '$REF' from $SOURCE_REPO" >&2
    exit 5
  }
else
  echo "==> Cloning $CLONE_URL_DISPLAY @$REF (shallow) ..."
  git clone --depth 1 --branch "$REF" "$CLONE_URL" "$TMPDIR/src" >/dev/null 2>&1 || {
    echo "    (shallow branch clone failed; trying full clone + checkout)"
    git clone "$CLONE_URL" "$TMPDIR/src" >/dev/null
    (cd "$TMPDIR/src" && git checkout "$REF" >/dev/null 2>&1) || {
      echo "error: could not check out ref '$REF' from $SOURCE_REPO" >&2
      exit 5
    }
  }
fi

SRC_SHA="$(cd "$TMPDIR/src" && git rev-parse HEAD)"
echo "==> Source SHA: $SRC_SHA"

# --- Build the history index (path → all-time blob SHAs) --------------------
HISTORY_INDEX="$TMPDIR/path-shas.tsv"
: > "$HISTORY_INDEX"
HISTORY_FLOOR=""
if [ "$HISTORY_CHECK" = "1" ]; then
  # Decide whether the prior-sync SHA can be used as the floor for the log
  # walk. Three preconditions:
  #   1. The previously-recorded source matches the current --source.
  #   2. The SHA is reachable in this (potentially partial-blob) clone —
  #      commits + trees are always fully fetched even under --filter=blob:none,
  #      so `git cat-file -e` is the right check.
  #   3. The SHA is non-empty (PREV_SYNC_SHA was read above).
  if [ -n "$PREV_SYNC_SHA" ] && [ "$PREV_SYNC_SOURCE" = "$SOURCE_REPO" ]; then
    if (cd "$TMPDIR/src" && git cat-file -e "$PREV_SYNC_SHA" 2>/dev/null); then
      HISTORY_FLOOR="$PREV_SYNC_SHA"
    else
      echo "    prior-sync SHA $PREV_SYNC_SHA not reachable in clone (likely rebased/dropped); falling back to --all"
    fi
  fi

  # Pick the git-log walk argument: floor SHA (commits reachable from it)
  # OR --all (every ref, no floor). With a floor, the index captures only
  # blobs staging knew about at our last sync — files matching only
  # post-sync blobs are then correctly rescued as user-authored.
  if [ -n "$HISTORY_FLOOR" ]; then
    echo "==> Indexing staging history up to floor $HISTORY_FLOOR ..."
    GIT_LOG_WALK_ARG="$HISTORY_FLOOR"
  else
    echo "==> Indexing staging history (path → blob SHAs across all commits) ..."
    GIT_LOG_WALK_ARG="--all"
  fi

  # `git log <walk-arg> --raw --no-renames --no-abbrev` emits one line per
  # file change with FULL 40-char blob SHAs (matching `git hash-object` output):
  #   :MODE MODE OLDSHA NEWSHA STATUS\tpath
  # We extract NEWSHA for A/M, OLDSHA for D, then dedupe. With
  # --pretty=format: there are no commit headers to filter out. Without
  # --no-abbrev git defaults to 7-char abbreviation, which silently
  # mismatches local hash-object SHAs and disables the gate entirely.
  (cd "$TMPDIR/src" && git log "$GIT_LOG_WALK_ARG" --raw --no-renames --no-abbrev --pretty=format: 2>/dev/null) \
    | awk -F'\t' 'NF>=2 {
        n=split($1, hdr, " ")
        if (n<5) next
        status=substr(hdr[5],1,1)
        path=$2
        if (status=="A" || status=="M") {
          print hdr[4] "\t" path
        } else if (status=="D") {
          print hdr[3] "\t" path
        }
      }' | LC_ALL=C sort -u > "$HISTORY_INDEX"
  n_pairs="$(wc -l < "$HISTORY_INDEX" | tr -d ' ')"
  if [ -n "$HISTORY_FLOOR" ]; then
    echo "    indexed $n_pairs (sha, path) pairs from history reachable from $HISTORY_FLOOR"
  else
    echo "    indexed $n_pairs unique (sha, path) pairs"
  fi
fi

# --- Build wipe/overlay arg sets per mode ------------------------------------
PRUNE_ARGS=()
RSYNC_EXCLUDES=()

if [ "${#NARROW_PATHS[@]}" -ne 0 ]; then
  for n in "${NARROW_PATHS[@]}"; do
    RSYNC_EXCLUDES+=( --include="/$n" )
    RSYNC_EXCLUDES+=( --include="/$n/***" )
  done
  RSYNC_EXCLUDES+=( --exclude='/*' )
else
  PRUNE_ARGS=( -not -name .git -not -name companies -not -name personal -not -name workspace -not -name repos -not -name .github -not -name .leak-scan -not -name .hq-sync-journal.json -not -name .hq -not -name .hq-conflicts )
  for p in "${EXTRA_PRESERVE[@]+"${EXTRA_PRESERVE[@]}"}"; do
    PRUNE_ARGS+=( -not -name "$p" )
  done
  RSYNC_EXCLUDES=(
    --exclude=.git
    --exclude=personal
    --exclude=workspace
    --exclude=repos
    --exclude=.github
    --exclude=.leak-scan
    --exclude=.hq-sync-journal.json
    --exclude=.hq
    --exclude=.hq-conflicts
    --include='/companies/'
    --include='/companies/_template/***'
    --exclude='/companies/*'
  )
  for p in "${EXTRA_PRESERVE[@]+"${EXTRA_PRESERVE[@]}"}"; do
    RSYNC_EXCLUDES+=( --exclude="$p" )
  done
fi

# --- Compute the wipe-set roots (top-level entries that will be deleted) -----
WIPE_TOPLEVEL=()
if [ "${#NARROW_PATHS[@]}" -ne 0 ]; then
  for n in "${NARROW_PATHS[@]}"; do
    [ -e "$HQ_ROOT/$n" ] && WIPE_TOPLEVEL+=("$n")
  done
else
  while IFS= read -r line; do
    rel="${line#./}"
    WIPE_TOPLEVEL+=("$rel")
  done < <( cd "$HQ_ROOT" && find . -mindepth 1 -maxdepth 1 "${PRUNE_ARGS[@]}" -print )
  # companies/_template carve-out (wiped in default mode)
  [ -d "$HQ_ROOT/companies/_template" ] && WIPE_TOPLEVEL+=("companies/_template")
fi

# --- Drift detection + rescue ------------------------------------------------
#
# For every file under each wipe-set root, compare to staging. If the file
# exists only locally OR differs from staging, MOVE it into personal/.

# Map a wipe-set-relative path to its personal/ rescue target. Echoes the
# rescue path (HQ-relative) on stdout.
map_rescue_target() {
  local rel="$1"

  # (a) hard-coded special case
  if [ "$rel" = ".claude/CLAUDE.md" ]; then
    echo "personal/CLAUDE.md"
    return
  fi

  # (b/c/d) try stripping .claude/ or core/ prefix, then check if
  # personal/<top-of-rest>/ is a real directory.
  local rest=""
  case "$rel" in
    .claude/*) rest="${rel#.claude/}" ;;
    core/*)    rest="${rel#core/}" ;;
    *)         rest="$rel" ;;
  esac

  local top="${rest%%/*}"
  if [ -n "$top" ] && [ "$top" != "$rest" ] && [ -d "$HQ_ROOT/personal/$top" ]; then
    # rest has at least one subdir AND personal/<top>/ exists
    echo "personal/$rest"
    return
  fi

  # (e) fall back to the per-run rescue bucket at `.hq-conflicts/rescue-<ts>/`
  echo "$RESCUE_BUCKET/$rel"
}

# Move a single drift file. Suffix with .drift-<ts> if the target exists.
rescue_one() {
  local rel="$1"
  local target
  target="$(map_rescue_target "$rel")"
  local dest="$HQ_ROOT/$target"
  mkdir -p "$(dirname "$dest")"
  if [ -e "$dest" ]; then
    dest="${dest}.drift-$(date +%s)-$$"
  fi
  mv "$HQ_ROOT/$rel" "$dest"
  echo "    drift: $rel  ->  ${dest#"$HQ_ROOT/"}"
}

# Walk a wipe-set root, comparing each file to staging.
walk_and_rescue() {
  local root_rel="$1"
  local root_abs="$HQ_ROOT/$root_rel"
  [ -e "$root_abs" ] || [ -L "$root_abs" ] || return 0

  # Symlinks (top-level OR mid-tree) are NEVER rescued. Two families:
  #   1. master-sync-generated wrappers/mirrors
  #      (.claude/skills/<ns>:<skill>/<file>, core/<type>/<entry>) — these
  #      will be regenerated on the next Stop/PostToolUse fire of
  #      .claude/hooks/master-sync.sh after the overlay completes.
  #   2. top-level convenience symlinks (AGENTS.md, MIGRATION.md) — the
  #      staging overlay restores them as part of the source tree.
  # Moving them via `mv` would re-anchor relative targets at the new path
  # (e.g. .hq-conflicts/rescue-<ts>/AGENTS.md -> .claude/CLAUDE.md would dangle).
  if [ -L "$root_abs" ]; then
    return 0
  fi

  if [ -f "$root_abs" ]; then
    compare_one "$root_rel"
    return
  fi

  # Directory: walk regular files only. `find -type f` with the default -P
  # mode does not match symlinks, so master-sync mirrors and any other
  # symlinks inside the wipe-set tree are skipped automatically. find also
  # does not descend INTO directory symlinks under -P, so symlinked
  # subtrees stay invisible to the rescue walk.
  #
  # Prune `node_modules/` and nested `.git/` from the walk before -type f
  # matching:
  #   * `node_modules/` — vendored dep trees that are never authored
  #     content. A single `pnpm install` can drop 100k+ files; with
  #     hq-core staging shipping no `node_modules/`, every file under
  #     one reads as local-only drift → every file gets `mv`'d into the
  #     rescue bucket. On Corey's v0.1.101 run that meant the script
  #     was actively moving 25 GB / 391k files across 150 cloned repos
  #     when he caught it (CPU pegged for tens of minutes; would have
  #     `rm -rf`'d `repos/` after the scan finished if `repos/` hadn't
  #     since been added to the preserve list in v0.1.102).
  #   * nested `.git/` — internal git plumbing. A loose-objects walk
  #     can be 10k+ files per repo, none of which match staging history
  #     (the staging repo has its own .git, not user repos'). Same
  #     "every file becomes drift" failure mode as `node_modules/`.
  # Pruning is defence-in-depth: with `repos/` preserved at the top
  # level (v0.1.102 fix), neither pattern should appear inside any
  # remaining wipe-set subtree under a normal HQ install. But the cost
  # of pruning is one extra find expression; the cost of NOT pruning
  # when one DOES sneak in (e.g. `core/packages/<pack>/node_modules`
  # if the carve-out logic ever misses) is hours of wasted I/O and a
  # broken rescue bucket. Belt-and-suspenders.
  while IFS= read -r -d '' f; do
    local rel="${f#"$HQ_ROOT/"}"
    compare_one "$rel"
  done < <(find "$root_abs" \( -type d \( -name node_modules -o -name .git \) -prune \) -o \( -type f -print0 \))
}

# True (exit 0) iff $local_path's git blob SHA ever appeared at $rel in the
# staging history. When this returns true, the local content is provably a
# point-in-history staging version that the user did not author — rescue
# would just resurrect upstream-deleted/superseded cruft.
local_matches_history() {
  local rel="$1" local_path="$2"
  [ "$HISTORY_CHECK" = "1" ] || return 1
  [ -s "$HISTORY_INDEX" ] || return 1
  local sha
  sha="$(git hash-object "$local_path" 2>/dev/null)" || return 1
  # -F (fixed string) + -x (whole line) on lines of the form "<sha>\t<path>".
  # No regex escaping needed for the path.
  LC_ALL=C grep -qFx "${sha}	${rel}" "$HISTORY_INDEX"
}

# True if $rel is under any always-preserved subpath. Drift detection skips
# these — they're shuttled out, the wipe+overlay runs, then they're restored
# unchanged. Detecting them as drifts would just move them to personal/ and
# then leave a phantom-restored copy back at their original location.
is_under_preserve() {
  local rel="$1" sp
  for sp in "${PRESERVE_SUBPATHS[@]+"${PRESERVE_SUBPATHS[@]}"}"; do
    case "$rel" in
      "$sp"|"$sp"/*) return 0 ;;
    esac
  done
  return 1
}

compare_one() {
  local rel="$1"
  local local_path="$HQ_ROOT/$rel"
  local src_path="$TMPDIR/src/$rel"

  # Always-preserved paths bypass drift detection entirely.
  if is_under_preserve "$rel"; then
    return 0
  fi

  # Conflict-resolution artifacts (HQ-Sync renames divergent local files to
  # `<name>.conflict-<timestamp>-<peer>.<ext>`). Never authored, never in
  # staging — let the wipe consume them rather than dragging them to personal/.
  case "${rel##*/}" in
    *.conflict-*) return 0 ;;
  esac

  local reason=""
  if [ ! -e "$src_path" ]; then
    reason="local-only"
  elif ! cmp -s "$local_path" "$src_path"; then
    reason="modified"
  else
    return 0  # identical -> overlay will replace cleanly
  fi

  # History gate. If the local content matches some past staging blob at
  # this path, the user provably didn't author the change.
  if local_matches_history "$rel" "$local_path"; then
    SKIPPED_BY_HISTORY=$((SKIPPED_BY_HISTORY + 1))
    if [ "$DRY_RUN" = "1" ]; then
      echo "    skip ($reason, matches staging history): $rel"
    fi
    return 0
  fi

  if [ "$DRY_RUN" = "1" ]; then
    echo "    drift ($reason):   $rel  ->  $(map_rescue_target "$rel")"
  else
    rescue_one "$rel"
  fi
}

echo ""
if [ "${#WIPE_TOPLEVEL[@]}" -eq 0 ]; then
  echo "==> Wipe set is empty; nothing to rescue or overlay."
else
  echo "==> Scanning wipe set for drifts vs. $SOURCE_REPO@$REF ..."
  for root_rel in "${WIPE_TOPLEVEL[@]}"; do
    # companies/_template is always overwritten from staging — never rescue it.
    [ "$root_rel" = "companies/_template" ] && continue
    walk_and_rescue "$root_rel"
  done
fi

if [ "$DRY_RUN" = "1" ]; then
  echo ""
  if [ "${#NARROW_PATHS[@]}" -ne 0 ]; then
    echo "==> DRY RUN: would delete these top-level entries from $HQ_ROOT:"
    for n in "${NARROW_PATHS[@]}"; do
      if [ -e "$HQ_ROOT/$n" ]; then
        echo "  - ./$n"
      else
        echo "  - ./$n  (does not exist locally; rsync will create from source if present)"
      fi
    done
    echo ""
    echo "==> DRY RUN: would copy these top-level entries from source:"
    for n in "${NARROW_PATHS[@]}"; do
      if [ -e "$TMPDIR/src/$n" ]; then
        echo "  + ./$n"
      else
        echo "  + ./$n  (NOT PRESENT in source — local copy will simply be deleted)"
      fi
    done
  else
    echo "==> DRY RUN: would delete these top-level entries from $HQ_ROOT:"
    ( cd "$HQ_ROOT" && find . -mindepth 1 -maxdepth 1 "${PRUNE_ARGS[@]}" | sed 's|^\./|  - |' )
    if [ -d "$HQ_ROOT/companies/_template" ]; then
      echo "  - ./companies/_template  (carve-out: re-pulled from source)"
    fi
    echo ""
    echo "==> DRY RUN: would copy these top-level entries from source:"
    ( cd "$TMPDIR/src" && find . -mindepth 1 -maxdepth 1 "${PRUNE_ARGS[@]}" | sed 's|^\./|  + |' )
    if [ -d "$TMPDIR/src/companies/_template" ]; then
      echo "  + ./companies/_template  (carve-out from $SOURCE_REPO@$REF)"
    fi
  fi
  if [ "${#PRESERVE_SUBPATHS[@]}" -ne 0 ]; then
    echo ""
    echo "==> DRY RUN: would back up + restore these sub-paths across the overlay:"
    for sp in "${PRESERVE_SUBPATHS[@]}"; do
      if [ -e "$HQ_ROOT/$sp" ]; then
        echo "  ~ ./$sp  (present — will survive)"
      else
        echo "  ~ ./$sp  (not present locally — no-op)"
      fi
    done
  fi
  echo ""
  echo "==> DRY RUN complete. No filesystem changes made."
  exit 0
fi

# --- Back up preserve-subpaths to a mktemp shuttle ---------------------------
SHUTTLE="$TMPDIR/preserve"
mkdir -p "$SHUTTLE"
: > "$TMPDIR/preserve.map"
shuttle_id=0
for sp in "${PRESERVE_SUBPATHS[@]+"${PRESERVE_SUBPATHS[@]}"}"; do
  src="$HQ_ROOT/$sp"
  if [ -e "$src" ]; then
    shuttle_id=$((shuttle_id + 1))
    cp -a "$src" "$SHUTTLE/$shuttle_id"
    printf '%s\t%s\n' "$shuttle_id" "$sp" >> "$TMPDIR/preserve.map"
    echo "==> Backed up $sp -> shuttle/$shuttle_id"
  fi
done

echo ""
if [ "${#NARROW_PATHS[@]}" -ne 0 ]; then
  echo "==> Wiping listed top-level entries (${NARROW_PATHS[*]}) ..."
  for n in "${NARROW_PATHS[@]}"; do
    if [ -e "$HQ_ROOT/$n" ]; then
      rm -rf "$HQ_ROOT/$n"
    fi
  done
else
  echo "==> Wiping HQ root (preserving .git, companies, personal, workspace, repos, .github, .leak-scan, .hq-sync-journal.json, .hq, .hq-conflicts${EXTRA_PRESERVE[*]+, ${EXTRA_PRESERVE[*]}}) ..."
  ( cd "$HQ_ROOT" && find . -mindepth 1 -maxdepth 1 "${PRUNE_ARGS[@]}" -exec rm -rf {} + )

  if [ -d "$HQ_ROOT/companies/_template" ]; then
    echo "==> Wiping companies/_template (will be re-pulled from source) ..."
    rm -rf "$HQ_ROOT/companies/_template"
  fi
fi

echo "==> Overlaying source onto HQ root ..."
rsync -a "${RSYNC_EXCLUDES[@]}" "$TMPDIR/src/" "$HQ_ROOT/"

if [ -s "$TMPDIR/preserve.map" ]; then
  echo "==> Restoring preserved sub-paths ..."
  while IFS=$'\t' read -r id relpath; do
    dest="$HQ_ROOT/$relpath"
    mkdir -p "$(dirname "$dest")"
    if [ -d "$SHUTTLE/$id" ]; then
      rm -rf "$dest"
      cp -a "$SHUTTLE/$id" "$dest"
    else
      cp -a "$SHUTTLE/$id" "$dest"
    fi
    echo "    restored $relpath"
  done < "$TMPDIR/preserve.map"
fi

# --- Stamp sync-point provenance into core/core.yaml ------------------------
# Default mode only — in narrow mode we only overlaid a subset of top-level
# entries, so claiming "the HQ root is at <sha>" would be misleading. yq
# is preferred (clean in-place edit, preserves comments mediocrely but well
# enough). Falls back to a python3+PyYAML one-liner if yq is missing.
if [ "${#NARROW_PATHS[@]}" -eq 0 ] && [ -f "$HQ_ROOT/core/core.yaml" ]; then
  NOW_UTC="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  if command -v yq >/dev/null 2>&1; then
    SHA="$SRC_SHA" SOURCE="$SOURCE_REPO" THE_REF="$REF" AT="$NOW_UTC" \
      yq -i '
        .replaced_from_staging.source       = strenv(SOURCE) |
        .replaced_from_staging.ref          = strenv(THE_REF) |
        .replaced_from_staging.last_sync_sha = strenv(SHA) |
        .replaced_from_staging.last_sync_at  = strenv(AT)
      ' "$HQ_ROOT/core/core.yaml"
    echo "==> Stamped core/core.yaml: replaced_from_staging.last_sync_sha=$SRC_SHA"
  elif command -v python3 >/dev/null 2>&1 && python3 -c 'import yaml' >/dev/null 2>&1; then
    SHA="$SRC_SHA" SOURCE="$SOURCE_REPO" THE_REF="$REF" AT="$NOW_UTC" CORE="$HQ_ROOT/core/core.yaml" \
      python3 -c '
import os, yaml
path = os.environ["CORE"]
try:
    with open(path) as f:
        d = yaml.safe_load(f) or {}
except FileNotFoundError:
    d = {}
d["replaced_from_staging"] = {
    "source": os.environ["SOURCE"],
    "ref": os.environ["THE_REF"],
    "last_sync_sha": os.environ["SHA"],
    "last_sync_at": os.environ["AT"],
}
with open(path, "w") as f:
    yaml.safe_dump(d, f, default_flow_style=False, sort_keys=False)
'
    echo "==> Stamped core/core.yaml: replaced_from_staging.last_sync_sha=$SRC_SHA"
  else
    echo "    WARN: neither yq nor python3+PyYAML available — skipping core/core.yaml stamp" >&2
  fi
fi

echo ""
echo "==> File count summary:"
for root_rel in "${WIPE_TOPLEVEL[@]}"; do
  if [ -e "$HQ_ROOT/$root_rel" ]; then
    n_files="$(find "$HQ_ROOT/$root_rel" -type f 2>/dev/null | wc -l | tr -d ' ')"
    echo "    $root_rel: $n_files files"
  fi
done
n_bucket="$(find "$HQ_ROOT/$RESCUE_BUCKET" -type f 2>/dev/null | wc -l | tr -d ' ')"
echo "    $RESCUE_BUCKET: $n_bucket files (drifts that had no personal/ home)"
if [ "$HISTORY_CHECK" = "1" ]; then
  echo "    skipped by history gate: $SKIPPED_BY_HISTORY (local content matched a past staging blob)"
fi

echo ""
echo "==> Done. Source: $SOURCE_REPO@$REF ($SRC_SHA)"
echo "    Drifts moved into personal/ (see scan output above)."
echo "    Review $RESCUE_BUCKET/ for files that need manual reconciliation."
