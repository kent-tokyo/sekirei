#!/usr/bin/env bash
# Redo quietset B/C experiment with full game coverage at depths 2,4.
#
# B = --min-stability 0.85  (hard filter, was weights_keep085)
# C = --stability-weighted  (soft weighting, was weights_weighted)
#
# Usage:
#   bash scripts/redo_quietset_bc.sh [CSA_DIR] [BASELINE_WEIGHTS]
#
# Environment:
#   OUT_B=path          output path for B weights (default: data/weights_v8_keep085.bin)
#   OUT_C=path          output path for C weights (default: data/weights_v8_weighted.bin)
#   GAMES=100           match games per variant   (default: 100)
#   MIN_PLY=20          minimum ply for extract   (default: 20)
#   MAX_PLY=160         maximum ply for extract   (default: 160)
#   EVERY_N_PLIES=16    extract every N plies     (default: 16)
#   MAX_POSITIONS=200000 cap positions after extract (default: 200000)
#   SHOGIESA=path       path to shogiesa binary   (default: auto-detect)
#
# Exit: 0 if both B and C pass the Elo gate, non-zero otherwise
set -e

CSA_DIR=${1:-./data/csa}
BASELINE=${2:-data/weights_v7.bin}
OUT_B=${OUT_B:-data/weights_v8_keep085.bin}
OUT_C=${OUT_C:-data/weights_v8_weighted.bin}
GAMES=${GAMES:-100}
MIN_PLY=${MIN_PLY:-20}
MAX_PLY=${MAX_PLY:-160}
EVERY_N_PLIES=${EVERY_N_PLIES:-16}
MAX_POSITIONS=${MAX_POSITIONS:-200000}
TIMESTAMP=$(date +%Y%m%d_%H%M%S)
RUN_DIR="data/runs/bc_redo_$TIMESTAMP"

# auto-detect shogiesa binary
if [ -z "$SHOGIESA" ]; then
  if command -v shogiesa >/dev/null 2>&1; then
    SHOGIESA=shogiesa
  elif [ -x "/Users/k_tanabe/Documents/Documents/oss_rust/shogiesa/target/release/shogiesa" ]; then
    SHOGIESA=/Users/k_tanabe/Documents/Documents/oss_rust/shogiesa/target/release/shogiesa
  else
    echo "error: shogiesa not found; set SHOGIESA=/path/to/shogiesa"; exit 127
  fi
fi
command -v "$SHOGIESA" >/dev/null 2>&1 || { echo "error: shogiesa not found at $SHOGIESA"; exit 127; }
command -v quietset >/dev/null 2>&1 || { echo "error: quietset not found"; exit 127; }
command -v cargo    >/dev/null 2>&1 || { echo "error: cargo not found";    exit 127; }
[ -d "$CSA_DIR"  ] || { echo "error: CSA dir not found: $CSA_DIR";   exit 1; }
[ -f "$BASELINE" ] || { echo "error: baseline not found: $BASELINE"; exit 1; }

echo "=== quietset B/C redo (depths 2,4, full coverage) ==="
echo "  CSA dir  : $CSA_DIR"
echo "  baseline : $BASELINE"
echo "  out B    : $OUT_B"
echo "  out C    : $OUT_C"
echo "  run dir  : $RUN_DIR"
echo ""

mkdir -p "$RUN_DIR"/{stage1,stage2,stage3,checkpoints_b,checkpoints_c} results
cargo build --release -q -p sekirei -p sekirei-train -p sekirei-match-runner

