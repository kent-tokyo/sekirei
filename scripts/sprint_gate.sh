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
# SPRT=1 bash scripts/sprint_gate.sh weights_new.bin weights_v007.bin 20
#   Opt-in early stopping: after every sprint, checks `gate --sprt` (H0:
#   elo<=ELO0 vs H1: elo>=ELO1, default 0/20, alpha=beta=0.05 by default --
#   see tasks/lessons.md, 2026-07-09, on why this project's gate suite
#   settled on standard-SPRT semantics rather than "prove CI lower bound
#   clears +20") on every sprint run so far and stops the moment it's
#   decisive, instead of always running all N_SPRINTS. ELO0/ELO1/ALPHA/BETA/
#   SPRT_VARIANT (wald|trinomial) env vars override the defaults. Aggregation
#   stays per-game (not per-position-pair): this project's gate suite has
#   measured within-pair correlation ≈ 0 (tasks/lessons.md, 2026-07-09), so
#   there's no pairing benefit to trade away here.
#
#   Hard cap: MAX_GAMES (default 1600, env override) stops the run and
#   reports the still-INCONCLUSIVE verdict as final once that many games
#   have been played, independent of N_SPRINTS -- a true effect strictly
#   between ELO0 and ELO1 can otherwise make SPRT run indefinitely, and
#   N_SPRINTS alone is not a real safety net against that (nothing stops
#   someone from passing a very large N_SPRINTS, or resuming the same RUN_ID
#   with a larger one later). This is a compute-budget ceiling, not a
#   statistical one -- it does not change the verdict, only when the loop
#   gives up on reaching one.
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

SPRT=${SPRT:-0}
ELO0=${ELO0:-0}
ELO1=${ELO1:-20}
ALPHA=${ALPHA:-0.05}
BETA=${BETA:-0.05}
SPRT_VARIANT=${SPRT_VARIANT:-wald}
MAX_GAMES=${MAX_GAMES:-1600}

OPENINGS=data/gate/openings_standard.sfen
[ -f "$OPENINGS" ] || { echo "error: $OPENINGS not found" >&2; exit 2; }
command -v jq >/dev/null 2>&1 || { echo "error: jq not found" >&2; exit 127; }

NEW_STEM=$(basename "$NEW" .bin)
BASE_STEM=$(basename "$BASE" .bin)
RUN_ID=${RUN_ID:-$(date +%Y%m%d_%H%M%S)_${NEW_STEM}_vs_${BASE_STEM}}
RUN_DIR="sprint_gate_runs/$RUN_ID"
mkdir -p "$RUN_DIR/shards"

if [ "$SPRT" = "1" ]; then
  echo "=== sprint_gate: $NEW vs $BASE ($N_SPRINTS sprints x games-per-position=$GAMES_PER_POSITION, SPRT early-stop: H0=$ELO0 H1=$ELO1 alpha=$ALPHA beta=$BETA) ==="
else
  echo "=== sprint_gate: $NEW vs $BASE ($N_SPRINTS sprints x games-per-position=$GAMES_PER_POSITION) ==="
fi
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

# Combines sprints 1..$1 (already-run ones only) into combined.jsonl/.json
# with diversity_ratio injected. Shared by the mid-loop SPRT check and the
# final gate, so both look at exactly the same reconstruction logic.
combine_sprints_upto() {
  local upto="$1"
  : > "$RUN_DIR/combined.jsonl"
  local sprint_jsons=()
  for ((j = 1; j <= upto; j++)); do
    jj=$(printf '%02d' "$j")
    [ -f "$RUN_DIR/sprint_${jj}.jsonl" ] || continue
    jq -c --arg p "sprint${jj}_" '.id = $p + .id' "$RUN_DIR/sprint_${jj}.jsonl" \
      >> "$RUN_DIR/combined.jsonl"
    sprint_jsons+=("$RUN_DIR/sprint_${jj}.json")
  done
  cargo run --release -q -p sekirei-match-runner -- summarize "$RUN_DIR/combined.jsonl" \
    --out "$RUN_DIR/combined.json"
  # Valid because shards are disjoint: unique_prefix20 sums cleanly across
  # sprints with no cross-sprint overlap to double-count.
  local diversity
  diversity=$(jq -s '(map(.unique_prefix20) | add) / (map(.games) | add)' "${sprint_jsons[@]}")
  jq --argjson d "$diversity" '.diversity_ratio = $d' "$RUN_DIR/combined.json" \
    > "$RUN_DIR/combined.json.tmp" && mv "$RUN_DIR/combined.json.tmp" "$RUN_DIR/combined.json"
  echo "combined diversity_ratio ($upto/$N_SPRINTS sprints): $diversity"
}

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
  else
    SHARD_POS=$(wc -l < "$SHARD" | tr -d ' ')
    echo "[sprint $ii] running ($SHARD_POS positions x $GAMES_PER_POSITION games/position)"
    # Threads=1 on both sides: without it, each self-play engine process
    # defaults to rayon's full-core-count pool, oversubscribing the
    # machine's cores -- see tasks/lessons.md.
    cargo run --release -q -p sekirei-match-runner -- \
      --engine1 ./target/release/sekirei --args1 "$NEW" \
      --engine2 ./target/release/sekirei --args2 "$BASE" \
      --engine-option1 "Threads=1" --engine-option2 "Threads=1" \
      --positions "$SHARD" --games-per-position "$GAMES_PER_POSITION" --byoyomi 1000 \
      --output "$RUN_DIR/kifu_${ii}" \
      --json "$RUN_DIR/sprint_${ii}.json"
  fi

  if [ "$SPRT" = "1" ]; then
    echo ""
    echo "=== SPRT check after sprint $ii ==="
    combine_sprints_upto "$i"
    set +e
    cargo run --release -q -p sekirei-match-runner -- gate "$RUN_DIR/combined.json" \
      --sprt --elo0 "$ELO0" --elo1 "$ELO1" --alpha "$ALPHA" --beta "$BETA" \
      --sprt-variant "$SPRT_VARIANT"
    RC=$?
    set -e
    if [ "$RC" != "2" ]; then
      echo "[sprint $ii] SPRT decisive after $i/$N_SPRINTS sprints -- stopping early"
      exit "$RC"
    fi
    GAMES_SO_FAR=$(jq -r '.games' "$RUN_DIR/combined.json")
    if [ "$GAMES_SO_FAR" -ge "$MAX_GAMES" ]; then
      echo "[sprint $ii] MAX_GAMES cap ($MAX_GAMES) reached at $GAMES_SO_FAR games without a decisive LLR -- stopping, verdict stays INCONCLUSIVE (compute-budget stop, not a statistical one)"
      exit 2
    fi
    echo ""
  fi
done

echo ""
echo "=== combining sprints ==="
combine_sprints_upto "$N_SPRINTS"

echo ""
if [ "$SPRT" = "1" ]; then
  echo "SPRT ran all $N_SPRINTS sprints without reaching a decisive bound -- final check:"
  cargo run --release -q -p sekirei-match-runner -- gate "$RUN_DIR/combined.json" \
    --sprt --elo0 "$ELO0" --elo1 "$ELO1" --alpha "$ALPHA" --beta "$BETA" \
    --sprt-variant "$SPRT_VARIANT"
else
  cargo run --release -q -p sekirei-match-runner -- gate "$RUN_DIR/combined.json" \
    --pass-elo 20 --pass-los 0.95 --fail-elo -10
fi
