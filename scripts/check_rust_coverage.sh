#!/usr/bin/env bash
set -euo pipefail

# Keep the public coverage badge focused on reusable Rust code and the shipped
# USI runtime. Benchmark/diagnostic binaries and orchestration-heavy CLI entry
# points are still exercised by normal CI, but are not allowed to dilute or
# inflate this engine/library coverage contract.
readonly COVERAGE_IGNORE_REGEX='(^|/)(target|crates/sekirei-bench|crates/sekirei-core/src/bin|crates/sekirei-csa/src/bin|crates/sekirei-train/src/bin)(/|$)|crates/(sekirei-csa|sekirei-match-runner|sekirei-train)/src/main\.rs$'
readonly COVERAGE_MIN_LINES="${COVERAGE_MIN_LINES:-98}"
readonly COVERAGE_DIR="${COVERAGE_DIR:-target/coverage}"

# Coverage instrumentation increases stack use in the parallel search tests,
# while incremental artifacts roughly double the peak disk footprint. Keep the
# CI contract deterministic on constrained macOS runners as well as Linux.
export RUST_MIN_STACK="${RUST_MIN_STACK:-16777216}"
export CARGO_INCREMENTAL="${CARGO_INCREMENTAL:-0}"

mkdir -p "${COVERAGE_DIR}"
coverage_tmp="$(mktemp -d "${TMPDIR:-/tmp}/sekirei-coverage.XXXXXX")"
trap 'rm -rf -- "${coverage_tmp}"' EXIT
cargo llvm-cov clean --workspace

# Run the complete default workspace first, then merge the optional opening
# book contract into the same profile set. The workspace contains tests that
# mutate process-wide search tunables. Run this first pass serially so the
# instrumentation overhead cannot combine otherwise independent test states
# into a pathological search that overflows a test thread's stack.
# cargo-llvm-cov 0.8.x does not allow --no-clean and --no-report together, so
# the feature run writes an intermediate package summary before the final
# workspace-wide reports.
RUST_TEST_THREADS=1 cargo llvm-cov --workspace --no-report
RUST_TEST_THREADS=1 cargo llvm-cov -p sekirei \
  --features opening-book \
  --no-clean \
  --json \
  --summary-only \
  --ignore-filename-regex "${COVERAGE_IGNORE_REGEX}" \
  --output-path "${COVERAGE_DIR}/opening-book-summary.json"

# The default build uses SEARCH_V2, while the tune feature intentionally keeps
# the legacy search and optional pruning policies available for A/B runs. Run
# their board-restoration/legal-move contract in an isolated process because
# tune parameters are process-wide atomics.
cargo llvm-cov -p sekirei-core \
  --features tune \
  --no-clean \
  --json \
  --summary-only \
  --ignore-filename-regex "${COVERAGE_IGNORE_REGEX}" \
  --output-path "${COVERAGE_DIR}/tuned-search-summary.json" \
  -- search::see_tests::tuned_search_paths_restore_the_board_and_return_legal_moves \
  --ignored \
  --exact

# The tunable public-path matrix covers complete alternative policy stacks,
# not just their board-restoration unit contract. It runs in its own process
# because tune parameters are process-wide atomics.
cargo llvm-cov -p sekirei-core \
  --features tune \
  --test public_path_coverage \
  --no-clean \
  --json \
  --summary-only \
  --ignore-filename-regex "${COVERAGE_IGNORE_REGEX}" \
  --output-path "${COVERAGE_DIR}/tuned-public-path-summary.json" \
  -- tuned_public_search_matrix_exercises_alternative_policies \
  --exact

# The disabled ancestor-NMP experiment must prove an actual correction in a
# controlled verified-null position, rather than only traversing inactive flags.
cargo llvm-cov -p sekirei-core \
  --features tune \
  --no-clean \
  --json \
  --summary-only \
  --ignore-filename-regex "${COVERAGE_IGNORE_REGEX}" \
  --output-path "${COVERAGE_DIR}/tuned-nmp-prior-summary.json" \
  -- search::v2::tests::tuned_nmp_prior_restores_board_and_returns_legal_move \
  --ignored \
  --exact

# The disabled quiet-LMP preparation shortcut must preserve fixed-depth
# signatures with material and own synthetic HalfKP in one isolated process.
# Tuning globals and evaluator activation must not overlap another test.
cargo llvm-cov -p sekirei-core \
  --features tune \
  --test quiet_lmp_skip \
  --no-clean \
  --json \
  --summary-only \
  --ignore-filename-regex "${COVERAGE_IGNORE_REGEX}" \
  --output-path "${COVERAGE_DIR}/tuned-quiet-lmp-skip-summary.json" \
  -- quiet_lmp_skip_preserves_material_and_own_halfkp_fixed_depth \
  --ignored \
  --exact \
  --test-threads=1

# Isolate the disabled SEE lower-bound cache with material and an own
# deterministic synthetic HalfKP evaluator; keep global tuning state serial.
cargo llvm-cov -p sekirei-core \
  --features tune \
  --test see_bound_cache \
  --no-clean \
  --json \
  --summary-only \
  --ignore-filename-regex "${COVERAGE_IGNORE_REGEX}" \
  --output-path "${COVERAGE_DIR}/tuned-see-bound-cache-summary.json" \
  -- see_bound_cache_preserves_material_and_own_halfkp_fixed_depth \
  --ignored \
  --exact \
  --test-threads=1

