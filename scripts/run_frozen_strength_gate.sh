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
games_per_position=$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["protocol"]["games_per_position"])' "$MANIFEST")

python3 scripts/gate_resource_preflight.py --parallel 1 --threads 1 --spec-top-n 0 --contention-job renkin

run_id=${RUN_ID:-next_candidate_seed42_formal_gate}
exec env RUN_ID="$run_id" MAX_GAMES=400 \
  bash scripts/sprint_gate.sh "$candidate" "$baseline" 4 "$games_per_position"
