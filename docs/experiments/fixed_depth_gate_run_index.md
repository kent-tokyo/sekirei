# Fixed-depth A/B gate: run index

Tracks every `workflow_dispatch` of `.github/workflows/fixed-depth-ab.yml`,
including invalid/discarded runs, per the run-record convention used
elsewhere in this directory (e.g. `pr4_gate_attempt_index.md`). Runs are
never deleted from GitHub Actions history when found invalid -- they're
recorded here as provenance for why the gate infrastructure itself changed.

## run 31364261314 (R17_0) -- VALID, GOOD

- workflow: `fixed-depth-ab.yml`, ref `fix/remote-gate-ref-resolution` (post-interactive-driver-fix)
- dispatched: 2026-08-10, `base_sha=main` (`9f45ccf7`), `candidate_sha=5def97690d3dc6df06846ff2a06048a2ace3f4be` (PR #17 head, `candidate_pr=17`), `depth=9`, `threads=1`, `spec_top_n=0`
- provenance: `base_is_ancestor_of_candidate=true`; both binaries advertise `Threads`+`SpecTopN` and complete the `usiok`/`readyok` handshake (confirmed in `usi_capabilities` in both result JSONs)
- **21/21 positions status=ok on both sides** -- zero panic/timeout/illegal/unexpected_resign/incomplete_output

### Results (first trustworthy PR #17 measurement)

- median node ratio (candidate/base): **0.9987**
- node ratio range: 0.7513 .. 1.0124 (no explosion; `opening_4ply` shows the largest reduction)
- bestmove differs: 1/21 (`king_danger_nyugyoku_full_army`: base `6i5h` cp 0 vs candidate `5c5d` cp 50 -- well under the 200cp "notably different" threshold)
- score_cp differs by >200: 0/21

Verdict per the GOOD/NEUTRAL/BAD framework: **GOOD** -- zero correctness issues, no unnatural bestmove/score divergence, no pathological node explosion. Still a draft-stays-draft result, not a merge decision by itself; this is the qsearch-TT-isolated measurement (`SpecTopN=0`, no speculative-search interaction).

## run 31364492445 (R17_3) -- VALID_MIXED

- workflow: `fixed-depth-ab.yml`, ref `fix/remote-gate-ref-resolution`
- `base_sha=9f45ccf75758b92e67eea7cd5ae05c63f6cca8d9`, `candidate_sha=5def97690d3dc6df06846ff2a06048a2ace3f4be` (PR #17 head, `candidate_pr=17`), `gate_tooling_sha=f97b6922aa6d6d68319b8c39186e94b65f1030d8`
- `depth=9`, `threads=1`, `spec_top_n=3` (production-default interaction case)
- provenance: `base_is_ancestor_of_candidate=true`; both binaries advertise `Threads`+`SpecTopN`, handshake completes
- **21/21 positions status=ok on both sides** -- zero panic/timeout/illegal/unexpected_resign/incomplete_output

### Results

- bestmove differs: **6/21** (`check_evasion_plain_sennichite`, `check_evasion_continuous_check_white`, `king_danger_nyugyoku_full_army`, `king_danger_nyugyoku_bare_king`, `king_danger_nyugyoku_insufficient_points`, `opening_4ply`)
- score_cp differs by >200: 0/21
- median node ratio (candidate/base): 0.99505 -- neutral
- node ratio range: **0.6115 .. 2.1541**
- notable outliers (absolute node counts, not just ratios):
  - `king_danger_nyugyoku_full_army`: base 684,641 -> candidate 1,474,816 nodes (ratio 2.1541, delta ~790k), bestmove `6i5h`->`5c5d`, score 150cp->50cp
  - `king_danger_nyugyoku_insufficient_points`: base 28,556 -> candidate 48,277 nodes (ratio 1.6906), bestmove `8b8a+`->`9c9b+`, score 2249cp->2300cp
  - `jishogi_mutual_impasse`: base 2,356 -> candidate 3,634 nodes (ratio 1.5424), bestmove unchanged
  - `check_evasion_plain_sennichite`: base 565 -> candidate 792 nodes (ratio 1.4018, delta ~227 nodes -- small-N artifact, not a large-magnitude outlier despite the ratio)

### Comparison against R17_0 (SpecTopN=0)

| | SpecTopN=0 (R17_0) | SpecTopN=3 (R17_3) |
|---|---|---|
| bestmove diffs | 1/21 | 6/21 |
| median node ratio | 0.9987 | 0.99505 |
| node ratio range | 0.7513 .. 1.0124 | 0.6115 .. 2.1541 |

qsearch TT alone (`SpecTopN=0`) is nearly neutral; the production-default
interaction with `SpeculativeSearcher`'s concurrent background tasks
(`SpecTopN=3`) shows materially more divergence. `king_danger_nyugyoku_full_army`
gives the same bestmove flip (`6i5h`->`5c5d`) in both R17_0 and R17_3, with
score deltas of 50cp and 100cp respectively, well under the 200cp
threshold -- a TT added to qsearch legitimately changes move ordering, so a
fixed-depth bestmove flip on a near-equal position here is an expected
consequence of the feature working, not by itself evidence of a defect.
The other 5 SpecTopN=3-only divergences and the node-count outliers are
the open question this ablation is chasing.

Verdict: **VALID_MIXED** -- provenance-clean and gate-clean (21/21 ok), but
not GOOD: the 6/21 bestmove divergence rate and the 1.4x-2.15x local node
outliers at the production `SpecTopN=3` default are not yet distinguishable
from a real search-quality effect. **PR #17 stays draft, not merge-recommended,
pending the ablation below.**

### Ablation plan (in progress)

Three arms, same `base_sha=9f45ccf...`, `depth=9`, `threads=1`, `spec_top_n=3`, same corpus:

- **Arm A** (`diag/qsearch-tt-arm-a`, `72dd301b3376f2ac0c6c39d2ac6fc77a701baf80`): PR #17's qsearch score/bound cutoff+store kept, TT-move ordering removed.
- **Arm B** (`diag/qsearch-tt-arm-b`, `c3ea17ad2c8515d42855a3ab9457d42a56348dfe`): PR #17's TT-move ordering kept, score/bound cutoff+store removed (forced `cacheable=false`).
- **Arm C**: PR #17 unmodified -- this is R17_3 above.

Both diagnostic branches are throwaway (not merge candidates); not committed
to PR #17 itself.

In parallel: two null A/A runs (`base_sha=candidate_sha=9f45ccf...`,
same `depth`/`threads`/`spec_top_n=3`) to measure how much bestmove/node-count
variance exists between two runs of the *identical* binary under concurrent
speculative scheduling alone -- the noise floor R17_3's 6/21 and 2.15x must be
compared against before attributing them to PR #17's code change.

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
