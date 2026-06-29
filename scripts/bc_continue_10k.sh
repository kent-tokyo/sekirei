#!/usr/bin/env bash
# Fast B/C iteration: reuse existing Stage-1 extraction, sample 10k, then
# label → score → train B/C → gate. Designed for a ~2h turnaround.
#
# B = --min-stability 0.85  (hard filter)
# C = --stability-weighted  (soft weighting)
# Baseline = data/weights_v7.bin (now loadable after the JANOSW03 magic fix).
set -e

RUN_DIR=data/runs/bc_redo_20260628_214103
SRC_POS="$RUN_DIR/stage1/positions_500k.jsonl"
SHOGIESA=/Users/k_tanabe/Documents/Documents/oss_rust/shogiesa/target/release/shogiesa
BASELINE=data/weights_v7.bin
OUT_B=data/weights_v8_keep085.bin
OUT_C=data/weights_v8_weighted.bin
N=10000
JOBS=8
GAMES=20
BYOYOMI=200
TS=$(date +%Y%m%d_%H%M%S)

cd /Users/k_tanabe/Documents/Documents/oss_rust/sekirei
mkdir -p "$RUN_DIR"/{stage1,stage2,stage3,checkpoints_b,checkpoints_c} results
cargo build --release -q -p sekirei -p sekirei-train -p sekirei-match-runner

POS="$RUN_DIR/stage1/positions_10k.jsonl"
echo "[1/5] sample $N positions"
shuf -n "$N" "$SRC_POS" > "$POS"
echo "  -> $(wc -l < "$POS") positions"

echo "[2/5] label (depths 2,4, jobs=$JOBS)  ~55min"
"$SHOGIESA" label \
  --input "$POS" --engine "./target/release/sekirei" \
  --depths 2,4 --timeout-ms 10000 --jobs "$JOBS" \
  --out "$RUN_DIR/stage2/obs_10k.jsonl"
echo "  -> $(wc -l < "$RUN_DIR/stage2/obs_10k.jsonl") observations"

echo "[3/5] flatten label -> quietset, then score (profile=game-ai)"
python3 scripts/flatten_label_to_quietset.py \
  < "$RUN_DIR/stage2/obs_10k.jsonl" > "$RUN_DIR/stage3/flat_10k.jsonl"
quietset score "$RUN_DIR/stage3/flat_10k.jsonl" --profile game-ai \
  > "$RUN_DIR/stage3/scored_10k.jsonl"
echo "  -> $(wc -l < "$RUN_DIR/stage3/scored_10k.jsonl") scored"

echo "[4a/5] train B (min-stability 0.85)"
cargo run --release -q -p sekirei-train -- \
  --positions "$POS" --scored "$RUN_DIR/stage3/scored_10k.jsonl" \
  --min-stability 0.85 --seed 42 \
  --checkpoint-dir "$RUN_DIR/checkpoints_b" --output "$OUT_B"

echo "[4b/5] train C (stability-weighted, keep ALL positions)"
# --min-stability 0: keep every scored position and weight its loss by
# stability_score. Without this it inherits the 0.85 default and drops the
# same positions B does, collapsing C into B.
cargo run --release -q -p sekirei-train -- \
  --positions "$POS" --scored "$RUN_DIR/stage3/scored_10k.jsonl" \
  --stability-weighted --min-stability 0 --seed 42 \
  --checkpoint-dir "$RUN_DIR/checkpoints_c" --output "$OUT_C"

echo "[5/5] gates ($GAMES games, byoyomi ${BYOYOMI}ms) — low power, directional only"
gate() { # weights1 weights2 label
  cargo run --release -q -p sekirei-match-runner -- \
    --engine1 ./target/release/sekirei --args1 "$1" \
    --engine2 ./target/release/sekirei --args2 "$2" \
    --games "$GAMES" --byoyomi "$BYOYOMI" --json "results/${TS}_$3.json"
  echo "  $3 → results/${TS}_$3.json"
}
gate "$OUT_B" "$OUT_C"      BvsC
gate "$OUT_B" "$BASELINE"   Bvsv7
gate "$OUT_C" "$BASELINE"   Cvsv7

echo ""
echo "=== summary ($TS) ==="
for tag in BvsC Bvsv7 Cvsv7; do
  echo "--- $tag ---"
  cargo run --release -q -p sekirei-match-runner -- gate "results/${TS}_$tag.json" \
    --pass-elo 20 --pass-los 0.95 --fail-elo -10 || true
done
echo "done."
