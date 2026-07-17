#!/usr/bin/env bash
# Stage 1 of the teacher-conflict-masking long-run + gate pipeline
# (docs/experiments/teacher_conflict_masking.md's follow-up): 2 arms
# (Control, Conflict-mask-FT) x 3 seeds x 20 epochs, all fixed to the same
# recipe as the short-run experiment so the two stay comparable:
# --wdl-lambda 0.7, data/gateA_csa_subset, --split-seed 42, --shuffle-seed 11,
# shared teacher cache, default --lr-schedule (step-half, unset),
# --validation-ratio 0.15 (matching l2_saturation_order_sensitivity_p1.md,
# the closest ancestor recipe using this same dataset/lambda/split-seed).
#
# Two-phase, not one shot:
#   --warmup-validation   Runs ONLY control_seed42, capped at 2 epochs
#                          (epoch 1 builds the teacher-search cache from
#                          cold -- ~2.7-2.9h one-time cost measured on this
#                          dataset; epoch 2 should be near-instant on a
#                          warm cache). Stops there and runs the full
#                          check_warmup_validation.py breakdown regardless
#                          of whether the training process itself looked
#                          clean -- diagnostic detail should never be
#                          hidden behind an early short-circuit. Does NOT
#                          touch the other 5 runs.
#   --resume               Runs the full set: control_seed42 at the full
#                          20 epochs (deterministically re-derives its own
#                          epoch 1-2 in seconds now that the cache is warm,
#                          identical seeds/flags to the warmup pass, then
#                          continues net-new to epoch 20), followed by
#                          control_seed{7,123} and conflict_ft_seed{42,7,123}.
#                          Every run is skipped if it already has a .done
#                          marker, so a second --resume after an
#                          interruption picks up where it left off.
#
# Runs are SEQUENTIAL, not parallel: all runs write back to the same
# --teacher-cache file at the end of their own epoch 1; concurrent
# non-atomic overwrites of that file could corrupt it (teacher_cache::write
# is itself now atomic via write-then-rename, but two writers racing to
# rename the same path is still a hazard the sequential-only design avoids
# entirely rather than relying on that alone).
set -uo pipefail

BIN=./target/release/train
GAMES=data/gateA_csa_subset
CACHE=data/teacher_cache_depth4.jsonl
OUT_DIR=data/runs/20260717_longrun_conflict_mask
SEEDS="42 7 123"
FULL_EPOCHS=20
WARMUP_EPOCHS=2

mkdir -p "$OUT_DIR"

cache_line_count() {
  [ -f "$CACHE" ] && wc -l < "$CACHE" | tr -d ' ' || echo 0
}

# Runs the trainer once: writes a manifest, tees the full log, returns the
# process's own exit code. No self-verification, no .done marker -- callers
# decide what "passed" means (run_one's own bar for the real 6-run job vs.
# --warmup-validation's more detailed check_warmup_validation.py).
run_training() {
  local arm=$1 seed=$2 epochs=$3 extra_flags=$4
  local stem="$OUT_DIR/${arm}_seed${seed}"

  echo "=== [$(date +%H:%M:%S)] arm=$arm seed=$seed epochs=$epochs -> $stem starting ==="
  cat > "${stem}.manifest.json" <<EOF
{
  "arm": "$arm",
  "seed": $seed,
  "epochs": $epochs,
  "games": "$GAMES",
  "teacher_cache": "$CACHE",
  "extra_flags": "$extra_flags",
  "pre_run_cache_line_count": $(cache_line_count),
  "started_at": "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
}
EOF

  # shellcheck disable=SC2086
  "$BIN" \
    --games "$GAMES" \
    --output "${stem}.bin" \
    --epochs "$epochs" \
    --lr-schedule-epochs "$FULL_EPOCHS" \
    --label-depth 4 \
    --wdl-lambda 0.7 \
    --validation-ratio 0.15 \
    --split-seed 42 \
    --shuffle-seed 11 \
    --init-seed "$seed" \
    --teacher-cache "$CACHE" \
    --reuse-teacher-cache \
    $extra_flags \
    2>&1 | tee "${stem}.log"
  return "${PIPESTATUS[0]}"
}

