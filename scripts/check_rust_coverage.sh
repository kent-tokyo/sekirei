#!/usr/bin/env bash
set -euo pipefail

# Keep the public coverage badge focused on reusable Rust code and the shipped
# USI runtime. Benchmark/diagnostic binaries and orchestration-heavy CLI entry
# points are still exercised by normal CI, but are not allowed to dilute or
# inflate this engine/library coverage contract.
readonly COVERAGE_IGNORE_REGEX='(^|/)(target|crates/sekirei-bench|crates/sekirei-core/src/bin|crates/sekirei-csa/src/bin|crates/sekirei-train/src/bin)(/|$)|crates/(sekirei-csa|sekirei-match-runner|sekirei-train)/src/main\.rs$'
readonly COVERAGE_MIN_LINES="${COVERAGE_MIN_LINES:-95}"
readonly COVERAGE_DIR="${COVERAGE_DIR:-target/coverage}"

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

# These bounded race tests are ignored by the ordinary unit-test pass because
# they deliberately repeat scheduler and colliding-write scenarios. They are
# fast enough for the coverage job and make the parallel-search release
# contract part of the published evidence instead of leaving their test bodies
# permanently uncovered.
for test_name in \
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
# percentage as well, so 94.99% cannot pass a 95% gate.
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
# the badge value and is not used to pass the 95% gate.
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
