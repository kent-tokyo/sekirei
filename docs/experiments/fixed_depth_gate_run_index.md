# Fixed-depth A/B gate: run index

Status: **historical measurement record (2026-08-10)**. This document keeps
the verdicts and numbers that changed the gate design. GitHub Actions retains
the full logs. Current runs must use `.github/workflows/fixed-depth-ab.yml` and
`scripts/run_fixed_depth_ab.py`; old commands and SHAs are not current launch
instructions.

## Main conclusion

`SpecTopN=3`, `Threads=1`, depth 9 was noise-dominated. Two identical-binary
A/A runs produced 5/21 and 6/21 best-move differences and node-ratio swings up
to 5.79x. `Threads=1` did not make the search single-threaded because the
speculative pool remained active and wrote to the shared TT.

Consequences:

- Use `SpecTopN=0` when fixed-depth node counts or best moves must be
  deterministic.
- For `SpecTopN>0`, measure within-binary repeatability before attributing a
  difference to a candidate.
- A clean harness run is correctness evidence, not a strength claim.

## Valid runs

| Run | Configuration | Result | Verdict |
|---|---|---|---|
| `31364261314` (R17_0) | PR #17 vs main, depth 9, threads 1, `SpecTopN=0` | 21/21 valid; median node ratio 0.9987; range 0.7513–1.0124; best move differed 1/21; no score difference above 200 cp | `VALID_GOOD_ISOLATED`: approximately node-neutral and structurally clean; not a strength result |
| `31364492445` (R17_3) | Same comparison with `SpecTopN=3` | 21/21 valid; median 0.99505; range 0.6115–2.1541; best move differed 6/21 | `VALID_NOISE_DOMINATED` |
| `31367406754` | PR #16 repeatability, three repeats, `SpecTopN=3` | best-move variance 9/21→3/21; median node swing 1.0906→1.0213; p90 1.4356→2.0954; max 2.3774→3.2015; no correctness failures | `PARTIAL PASS`: parent-hash race was one source, not the only source |
| `31398746511` | PR #17 after PR #16, three repeats, `SpecTopN=3` | best-move variance 2/21→4/21; median node swing 1.0316→1.1242; p90 1.7355→1.7056; max 2.6578→2.3749 | Not merge-recommended; magnitude unresolved at three repeats |
| `31399124391` | PR #17 control, `SpecTopN=0`, three repeats | both sides 0/21 best-move variance and node ratio 1.0 | Deterministic control passed |

The R17_3 result lies inside the identical-binary noise envelope:

| Run | Code delta | Best-move differences | Node-ratio range |
|---|---|---:|---:|
| R17_3 | full PR #17 | 6/21 | 0.61–2.15 |
| A/A #1 (`31365516737`) | none | 6/21 | 0.56–5.79 |
| A/A #2 (`31365820361`) | none | 5/21 | 0.31–2.21 |
| qsearch cutoff/store only (`31365816852`) | partial | 6/21 | 0.12–1.96 |
| TT ordering only (`31365818510`) | partial | 5/21 | 0.54–1.44 |

The two PR #17 ablation arms therefore remained inconclusive. Neither half
showed a signal above the A/A noise floor.

## Invalid runs and harness fixes

| Run | Classification | Why it is invalid | Durable fix |
|---|---|---|---|
| `31362228815` | `INVALID_CONFIG` | Compared binaries did not support the same requested USI options and branch ancestry was not guaranteed | Capability handshake, exact option checks, ancestry guard, and provenance fields |
| `31363151597` | `INVALID_HARNESS` | The driver sent `go` and `quit` together; asynchronous search was aborted and could emit `bestmove resign` | Interactive USI driver waits for `usiok`, `readyok`, and a real `bestmove`; bounded timeout escalation and output classification |

Results from invalid runs must not be cited as candidate performance. They are
useful only as evidence that the guards were needed.

## Interpretation of PR #16 and PR #17

PR #16 removed a guaranteed parent-hash last-writer-wins collision. Its clean
reduction in best-move variance justified merging the correctness fix, while
the unresolved tail remained follow-up work.

PR #17's post-fix repeatability result moved the primary stability metrics in
the wrong direction and did not clear the adoption bar. The static shared-TT
audit is summarized in
[`../design/shared_tt_write_topology_audit.md`](../design/shared_tt_write_topology_audit.md).

## Reuse rules

1. Freeze both revisions, binary hashes, corpus hash, depth, options, and
   timeout before execution.
2. Refuse unsupported options or a non-descendant candidate before searching.
3. Reject timeout, panic, illegal move, unexpected resignation, or missing
   completed-depth output from comparison statistics.
4. Run an identical-binary A/A control for nondeterministic configurations.
5. Keep `PASS`, `FAIL`, `INCONCLUSIVE`, and invalid-execution states distinct.

These rules are the durable output of this run series; the individual old
branches and workflow dispatches are retained only for provenance.
