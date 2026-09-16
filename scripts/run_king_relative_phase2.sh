#!/usr/bin/env bash
# Phase 2 of the king-relative NNUE experiment (docs/design/nnue_architecture_next_candidate.md,
# PR #41): architecture A (flat baseline) vs architecture B-small
# (king_relative_b_small, 9-zone) x 3 seeds, same corpus/teacher-cache/split
# for comparability -- same shape as scripts/run_longrun_conflict_mask.sh's
# precedent (2 arms x 3 seeds), with one structural difference: the two
# arms here are two BUILD-time Cargo-feature variants (two distinct
# binaries), not two runtime flags on one binary, since `king_relative_b_small`
# changes NNUE's compile-time INPUT dimension. --build produces both
# binaries up front; --resume never rebuilds mid-run.
#
# Runs are SEQUENTIAL, not parallel -- same reason as the precedent script:
# all runs write back to the same --teacher-cache file at the end of their
# own epoch 1, and concurrent non-atomic overwrites of that file are a
# hazard even though teacher_cache::write is itself atomic (write-then-rename).
#
# NOT run as part of writing this script. Before --resume, run the
# standing 5-signal resource check (no competing heavy job actually
# exited, load average settled, memory_pressure/vm_stat normal, no
# still-climbing swap, no leftover sekirei-train processes from a prior
# attempt) -- this is real, sustained CPU/RSS work (~10x heavier per-run
# on the B-small side, per docs/design/nnue_architecture_next_candidate.md's
# sizing note), not something to launch opportunistically.
set -uo pipefail

GAMES=data/gateA_csa_subset
CACHE=data/teacher_cache_depth4.jsonl
OUT_DIR=data/runs/king_relative_phase2
SEEDS="42 7 123"
EPOCHS=20
BIN_A=target/release/train_arch_a
BIN_B=target/release/train_arch_b

mkdir -p "$OUT_DIR"

cache_line_count() {
  [ -f "$CACHE" ] && wc -l < "$CACHE" | tr -d ' ' || echo 0
}

# Builds both architecture binaries and copies each to a distinct path --
# `cargo build`'s own `target/release/train` output is overwritten by
# whichever build ran last, so each variant must be copied out immediately
# after its own build, before the other one starts.
build_binaries() {
  echo "=== [$(date +%H:%M:%S)] building architecture A (flat baseline) ==="
  cargo build --release -p sekirei-train || return 1
  cp target/release/train "$BIN_A"

  echo "=== [$(date +%H:%M:%S)] building architecture B-small (king_relative_b_small) ==="
  cargo build --release -p sekirei-train --features king_relative_b_small || return 1
  cp target/release/train "$BIN_B"

  echo "=== both binaries built: $BIN_A, $BIN_B ==="
}

# Runs the trainer once against one architecture binary: writes a manifest,
# tees the full log, returns the process's own exit code. No self-check,
# no .done marker -- run_one (below) owns pass/fail handling, same division
# of responsibility as the precedent script.
run_training() {
  local arch=$1 bin=$2 seed=$3
  local stem="$OUT_DIR/${arch}_seed${seed}"

  echo "=== [$(date +%H:%M:%S)] arch=$arch seed=$seed epochs=$EPOCHS -> $stem starting ==="
  cat > "${stem}.manifest.json" <<EOF
{
  "arch": "$arch",
  "binary": "$bin",
  "seed": $seed,
  "epochs": $EPOCHS,
  "games": "$GAMES",
  "teacher_cache": "$CACHE",
  "pre_run_cache_line_count": $(cache_line_count),
  "started_at": "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
}
EOF

  "$bin" \
    --games "$GAMES" \
    --output "${stem}.bin" \
    --epochs "$EPOCHS" \
    --label-depth 4 \
    --wdl-lambda 0.7 \
    --validation-ratio 0.15 \
    --split-seed 42 \
    --shuffle-seed 11 \
    --init-seed "$seed" \
    --teacher-cache "$CACHE" \
    --reuse-teacher-cache \
    2>&1 | tee "${stem}.log"
  return "${PIPESTATUS[0]}"
}

# Full-job wrapper: run_training + completion check + .done marker +
# resume-skip. Lighter than the precedent script's check_longrun_meta.py
# (that checker is specific to conflict-mask diagnostics this experiment
# doesn't have) -- just confirms the final epoch's meta.json exists and
# parses, deliberately deferring "is this architecture actually better"
# entirely to Phase 3's comparison step, not this launcher.
run_one() {
  local arch=$1 bin=$2 seed=$3
  local stem="$OUT_DIR/${arch}_seed${seed}"

  if [ -f "${stem}.done" ]; then
    echo "=== [$(date +%H:%M:%S)] arch=$arch seed=$seed already done (${stem}.done exists) -- skipping ==="
    return 0
  fi

  run_training "$arch" "$bin" "$seed"
  local exit_code=$?
  if [ "$exit_code" -ne 0 ]; then
    echo "=== [$(date +%H:%M:%S)] arch=$arch seed=$seed FAILED (exit $exit_code) -- no .done marker written ==="
    return 1
  fi

  local final_meta="${stem}.epoch${EPOCHS}.meta.json"
  if [ ! -f "$final_meta" ] || ! python3 -c "import json,sys; json.load(open(sys.argv[1]))" "$final_meta" 2>/dev/null; then
    echo "=== [$(date +%H:%M:%S)] arch=$arch seed=$seed: $final_meta missing or not valid JSON -- no .done marker written ==="
    return 1
  fi

  date -u +%Y-%m-%dT%H:%M:%SZ > "${stem}.done"
  echo "=== [$(date +%H:%M:%S)] arch=$arch seed=$seed complete -> ${stem}.done ==="
}

mode=${1:-}

if [ "$mode" = "--build" ]; then
  build_binaries

elif [ "$mode" = "--resume" ]; then
  [ -x "$BIN_A" ] || { echo "error: $BIN_A missing -- run '$0 --build' first"; exit 1; }
  [ -x "$BIN_B" ] || { echo "error: $BIN_B missing -- run '$0 --build' first"; exit 1; }
  for seed in $SEEDS; do
    run_one arch_a "$BIN_A" "$seed" || exit 1
  done
  for seed in $SEEDS; do
    run_one arch_b "$BIN_B" "$seed" || exit 1
  done
  echo "=== all 6 runs complete ==="

else
  echo "usage: $0 --build | --resume"
  echo "  --build    build both architecture binaries ($BIN_A, $BIN_B)"
  echo "  --resume   run the full 6-run x $EPOCHS-epoch job (skips any run with an existing .done marker)"
  exit 1
fi
