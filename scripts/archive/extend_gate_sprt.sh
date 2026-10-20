#!/usr/bin/env bash
# Extends an already-completed sprint_gate.sh run (sprints 01..04, 396 games,
# INCONCLUSIVE) toward a decisive SPRT verdict, without discarding or
# re-running any existing game. Reuses the same 4 shard files (same 100
# positions, same paired color-swap structure) cyclically for each new
# round -- same convention as reusing an opening book across many SPRT game
# pairs. Same checkpoints, same binary, same Threads=1, same byoyomi.
#
# Stops on: SPRT decisive (PASS/FAIL), MAX_GAMES reached, or any fail-fast
# signature (ILLEGAL BESTMOVE DETECTED / POSITION DESYNC DETECTED / engine
# error / panic) in a round's own log -- the latter re-invalidates the
# whole strength gate exactly as the base run's protocol requires.
set -uo pipefail

RUN_DIR=sprint_gate_runs/20260718_invariant_rerun_conflict_ft_seed123_epoch7_vs_control_seed123_epoch7
NEW=data/runs/20260717_longrun_conflict_mask/conflict_ft_seed123.epoch7.bin
BASE=data/runs/20260717_longrun_conflict_mask/control_seed123.epoch7.bin
GAMES_PER_POSITION=4
ELO0=0
ELO1=20
ALPHA=0.05
BETA=0.05
SPRT_VARIANT=wald
MAX_GAMES=1600
SHARDS=(
  "$RUN_DIR/shards/shard_01.sfen"
  "$RUN_DIR/shards/shard_02.sfen"
  "$RUN_DIR/shards/shard_03.sfen"
  "$RUN_DIR/shards/shard_04.sfen"
)

FAIL_FAST_PATTERN='ILLEGAL BESTMOVE DETECTED|POSITION DESYNC DETECTED|engine error|panicked at'

combine_all() {
  : > "$RUN_DIR/combined_extended.jsonl"
  for jj in 01 02 03 04; do
    jq -c --arg p "sprint${jj}_" '.id = $p + .id' "$RUN_DIR/sprint_${jj}.jsonl" \
      >> "$RUN_DIR/combined_extended.jsonl"
  done
  for f in "$RUN_DIR"/sprint_ext*.jsonl; do
    [ -f "$f" ] || continue
    tag=$(basename "$f" .jsonl)
    jq -c --arg p "${tag}_" '.id = $p + .id' "$f" >> "$RUN_DIR/combined_extended.jsonl"
  done
  cargo run --release -q -p sekirei-match-runner -- summarize \
    "$RUN_DIR/combined_extended.jsonl" --out "$RUN_DIR/combined_extended.json"
}

round=5
while true; do
  rr=$(printf '%02d' "$round")
  shard_idx=$(( (round - 1) % 4 ))
  SHARD="${SHARDS[$shard_idx]}"
  ROUND_LOG="$RUN_DIR/sprint_ext${rr}.log"

  echo "=== [extend round $rr] $(date -u +%Y-%m-%dT%H:%M:%SZ) using $SHARD ==="

  cargo run --release -q -p sekirei-match-runner -- \
    --engine1 ./target/release/sekirei --args1 "$NEW" \
    --engine2 ./target/release/sekirei --args2 "$BASE" \
    --engine-option1 "Threads=1" --engine-option2 "Threads=1" \
    --positions "$SHARD" --games-per-position "$GAMES_PER_POSITION" --byoyomi 1000 \
    --output "$RUN_DIR/kifu_ext${rr}" \
    --json "$RUN_DIR/sprint_ext${rr}.json" \
    > "$ROUND_LOG" 2>&1
  ROUND_RC=$?

  if grep -qE "$FAIL_FAST_PATTERN" "$ROUND_LOG"; then
    echo "=== FAIL-FAST: round $rr tripped an invariant/engine-error signature -- see $ROUND_LOG ==="
    echo "=== strength gate is INVALID as of round $rr; halting extension, no further rounds ==="
    exit 3
  fi
  if [ "$ROUND_RC" -ne 0 ] || [ ! -f "$RUN_DIR/sprint_ext${rr}.jsonl" ]; then
    echo "=== round $rr: match-runner exited $ROUND_RC or produced no jsonl -- treating as fail-fast (missing game records) ==="
    exit 3
  fi

  combine_all
  GAMES_SO_FAR=$(jq -r '.games' "$RUN_DIR/combined_extended.json")
  echo "=== round $rr done, cumulative games=$GAMES_SO_FAR ==="

  set +e
  cargo run --release -q -p sekirei-match-runner -- gate "$RUN_DIR/combined_extended.json" \
    --sprt --elo0 "$ELO0" --elo1 "$ELO1" --alpha "$ALPHA" --beta "$BETA" \
    --sprt-variant "$SPRT_VARIANT"
  RC=$?
  set -e

  if [ "$RC" != "2" ]; then
    echo "=== SPRT decisive after round $rr ($GAMES_SO_FAR games): exit $RC ==="
    exit "$RC"
  fi
  if [ "$GAMES_SO_FAR" -ge "$MAX_GAMES" ]; then
    echo "=== MAX_GAMES cap ($MAX_GAMES) reached at $GAMES_SO_FAR games without a decisive LLR -- stopping, INCONCLUSIVE stands ==="
    exit 2
  fi
  round=$((round + 1))
done
