#!/usr/bin/env bash
set -euo pipefail

# Keep the public coverage badge focused on reusable Rust code and the shipped
# USI runtime. Benchmark/diagnostic binaries and orchestration-heavy CLI entry
# points are still exercised by normal CI, but are not allowed to dilute or
# inflate this engine/library coverage contract.
readonly COVERAGE_IGNORE_REGEX='(^|/)(target|crates/sekirei-bench|crates/sekirei-core/src/bin|crates/sekirei-csa/src/bin|crates/sekirei-train/src/bin)(/|$)|crates/(sekirei-csa|sekirei-match-runner|sekirei-train)/src/main\.rs$'
readonly COVERAGE_MIN_LINES="${COVERAGE_MIN_LINES:-92}"
readonly COVERAGE_DIR="${COVERAGE_DIR:-target/coverage}"

mkdir -p "${COVERAGE_DIR}"
cargo llvm-cov clean --workspace

# Run the complete default workspace first, then merge the optional opening
# book contract into the same profile set. cargo-llvm-cov 0.8.x does not allow
# --no-clean and --no-report together, so the feature run writes an
# intermediate package summary before the final workspace-wide reports.
cargo llvm-cov --workspace --no-report
cargo llvm-cov -p sekirei \
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
# percentage as well, so 91.77% cannot pass a 92% gate.
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
# the badge value and is not used to pass the 92% gate.
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
