#!/usr/bin/env bash
# Strength regression: compare two weight files and apply the Elo gate.
#
# Usage:
#   bash scripts/strength_regression.sh <new_weights.bin> <base_weights.bin> [games]
#
# Examples:
#   bash scripts/strength_regression.sh weights_new.bin weights_v7.bin
#   bash scripts/strength_regression.sh weights_new.bin weights_v7.bin 200
#
# Exit code:
#   0 = PASS (new is clearly stronger)
#   1 = FAIL (new is clearly weaker)
#   2 = INCONCLUSIVE or error
set -e

NEW=${1:?Usage: $0 <new_weights.bin> <base_weights.bin> [games]}
BASE=${2:?Usage: $0 <new_weights.bin> <base_weights.bin> [games]}
GAMES=${3:-400}

TIMESTAMP=$(date +%Y%m%d_%H%M%S)
mkdir -p results
# Naming convention: <timestamp>_<candidate>_vs_<baseline>.json -- so the
# filename alone says what was compared (previously just the timestamp,
# leaving which weights were tested unrecoverable from results/ alone).
NEW_STEM=$(basename "$NEW" .bin)
BASE_STEM=$(basename "$BASE" .bin)
OUT="results/${TIMESTAMP}_${NEW_STEM}_vs_${BASE_STEM}.json"
# Per-game kifu (USI position/moves records) for scripts/gate_dashboard.py's
# kifu viewer -- pass this directory as the dashboard's 4th argument.
KIFU_DIR="results/kifu/${TIMESTAMP}_${NEW_STEM}_vs_${BASE_STEM}"
mkdir -p "$KIFU_DIR"

echo "=== Strength regression: $NEW vs $BASE ($GAMES games) ==="
cargo build --release -q -p sekirei-match-runner -p sekirei

cargo run --release -q -p sekirei-match-runner -- \
  --engine1 ./target/release/sekirei --args1 "$NEW" \
  --engine2 ./target/release/sekirei --args2 "$BASE" \
  --games "$GAMES" --byoyomi 1000 \
  --output "$KIFU_DIR" \
  --json "$OUT"

echo ""
cargo run --release -q -p sekirei-match-runner -- gate "$OUT" \
  --pass-elo 20 --pass-los 0.95 --fail-elo -10
