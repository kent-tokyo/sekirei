# Search implementation lineage after PR #5 (static audit)

Status: static, read-only audit. No builds, no matches were run to produce this
document — all claims are derived from `git log`/`git merge-base`/`git show`
and from existing result artifacts already on disk.

Purpose: answer, with commit-level evidence, whether pre-PR-#5 strength/weight
results (in particular the Phase A2 B1-vs-A burn-in) can be generalized to the
post-PR-#5 search implementation, and what each existing match artifact is and
isn't evidence for.

## 1. Commit lineage (A–F)

| ID | What | Commit(s) | Notes |
|----|------|-----------|-------|
| A | `origin/main` tip immediately before PR #5 | `fa3e411c7845cf2268814e263bb8621a934bdd41` (2026-08-08) | Contains the depth-stall bug: `SpecGroup::spawn` submits speculative tasks onto rayon's *global* pool, the same pool `alpha_beta`'s own YBW dispatch depends on; an unbounded-lifetime spec task can hold a worker for the rest of the search. Root-caused via `sample` (see `sekirei-abtest-results/sample_stall_proof.txt`, `FINDINGS_INTERIM.md`). |
| B | PR #5 fix content | `0bb42214e9fa3921989da092dd1eabd9b2066839` | "isolate speculative tasks onto a dedicated thread pool." Single commit, rebase-merged (no separate merge commit). |
| C | `origin/main` tip after PR #5 merge | `0bb4221...` (same SHA as B — rebase merge) | Merged 2026-08-09T13:04:50Z. Tree-identical to `fix/spec-pool-isolation`@`8e6a145` (verified `git rev-parse ...^{tree}` equality in the prior session). |
| D | PR #4 rebased onto C | `9b61ed4` on `feat/next-strength-candidate` | Singular-Extension verification-search fix, rebased cleanly (0 conflicts) onto C. Adds the SE fix on top of B/C's depth fix. |
| E | Phase A2 B1-vs-A exploratory burn-in engine | `c399a7cfc8fc76882cb968cdb261bca3db314a32` (2026-07-25T22:23:36+09:00), per `docs/experiments/phase_a2_b1_vs_a_exploratory_burnin.md` manifest | See §2 for ancestry proof. **Does not contain B.** |
| F | PR #4's 3 engineering-gate re-run attempts | base `0bb4221` (=C), candidate `9b61ed4` (=D) | All 3 attempts share the same base/candidate SHAs; they differ only in execution shape (parallel/sequential/micro-batch) and all were killed by host resource contention before producing usable data. Full per-attempt detail: `docs/experiments/pr4_gate_attempt_index.md`. |

## 2. Does E (Phase A2 B1-vs-A) predate PR #5? — proof

```
$ git merge-base --is-ancestor c399a7cfc8fc76882cb968cdb261bca3db314a32 fa3e411   # A, pre-PR5 main
exit 1  (NOT an ancestor)
$ git merge-base c399a7cfc8fc76882cb968cdb261bca3db314a32 fa3e411
4a86234c81c82c1eeb844c4ecff10f456e937504
```

`E` (`c399a7c`) is not an ancestor of `A` (`fa3e411`) — it sits on a separate,
never-pushed-to-`origin` local branch that forked from the shared history at
`4a86234` (2026-07-2x, "fix: correct L2 layer width from 16 to 32 in
checkpoint selection"), well *before* the fix commits that eventually became
`fa3e411`/`A` and `B`/`0bb4221`. Since `B` only exists on the `A`→`B` line and
`E` never merged that line back in, **`E`'s engine binary structurally cannot
contain the pool-isolation fix** — this isn't circumstantial (build dates,
etc.), it's a hard ancestry fact from `git merge-base`.

Corroborating evidence from the burn-in's own manifest
(`phase_a2_b1_vs_a_exploratory_burnin.md`, "Manifest / provenance"):
`target/release/sekirei` sha256 `792dbed1...` — a different hash from both
`sekirei-spec-fix`'s post-fix binary (`3ac3d456...`) and
`sekirei-next-strength`'s rebased-candidate binary (`334623ae...`) recorded in
`pr4_regate_match/MATCH_CONFIG.md`. Three distinct binaries, three distinct
points in the lineage.

## 3. Does the depth-stall bug plausibly apply to the B1-vs-A burn-in?

Yes, very likely — with a specific, non-obvious implication for how to read
that data, not a blanket invalidation.

- `FINDINGS_INTERIM.md` established the bug triggers whenever
  `SpeculativeSearcher::new(tt, top_n)` runs with `top_n=3`, which is the
  **hardcoded production default** for the real `go` command
  (`sekirei-usi/src/main.rs:492`, per that doc). The B1-vs-A burn-in drove the
  real production `sekirei` binary via USI, not a custom `top_n=0` test
  harness — so on priors it ran with the buggy config, and both B1 and A
  sides of every game were subject to the same depth-freeze behavior
  (`FINDINGS_INTERIM.md`'s Check 1 table shows `top_n=3` search freezing to a
  near-constant, low depth regardless of allotted time).
- Because the freeze applies symmetrically to both engines in every game (same
  binary, same bug, only the weight file differs), this is **not** a
  no-signal / pure-noise situation — both sides played at the same
  (anomalously shallow) effective search depth.
