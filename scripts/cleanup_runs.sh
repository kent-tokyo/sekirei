#!/usr/bin/env bash
# Prune stage1/stage2/stage3 intermediate files from completed data/runs/*
# pipeline directories (redo_quietset_bc.sh / train_with_loss_mining.sh /
# train_with_shogiesa_quietset.sh). A run is "completed" once it has a
# manifest.json (written by those scripts as their last step) -- by then
# every stageN file has already been consumed into the run's real output
# (data/weights_*.bin, checkpoints/), so the raw/intermediate copies
# (extracted positions, label observations, scored jsonl -- often multi-GB)
# are pure disk cost with no further use.
#
# Skips:
#   - any run directory with no manifest.json (still running, or ad-hoc --
#     not safe to assume it's done)
#   - any run directory referenced by name in a shell or Python file under
#     scripts/. For example, train_with_loss_mining.sh's BASE_POSITIONS and
#     BASE_SCORED may point at derived files from an older run. Those are live
#     cross-run dependencies even if the older run has finished.
#   - runs newer than MIN_AGE_DAYS, so a just-finished run stays inspectable
#     (e.g. to debug a gate failure) before its stage dirs disappear
#
# Default: dry run (prints what would be deleted). Set APPLY=1 to actually delete.
#
# Environment:
#   APPLY=1          actually delete (default: 0, dry-run only)
#   MIN_AGE_DAYS=3   only touch runs whose manifest.json is older than this (default: 3)
#
# Usage: bash scripts/cleanup_runs.sh
set -e

APPLY=${APPLY:-0}
MIN_AGE_DAYS=${MIN_AGE_DAYS:-3}
RUNS_DIR=data/runs
SCRIPT_DIR="$(dirname "$0")"

[ -d "$RUNS_DIR" ] || exit 0

PROTECTED=$(
  find "$SCRIPT_DIR" -type f \( -name '*.sh' -o -name '*.py' \) \
    -exec grep -ohE 'data/runs/[A-Za-z0-9_.-]+' {} + 2>/dev/null \
    | sed 's#data/runs/##' | sort -u
)

is_protected() {
  printf '%s\n' "$PROTECTED" | grep -qxF "$1"
}

for run in "$RUNS_DIR"/*/; do
  [ -d "$run" ] || continue
  name=$(basename "$run")

  if is_protected "$name"; then
    echo "skip (protected, referenced by a script): $name"
    continue
  fi

  if [ ! -f "$run/manifest.json" ]; then
    echo "skip (no manifest.json -- still running or ad-hoc): $name"
    continue
  fi

  if find "$run/manifest.json" -mtime "-${MIN_AGE_DAYS}" -print -quit | grep -q .; then
    echo "skip (finished less than ${MIN_AGE_DAYS}d ago): $name"
    continue
  fi

  for stage in stage1 stage2 stage3; do
    [ -d "$run/$stage" ] || continue
    sz=$(du -sh "$run/$stage" 2>/dev/null | cut -f1)
    if [ "$APPLY" = "1" ]; then
      rm -rf "${run:?}/$stage"
      echo "deleted ($sz): $run$stage"
    else
      echo "would delete ($sz): $run$stage  [dry-run, set APPLY=1 to actually delete]"
    fi
  done
done
