#!/usr/bin/env bash
set -euo pipefail

root_dir=$(cd "$(dirname "$0")/.." && pwd)
run_dir=$(mktemp -d "${TMPDIR:-/tmp}/sekirei-teacher-eval.XXXXXX")
trap 'rm -rf "$run_dir"' EXIT
cd "$root_dir"

fixture="scripts/fixtures/nnue_phase3_pilot.jsonl"
common=(--offline -p sekirei-train -- --positions "$fixture" --epochs 1 --sample 1 --label-depth 1 --lr 0.02 --init-seed 7 --split-seed 42)

# Produce a small, valid fixed teacher using the legacy/default material
# contract. This is fixture setup, not a quality or strength measurement.
cargo run "${common[@]}" \
  --output "$run_dir/teacher.bin" \
  --teacher-cache "$run_dir/material-cache.jsonl" \
  >"$run_dir/material.log" 2>&1

teacher_identity=$(python3 -c 'import sys
h = 14695981039346656037
for b in open(sys.argv[1], "rb").read():
    h = ((h ^ b) * 1099511628211) & ((1 << 64) - 1)
print(f"nnue:{h:016x}")' "$run_dir/teacher.bin")

cargo run "${common[@]}" \
  --teacher-eval nnue --teacher-weights "$run_dir/teacher.bin" \
  --teacher-cache "$run_dir/nnue-cache.jsonl" \
  --output "$run_dir/student.bin" \
  >"$run_dir/nnue.log" 2>&1

grep -F "Teacher evaluator: $teacher_identity" "$run_dir/nnue.log" >/dev/null
python3 -c 'import json, sys
expected = sys.argv[2]
rows = [json.loads(line) for line in open(sys.argv[1]) if line.strip()]
assert len(rows) == 6
assert all(row["teacher_identity"] == expected for row in rows)' \
  "$run_dir/nnue-cache.jsonl" "$teacher_identity"

# Exact identity reuse must avoid search; material mode must reject every
# fixed-NNUE cache entry rather than silently mixing teacher signals.
cargo run "${common[@]}" \
  --teacher-eval nnue --teacher-weights "$run_dir/teacher.bin" \
  --teacher-cache "$run_dir/nnue-cache.jsonl" --reuse-teacher-cache --cache-only \
  --output "$run_dir/cached-student.bin" \
  >"$run_dir/cache-hit.log" 2>&1
grep -F "all 6 entries from cache (no search)" "$run_dir/cache-hit.log" >/dev/null

# A bounded search is a distinct labeling contract even with identical
# weights and requested depth; an unlimited cache must not be reused.
if cargo run "${common[@]}" \
  --teacher-eval nnue --teacher-weights "$run_dir/teacher.bin" --label-time-ms 1 \
  --teacher-cache "$run_dir/nnue-cache.jsonl" --reuse-teacher-cache --cache-only \
  --output "$run_dir/wrong-budget.bin" \
  >"$run_dir/cache-budget-mismatch.log" 2>&1; then
  echo "teacher fixture failed: bounded mode accepted an unlimited cache" >&2
  exit 1
fi

# A deterministic node budget is also isolated from an unlimited cache.
if cargo run "${common[@]}" \
  --teacher-eval nnue --teacher-weights "$run_dir/teacher.bin" --label-nodes 64 \
  --teacher-cache "$run_dir/nnue-cache.jsonl" --reuse-teacher-cache --cache-only \
  --output "$run_dir/wrong-node-budget.bin" \
  >"$run_dir/cache-node-budget-mismatch.log" 2>&1; then
  echo "teacher fixture failed: node-bounded mode accepted an unlimited cache" >&2
  exit 1
fi

# Two fresh node-bounded runs must produce byte-identical cache labels. This
# exercises the deterministic budget through the real CLI, not only Budget's
# unit test.
for suffix in a b; do
  cargo run "${common[@]}" \
    --teacher-eval nnue --teacher-weights "$run_dir/teacher.bin" --label-nodes 64 \
    --teacher-cache "$run_dir/node-cache-$suffix.jsonl" \
    --output "$run_dir/node-student-$suffix.bin" \
    >"$run_dir/node-$suffix.log" 2>&1
done
cmp "$run_dir/node-cache-a.jsonl" "$run_dir/node-cache-b.jsonl"
grep -F "Teacher evaluator: ${teacher_identity}:nodes64" "$run_dir/node-a.log" >/dev/null

if cargo run "${common[@]}" \
  --teacher-cache "$run_dir/nnue-cache.jsonl" --reuse-teacher-cache --cache-only \
  --output "$run_dir/wrong-teacher.bin" \
  >"$run_dir/cache-mismatch.log" 2>&1; then
  echo "teacher fixture failed: material mode accepted a fixed-NNUE cache" >&2
  exit 1
fi

echo "teacher eval fixture OK: fixed NNUE loaded and cache identity enforced"
