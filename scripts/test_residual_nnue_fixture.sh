#!/usr/bin/env bash
set -euo pipefail

root_dir=$(cd "$(dirname "$0")/.." && pwd)
run_dir=$(mktemp -d "${TMPDIR:-/tmp}/sekirei-residual-nnue.XXXXXX")
trap 'rm -rf "$run_dir"' EXIT
cd "$root_dir"

fixture="scripts/fixtures/nnue_phase3_pilot.jsonl"
output="$run_dir/residual.bin"

# This is a contract smoke test, not a strength measurement: six fixed
# positions, one epoch, and the default material teacher.
cargo run --offline -p sekirei-train -- \
  --positions "$fixture" --strict-positions --epochs 1 --sample 1 \
  --label-depth 1 --lr 0.01 --init-seed 7 --split-seed 42 \
  --validation-ratio 0.5 --nnue-output residual-material --output "$output" \
  >"$run_dir/train.log" 2>&1

python3 - "$output" <<'PY'
import json
import sys

for output in (sys.argv[1], sys.argv[1].replace('.bin', '.best.bin')):
    meta = json.load(open(output.rsplit('.', 1)[0] + '.meta.json'))
    assert meta['format'] == 'sekirei-nnue-output-v1'
    assert meta['nnue_output'] == 'residual-material'
    assert meta['baseline'] == 'material-v1'
    assert len(meta['checkpoint_hash']) == 16
PY

# Residual targets have no compatible absolute WDL target yet.  Refusing the
# combination is the fail-closed boundary that prevents a mixed objective.
if cargo run --offline -p sekirei-train -- \
  --positions "$fixture" --epochs 1 --nnue-output residual-material \
  --wdl-lambda 0.5 --output "$run_dir/invalid.bin" \
  >"$run_dir/wdl-mismatch.log" 2>&1; then
  echo "residual NNUE fixture failed: mixed WDL objective was accepted" >&2
  exit 1
fi
grep -F -- "--nnue-output residual-material does not support --wdl-lambda" \
  "$run_dir/wdl-mismatch.log" >/dev/null

echo "residual NNUE fixture OK: output metadata and objective boundary verified"