# ---- Stage 1: extract -------------------------------------------------------
echo "[1/5] shogiesa extract  (min-ply=$MIN_PLY max-ply=$MAX_PLY every-n-plies=$EVERY_N_PLIES max=$MAX_POSITIONS)"
# shogiesa does not recurse into subdirectories; run per subdir and merge.
_extract_dirs=()
for d in "$CSA_DIR"/*/; do
  [ -d "$d" ] && _extract_dirs+=("$d")
done
[ ${#_extract_dirs[@]} -eq 0 ] && _extract_dirs=("$CSA_DIR")

for _dir in "${_extract_dirs[@]}"; do
  _slug=$(basename "$_dir")
  "$SHOGIESA" extract \
    --input "$_dir" \
    --out "$RUN_DIR/stage1/pos_${_slug}.jsonl" \
    --min-ply "$MIN_PLY" \
    --max-ply "$MAX_PLY" \
    --every-n-plies "$EVERY_N_PLIES" \
    --dedup-zobrist
done
cat "$RUN_DIR/stage1"/pos_*.jsonl | shuf -n "$MAX_POSITIONS" > "$RUN_DIR/stage1/positions.jsonl"
echo "  -> $(wc -l < "$RUN_DIR/stage1/positions.jsonl") positions"

# ---- Stage 2: label at depths 2,4 -------------------------------------------
echo "[2/5] shogiesa label  (depths 2,4)"
"$SHOGIESA" label \
  --input "$RUN_DIR/stage1/positions.jsonl" \
  --engine "./target/release/sekirei" \
  --depths 2,4 \
  --timeout-ms 10000 \
  --out "$RUN_DIR/stage2/observations.jsonl"
echo "  -> $(wc -l < "$RUN_DIR/stage2/observations.jsonl") observations"

# ---- Stage 3: score ---------------------------------------------------------
echo "[3/5] quietset score  (profile=game-ai)"
quietset score "$RUN_DIR/stage2/observations.jsonl" \
  --profile game-ai \
  > "$RUN_DIR/stage3/scored_d4.jsonl"
echo "  -> $(wc -l < "$RUN_DIR/stage3/scored_d4.jsonl") scored positions"

# ---- Train B ----------------------------------------------------------------
echo "[4a/5] train B  (--min-stability 0.85)"
cargo run --release -q -p sekirei-train -- \
  --positions "$RUN_DIR/stage1/positions.jsonl" \
  --scored "$RUN_DIR/stage3/scored_d4.jsonl" \
  --min-stability 0.85 \
  --validation-ratio 0.1 \
  --seed 42 \
  --checkpoint-dir "$RUN_DIR/checkpoints_b" \
  --output "$OUT_B"
echo "  -> $OUT_B"

# ---- Train C ----------------------------------------------------------------
echo "[4b/5] train C  (--stability-weighted)"
cargo run --release -q -p sekirei-train -- \
  --positions "$RUN_DIR/stage1/positions.jsonl" \
  --scored "$RUN_DIR/stage3/scored_d4.jsonl" \
  --stability-weighted \
  --validation-ratio 0.1 \
  --seed 42 \
  --checkpoint-dir "$RUN_DIR/checkpoints_c" \
  --output "$OUT_C"
echo "  -> $OUT_C"

# ---- Gate B and C -----------------------------------------------------------
echo "[5/5] strength gate  ($GAMES games each)"
RESULT_B="results/${TIMESTAMP}_B.json"
RESULT_C="results/${TIMESTAMP}_C.json"

cargo run --release -q -p sekirei-match-runner -- \
  --engine1 "./target/release/sekirei $OUT_B" \
  --engine2 "./target/release/sekirei $BASELINE" \
  --games "$GAMES" --byoyomi 1000 --json "$RESULT_B"

cargo run --release -q -p sekirei-match-runner -- \
  --engine1 "./target/release/sekirei $OUT_C" \
  --engine2 "./target/release/sekirei $BASELINE" \
  --games "$GAMES" --byoyomi 1000 --json "$RESULT_C"

cat > "$RUN_DIR/manifest.json" <<EOF
{"timestamp":"$TIMESTAMP","csa_dir":"$CSA_DIR","baseline":"$BASELINE","depths":"2,4","out_b":"$OUT_B","out_c":"$OUT_C","result_b":"$RESULT_B","result_c":"$RESULT_C"}
EOF

echo ""
echo "=== B (min-stability 0.85) ==="
set +e
cargo run --release -q -p sekirei-match-runner -- gate "$RESULT_B" \
  --pass-elo 20 --pass-los 0.95 --fail-elo -10
GATE_B=$?

echo ""
echo "=== C (stability-weighted) ==="
cargo run --release -q -p sekirei-match-runner -- gate "$RESULT_C" \
  --pass-elo 20 --pass-los 0.95 --fail-elo -10
GATE_C=$?
set -e

exit $(( GATE_B | GATE_C ))
