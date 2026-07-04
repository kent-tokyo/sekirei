#!/usr/bin/env bash
# Full shogiesa + quietset + sekirei training pipeline.
#
# Verified against: shogiesa @ 886c78d (github.com/kent-tokyo/shogiesa,
# schema_version 5), quietset-cli 0.11.0. Neither tool has a --version flag;
# if this script breaks on `label`/`score` output shape, check for a schema
# change against those versions first (see commit dddb33a for a prior
# instance of this).
#
# Usage:
#   bash scripts/train_with_shogiesa_quietset.sh [CSA_DIR] [OUTPUT_WEIGHTS] [BASELINE_WEIGHTS]
#
# Environment overrides:
#   DEPTHS=2,4,6         search depths for shogiesa label (default: 2,4)
#   LABEL_DEPTH=4        sekirei-train teacher re-search depth (default: 4). Must be
#                        passed explicitly: sekirei-train's teacher score is its own
#                        --label-depth re-search, NOT shogiesa's --depths label (see
#                        tasks/lessons.md "shogiesa/quietset teacher-depth bug").
#                        Keep it aligned with (typically the deepest of) DEPTHS.
#   GAMES=400            games for Elo comparison (default: 400)
#   MIN_PLY=20           minimum ply to extract (default: 20)
#   MAX_PLY=160          maximum ply to extract (default: 160)
#   RUN_DIR=data/runs/X  intermediate file directory (default: data/runs/<timestamp>)
#   EXTRA_SCORED=path    extra scored.jsonl to merge before training (for Tier 3 deep relabel)
#   JOBS=N               parallel shogiesa label workers (default: physical cores - 2)
#   SHOGIESA=path        path to shogiesa binary (default: auto-detect, see below)
#
# Examples:
#   bash scripts/train_with_shogiesa_quietset.sh
#   DEPTHS=2,4,6 bash scripts/train_with_shogiesa_quietset.sh data/csa weights_new.bin data/weights_v7.bin
#   EXTRA_SCORED=data/stage3/deep_scored.jsonl DEPTHS=2,4,6 \
#     bash scripts/train_with_shogiesa_quietset.sh data/csa weights_deep.bin data/weights_v7.bin
#
# Exit code: forwarded from 'sekirei-match gate' (0=PASS, 1=FAIL, 2=INCONCLUSIVE)
set -e

CSA_DIR=${1:-./data/csa}
OUTPUT=${2:-data/weights_new.bin}
BASELINE=${3:-data/weights_v7.bin}
DEPTHS=${DEPTHS:-2,4}
LABEL_DEPTH=${LABEL_DEPTH:-4}
GAMES=${GAMES:-400}
MIN_PLY=${MIN_PLY:-20}
MAX_PLY=${MAX_PLY:-160}
EXTRA_SCORED=${EXTRA_SCORED:-}
TIMESTAMP=$(date +%Y%m%d_%H%M%S)
RUN_DIR=${RUN_DIR:-"data/runs/$TIMESTAMP"}
# Parallel label workers -- see scripts/redo_quietset_bc.sh for the same pattern.
JOBS=${JOBS:-$(( $(sysctl -n hw.ncpu 2>/dev/null || echo 4) - 2 ))}
[ "$JOBS" -lt 1 ] && JOBS=1

# ---- Preflight ---------------------------------------------------------------
# auto-detect shogiesa binary (same convention as scripts/redo_quietset_bc.sh)
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
command -v quietset >/dev/null || { echo "error: quietset not found";          exit 127; }
command -v cargo    >/dev/null || { echo "error: cargo not found";             exit 127; }
[ -d "$CSA_DIR"  ]             || { echo "error: CSA dir not found: $CSA_DIR"; exit 1;   }
[ -f "$BASELINE" ]             || { echo "error: baseline weights not found: $BASELINE"; exit 1; }

echo "=== shogiesa + quietset + sekirei pipeline ==="
echo "  CSA dir   : $CSA_DIR"
echo "  output    : $OUTPUT"
echo "  baseline  : $BASELINE"
echo "  depths    : $DEPTHS"
echo "  games     : $GAMES"
echo "  run dir   : $RUN_DIR"
[ -n "$EXTRA_SCORED" ] && echo "  extra     : $EXTRA_SCORED"
echo ""

mkdir -p "$RUN_DIR"/{stage1,stage2,stage3,checkpoints} results

