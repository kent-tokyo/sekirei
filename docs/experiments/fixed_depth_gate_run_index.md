# Fixed-depth A/B gate: run index

Tracks every `workflow_dispatch` of `.github/workflows/fixed-depth-ab.yml`,
including invalid/discarded runs, per the run-record convention used
elsewhere in this directory (e.g. `pr4_gate_attempt_index.md`). Runs are
never deleted from GitHub Actions history when found invalid -- they're
recorded here as provenance for why the gate infrastructure itself changed.

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
