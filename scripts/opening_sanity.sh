#!/usr/bin/env bash
# Opening sanity suite: runs a fixed set of early-game positions through the
# engine at a fixed depth and prints the chosen move for each. Meant to be
# run before and after a training change to opening-phase data, so "did the
# opening judgment actually improve" is a diff of this script's output
# instead of eyeballing 2-3 kifu games.
#
# Usage:
#   bash scripts/opening_sanity.sh [--json] <weights.bin> [depth]
set -e

JSON_MODE=0
ARGS=()
for a in "$@"; do
  if [ "$a" = "--json" ]; then
    JSON_MODE=1
  else
    ARGS+=("$a")
  fi
done
set -- "${ARGS[@]}"

WEIGHTS=${1:?Usage: $0 [--json] <weights.bin> [depth]}
DEPTH=${2:-6}
ENGINE=./target/release/sekirei

# name|USI move sequence from startpos (empty = startpos itself)
CASES=(
  "startpos|"
  "aigakari|2g2f 8c8d 2f2e"
  "kakugawari|2g2f 3c3d 7g7f 8c8d"
  "ibisha_vs_furibisha|2g2f 3c3d 7g7f 4c4d"
  "hayaishida|2g2f 3c3d 7g7f 3d3e"
  "edge_lance_trap|7g7f 9c9d 8g8f 8c8d 2g2f 8d8e 4i4h 8e8f"
)

if [ "$JSON_MODE" = "0" ]; then
  echo "=== Opening sanity: $WEIGHTS (depth $DEPTH) ==="
else
  echo -n "["
fi

first=1
for case in "${CASES[@]}"; do
  name="${case%%|*}"
  moves="${case#*|}"
  if [ -z "$moves" ]; then
    pos_cmd="position startpos"
  else
    pos_cmd="position startpos moves $moves"
  fi
  out=$(
    {
      echo "usi"
      sleep 0.1
      echo "setoption name EvalFile value $WEIGHTS"
      echo "isready"
      sleep 0.1
      echo "$pos_cmd"
      echo "go depth $DEPTH"
      sleep 1
      echo "quit"
    } | $ENGINE 2>&1 | grep -E "^info depth $DEPTH|^bestmove"
  )
  best=$(echo "$out" | grep "^bestmove" | awk '{print $2}')
  score=$(echo "$out" | grep "^info depth $DEPTH" | tail -1 | grep -oE "score cp [-0-9]+" | awk '{print $3}')

  if [ "$JSON_MODE" = "0" ]; then
    printf "  %-22s bestmove=%-8s score_cp=%s\n" "$name" "$best" "$score"
  else
    bm_json="null"; [ -n "$best" ] && bm_json="\"$best\""
    sc_json="null"; [ -n "$score" ] && sc_json="$score"
    [ "$first" = "1" ] && first=0 || echo -n ","
    printf '{"name":"%s","bestmove":%s,"score_cp":%s}' "$name" "$bm_json" "$sc_json"
  fi
done

[ "$JSON_MODE" = "1" ] && echo "]"