- However, "symmetric bug, so the comparison is still fair" is not quite
  correct either, and this is the caution the user's framing correctly
  anticipates: a search that is artificially capped at a very shallow,
  near-constant depth relies much more heavily on the NNUE static evaluation
  of the position actually reached, and much less on lookahead/tactics, than
  the search the candidate weight would face in production (deep, unbounded
  by this bug). A weight whose edge comes mostly from *positional* evaluation
  quality would be favored under the frozen-shallow regime; a weight whose
  edge comes mostly from being a better *tiebreaker deep in tactical lines*
  would be underweighted. The bug therefore doesn't just add noise or shift
  both sides down equally in some strength-invariant way — it changes *what
  kind* of strength the match was measuring.

**Conclusion (per the user's suggested framing, confirmed rather than
assumed):**

> B1-vs-A burn-in results are valid evidence of a weight-quality difference
> **as measured under the pre-PR-#5, depth-frozen search** — they are not
> automatically invalid — but they require additional confirmation before
> being used as a baseline-promotion justification against the current
> (post-PR-#5, depth-unfrozen) search, because the depth regime itself likely
> changes which kind of weight strength dominates the outcome.

Concretely: before promoting B1 over A (or vice versa) as a production
baseline decision, the comparison should be re-run (or at minimum spot-checked
at low games/fixed-node scale, see `docs/experiments/gate_redesign_low_load.md`
§5A once written) on a `main`-tree engine binary (post-`0bb4221`), not
reused as-is from the `c399a7c` burn-in.

## 4. `depth_fix_match` and `se_on_fix_match` — what they actually validated

These two match sets (in `sekirei-abtest-results/`) are a different kind of
artifact from the B1-vs-A burn-in: same weight file both sides
(`weights_v011_opening_combined.bin`), different **engine code** each side —
i.e., engine-version A/B tests, not weight A/B tests.

| Match | engine1 (base) | engine2 (candidate) | Games | engine2 score | Elo (engine2) | Reusable as |
|---|---|---|---|---|---|---|
| `depth_fix_match` (4 shards, aa–ad) | `sekirei-abtest-base` = pre-PR5 (**A**) | `sekirei-spec-fix` = PR #5 fix (**B**) | 186 (36+50+50+50) | 104/186 = 55.9% | net positive for the fix, strongest in shard `ac` (18/32, 64%) | **Valid, reusable evidence that PR #5's depth fix is a real improvement over A**, at the code-version level. Weight held constant. |
| `se_on_fix_match` (2 shards, aa–ab) | `sekirei-spec-fix` = PR #5 only (**B**) | `sekirei-se-on-fix` = an earlier, pre-rebase manual combination of PR #4 + PR #5 (worktree since removed; **not** identical to **D**, which is PR #4 cleanly rebased onto `0bb4221`) | 100 (50+50) | 49/100 = 49.0%, elo_diff ≈ −6.9, 95% CI crosses zero | **INCONCLUSIVE** (matches the prior session's stated result exactly) | Weak/limited evidence only — small N, wide CI, and built from a branch state that predates the clean rebase (**D**). Should not be treated as a completed test of **D**; superseded in intent (not in validity) by the still-unrun 300-pair `pr4_regate_match`. |

Note: `depth_fix_match/shard_ab` and `se_on_fix_match/shard_ab` happen to have
*identical* aggregate W/D/L/Elo (23-27-0, elo −27.85) — checked directly
(`diff` on the underlying per-game logs and opening files): the games and
positions are entirely different in each; the matching aggregate is
coincidental, not a copy/paste artifact. Flagged here only so a future reader
doesn't independently notice the coincidence and suspect data corruption.

## 5. Reusable vs non-reusable artifact summary

| Artifact | Evidence for | Reusable? |
|---|---|---|
| `depth_fix_match/*` | PR #5 fix (B) > pre-fix (A), same weights, engine-code A/B | **Yes** — cite as-is |
| `se_on_fix_match/*` | Early, inconclusive signal on an SE+depth-fix combination that predates the clean rebase | Reference only, not a substitute for a fresh **D** vs **C** test |
| `phase_a2_b1_vs_a_exploratory_burnin` (E) | B1-vs-A weight difference under pre-PR5, depth-frozen search | **Yes, but scoped**: valid for weight comparison under old search; needs re-confirmation before use as a baseline-promotion justification under current (C-tree) search |
| `pr4_regate_match/run1_contaminated_load_spike/*` (attempt 1) | Nothing — 38/600 games, 74% TimeForfeit | **No** — do not aggregate, do not cite as W/D/L signal (see `pr4_gate_attempt_index.md`) |
| `pr4_regate_match/shard_aa.log` + `shard_aa_out/` (attempt 2, top level, unarchived at time of writing) | Nothing — 3/100 games, 2/3 TimeForfeit | **No** — same reason |
| `pr4_regate_match/MATCH_CONFIG.md`, `aggregate.py`, `positions_300.sfen`, `shard_a{a..f}.sfen` | Reusable *infrastructure* for a future retry of **D** vs **C**, not itself a result | N/A — config/tooling, not data |
