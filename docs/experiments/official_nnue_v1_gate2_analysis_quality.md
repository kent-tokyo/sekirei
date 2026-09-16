# Official NNUE v1, Gate 2 — analysis-quality comparison

Compares `candidate_seed7.best.bin` (Tier 2's confirmed median-seed candidate, PR #61, Gate 1 HEALTHY per
PR #63) against Sekirei's own material-fallback evaluation, same binary, per the preregistration's Gate 2
section (`docs/experiments/official_nnue_v1_preregistration.md`). **Diagnostic only — no strength/Elo claim**,
per `docs/amateur_analysis_benchmark.md`'s explicit "Do not overclaim" rule. That is Gate 3's question, not
this one.

- **Phase**: Gate 2 (analysis-quality comparison).
- **Branch / worktree**: `experiment/official-nnue-v1-analysis-gate`,
  `/Users/k_tanabe/Documents/Documents/oss_rust/sekirei-nnue-v1-gate2`.
- **Base SHA**: `38d30f6` (main tip after PR #63 merged).
- **Changed files**: `scripts/gate2_compare_analysis_runs.py` + `scripts/test_gate2_compare_analysis_runs.py`
  (new tool, committed separately, 15 unit tests), this document. Run artifacts under
  `data/runs/nnue_v1_gate2/` are gitignored, not committed — SHA-256 checksums below make them citable.

## Corpus — decision and rationale

**In-sample, 100 positions, deterministic stride sample from `data/runs/nnue_v1_tier2/teacher_cache.bin`**
(user decision, 2026-08-23, after a proven dry-run harness and real cost measurements were presented — see
"What was tried and ruled out" below). `teacher_cache.bin` holds the 5,159 real positions Tier 2's training
used (all three seeds, train+valid combined). Sampled every 51st line (`stride = 5159 // 100`), 100 positions,
`sample_id = "tier2_teacher_cache:<line index>"`. Corpus SHA-256:
`f4c269bc0c4b6c760fbc0a80eb082d0ac12cb79686770843ee002252952920c6`.

**This is in-sample** — every position was seen during seed 7's training (as train or validation data). Gate 2
therefore measures *"does the trained candidate's search-time judgment differ from material counting on the
positions it was fit to,"* not out-of-sample generalization. Not a defect in this report — the preregistration
never named a specific corpus, and the user explicitly chose in-sample for this round given its zero extraction
cost and this gate's own diagnostic-only (not strength-claim) framing. An out-of-sample corpus (unused CSA files
301+) remains available as a documented option if a stronger claim is wanted later.

### What was tried and ruled out

- `scripts/fixed_depth_corpus.json` (21 positions, existing CI fixture for the fixed-depth A/B search-regression
  gate): rejected. Its own majority category is `king-danger`/`check-evasion` rules edge-cases (nyugyoku,
  jishogi, continuous check), not representative "does eval disagree with material" positions, and 6 of its 21
  entries are shallow openings where Sekirei's opening book (`UseBook`, default on) would short-circuit search
  entirely before any eval difference could show up.
- `--depth 6`: rejected after measurement. One position's NNUE-eval search ran >50s without finishing (killed
  manually) — depth 6 with `--multipv 3` is not tractable at this corpus scale. `--depth 4` (matching Tier 2's
  own `--label-depth 4`) was used instead.

## Commands run

```sh
python3 scripts/usi_analysis_export.py \
  --engine-binary target/release/sekirei \
  --depth 4 --threads 1 --spec-top-n 0 --multipv 3 \
  --setoption UseBook=false \
  [--eval-file data/runs/nnue_v1_tier2/selected/official_nnue_v1_candidate.bin]  # omitted for the material arm \
  --corpus data/runs/nnue_v1_gate2/corpus100.jsonl \
  --output <with|without>_eval.jsonl --manifest <with|without>_eval.manifest.json

python3 scripts/gate2_compare_analysis_runs.py \
  --with-eval data/runs/nnue_v1_gate2/with_eval.jsonl \
  --without-eval data/runs/nnue_v1_gate2/without_eval.jsonl \
  --output data/runs/nnue_v1_gate2/comparison.json
```

`UseBook=false` on both arms: with `--min-ply 20`-sourced positions this matters less than for raw openings, but
set explicitly rather than assumed, since `BookMaxPly` defaults to 30 and several sampled positions are ply
20-30. **Not run**: Gate 3 (SPRT strength gate), any match play. No merge/version bump/tag/release/publish.