# ---- Stage 1: extract positions ----------------------------------------
echo "[1/5] shogiesa extract  (min-ply=$MIN_PLY max-ply=$MAX_PLY every-n-plies=4)"
"$SHOGIESA" extract \
  --input "$CSA_DIR" \
  --out "$RUN_DIR/stage1/positions.jsonl" \
  --min-ply "$MIN_PLY" \
  --max-ply "$MAX_PLY" \
  --every-n-plies 4 \
  --dedup
echo "  -> $RUN_DIR/stage1/positions.jsonl ($(wc -l < "$RUN_DIR/stage1/positions.jsonl") positions)"

# ---- Stage 2: label with sekirei ----------------------------------------
echo "[2/5] shogiesa label  (engine=sekirei depths=$DEPTHS jobs=$JOBS)"
cargo build --release -q -p sekirei
"$SHOGIESA" label \
  --input "$RUN_DIR/stage1/positions.jsonl" \
  --engine "./target/release/sekirei" \
  --depths "$DEPTHS" \
  --timeout-ms 10000 \
  --jobs "$JOBS" \
  --skip-existing \
  --manifest "$RUN_DIR/stage2/label_manifest.json" \
  --out "$RUN_DIR/stage2/observations.jsonl"
echo "  -> $RUN_DIR/stage2/observations.jsonl ($(wc -l < "$RUN_DIR/stage2/observations.jsonl") observations)"

# ---- Stage 3: flatten + score with quietset -------------------------------
# shogiesa's `label` emits one nested record per position (observations: [...]);
# quietset's `score` wants one flat row per observation keyed by sample_id.
# Bridge with the same flattener scripts/redo_quietset_bc.sh uses.
echo "[3/5] flatten label -> quietset, then score  (profile=game-ai)"
python3 "$(dirname "$0")/flatten_label_to_quietset.py" \
  < "$RUN_DIR/stage2/observations.jsonl" > "$RUN_DIR/stage3/flat.jsonl"
quietset score "$RUN_DIR/stage3/flat.jsonl" \
  --profile game-ai \
  > "$RUN_DIR/stage3/scored.jsonl"
echo "  -> $RUN_DIR/stage3/scored.jsonl ($(wc -l < "$RUN_DIR/stage3/scored.jsonl") scored positions)"

SCORED="$RUN_DIR/stage3/scored.jsonl"
if [ -n "$EXTRA_SCORED" ]; then
  echo "  -> merging extra scored: $EXTRA_SCORED"
  cat "$RUN_DIR/stage3/scored.jsonl" "$EXTRA_SCORED" > "$RUN_DIR/stage3/scored_merged.jsonl"
  SCORED="$RUN_DIR/stage3/scored_merged.jsonl"
  echo "  -> merged: $(wc -l < "$SCORED") total scored positions"
fi

# ---- Train ---------------------------------------------------------------
echo "[4/5] sekirei-train  (stability-weighted validation-ratio=0.1)"
cargo run --release -q -p sekirei-train -- \
  --positions "$RUN_DIR/stage1/positions.jsonl" \
  --scored "$SCORED" \
  --stability-weighted \
  --label-depth "$LABEL_DEPTH" \
  --validation-ratio 0.1 \
  --seed 42 \
  --checkpoint-dir "$RUN_DIR/checkpoints" \
  --output "$OUTPUT"
echo "  -> $OUTPUT"

# ---- Elo comparison -------------------------------------------------------
echo "[5/5] strength regression  ($GAMES games)"
# Naming convention: <timestamp>_<candidate>_vs_<baseline>.json -- matches
# scripts/strength_regression.sh and scripts/redo_quietset_bc.sh.
OUT_JSON="results/${TIMESTAMP}_$(basename "$OUTPUT" .bin)_vs_$(basename "$BASELINE" .bin).json"
cargo run --release -q -p sekirei-match-runner -- \
  --engine1 ./target/release/sekirei --args1 "$OUTPUT" \
  --engine2 ./target/release/sekirei --args2 "$BASELINE" \
  --games "$GAMES" \
  --byoyomi 1000 \
  --json "$OUT_JSON"

# ---- Manifest ------------------------------------------------------------
cat > "$RUN_DIR/manifest.json" <<EOF
{"timestamp":"$TIMESTAMP","csa_dir":"$CSA_DIR","output":"$OUTPUT","baseline":"$BASELINE","depths":"$DEPTHS","label_depth":"$LABEL_DEPTH","games":"$GAMES","extra_scored":"$EXTRA_SCORED","result":"$OUT_JSON"}
EOF
echo "  -> manifest: $RUN_DIR/manifest.json"

echo ""
cargo run --release -q -p sekirei-match-runner -- gate "$OUT_JSON" \
  --pass-elo 20 --pass-los 0.95 --fail-elo -10
