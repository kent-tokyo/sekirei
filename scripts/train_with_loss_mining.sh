#!/usr/bin/env bash
# Loss-position mining: train a candidate from v010's original 10k dataset
# PLUS positions mined from self-play games where the engine actually lost
# -- an alternative to sampling more CSA positions uniformly (which didn't
# move Elo for v011's opening-data addition; see tasks/lessons.md).
#
# Only mines from post-diversity-fix kifu (position sfen ..., not startpos):
# older startpos-only runs are the exact game-diversity-collapse data this
# session already flagged as unreliable (repeated openings, not independent
# trials) -- see tasks/lessons.md "Strength gates -- startpos-only matches
# collapse". Requires shogiesa >= the from-match sfen fix (commit ff90198).
#
# Label depth is deliberately NOT --engine-option Threads=1: self-play games
# played at Threads=1/1000ms byoyomi reach only ~depth 2-4 effectively, so
# labeling at that same shallow depth would just be training the engine on
# its own blind spot, not correcting it. Full multicore per position lets
# more positions actually reach a meaningfully deeper depth before timeout.
# Measured directly (2026-07-07): depth 6, 15s timeout, jobs=1 (sequential,
# full multicore) -> ~70% of positions label successfully; the ~30% that
# don't skew toward higher ply (harder, more complex late-game positions) --
# an accepted, known limitation of this run, not solved here.
#
# Usage:
#   bash scripts/train_with_loss_mining.sh [OUTPUT_WEIGHTS] [BASELINE_WEIGHTS]
#
# Environment overrides:
#   MINE_DEPTHS=4,6      search depths for shogiesa label on mined positions
#   LABEL_DEPTH=4        sekirei-train teacher re-search depth (default: 4,
#                        matches v010/v011 so this run isolates loss-mining
#                        as the only variable -- see tasks/lessons.md
#                        "shogiesa/quietset teacher-depth bug")
#   MIN_PLY=10           minimum ply to mine from a lost game (default: 10)
#   EVERY_N_PLIES=4      sample every N plies from a lost game (default: 4)
#   RUN_DIR=data/runs/X  intermediate file directory (default: data/runs/loss_mine_<timestamp>)
#   SHOGIESA=path        path to shogiesa binary (default: auto-detect, see below)
#
# Deliberately does NOT run a gate at the end -- run
# scripts/sprint_gate.sh <OUTPUT> <BASELINE> <n_sprints> as an explicit,
# separate step, so the game budget is a visible decision.
#
# Exit code: 0 on successful training; non-zero on any preflight/stage failure.
set -e

OUTPUT=${1:-data/weights_v012_loss_mined.bin}
BASELINE=${2:-data/weights_v010_10k_full.bin}
MINE_DEPTHS=${MINE_DEPTHS:-4,6}
LABEL_DEPTH=${LABEL_DEPTH:-4}
MIN_PLY=${MIN_PLY:-10}
EVERY_N_PLIES=${EVERY_N_PLIES:-4}
TIMESTAMP=$(date +%Y%m%d_%H%M%S)
RUN_DIR=${RUN_DIR:-"data/runs/loss_mine_$TIMESTAMP"}

BASE_POSITIONS=data/runs/bc_redo_20260628_214103/stage1/positions_10k.jsonl
BASE_SCORED=data/runs/bc_redo_20260628_214103/stage3/scored_10k.jsonl

# Prune old completed runs' stage1-3 intermediates before adding a new one --
# see scripts/cleanup_runs.sh. Safe by default: this run's own BASE_POSITIONS/
# BASE_SCORED above are in bc_redo_20260628_214103, which cleanup_runs.sh
# detects as protected (referenced by name here) and never touches.
APPLY=1 bash "$(dirname "$0")/cleanup_runs.sh" || true

# Persist everything (not just display it) so scripts/gate_dashboard.py's
# Pipeline page can show live stage progress after the fact -- this is the
# only way to recover quietset's "kept X/Y" line (stderr only, never written
# to any JSON) and sekirei-train's "Epoch N/M" progress.
mkdir -p "$RUN_DIR"
exec > >(tee "$RUN_DIR/pipeline.log") 2>&1

# Only the post-diversity-fix (sfen-form) kifu directories -- see header comment.
KIFU_DIRS=(
  results/kifu/20260706_051830_v11_vs_v10_positions
  results/kifu/20260706_053415_v11_vs_v10_positions
  results/kifu/20260706_182429_weights_v011_opening_combined_vs_weights_v010_10k_full
  results/kifu/20260706_202722_weights_v011_opening_combined_vs_weights_v010_10k_full
)

# ---- Preflight ---------------------------------------------------------------
# auto-detect shogiesa binary (same convention as scripts/train_with_shogiesa_quietset.sh)
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
command -v quietset >/dev/null || { echo "error: quietset not found"; exit 127; }
command -v cargo    >/dev/null || { echo "error: cargo not found";    exit 127; }
[ -f "$BASE_POSITIONS" ] || { echo "error: v010 baseline positions not found: $BASE_POSITIONS"; exit 1; }
[ -f "$BASE_SCORED" ]    || { echo "error: v010 baseline scored not found: $BASE_SCORED"; exit 1; }
[ -f "$BASELINE" ]       || { echo "error: baseline weights not found: $BASELINE"; exit 1; }

