#!/usr/bin/env bash
set -euo pipefail

root_dir=$(cd "$(dirname "$0")/.." && pwd)
run_dir=$(mktemp -d "${TMPDIR:-/tmp}/sekirei-resume.XXXXXX")
trap 'rm -rf "$run_dir"' EXIT

fixture="$root_dir/scripts/fixtures/nnue_phase3_pilot.jsonl"
common=(--offline -p sekirei-train -- --positions "$fixture" --epochs 2 --sample 1 --label-depth 1 --lr 0.01 --init-seed 7 --split-seed 42)

cargo run "${common[@]}" --output "$run_dir/interrupted.bin" --resume-checkpoint-every-games 2 --stop-after-resume-checkpoint >"$run_dir/interrupted.log" 2>&1
resume="$run_dir/interrupted.resume.json"
test -s "$resume"
python3 -c 'import json, sys; d=json.load(open(sys.argv[1])); assert d["schema"] == "sekirei.resume-checkpoint.v1"; assert d["epoch_completed"] == 0; assert d["next_game_index"] == 2; assert len(d["teacher_cache"]) == 2; assert d["optimizer"]["schema"] == "sekirei.adam-checkpoint.v1"' "$resume"
python3 scripts/record_resume_run.py --checkpoint "$resume" --log "$run_dir/interrupted.log" --dataset scripts/fixtures/nnue_phase3_pilot.jsonl --output "$run_dir/resume-manifest.json" >/dev/null
python3 scripts/validate_resume_manifest.py "$run_dir/resume-manifest.json" >/dev/null

cargo run "${common[@]}" --output "$run_dir/resumed.bin" --resume-checkpoint "$resume" >/dev/null 2>&1
if cargo run "${common[@]}" --lr 0.011 --output "$run_dir/mismatch.bin" --resume-checkpoint "$resume" >/dev/null 2>&1; then
  echo "resume CLI fixture failed: recipe mismatch was accepted" >&2
  exit 1
fi
cargo run "${common[@]}" --output "$run_dir/clean.bin" >/dev/null 2>&1

cmp "$run_dir/resumed.bin" "$run_dir/clean.bin"
echo "resume CLI fixture OK: resumed weights are byte-identical to clean run"