# Full-job wrapper: run_training + self-check + .done marker + resume-skip.
# Used only by --resume, never by --warmup-validation (see run_training's
# doc comment for why the two need different pass/fail handling).
run_one() {
  local arm=$1 seed=$2 epochs=$3 extra_flags=$4
  local stem="$OUT_DIR/${arm}_seed${seed}"
  local expect_mask="none"
  [ "$arm" = "conflict_ft" ] && expect_mask="ft"

  if [ -f "${stem}.done" ]; then
    echo "=== [$(date +%H:%M:%S)] arm=$arm seed=$seed already done (${stem}.done exists) -- skipping ==="
    return 0
  fi

  run_training "$arm" "$seed" "$epochs" "$extra_flags"
  local exit_code=$?
  if [ "$exit_code" -ne 0 ]; then
    echo "=== [$(date +%H:%M:%S)] arm=$arm seed=$seed FAILED (exit $exit_code) -- no .done marker written ==="
    return 1
  fi

  local final_meta="${stem}.epoch${epochs}.meta.json"
  if [ ! -f "$final_meta" ]; then
    echo "=== [$(date +%H:%M:%S)] arm=$arm seed=$seed: $final_meta missing -- no .done marker written ==="
    return 1
  fi

  if python3 scripts/check_longrun_meta.py single "$final_meta" "$expect_mask"; then
    date -u +%Y-%m-%dT%H:%M:%SZ > "${stem}.done"
    echo "=== [$(date +%H:%M:%S)] arm=$arm seed=$seed complete, self-verified -> ${stem}.done ==="
  else
    echo "=== [$(date +%H:%M:%S)] arm=$arm seed=$seed: self-verification FAILED -- no .done marker written ==="
    return 1
  fi
}

mode=${1:-}

if [ "$mode" = "--warmup-validation" ]; then
  stem="$OUT_DIR/control_seed42"
  pre_run_count=$(cache_line_count)
  echo "pre-run cache line count: $pre_run_count"

  run_training control 42 "$WARMUP_EPOCHS" ""
  train_exit=$?
  if [ "$train_exit" -ne 0 ]; then
    echo "=== [$(date +%H:%M:%S)] warmup training process itself failed (exit $train_exit) ==="
  fi

  if [ ! -f "${stem}.epoch1.meta.json" ] || [ ! -f "${stem}.epoch2.meta.json" ]; then
    echo "=== epoch1/epoch2 meta.json missing -- cannot run the Phase A check ==="
    exit 1
  fi

  echo "=== running Phase A pass/fail check ==="
  python3 scripts/check_warmup_validation.py \
    "${stem}.epoch1.meta.json" "${stem}.epoch2.meta.json" \
    "$pre_run_count" "$CACHE"
  check_exit=$?

  # A capped 2-epoch warmup run must never be treated as satisfying the
  # real 20-epoch target -- --resume always re-runs control_seed42 at the
  # full epoch count regardless of what happened here.
  rm -f "${stem}.done"
  exit $check_exit

elif [ "$mode" = "--resume" ]; then
  for seed in $SEEDS; do
    run_one control "$seed" "$FULL_EPOCHS" "" || exit 1
  done
  for seed in $SEEDS; do
    run_one conflict_ft "$seed" "$FULL_EPOCHS" "--diagnostic-conflict-mask ft" || exit 1
  done
  echo "=== all 6 runs complete ==="

else
  echo "usage: $0 --warmup-validation | --resume"
  echo "  --warmup-validation  control_seed42 only, capped at $WARMUP_EPOCHS epochs, then check_warmup_validation.py"
  echo "  --resume             full 6-run x $FULL_EPOCHS-epoch job (skips any run with an existing .done marker)"
  exit 1
fi
