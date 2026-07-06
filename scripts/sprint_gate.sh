#!/usr/bin/env bash
# Chunked/resumable strength gate: splits data/gate/openings_standard.sfen
# into N disjoint shards, runs each shard as an independent short match
# session (safe to interrupt between sprints and resume later by re-running
# with the same RUN_ID), then combines all sprints into one gate-able
# verdict via the existing `summarize`/`gate` subcommands.
#
# Sharding is by POSITION, not by game-number: each sprint runs the same
# --games-per-position on its own slice of positions, so total coverage and
# per-position color-pairing exactly match a single non-sharded run at the
# same total game count -- just distributed across several short sessions
# instead of one long one.
#
# Per-game ids (game0001, game0002...) restart at 1 in every match-runner
# invocation, so sprints are id-prefixed (sprintNN_) before their .jsonl
# files are concatenated -- otherwise veridict's low_id_diversity check
# (added upstream in response to this session's own feedback) would flag
# every combined run as looking non-independent, since e.g. "game0001"
# would appear once per sprint.
#
# summarize's combined.json has no diversity_ratio (it only sees
# {"id","result"} records, no move data) -- gate would silently skip the
# diversity check on a combined run otherwise, reintroducing the exact
# startpos-diversity-collapse risk this session already fixed once. Since
# shards are disjoint, the combined ratio is reconstructed exactly as
# sum(unique_prefix20) / sum(games) across the per-sprint JSON outputs and
# injected into combined.json before gating.
#
# Usage:
#   bash scripts/sprint_gate.sh <new_weights.bin> <base_weights.bin> <n_sprints> [games_per_position]
#
# Examples:
#   bash scripts/sprint_gate.sh weights_new.bin weights_v007.bin 4
#   bash scripts/sprint_gate.sh weights_new.bin weights_v007.bin 4 2
#   RUN_ID=my_run bash scripts/sprint_gate.sh weights_new.bin weights_v007.bin 4   # resume my_run
#
# Manifest validation (weights/config fingerprinting across separately-run
# sprints) is deliberately not built here: within one invocation the
# weights pair and run dir are constant by construction, so there's
# nothing to validate against. Only needed if you plan to hand-combine
# sprints from separate invocations -- ask if so.
#
# Exit code: forwarded from `sekirei-match gate` (0=PASS, 1=FAIL, 2=INCONCLUSIVE)
set -e

NEW=${1:?Usage: $0 <new_weights.bin> <base_weights.bin> <n_sprints> [games_per_position]}
BASE=${2:?Usage: $0 <new_weights.bin> <base_weights.bin> <n_sprints> [games_per_position]}
N_SPRINTS=${3:?Usage: $0 <new_weights.bin> <base_weights.bin> <n_sprints> [games_per_position]}
GAMES_PER_POSITION=${4:-4}

OPENINGS=data/gate/openings_standard.sfen
[ -f "$OPENINGS" ] || { echo "error: $OPENINGS not found" >&2; exit 2; }
command -v jq >/dev/null 2>&1 || { echo "error: jq not found" >&2; exit 127; }

NEW_STEM=$(basename "$NEW" .bin)
BASE_STEM=$(basename "$BASE" .bin)
RUN_ID=${RUN_ID:-$(date +%Y%m%d_%H%M%S)_${NEW_STEM}_vs_${BASE_STEM}}
RUN_DIR="sprint_gate_runs/$RUN_ID"
mkdir -p "$RUN_DIR/shards"

echo "=== sprint_gate: $NEW vs $BASE ($N_SPRINTS sprints x games-per-position=$GAMES_PER_POSITION) ==="
echo "  run dir: $RUN_DIR"

cargo build --release -q -p sekirei-match-runner -p sekirei

# ---- Shard the opening suite into N disjoint line-range slices ------------
TOTAL_POS=$(grep -vc '^#' "$OPENINGS")
PER_SHARD=$(( (TOTAL_POS + N_SPRINTS - 1) / N_SPRINTS ))
grep -v '^#' "$OPENINGS" > "$RUN_DIR/shards/all_positions.sfen"
for ((i = 1; i <= N_SPRINTS; i++)); do
  ii=$(printf '%02d' "$i")
  start=$(( (i - 1) * PER_SHARD + 1 ))
  end=$(( i * PER_SHARD ))
  sed -n "${start},${end}p" "$RUN_DIR/shards/all_positions.sfen" > "$RUN_DIR/shards/shard_${ii}.sfen"
done

# ---- Run each sprint (skip if already done -- resumability) ---------------
for ((i = 1; i <= N_SPRINTS; i++)); do
  ii=$(printf '%02d' "$i")
  SHARD="$RUN_DIR/shards/shard_${ii}.sfen"
  if [ ! -s "$SHARD" ]; then
    echo "[sprint $ii] shard is empty, skipping (n_sprints > available positions?)"
    continue
  fi
  if [ -f "$RUN_DIR/sprint_${ii}.json" ]; then
    echo "[sprint $ii] already done, skipping"
    continue
  fi
  SHARD_POS=$(wc -l < "$SHARD" | tr -d ' ')
  echo "[sprint $ii] running ($SHARD_POS positions x $GAMES_PER_POSITION games/position)"
  # Threads=1 on both sides: without it, each self-play engine process
  # defaults to rayon's full-core-count pool, oversubscribing the machine's
  # cores -- see tasks/lessons.md.
  cargo run --release -q -p sekirei-match-runner -- \
    --engine1 ./target/release/sekirei --args1 "$NEW" \
    --engine2 ./target/release/sekirei --args2 "$BASE" \
    --engine-option1 "Threads=1" --engine-option2 "Threads=1" \
    --positions "$SHARD" --games-per-position "$GAMES_PER_POSITION" --byoyomi 1000 \
    --output "$RUN_DIR/kifu_${ii}" \
    --json "$RUN_DIR/sprint_${ii}.json"
done

# ---- ID-prefix + concatenate all sprints -----------------------------------
: > "$RUN_DIR/combined.jsonl"
for ((i = 1; i <= N_SPRINTS; i++)); do
  ii=$(printf '%02d' "$i")
  [ -f "$RUN_DIR/sprint_${ii}.jsonl" ] || continue
  jq -c --arg p "sprint${ii}_" '.id = $p + .id' "$RUN_DIR/sprint_${ii}.jsonl" \
    >> "$RUN_DIR/combined.jsonl"
done

echo ""
echo "=== combining sprints ==="
cargo run --release -q -p sekirei-match-runner -- summarize "$RUN_DIR/combined.jsonl" \
  --out "$RUN_DIR/combined.json"

# ---- Reconstruct the combined diversity_ratio and inject it ---------------
# Valid because shards are disjoint: unique_prefix20 sums cleanly across
# sprints with no cross-sprint overlap to double-count.
DIVERSITY=$(jq -s '(map(.unique_prefix20) | add) / (map(.games) | add)' \
  "$RUN_DIR"/sprint_*.json)
jq --argjson d "$DIVERSITY" '.diversity_ratio = $d' "$RUN_DIR/combined.json" \
  > "$RUN_DIR/combined.json.tmp" && mv "$RUN_DIR/combined.json.tmp" "$RUN_DIR/combined.json"
echo "combined diversity_ratio: $DIVERSITY"

echo ""
cargo run --release -q -p sekirei-match-runner -- gate "$RUN_DIR/combined.json" \
  --pass-elo 20 --pass-los 0.95 --fail-elo -10
