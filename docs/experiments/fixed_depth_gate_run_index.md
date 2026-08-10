# Fixed-depth A/B gate: run index

Tracks every `workflow_dispatch` of `.github/workflows/fixed-depth-ab.yml`,
including invalid/discarded runs, per the run-record convention used
elsewhere in this directory (e.g. `pr4_gate_attempt_index.md`). Runs are
never deleted from GitHub Actions history when found invalid -- they're
recorded here as provenance for why the gate infrastructure itself changed.

## SpecTopN=3 fixed-depth results are noise-dominated (major finding, 2026-08-10)

**Before reading any SpecTopN=3 result below**: two null A/A runs (the
*identical* binary, `base_sha == candidate_sha`, run twice) at
`SpecTopN=3, threads=1, depth=9` produced 5/21 and 6/21 bestmove diffs and
node-ratio swings up to **5.79x** -- with zero code difference between the
two sides. R17_3's 6/21 bestmove diffs and 2.15x max node ratio (PR #17 vs
`main`) sit entirely inside that same-binary noise envelope. **The 6/21 and
2.15x cannot be attributed to PR #17's code.**

Mechanism: `SpecTopN=3` builds `SpeculativeSearcher`'s own rayon thread
pool (`SpecState.pool`, sized `top_n.max(1)` in `SpeculativeSearcher::new`)
independently of the `Threads` USI option -- `Threads=1` does not make a
`SpecTopN=3` run single-threaded. Those background workers write the
*shared* `Tt` concurrently; write ordering depends on OS thread scheduling,
which varies run to run even for the identical binary on identical
positions. **`SpecTopN=0` is currently the gate's only configuration with
resolving power for node-count/bestmove comparison** -- confirmed
separately below (run 31366255282: 0/21 bestmove diffs, node ratio exactly
1.0 on every position at `SpecTopN=0`, same-binary). This governs how PR
#16 and PR #4 must be evaluated too, not just PR #17.

This reframes R17_3 from "MIXED, possibly a PR #17 regression" to
**"no measurement -- the tool's SpecTopN=3 mode cannot currently
distinguish a real effect from scheduling noise at this magnitude."** See
`king_danger_nyugyoku_full_army`/`jishogi_mutual_impasse`/etc. below for
the specific runs.

