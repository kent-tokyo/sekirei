# Shared-TT write topology audit

Status: **historical static audit for issue #32 (2026-08-11)**. Source line
numbers and branch SHAs in the original report aged quickly, so this condensed
record keeps the verified control-flow conclusions and their evidence class.
It is not proof that an untraced race occurred in a particular game.

## Why the audit was opened

The fixed-depth repeatability gate observed `SpecTopN=3` best-move and node
count variation even for identical binaries. PR #16 removed one guaranteed
collision: sibling speculative tasks writing competing results to the same
parent hash. Variance fell from 9/21 to 3/21 positions but did not disappear.

The audit asked which remaining search paths can write the same lock-free TT
concurrently and whether equal-depth replacement can make final contents
schedule-dependent.

## TT semantics established by inspection

- Entries are packed into atomics; readers do not observe fields mixed from
  different writers.
- Greater depth replaces lower depth. Equal-depth entries had no semantic
  tie-break in the audited version, so the physical last writer won.
- Read-check-write is not a transaction: another writer may intervene between
  the read and compare/exchange loop.
- Root and interior alpha-beta, speculative alpha-beta, and the audited
  qsearch branch all wrote to the same table.
- Root TT use for move ordering is safe even if the hint came from another
  producer; score/bound cutoffs require the producer's depth contract.

## Producer interaction summary

| Pair | Can overlap? | Same hash/depth possible? | Audit classification |
|---|---|---|---|
| YBW sibling searches | yes | through a transposition at equal remaining depth | plausible, not traced |
| Main search vs speculative search | yes | through a transposition; windows can differ | plausible, not traced |
| Speculative sibling tasks | yes | through a descendant transposition | plausible, not traced |
| Promoted task vs next iteration | yes | wider overlap window | plausible, not traced |
| qsearch vs qsearch in PR #17 | yes | both wrote depth 0 through YBW leaves | plausible, not traced |
| qsearch depth 0 vs real depth ≥1 | yes | hash can match but depth cannot tie | deterministic depth preference; not a real-depth cutoff corruption path |

Position hashes contain board, hands, and side to move rather than path
identity. Different move orders can therefore reach the same hash. A
transposition is structurally possible, but a static audit cannot show that a
specific test position actually produced the collision.

## Findings

### 1. Equal-depth writes are schedule-sensitive

Two correct searches can reach the same position and depth with different
alpha/beta windows. Fail-soft scores and bound types may legitimately differ.
Without a deterministic tie-break, whichever thread writes last controls
future probes. This is an architectural source of repeatability noise, not by
itself a memory-safety failure.

### 2. `SpecTopN=0` does not remove all parallel TT interactions

Disabling the speculative pool removes one source of overlap but YBW can still
run alpha-beta leaves concurrently. PR #17's depth-0 qsearch writes could
therefore race with other qsearch writes without a speculative task being the
colliding producer. The clean control showed a lower-contention condition, not
proof of a unique mechanism.

### 3. Abort-time TT storage was a confirmed correctness bug

Some move loops could stop on budget exhaustion and still fall through to a
final `Exact` or `Upper` store despite leaving moves unsearched. A recursive
call could also notice the abort and return a placeholder score that the
caller used before rechecking the budget.

This did not need a trace: the invalid bound followed directly from control
flow. It was filed as issue #36 and fixed by PR #37 by rechecking aborts after
recursive calls and skipping incomplete stores. Deterministic regression tests
cover the confirmed paths.

## Evidence classification

- **Confirmed:** incomplete searches could store unjustified bounds in the
  audited code; fixed by PR #37.
- **Plausible:** equal-depth transposition collisions among main,
  speculative, or qsearch producers.
- **Ruled out:** a depth-0 qsearch entry winning an equal-depth replacement
  against a real-depth entry.
- **Not concluded:** “PR #17 qsearch TT races caused the measured variance.”
  Correlation and static reachability were insufficient.

## Options considered

| Option | Coverage | Cost / risk | Historical ranking |
|---|---|---|---|
| Deterministic equal-depth bound tie-break | all equal-depth producer pairs | one hot-path branch; no extra memory | first |
| Main-writer priority tag | main/spec interactions | tag plumbing; does not cover qsearch self-collision | second |
| Probe-side producer/generation gate | selected cutoff consumers | tag and read-side logic | second |
| Make speculative search TT read-only | removes spec writes | may lose useful promoted-task entries | later |
| Separate speculative TT | isolates spec traffic | extra memory and policy complexity | last |

The audit did not implement these speculative fixes because none of the
remaining race-shaped findings had a trace. PR #37 addressed the separate,
proven abort bug only.

## Reproduction standard for future work

Before changing replacement policy, capture debug-only events containing at
least `(hash, depth, bound, producer, thread, sequence)` during a repeatable
fixed-depth run. A useful artifact must show two overlapping writes to the same
hash and depth and connect them to a changed probe or result. Then:

1. add a deterministic regression for the exact interaction;
2. change one policy at a time;
3. rerun identical-binary A/A and candidate A/B repeatability;
4. keep correctness, repeatability, node cost, and playing strength as
   separate verdicts.

Related measurement history:
[`../experiments/fixed_depth_gate_run_index.md`](../experiments/fixed_depth_gate_run_index.md).
