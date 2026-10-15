# Lazy SMP depth-preserving consensus (v0.4.0 base)

Status: fixed diagnostics passed on 2026-10-13. The independent gate reached
its cap on 2026-10-14 but is invalid for adoption because its evidence set is
incomplete. The playing default is unchanged.

## Candidate

`LazyFlags` bit 1024 changes only the final worker-result selector. It first
restricts eligible results to workers that completed the maximum depth. Every
worker may then support one of those eligible moves, but a move found only by
shallower workers cannot win. The selected score, PV and node count still come
from a maximum-depth worker.

The candidate is `LazyFlags=1039`; the baseline is `LazyFlags=15`. Worker
search, evaluator, pruning, time limits, root order and TT sharing are
otherwise identical. A completed exact winning mate still overrides ordinary
depth and consensus selection.

The source base is `origin/main` at
`5addafbba4e8a692b3bb6575df3a86800cf6ce64`.

## Deterministic checks

Unit tests require all of the following:

- a supported move may win only when a worker completed it at maximum depth;
- a shallower majority cannot reduce the selected completed depth;
- no-move results use deterministic score and move tie-breaks;
- a completed exact winning mate remains stronger than a deeper heuristic
  result;
- the public selector path and the advertised USI flag range remain covered.

## Fixed diagnostic

Use material evaluation, `Hash=64`, four threads, 300 ms, two repeats and 20
positions from the Sekirei-generated `openings_mat1000.sfen`. The complete
source corpus has SHA-256
`af0e7d51a130f69fada0ed3f4e664dfa08e8d24927b0d24df4f01261e3b71371`.
Freeze the diagnostic positions before running and run both baseline-first and
candidate-first passes.

The candidate advances only if it preserves median completed depth in both
orders, changes at least one returned move on the fixed sample, and does not
reduce repeated-bestmove stability by more than 10 percentage points. NPS is
reported but is not an advancement criterion because the selector runs after
worker search finishes.

The frozen 20-position subset has SHA-256
`4c3ad4acad80aebee2648e7d708878522943a1ca15b1fd80102dc59aba0920d0`.
After discarding one run whose JSON write failed because the host disk was
full, the complete ABBA rerun produced 80 observations per side:

| Selector | Median selected depth | Repeated-bestmove stability |
|---|---:|---:|
| baseline, flags 15 | 17.0 | 77.5% |
| candidate, flags 1039 | 18.0 | 75.0% |

The candidate changed the modal returned move on 2 of 20 positions. Its
position-wise median selected-depth delta was +1.5 plies; 19 positions were
positive or unchanged and one was -0.5. The 2.5-point stability reduction is
inside the preregistered 10-point limit. The candidate therefore advances to
the independent gate. The discarded ENOSPC run is not pooled with these
figures.

## Independent gate

If the diagnostic advances, freeze 80 different positions from the same
Sekirei-generated corpus before playing any gate games. Compare flags 1039
against 15 with material evaluation, `Hash=64`, four threads per side, 300 ms,
`UseBook=false`, colour reversal and paired trinomial SPRT. Fix H0=0 Elo,
H1=+10 Elo, alpha=beta=0.05 and an 800-game cap before starting.

Only H1 from that independent run permits a default change. H0 rejects the
candidate. A capped run, incomplete colour pair, invalid game, missing raw
trial record or artifact-write failure is `INCONCLUSIVE` and cannot be pooled
with another run.

The frozen 80-position gate subset has SHA-256
`d801423c58d889683aff2c7c4281411b5f577233fe04fd817066acdf185a2092`.

The run reached the 800-game cap at 377-384-39. Its paired trinomial SPRT was
`INCONCLUSIVE` with LLR -0.4024 (bounds +/-2.9444), and the candidate Elo
estimate was -3.0 +/-23.5. The terminal result has SHA-256
`f81b103181cc3cded335b897f755c036bcc699671dbccb82c93002b7b1848203`.

Those statistics are diagnostic only. The durable evidence contains 799 of
800 expected kifu files: `game0064.txt` was not written after the host ran out
of disk space. It also records `game0031: TimeForfeit` and an artifact-write
failure. The result therefore fails the preregistered completeness contract
and cannot authorize a default change, be repaired after the fact, or be
pooled with another run. The LaunchAgent has terminated; any later candidate
requires a fresh run and unused openings.