echo "=== loss-position mining + train ==="
echo "  output    : $OUTPUT"
echo "  baseline  : $BASELINE"
echo "  mine depths: $MINE_DEPTHS"
echo "  label depth: $LABEL_DEPTH"
echo "  run dir   : $RUN_DIR"
echo ""

mkdir -p "$RUN_DIR"/{stage1,stage2,stage3,checkpoints}
cargo build --release -q -p sekirei -p sekirei-train


# ---- Stage 1: mine loss positions from trustworthy kifu -------------------
echo "[1/4] mine loss positions (min-ply=$MIN_PLY every-n-plies=$EVERY_N_PLIES)"
: > "$RUN_DIR/stage1/mined_positions.jsonl"
for d in "${KIFU_DIRS[@]}"; do
  if [ ! -d "$d" ]; then
    echo "  warning: $d not found, skipping"
    continue
  fi
  for side in engine1 engine2; do
    "$SHOGIESA" from-match --input "$d" --out "$RUN_DIR/stage1/tmp.jsonl" \
      --losing-side "$side" --min-ply "$MIN_PLY" --every-n-plies "$EVERY_N_PLIES" --dedup
    cat "$RUN_DIR/stage1/tmp.jsonl" >> "$RUN_DIR/stage1/mined_positions.jsonl"
  done
done
rm -f "$RUN_DIR/stage1/tmp.jsonl"
echo "  -> $RUN_DIR/stage1/mined_positions.jsonl ($(wc -l < "$RUN_DIR/stage1/mined_positions.jsonl") positions)"

# ---- Stage 2: label mined positions at depth, full multicore --------------
echo "[2/4] shogiesa label  (depths=$MINE_DEPTHS, full multicore per position)"
"$SHOGIESA" label \
  --input "$RUN_DIR/stage1/mined_positions.jsonl" \
  --engine "./target/release/sekirei" \
  --depths "$MINE_DEPTHS" \
  --timeout-ms 15000 \
  --jobs 1 \
  --cache-dir "data/shogiesa_label_cache" \
  --manifest "$RUN_DIR/stage2/label_manifest.json" \
  --out "$RUN_DIR/stage2/observations.jsonl"
echo "  -> $RUN_DIR/stage2/observations.jsonl ($(wc -l < "$RUN_DIR/stage2/observations.jsonl") observations)"

# ---- Stage 3: flatten + score with quietset --------------------------------
echo "[3/4] flatten label -> quietset, then score  (profile=game-ai-single-engine)"
python3 "$(dirname "$0")/flatten_label_to_quietset.py" \
  < "$RUN_DIR/stage2/observations.jsonl" > "$RUN_DIR/stage3/flat.jsonl"
quietset score "$RUN_DIR/stage3/flat.jsonl" \
  --profile game-ai-single-engine \
  > "$RUN_DIR/stage3/mined_scored.jsonl"
echo "  -> $RUN_DIR/stage3/mined_scored.jsonl ($(wc -l < "$RUN_DIR/stage3/mined_scored.jsonl") scored positions)"

# ---- Stage 4: combine with v010's original dataset + train ----------------
echo "[4/4] combine with v010 baseline dataset, train"
cat "$BASE_POSITIONS" "$RUN_DIR/stage1/mined_positions.jsonl" > "$RUN_DIR/stage3/positions_combined.jsonl"
cat "$BASE_SCORED" "$RUN_DIR/stage3/mined_scored.jsonl" > "$RUN_DIR/stage3/scored_combined.jsonl"
echo "  -> $(wc -l < "$RUN_DIR/stage3/positions_combined.jsonl") combined positions"

# --min-stability 0 is required alongside --stability-weighted: --min-stability
# defaults to 0.85 and load_scored() filters on it BEFORE stability-weighting is
# ever applied, regardless of --stability-weighted -- so without this, only
# positions already >= 0.85 stable are included at all (weighted among
# themselves), silently dropping everything below that threshold instead of
# down-weighting it. Only 64/792 mined positions clear 0.85 (mined positions
# are inherently less stable -- that's what makes them worth mining), so
# omitting this would mean ~92% of the new data never reaches training. See
# tasks/lessons.md.
cargo run --release -q -p sekirei-train -- \
  --positions "$RUN_DIR/stage3/positions_combined.jsonl" \
  --scored "$RUN_DIR/stage3/scored_combined.jsonl" \
  --stability-weighted \
  --min-stability 0 \
  --label-depth "$LABEL_DEPTH" \
  --validation-ratio 0.1 \
  --seed 42 \
  --checkpoint-dir "$RUN_DIR/checkpoints" \
  --output "$OUTPUT"
echo "  -> $OUTPUT"

cat > "$RUN_DIR/manifest.json" <<EOF
{"timestamp":"$TIMESTAMP","output":"$OUTPUT","baseline":"$BASELINE","mine_depths":"$MINE_DEPTHS","label_depth":"$LABEL_DEPTH","min_ply":"$MIN_PLY","every_n_plies":"$EVERY_N_PLIES"}
EOF
echo "  -> manifest: $RUN_DIR/manifest.json"

echo ""
echo "Training done. Not gating automatically -- run explicitly with a chosen game budget:"
echo "  bash scripts/sprint_gate.sh $OUTPUT $BASELINE 4"