# Isolate the completed-score aspiration flag while checking real activation,
# fixed-root/MultiPV exclusions and observer noninterference.
cargo llvm-cov -p sekirei-core \
  --features tune \
  --no-clean \
  --json \
  --summary-only \
  --ignore-filename-regex "${COVERAGE_IGNORE_REGEX}" \
  --output-path "${COVERAGE_DIR}/tuned-aspiration-summary.json" \
  -- search::aspiration::tests::adaptive_root_windows_preserve_contracts_and_observer_noninterference \
  --ignored \
  --exact

# The repetition filter is a compile-time CPU experiment: exercise its
# alternative bit rule without a runtime mode change or release counters.
cargo llvm-cov -p sekirei-core \
  --features repetition_filter2 \
  --lib \
  --no-clean \
  --json \
  --summary-only \
  --ignore-filename-regex "${COVERAGE_IGNORE_REGEX}" \
  --output-path "${COVERAGE_DIR}/repetition-filter2-summary.json" \
  -- repetition_filter \
  --test-threads=1

# Preserve the historical correction-key fold while exercising the opt-in
# fixed-kind expansion. No coverage exclusions or thresholds are changed.
cargo llvm-cov -p sekirei-core \
  --features correction_key_unroll \
  --lib \
  --no-clean \
  --json \
  --summary-only \
  --ignore-filename-regex "${COVERAGE_IGNORE_REGEX}" \
  --output-path "${COVERAGE_DIR}/correction-key-unroll-summary.json" \
  -- correction_key_fold \
  --test-threads=1

# These bounded race tests are ignored by the ordinary unit-test pass because
# they deliberately repeat scheduler and colliding-write scenarios. They are
# fast enough for the coverage job and make the parallel-search release
# contract part of the published evidence instead of leaving their test bodies
# permanently uncovered.
for test_name in \
  tests::perft5_startpos \
  tt::tests::concurrent_colliding_writes_stay_well_formed_under_stress \
  mcts::tests::root_parallelism_remains_equivalent_across_repeated_scheduler_runs
do
  cargo llvm-cov -p sekirei-core \
    --no-clean \
    --json \
    --summary-only \
    --ignore-filename-regex "${COVERAGE_IGNORE_REGEX}" \
    --output-path "${COVERAGE_DIR}/$(echo "${test_name}" | tr ':' '-').json" \
    -- "${test_name}" \
    --ignored \
    --exact
done

# Browser-only wasm-bindgen tests cover JS Array conversion separately. This
# native contract exercises the exported success-path wrappers and scalar
# getters without requiring a browser in the coverage job.
cargo llvm-cov -p sekirei-wasm \
  --no-clean \
  --json \
  --summary-only \
  --ignore-filename-regex "${COVERAGE_IGNORE_REGEX}" \
  --output-path "${COVERAGE_DIR}/wasm-native-contract-summary.json" \
  -- tests::exported_analysis_getters_match_the_native_results \
  --exact

# Exercise the real trainer entry point over a tiny repository fixture. The
# CLI main itself remains outside the badge denominator, while the reusable
# parsing, labeling, validation, checkpoint, and export paths stay covered.
cargo llvm-cov run -p sekirei-train \
  --bin train \
  --no-clean \
  --json \
  --summary-only \
  --ignore-filename-regex "${COVERAGE_IGNORE_REGEX}" \
  --output-path "${COVERAGE_DIR}/trainer-smoke-summary.json" \
  -- \
  --positions scripts/fixtures/nnue_phase3_pilot.jsonl \
  --output "${coverage_tmp}/weights.bin" \
  --epochs 1 \
  --label-depth 1 \
  --label-nodes 10000 \
  --validation-ratio 0.34 \
  --checkpoint-dir "${coverage_tmp}/checkpoints" \
  --teacher-cache "${coverage_tmp}/teacher-cache.jsonl"

cargo llvm-cov report \
  --lcov \
  --ignore-filename-regex "${COVERAGE_IGNORE_REGEX}" \
  --fail-under-lines "${COVERAGE_MIN_LINES}" \
  --output-path "${COVERAGE_DIR}/lcov.info"

cargo llvm-cov report \
  --json \
  --summary-only \
  --ignore-filename-regex "${COVERAGE_IGNORE_REGEX}" \
  --output-path "${COVERAGE_DIR}/summary.json"

# cargo-llvm-cov compares an integer-rounded percentage for
# --fail-under-lines. Enforce the published contract against the exact JSON
# percentage as well, so 97.99% cannot pass a 98% gate.
if ! jq --argjson minimum "${COVERAGE_MIN_LINES}" -e \
  '.data[0].totals.lines.percent >= $minimum' \
  "${COVERAGE_DIR}/summary.json" >/dev/null; then
  jq -r --arg minimum "${COVERAGE_MIN_LINES}" '
    .data[0].totals.lines
    | "Rust line coverage \(.percent)% is below the required \($minimum)%"
  ' "${COVERAGE_DIR}/summary.json" >&2
  exit 1
fi

# Preserve the unfiltered workspace number as a separate diagnostic. It is not
# the badge value and is not used to pass the 98% gate.
cargo llvm-cov report \
  --json \
  --summary-only \
  --output-path "${COVERAGE_DIR}/full-workspace-summary.json"

jq -r '
  .data[0].totals.lines
  | "Rust engine/library line coverage: \(.covered)/\(.count) (\(.percent)%)"
' "${COVERAGE_DIR}/summary.json"

jq -r '
  .data[0].totals.lines
  | "Full workspace line coverage (diagnostic): \(.covered)/\(.count) (\(.percent)%)"
' "${COVERAGE_DIR}/full-workspace-summary.json"