## Harness verification (before trusting any result)

- **Fresh process per position, confirmed both by source (`usi_analysis_export.py`'s own `run_one_analysis`
  docstring) and by observed behavior** (`ps` showed a new `target/release/sekirei` PID per position throughout
  the run) — rules out the known `sekirei_core::nnue` `OnceLock`-staleness bug
  (`scripts/search_ablation.rs`'s P0, `tasks/todo.md`) ever affecting this comparison: each process loads its
  own weights (or none) fresh, there is no possibility of the second arm silently reusing the first's.
- **Determinism re-verified on a 5-position dry run**: ran the with-eval arm twice: `nodes`, `score_cp`,
  `bestmove`, and `pv` were byte-identical between the two runs at every position; only `nps`/`time_ms`
  (wall-clock measurements) differed — the same "determinism modulo wall-clock" property already established
  for training (PR #61's seed-7 re-run).

## Coverage — a real, load-bearing finding on its own

| Arm | ok | timeout | incomplete | engine_error | non-ok rate |
|---|---|---|---|---|---|
| with-eval (NNUE) | 65 | 35 | 0 | 0 | **35%** |
| without-eval (material) | 100 | 0 | 0 | 0 | 0% |

At `--depth 4` with a 60s per-position timeout, **35 of 100 positions could not complete search with the NNUE
candidate loaded**, while the identical positions all completed under material evaluation. No crashes or engine
errors on either arm — every non-completion is a clean timeout, not a fault.

Node-count and wall-time comparison (completed/`ok` positions only — note this is a survivorship-biased
comparison, since the NNUE arm's hardest 35% are excluded entirely, not represented at all in these numbers):

| Arm | mean nodes (ok only) | mean wall_time_ms (ok only) |
|---|---|---|
| with-eval (NNUE) | 3,047,334 | 14,267 |
| without-eval (material) | 231,848 | 2,327 |

The NNUE arm's completed positions average ~13x the node count and ~6x the wall-clock of material's. This
report does not assert a specific mechanism (e.g. whether this is per-node NNUE evaluation cost, or a change in
move-ordering/pruning effectiveness that lets more nodes through alpha-beta cutoffs under the trained eval) —
that would need a dedicated node-by-node investigation this report didn't do. The observation itself (real, not
guessed) is directly relevant to Gate 4's eventual model-card and to `docs/mobile_integration.md`-adjacent
questions, flagged as discovered work below, not resolved here.

## Metrics (per `docs/amateur_analysis_benchmark.md`'s existing definitions — no new formulas)

Computed by `scripts/gate2_compare_analysis_runs.py` (15 unit tests) over the 65 positions where **both** arms
reached `status: "ok"`:

| Metric | Value |
|---|---|
| Top-1 agreement | **10.8%** (7/65) |
| Top-3 candidate overlap (mean Jaccard) | 0.265 |
| Top-3 exact-set match rate | 13.8% (9/65) |
| Mate agreement | 100% (n=1 — only one position had a `score_mate` line on both sides; far too thin to generalize) |

Full JSON: `data/runs/nnue_v1_gate2/comparison.json`
(SHA-256 `82e8b3e7ae10fb1671195641e10815e03d9f3965d96b898c81484bc30f6e473b`).

### score_cp shift — real number, but read the scale caveat before using it

`score_cp_shift`: n=64 (1 excluded for a mate-vs-cp mismatch), mean=**−160.1**, median=**−62.0**, stdev=1654.3.

**This number is confounded by a scale mismatch between the two arms' `score_cp`, not just a magnitude-of-
disagreement signal:**

| Arm | score_cp range (ok positions) | mean | median | stdev |
|---|---|---|---|---|
| with-eval (NNUE) | **−73 to +73** | 11.6 | 19.0 | 42.3 |
| without-eval (material) | **−4910 to +7680** | 196.5 | 0.0 | 1677.3 |

The NNUE candidate's own output is compressed into a narrow ±~75 band — consistent with, not contradicting,
Gate 1's own `valid_output_range=93.14` finding for this exact checkpoint (PR #63). Material's raw piece-count
`score_cp` spans thousands. `docs/amateur_analysis_benchmark.md`'s own rule — "Raw CP is not comparable across
engines... only compare `score_cp` deltas *within* one engine's own outputs" — was written with two different
*engines* in mind; here it is one engine but two structurally different value heads (a trained, WDL-influenced
network vs. raw material count) sharing a binary. **Treat `score_cp_shift`'s mean/median as dominated by this
scale gap, not as a calibrated "NNUE thinks positions are ~160cp worse" statement.** The large stdev (1654,
comparable in magnitude to material's own stdev of 1677) reflects material's wide range driving most of the
variance, not NNUE instability.

### Concrete examples (top-1 disagreements, illustrative not exhaustive)

```
tier2_teacher_cache:0    (ply 49)   NNUE: L*3f (cp -64)    material: 4i3h (cp -200)
tier2_teacher_cache:102  (ply 25)   NNUE: 8e8d (cp 22)     material: 4g4f (cp 0)
tier2_teacher_cache:1020 (ply 69)   NNUE: 6g6f (cp 18)     material: 4h3h (cp 750)
```

## Interpretation — diagnostic, not a verdict

A 10.8% top-1 agreement rate is low, but **low agreement with material counting is the expected, intended
outcome of training an evaluation network** — an NNUE that mostly picked the same moves as raw piece-counting
would not be doing anything a much simpler heuristic couldn't. This number says the candidate's search-time
judgment is *substantially different* from material counting; it says nothing about whether that difference is
an *improvement*. That question belongs to Gate 3 (SPRT vs. material baseline), not here.

The 35% timeout rate at `--depth 4` is a genuine, separate finding about search cost with this NNUE loaded —
relevant to any real-time/mobile deployment question, independent of whether the candidate eventually passes
Gate 3.

## Gate result

**Not a strength/quality verdict — diagnostic data gathered as specified.** Execution itself:
**PASS (execution)** — harness verified (fresh-process isolation, determinism), corpus/depth choices measured
and justified (not guessed), metrics computed via the project's own existing metric definitions, no crashes on
either arm. Coverage (35% NNUE-arm timeout) and the score_cp scale-mismatch caveat are reported as load-bearing
context for reading the other numbers, not swept into a single pass/fail judgment — Gate 2 itself has no
HEALTHY/WARNING-style verdict vocabulary in the preregistration; it produces comparison data for Gate 3 and
Gate 4 to draw on.

## Known limitations

- In-sample corpus (see "Corpus" above) — measures agreement with the training distribution, not
  generalization.
- 35% of the NNUE arm timed out at 60s/`--depth 4` — the reported node/timing stats for that arm are
  survivorship-biased toward its easier 65%.
- `score_cp_shift`'s mean/median are scale-confounded (see above) — use `top1_agreement`/`top3_overlap` as the
  primary signal, not the cp-delta numbers.
- n=65 (or n=64/n=1 for some metrics) is a modest sample for a diagnostic pass; not intended as a
  statistically-powered claim.
- The ~13x node-count / ~6x wall-time gap's mechanism is not investigated here (see "Coverage" above).

## Discovered work (not implemented here)

- Investigate why NNUE-loaded search explores far more nodes than material-eval search at the same nominal
  depth (move-ordering effectiveness? extension triggers? per-node cost alone doesn't obviously explain a 13x
  node-count gap) — relevant to `docs/mobile_integration.md`-adjacent real-time-budget questions.
- If a stronger, non-in-sample claim is wanted later: build an out-of-sample corpus from CSA files 301+
  (~18-20 min estimated cache-harvest per the same recipe already used for Phase 1, per `advisor()`'s guidance
  this session) and re-run this same tooling unchanged.
- A longer `--timeout` (or shallower `--depth`) would recover some of the 35 timed-out positions' data, at
  proportionally higher wall-clock cost — not attempted this round given the diagnostic (not statistically-
  powered) framing of this gate.

## Next operation

**Not started this round.** Gate 3 (SPRT strength gate vs. material baseline, branch
`experiment/official-nnue-v1-strength-gate`) requires separate explicit approval per this roadmap's standing
rule — this report surfaces Gate 2's findings as input to that decision, not as a request to proceed.

## Merge status

Not merged. PR to be opened from this branch, reporting only.

## Items needing approval / explicit confirmation

Whether to proceed to Gate 3 next, and whether the discovered-work items above (node-count investigation,
out-of-sample re-run, longer-timeout re-run) are worth doing before or instead of Gate 3.
