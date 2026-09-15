#!/usr/bin/env bash
# Resume the frozen seed-42 strength gate only when its manifest and resources
# are valid. This wrapper never bypasses the resource preflight.
set -euo pipefail

ROOT=$(cd "$(dirname "$0")/.." && pwd)
MANIFEST=${1:-"$ROOT/results/next_candidate_seed42_strength_gate_plan.json"}

cd "$ROOT"
python3 scripts/validate_strength_gate_manifest.py "$MANIFEST"

candidate=$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["candidate"]["path"])' "$MANIFEST")
baseline=$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["baseline"]["path"])' "$MANIFEST")
openings=$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["openings"]["path"])' "$MANIFEST")
games_per_position=$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["protocol"]["games_per_position"])' "$MANIFEST")
max_games=$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["protocol"]["max_games"])' "$MANIFEST")
byoyomi_ms=$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["protocol"]["byoyomi_ms"])' "$MANIFEST")
search_mode=$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["protocol"]["engine_options"]["SearchMode"])' "$MANIFEST")
candidate_nnue_output=$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["evaluation"]["candidate"]["mode"])' "$MANIFEST")
baseline_nnue_output=$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["evaluation"]["baseline"]["mode"])' "$MANIFEST")

plan_stem=$(basename "$MANIFEST" .json)
run_id=${RUN_ID:-"$plan_stem"}
outdir=${GATE_OUTDIR:-"$ROOT/results/strength_gate_runs/$run_id"}
mkdir -p "$outdir"
if [ -e "$outdir/plan.json" ]; then
  cmp -s "$MANIFEST" "$outdir/plan.json" || {
    echo "error: existing run directory has a different frozen plan" >&2
    exit 2
  }
else
  cp "$MANIFEST" "$outdir/plan.json"
fi

python3 scripts/gate_resource_preflight.py --parallel 1 --threads 1 --spec-top-n 0 \
  --contention-job renkin --output "$outdir/preflight.json"

common_args=(
  --outdir "$outdir" --threads 1 --parallel 1 --byoyomi "$byoyomi_ms"
  --shard-positions 1 --max-positions 200 --engine-bin ./target/release/sekirei
  --corpus "$openings" --option1 "EvalFile=$candidate" --option2 "EvalFile=$baseline"
  --option1 "NnueOutput=$candidate_nnue_output" --option2 "NnueOutput=$baseline_nnue_output"
  --option1 "Threads=1" --option1 "SpecTopN=0" --option1 "UseBook=false" --option1 "SearchMode=$search_mode"
  --option2 "Threads=1" --option2 "SpecTopN=0" --option2 "UseBook=false" --option2 "SearchMode=$search_mode"
  --elo0 0 --elo1 20 --alpha 0.05 --beta 0.05
)
python3 scripts/gate_orchestrator.py run "${common_args[@]}" --initialize-only
if [ ! -e "$outdir/execution.json" ]; then
  python3 scripts/record_strength_gate_execution.py --plan "$outdir/plan.json" \
    --binary ./target/release/sekirei --preflight "$outdir/preflight.json" \
    --state "$outdir/state.json" --output "$outdir/execution.json"
fi

if [ "${PREPARE_ONLY:-0}" = "1" ]; then
  echo "prepared frozen gate evidence at $outdir; no shard launched"
  exit 0
fi

set +e
# This runner can outlive the interactive terminal that started it.  Keep the
# orchestrator's progress output in the run directory rather than in that
# terminal's pipe: an unread pipe can fill and block the parent even after a
# shard has written its durable JSON/JSONL result files.
python3 scripts/gate_orchestrator.py run "${common_args[@]}" \
  >> "$outdir/orchestrator.log" 2>&1
gate_rc=$?
set -e
python3 scripts/finalize_strength_gate_execution.py --execution-manifest "$outdir/execution.json" \
  --state "$outdir/state.json" --output "$outdir/final.json"
exit "$gate_rc"
