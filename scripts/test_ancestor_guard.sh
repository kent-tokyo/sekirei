#!/bin/sh
# Regression test for the branch-ancestry guard in
# .github/workflows/fixed-depth-ab.yml's "Resolve inputs" step
# (git merge-base --is-ancestor "$BASE_SHA" "$CANDIDATE_SHA").
#
# Builds a throwaway git repo under /tmp so this never touches the real
# sekirei repo or requires cargo/rustc -- pure git plumbing, cheap enough to
# run on a loaded host.
#
# Run: sh scripts/test_ancestor_guard.sh
set -eu

TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

git -C "$TMP" init -q
git -C "$TMP" config user.email test@example.com
git -C "$TMP" config user.name test

echo base > "$TMP/f"
git -C "$TMP" add f
git -C "$TMP" commit -q -m base
BASE=$(git -C "$TMP" rev-parse HEAD)

echo descendant > "$TMP/f"
git -C "$TMP" commit -q -am descendant
DESCENDANT=$(git -C "$TMP" rev-parse HEAD)

# Diverged: branch off BASE's parent-less start again from an orphan root,
# so it shares no history with BASE at all.
git -C "$TMP" checkout -q --orphan diverged
git -C "$TMP" rm -rf -q .
echo unrelated > "$TMP/g"
git -C "$TMP" add g
git -C "$TMP" commit -q -m diverged
DIVERGED=$(git -C "$TMP" rev-parse HEAD)

fail() { echo "FAIL: $1" >&2; exit 1; }

if git -C "$TMP" merge-base --is-ancestor "$BASE" "$DESCENDANT"; then
    echo "PASS: base is an ancestor of its own descendant"
else
    fail "expected base to be an ancestor of descendant"
fi

if git -C "$TMP" merge-base --is-ancestor "$BASE" "$DIVERGED"; then
    fail "expected diverged commit to NOT have base as an ancestor"
else
    echo "PASS: diverged commit correctly rejected (base is not an ancestor)"
fi

echo "test_ancestor_guard.sh: all checks passed"
