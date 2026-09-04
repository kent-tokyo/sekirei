#!/usr/bin/env bash
# Reproducible, resume-aware fixed-NNUE self-distillation run.
#
# The cold cache warm-up is separated from the three candidate runs.  This
# keeps seed 7/42/123 comparable: all candidates start from epoch 1 with the
# same complete teacher-label cache and therefore pay zero teacher searches.
set -euo pipefail

REPO_ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
cd "$REPO_ROOT"

RUN_DIR=${1:-data/runs/self_distill_nnue_teacher_20260904}
TRAIN_BIN=${TRAIN_BIN:-target/release/train}
GAMES_DIR=${GAMES_DIR:-data/gateA_csa_subset}
TEACHER=${TEACHER:-data/runs/nnue_v1_tier2/selected/official_nnue_v1_candidate.bin}
EXPECTED_TEACHER_SHA256=e4da09316ef8e5892ea58f1a338b13851ff9db54b11b5634aac2492fd05d8da4
CACHE="$RUN_DIR/teacher_cache_depth2.jsonl"
SHARDS=${SHARDS:-8}

mkdir -p "$RUN_DIR"

if [[ ! "$SHARDS" =~ ^[1-9][0-9]*$ ]]; then
  echo "error: SHARDS must be a positive integer: $SHARDS" >&2
  exit 1
fi
if [[ ! -x "$TRAIN_BIN" ]]; then
  echo "error: training binary is missing or not executable: $TRAIN_BIN" >&2
  exit 1
fi
if [[ ! -d "$GAMES_DIR" ]]; then
  echo "error: CSA directory is missing: $GAMES_DIR" >&2
  exit 1
fi
actual_teacher_sha=$(shasum -a 256 "$TEACHER" | awk '{print $1}')
if [[ "$actual_teacher_sha" != "$EXPECTED_TEACHER_SHA256" ]]; then
  echo "error: teacher SHA-256 mismatch: $actual_teacher_sha" >&2
  exit 1
fi

common_args=(
  --label-depth 2
  --label-nodes 250000
  --teacher-eval nnue
  --teacher-weights "$TEACHER"
  --wdl-lambda 0.7
  --min-ply 20
  --min-rate 1800
  --validation-ratio 0.15
  --split-seed 42
  --lr 0.001
  --lr-schedule constant
  --lr-schedule-epochs 20
)

run_logged() {
  local log_path=$1
  shift
  "$@" 2>&1 | tee -a "$log_path"
}

# Populate every train and validation label once. Depth 2 remains below the
# searcher's depth-3 parallel split, so independent CSA-file shards use one
# deterministic search thread each. Mid-epoch snapshots include each shard's
# in-memory cache, preserving completed teacher searches across interruption.
if [[ ! -s "$CACHE" ]]; then
  shard_root="$RUN_DIR/cache_warmup_shards"
  mkdir -p "$shard_root"
  file_index=0
  while IFS= read -r source_file; do
    shard_index=$((file_index % SHARDS))
    shard_dir="$shard_root/shard$shard_index"
    mkdir -p "$shard_dir/games" "$shard_dir/checkpoints"
    if [[ "$source_file" = /* ]]; then
      source_abs=$source_file
    else
      source_abs="$REPO_ROOT/$source_file"
    fi
    ln -sf "$source_abs" "$shard_dir/games/$(basename "$source_file")"
    file_index=$((file_index + 1))
  done < <(find "$GAMES_DIR" -maxdepth 1 -type f -name '*.csa' -print | sort)
  if [[ "$file_index" -eq 0 ]]; then
    echo "error: no CSA files found in $GAMES_DIR" >&2
    exit 1
  fi
  if ((SHARDS > file_index)); then
    echo "error: SHARDS ($SHARDS) exceeds CSA file count ($file_index)" >&2
    exit 1
  fi

  pids=()
  shard_caches=()
  for ((shard_index = 0; shard_index < SHARDS; shard_index++)); do
    shard_dir="$shard_root/shard$shard_index"
    shard_cache="$shard_dir/teacher_cache.jsonl"
    shard_caches+=("$shard_cache")
    if [[ -s "$shard_cache" ]]; then
      echo "cache shard $shard_index already complete"
      continue
    fi
    warm_output="$shard_dir/warmup.bin"
    warm_resume="$shard_dir/warmup.resume.json"
    warm_args=(
      --games "$shard_dir/games"
      "${common_args[@]}"
      --epochs 1
      --init-seed 7
      --shuffle-seed 7
      --teacher-cache "$shard_cache"
      --resume-checkpoint-every-games 5
      --checkpoint-dir "$shard_dir/checkpoints"
      --output "$warm_output"
    )
    if [[ -s "$warm_resume" ]]; then
      warm_args+=(--resume-checkpoint "$warm_resume")
      echo "resuming cache shard $shard_index"
    else
      echo "starting cache shard $shard_index"
    fi
    ("$TRAIN_BIN" "${warm_args[@]}" >>"$shard_dir/training.log" 2>&1) &
    pids+=("$!")
  done
  failures=0
  for pid in "${pids[@]}"; do
    if ! wait "$pid"; then
      failures=$((failures + 1))
    fi
  done
  if [[ "$failures" -ne 0 ]]; then
    echo "error: $failures cache shard(s) failed; rerun to resume" >&2
    exit 1
  fi
  python3 scripts/merge_teacher_caches.py --output "$CACHE" "${shard_caches[@]}"
fi

if [[ ! -s "$CACHE" ]]; then
  echo "error: cache warm-up finished without producing $CACHE" >&2
  exit 1
fi

for seed in 7 42 123; do
  seed_dir="$RUN_DIR/seed$seed"
  output="$seed_dir/candidate.bin"
  mkdir -p "$seed_dir/checkpoints"
  if [[ -s "$output" ]]; then
    echo "seed $seed already complete: $output"
    continue
  fi

  seed_args=(
    --games "$GAMES_DIR"
    "${common_args[@]}"
    --epochs 20
    --init-seed "$seed"
    --shuffle-seed "$seed"
    --teacher-cache "$CACHE"
    --reuse-teacher-cache
    --checkpoint-dir "$seed_dir/checkpoints"
    --output "$output"
  )
  latest_resume=$(find "$seed_dir/checkpoints" -type f -name '*.resume.json' -print | sort -V | tail -n 1 || true)
  if [[ -n "$latest_resume" ]]; then
    seed_args+=(--resume-checkpoint "$latest_resume")
    echo "resuming seed $seed from $latest_resume"
  fi
  run_logged "$seed_dir/training.log" "$TRAIN_BIN" "${seed_args[@]}"
done

python3 scripts/summarize_self_distill.py "$RUN_DIR" \
  --teacher "$TEACHER" --output "$RUN_DIR/selection_manifest.json"

echo "self-distillation multi-seed run complete: $RUN_DIR"
