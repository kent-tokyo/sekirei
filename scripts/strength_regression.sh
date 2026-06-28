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
OUT="results/${TIMESTAMP}.json"

echo "=== Strength regression: $NEW vs $BASE ($GAMES games) ==="
cargo build --release -q -p sekirei-match-runner -p sekirei

cargo run --release -q -p sekirei-match-runner -- \
  --engine1 "./target/release/sekirei $NEW" \
  --engine2 "./target/release/sekirei $BASE" \
  --games "$GAMES" --byoyomi 1000 \
  --json "$OUT"

echo ""
cargo run --release -q -p sekirei-match-runner -- gate "$OUT" \
  --pass-elo 20 --pass-los 0.95 --fail-elo -10