Given this, `SpeculativeSearcher`'s shared-TT nondeterminism itself is now
a higher-priority target than PR #17's fine-grained qsearch tuning -- see
"Next: PR #16 as a reproducibility fix candidate" below. PR #17's own
fixed-depth investigation is paused until a fix (PR #16 or a successor)
demonstrably reduces this variance; re-evaluating PR #17 against noise this
large isn't a meaningful measurement.

## run 31364261314 (R17_0) -- VALID_GOOD_ISOLATED

- workflow: `fixed-depth-ab.yml`, ref `fix/remote-gate-ref-resolution` (post-interactive-driver-fix)
- `base_sha=main` (`9f45ccf7`), `candidate_sha=5def97690d3dc6df06846ff2a06048a2ace3f4be` (PR #17 head, `candidate_pr=17`), `depth=9`, `threads=1`, **`spec_top_n=0`**
- provenance: `base_is_ancestor_of_candidate=true`; both binaries advertise `Threads`+`SpecTopN`, handshake completes
- **21/21 positions status=ok on both sides** -- zero panic/timeout/illegal/unexpected_resign/incomplete_output

### Results

- median node ratio (candidate/base): **0.9987**
- node ratio range: 0.7513 .. 1.0124
- bestmove differs: 1/21 (`king_danger_nyugyoku_full_army`: base `6i5h` cp 0 vs candidate `5c5d` cp 50 -- well under the 200cp threshold)
- score_cp differs by >200: 0/21

**Validated deterministic** by run 31366255282 (same-binary null A/A at
`SpecTopN=0`: 0/21 bestmove diffs, node ratio exactly 1.0 everywhere) -- so
this 1/21 diff and 0.75-1.01 range are real signal from PR #17's code, not
noise.

Verdict: **VALID_GOOD_ISOLATED** -- the qsearch-TT change itself, with
speculation disabled, is structurally clean and approximately node-neutral.
**Not** a strength-improvement claim -- only that it doesn't regress
correctness or blow up node counts in isolation. Still a draft-stays-draft
result on its own; see "PR #17 does not merge yet" below.

## run 31364492445 (R17_3) -- VALID_NOISE_DOMINATED

- workflow: `fixed-depth-ab.yml`, ref `fix/remote-gate-ref-resolution`
- `base_sha=9f45ccf75758b92e67eea7cd5ae05c63f6cca8d9`, `candidate_sha=5def97690d3dc6df06846ff2a06048a2ace3f4be` (PR #17 head, `candidate_pr=17`), `gate_tooling_sha=f97b6922aa6d6d68319b8c39186e94b65f1030d8`
- `depth=9`, `threads=1`, `spec_top_n=3` (production-default interaction case)
- provenance: `base_is_ancestor_of_candidate=true`; both binaries advertise `Threads`+`SpecTopN`, handshake completes
- **21/21 positions status=ok on both sides** -- zero panic/timeout/illegal/unexpected_resign/incomplete_output

### Results

- bestmove differs: **6/21** (`check_evasion_plain_sennichite`, `check_evasion_continuous_check_white`, `king_danger_nyugyoku_full_army`, `king_danger_nyugyoku_bare_king`, `king_danger_nyugyoku_insufficient_points`, `opening_4ply`)
- score_cp differs by >200: 0/21
- median node ratio (candidate/base): 0.99505
- node ratio range: **0.6115 .. 2.1541**

### Noise-floor comparison (decisive)

| run | base | candidate | code delta | bestmove diffs | node ratio range |
|---|---|---|---|---|---|
| R17_3 (31364492445) | main | PR #17 head | qsearch TT (all of it) | 6/21 | 0.61 .. 2.15 |
| null A/A #1 (31365516737) | main | main (identical) | **none** | 6/21 | 0.56 .. **5.79** |
| null A/A #2 (31365820361) | main | main (identical) | **none** | 5/21 | 0.31 .. 2.21 |
| Arm A (31365816852) | main | qsearch cutoff+store only, no TT ordering | partial | 6/21 | 0.12 .. 1.96 |
| Arm B (31365818510) | main | TT ordering only, no cutoff+store | partial | 5/21 | 0.54 .. 1.44 |

R17_3's 6/21 and 2.15x are **inside** the range produced by two
zero-code-delta null A/A runs (which independently reach 5-6/21 and, in one
case, 5.79x -- larger than R17_3's own max). Arm A and Arm B (the
interaction ablation, see below) land in the same range regardless of which
half of PR #17's change each keeps. **No configuration here shows a signal
distinguishable from the SpecTopN=3 same-binary noise floor.**

Verdict: **VALID_NOISE_DOMINATED** -- provenance-clean and gate-clean
(21/21 ok), but the measurement itself has no resolving power at this
`SpecTopN=3`/depth/corpus configuration. **No candidate-specific regression
has been demonstrated, and no candidate-specific improvement has either.**
The earlier "MIXED, possibly a real interaction effect" framing is
retracted -- it was a missing-noise-floor-baseline artifact, not evidence
against PR #17.

### Ablation (completed, inconclusive by construction)

Three arms, same `base_sha=9f45ccf...`, `depth=9`, `threads=1`,
`spec_top_n=3`, same corpus, run once each:

- **Arm A** (`diag/qsearch-tt-arm-a`, `72dd301b3376f2ac0c6c39d2ac6fc77a701baf80`): PR #17's qsearch score/bound cutoff+store kept, TT-move ordering removed. Run 31365816852: 6/21 bestmove diffs, node ratio 0.12 .. 1.96.
- **Arm B** (`diag/qsearch-tt-arm-b`, `c3ea17ad2c8515d42855a3ab9457d42a56348dfe`): PR #17's TT-move ordering kept, score/bound cutoff+store removed (forced `cacheable=false`). Run 31365818510: 5/21 bestmove diffs, node ratio 0.54 .. 1.44.
- **Arm C**: PR #17 unmodified -- R17_3 above.

**Ablation inconclusive because each arm's observed differences lie within
the SpecTopN=3 same-binary noise envelope** (see table above) -- there is no
signal above the noise floor to attribute to either the score/bound-caching
half or the TT-move-ordering half of PR #17's change. No further runs
planned for these arms; both diagnostic branches are kept on `origin` as
evidence but will not be opened as PRs.

## Next: PR #16 as a reproducibility fix candidate

PR #16 (`fix/spec-parent-tt-race`, issue #14) removes `SpecGroup::spawn`'s
closure store to the **parent** hash -- i.e. it removes exactly the shape
of nondeterministic write (`SpecGroup` candidate tasks racing to store
competing `Bound::Exact` entries at the same hash, final content dependent
on completion order) that plausibly explains what the null A/A runs just
measured (same binary, same position, same depth, different bestmove, node
swings up to 5.79x). PR #16 is being evaluated as a **search-reproducibility
fix candidate**, not merely by average node count, using a `repeats`-mode
extension to the gate (within-binary variance across N repeats, base vs
candidate) -- see below and the PR #16 section of this doc once results
land.

PR #17's own re-evaluation is paused until PR #16 (or a successor fix)
demonstrably reduces `SpecTopN=3` variance -- re-measuring PR #17 against
noise this large isn't informative. Same reasoning applies to PR #4's
fixed-depth evaluation, which is also paused, independent of the local-CPU
resource situation.

## run 31362228815 -- INVALID_CONFIG

- workflow: `fixed-depth-ab.yml`, ref `main`
- dispatched: 2026-08-10, `base_sha=main`, `candidate_sha=7274b5f686051d98ca2d2e19caef117dc51ef380`, `depth=9`, `threads=1`, `spec_top_n=0`
- resolved: `base_sha=9f45ccf75758b92e67eea7cd5ae05c63f6cca8d9` (post-PR#19 main), `candidate_sha=7274b5f686051d98ca2d2e19caef117dc51ef380` (PR #17's rebased-onto-pre-PR#18-main head)
- status: **INVALID_CONFIG** -- not usable as PR #17 fixed-depth A/B evidence

### Reason

PR #18 (SpecTopN USI option) merged to `main` at 06:06 UTC, before this
run's `base_sha=main` was resolved -- so base advertised and honored
`setoption name SpecTopN`. The candidate SHA (`7274b5f...`) was PR #17's
head at the time, which was rebased onto a **pre-PR#18** main and therefore
never had the `SpecTopN` option at all. `run_fixed_depth_ab.py` (at this
run's version) sent `setoption name SpecTopN value 0` to both binaries
unconditionally and had no way to detect that the candidate silently
ignored an unrecognized option and kept running its old hardcoded
`top_n=3`.

Net effect: the intended comparison (`SpecTopN=0` vs `SpecTopN=0`, isolating
the qsearch-TT change) was actually `SpecTopN=0` (base) vs `SpecTopN=3`
(candidate, unintentionally) -- a configuration asymmetry, not a measurement
of PR #17's qsearch-TT effect.

### Corroborating red flag in the data

`jishogi_mutual_impasse_boundary`: base `depth=1, nodes=29` vs candidate
`depth=9, nodes=6916` -- node ratio 238.48x. A magnitude far outside anything
plausible from a qsearch-TT-only change, consistent with the candidate
actually running full 3-way speculative search while base ran none.

### Results NOT to be cited as PR #17 evidence

- median node ratio: 0.99895
- bestmove differs: 1/21
- `king_danger_nyugyoku_full_army` node ratio: 0.8357
- `opening_4ply` node ratio: 0.7513

None of the above are attributable to the qsearch-TT change in isolation --
discard as PR #17 evidence.

### What IS still valid from this run

Infrastructure smoke evidence only: 21/21 positions completed on both
sides with zero panics, zero timeouts, zero illegal-move detections, and
the full build -> run -> compare -> artifact-upload pipeline completed
end to end. This confirms the remote gate's mechanics work; it says nothing
about qsearch-TT correctness or performance.

### Follow-up (implemented same day)

1. `probe_usi_capabilities()` / `require_usi_capabilities()` added to
   `run_fixed_depth_ab.py` -- hard-fails a `run` invocation with
   `CONFIG_UNSUPPORTED` if the binary doesn't advertise `Threads`/`SpecTopN`
   or doesn't complete the `usiok`/`readyok` handshake, before any corpus
   position runs. Regression tests: `scripts/test_run_fixed_depth_ab.py`.
2. Branch-ancestry guard added to `fixed-depth-ab.yml`'s "Resolve inputs"
   step -- `git merge-base --is-ancestor "$BASE_SHA" "$CANDIDATE_SHA"`,
   hard-fails with `CANDIDATE_NOT_BASED_ON_BASE` unless
   `allow_non_ancestor=true` is explicitly passed. Regression test:
   `scripts/test_ancestor_guard.sh`.
3. `metadata.json` extended with `base_input`, `candidate_input`,
   `candidate_pr`, `base_is_ancestor_of_candidate` for provenance.
4. PR #17 rebased onto latest `main` (post-PR#18) so a re-run's candidate
   SHA is a true descendant of base and advertises every option base does.

See `fix/remote-gate-ref-resolution` branch for the guard implementation
and `.github/workflows/fixed-depth-ab.yml` for the current guarded version.

## run 31363151597 -- INVALID_HARNESS

- workflow: `fixed-depth-ab.yml`, ref `fix/remote-gate-ref-resolution`
- dispatched: 2026-08-10, `base_sha=main`, `candidate_sha=5def97690d3dc6df06846ff2a06048a2ace3f4be` (PR #17's head, freshly rebased onto post-PR#19 main), `depth=9`, `threads=1`, `spec_top_n=0`
- status: **INVALID_HARNESS** -- not usable as PR #17 fixed-depth A/B evidence

### Configuration provenance: VALID

Unlike run 31362228815, this run's setup was correct:

- base is an ancestor of candidate (branch-ancestry guard passed)
- both binaries advertise `Threads`
- both binaries advertise `SpecTopN`
- the requested `SpecTopN=0` was accepted through the USI handshake on
  both sides (option-capability guard passed)

### Invalidation reason

`run_fixed_depth_ab.py`'s `run_one_position()` (at this run's version)
sent the entire command script -- `usi`, `setoption`, `isready`,
`position`, `go depth N`, `quit` -- as one string via
`subprocess.run(input=...)`. Sekirei's `go` is asynchronous: it spawns a
search thread and the main USI loop returns immediately to read the next
stdin line, which in this driver's script was always `quit`. The main
loop then read `quit` and called `abort_and_join_inflight_search()`,
which aborts the in-flight search before it can complete -- the search
thread's own abort path still emits a `bestmove` line, but with
`info.best_move == None` it prints `bestmove resign` instead of a real
move, with no preceding `info depth ...` line.

In effect, every position's actual search time was however long it took
the OS to schedule `quit` behind `go` -- close to zero, and racing against
however long that position's search would otherwise take. Positions whose
search happened to still complete first for other unrelated timing reasons
returned real results; positions that lost the race returned
`bestmove resign` with `depth=None, nodes=None`. Observed directly in this
run's data: `opening_startpos` (base), `check_evasion_continuous_check_white`
and `opening_1ply_7f` (candidate) all returned `bestmove resign`.

This is a bug in the gate driver, not a PR #17 regression.

### Results NOT to be cited as PR #17 evidence

No node ratio, bestmove-difference, or score comparison from this run may
be used as PR #17 evidence -- an unknown number of the remaining "ok"-looking
positions may also have won the race by chance rather than completing a
real depth-9 search, so even superficially clean-looking rows aren't
trustworthy from this run.

### What IS still valid from this run

Provenance-guard smoke evidence only: the branch-ancestry guard and the
USI option-capability guard (added after run 31362228815) both passed
correctly on this run, confirming those two guards work as intended. This
says nothing about qsearch-TT correctness or performance.

### Follow-up (implemented same day)

`run_one_position()` rewritten from a single `subprocess.run(input=...)`
call to an interactive `subprocess.Popen`-based USI driver (reader
thread + queue for stdout, bounded by an overall per-position deadline) that
waits for `usiok`, then `readyok`, then an actual `bestmove` line before
ever sending `quit`. A timeout now escalates `stop` -> short grace period
for a bestmove -> `quit` -> kill, and never treats anything read during
that escalation as a normal-timing result. `bestmove resign` when not
explicitly allowed by the corpus entry (`allow_resign`, default `false`)
is now a hard `unexpected_resign` correctness failure, and a non-resign
bestmove with no `depth_reached` is `incomplete_output` -- both excluded
from node-ratio/bestmove-diff computation in `compare`, matching how
panic/timeout/illegal were already excluded. Regression tests:
`scripts/test_run_fixed_depth_ab.py` (`InteractiveDriverTests`,
`ClassifyResignTests`) -- a fake engine models the real engine's
asynchronous `go`, confirming the old all-at-once-input approach still
reproduces the resign race against it while the new interactive driver
gets the real result.
