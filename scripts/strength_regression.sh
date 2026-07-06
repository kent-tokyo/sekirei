#!/usr/bin/env bash
# Strength regression: compare two weight files and apply the Elo gate.
#
# Usage:
#   bash scripts/strength_regression.sh <new_weights.bin> <base_weights.bin> [games]
#
# By default, games are drawn from data/gate/openings_standard.sfen (100
# positions, each played by both colors twice via --games-per-position 4 =
# 400 games total). A startpos-only match between deterministic engines can
# collapse into the same handful of games replayed hundreds of times --
# confirmed directly this session (see tasks/lessons.md): one 350-game
# startpos-only batch had a single game replayed 19 times. That isn't 350
# independent trials, so a real strength gate requires opening diversity.
#
# For a quick startpos-only smoke check (engine runs, no illegal moves, no
# instant crashes -- NOT a strength measurement), set ALLOW_STARTPOS_GATE=1.
#
# Examples:
#   bash scripts/strength_regression.sh weights_new.bin weights_v007.bin
#   GAMES_PER_POSITION=2 bash scripts/strength_regression.sh weights_new.bin weights_v007.bin
#   ALLOW_STARTPOS_GATE=1 bash scripts/strength_regression.sh weights_new.bin weights_v007.bin 50
#
# Exit code:
#   0 = PASS (new is clearly stronger)
#   1 = FAIL (new is clearly weaker)
#   2 = INCONCLUSIVE or error
set -e

NEW=${1:?Usage: $0 <new_weights.bin> <base_weights.bin> [games]}
BASE=${2:?Usage: $0 <new_weights.bin> <base_weights.bin> [games]}
GAMES=${3:-400}

OPENINGS=data/gate/openings_standard.sfen
GAMES_PER_POSITION=${GAMES_PER_POSITION:-4}

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

cargo build --release -q -p sekirei-match-runner -p sekirei

if [ "${ALLOW_STARTPOS_GATE:-0}" = "1" ]; then
  echo "=== Strength SMOKE-CHECK ($NEW vs $BASE, $GAMES games, startpos only -- NOT a strength measurement) ==="
  POSITION_ARGS=(--games "$GAMES")
elif [ -f "$OPENINGS" ]; then
  TOTAL_GAMES=$(( $(grep -vc '^#' "$OPENINGS") * GAMES_PER_POSITION ))
  echo "=== Strength regression: $NEW vs $BASE ($OPENINGS x $GAMES_PER_POSITION games/position = $TOTAL_GAMES games) ==="
  POSITION_ARGS=(--positions "$OPENINGS" --games-per-position "$GAMES_PER_POSITION")
else
  echo "error: strength gate requires --positions ($OPENINGS not found)." >&2
  echo "Use data/gate/openings_standard.sfen, or set ALLOW_STARTPOS_GATE=1 for a startpos-only debug smoke-check (not a strength measurement)." >&2
  exit 2
fi

# Threads=1 on both sides: without it, each of the two self-play engine
# processes defaults to rayon's full-core-count pool, so they oversubscribe
# the machine's cores by up to 2x. That makes the actual search depth
# reached mid-match depend on how much the two engines are contending for
# CPU at that instant -- inconsistent with a standalone re-check of the
# same position and non-reproducible run to run. See tasks/lessons.md.
cargo run --release -q -p sekirei-match-runner -- \
  --engine1 ./target/release/sekirei --args1 "$NEW" \
  --engine2 ./target/release/sekirei --args2 "$BASE" \
  --engine-option1 "Threads=1" --engine-option2 "Threads=1" \
  "${POSITION_ARGS[@]}" --byoyomi 1000 \
  --output "$KIFU_DIR" \
  --json "$OUT"

echo ""
cargo run --release -q -p sekirei-match-runner -- gate "$OUT" \
  --pass-elo 20 --pass-los 0.95 --fail-elo -10
