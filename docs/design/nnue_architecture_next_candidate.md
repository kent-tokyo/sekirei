# NNUE architecture: candidate comparison for the next experiment

Status: design-only. No training run, no code change, no benchmark executed
to produce this doc — every number below is computed from reading
`crates/sekirei-core/src/nnue.rs`'s actual binary-format arithmetic (verified
against the real 1,305,356-byte size of `data/weights_v011_opening_combined.bin`
and siblings) and `crates/sekirei-train/`, plus a survey of this repo's own
prior NNUE-training experiment docs. Written to close the gap `ROADMAP.md`
§6 flags: *"no NNUE architecture upgrade path has been researched or decided
yet... This needs its own research pass before implementation."*

## Current architecture (baseline, "A")

From `crates/sekirei-core/src/nnue.rs`: plain piece-square + hand features,
**no king-relative conditioning at all** — `feature_index(sq, kind,
piece_color, perspective)` (`nnue.rs:334`) depends only on the piece and
perspective, never on either king's square. `INPUT=2420` (2268 board + 152
hand), `L1=256`/perspective, single `L2=32`, single scalar output. Weight
file: SEKIRW01 binary, 1.31 MB, ~636K parameters. This is the architecture
every `data/weights_*.bin` file in the repo was trained under.

## The three candidates

**B. Add king-relative board features.** Condition each board feature on
the *owning perspective's own* king square (standard "Half-KP"-style
design, not full KKP) — hand features stay unconditioned (they already
don't have a natural square to relate to a king). Sub-variants by bucket
granularity, since full per-square conditioning (81 squares on a 9×9 board)
is the expensive end of a real range, not the only option:

| Variant | King granularity | `INPUT` | File size | Params |
|---|---|---|---|---|
| B-small | 9 zones (e.g. 3×3 board regions) | 20,564 | 10.6 MB | 5.28M |
| B-mid | 27 buckets (file × rank-band) | 61,388 | 31.5 MB | 15.7M |
| B-full | 81 (every king square, HalfKP-standard) | 183,860 | 94.2 MB | 47.1M |

**C. Widen the feature transformer, `L1` 256→512.** `INPUT` and `L2`
unchanged. File size: 2.61 MB, ~1.27M params (≈2× baseline — FT dominates
total params at this scale, 97%+, so doubling `L1` ≈ doubling everything).

**D. Widen the second hidden layer, `L2` 32→64.** `INPUT` and `L1`
unchanged. File size: 1.37 MB, ~653K params (+2.6% — `L2` is a small
fraction of total params regardless of `L1`, so this is a cheap change by
size alone).

## Comparison table

| Axis | A (current) | B (king-relative) | C (FT 256→512) | D (L2 32→64) |
|---|---|---|---|---|
| Params | 636K | 5.3M–47.1M (variant-dependent) | 1.27M | 653K |
| File size | 1.31 MB | 10.6–94.2 MB | 2.61 MB | 1.37 MB |
| Inference cost (L2 forward pass) | baseline (`L1×L2` = 8,192 MACs) | **unchanged** (same `L1`/`L2`) | 2× (16,384 MACs) | 2× (16,384 MACs) |
| Incremental-update cost | O(1) per piece move (`add_col`/`sub_col`, `nnue.rs:468`/`478`) | O(1) for non-king moves; **a king move forces a full `NnueAcc::refresh`** (`nnue.rs:358`, scans every piece on the board) — this is the standard NNUE tradeoff, not a bug, but a genuinely new cost class this codebase doesn't have today | O(1), same shape, just longer per-move SIMD vectors (linear in `L1`, not a new complexity class) | O(1), unaffected — `L2` isn't touched by incremental updates at all |
| Representational power | No king-safety/mating-net signal representable except through what L2/output can compensate for indirectly | Directly represents "this piece/square matters differently depending on king position" — the specific gap vs. HalfKP-class engines (YaneuraOu/Suisho, `ROADMAP.md` §0's named competitive targets) | More per-feature nonlinear capacity, same feature semantics — helps if the current bottleneck is FT underfitting, not feature expressiveness | More L2 capacity — **but see the saturation risk below, which argues this doesn't currently translate to more usable capacity** |
| Existing weight compatibility | — | **Breaks all existing `data/weights_*.bin`** (`read_weights`'s size check, `nnue.rs:221-250`, rejects any file whose length doesn't match compile-time `INPUT/L1/L2` — same mechanism for B/C/D, no discriminating factor here) | Breaks all existing weights (same mechanism) | Breaks all existing weights (same mechanism) |
| Teacher-label / training-data cost | — | **None** — confirmed `sekirei-train` never calls `nnue::load_weights` (label-depth search always runs on the material-count fallback, independent of NNUE weights); `teacher_cache.rs`'s cache is keyed on SFEN + `label_depth` only, architecture-agnostic. A full **weight retrain** is still required (new `INPUT` shape), just not new teacher labels. | Same: no new teacher data, full weight retrain required | Same: no new teacher data, full weight retrain required |
| Engineering cost beyond training | Feature-index change is one function (`nnue.rs:334`), trainer picks it up automatically (`sekirei-train/src/trainer.rs:2985` calls the same `feature_index`) — but `NnueAcc` needs a new king-move-triggers-refresh code path, which doesn't exist today | Pure hyperparameter change, no new code path | Pure hyperparameter change, no new code path | — |

## The finding that should drive this decision: L2 is already saturating

This repo has an extensive, causally-verified prior investigation
(`docs/experiments/l2_saturation_mechanism_p0.md`,
`l2_saturation_freeze_diagnostic.md`, task #91) into an **unresolved**
training pathology: by ~1/4 into epoch 1, L2's gradient path is ~100%
closed (only ~0.1% of activations remain in the non-clamped/"linear"
region; the rest is split between dead and saturated), consistent across 3
seeds. Proven causal (not just correlational) via selective-freeze
experiments: freezing FT-output *or* L2-weight updates alone fully blocks
new saturation for as long as the freeze holds. Root cause identified:
`z_L2 = FT_output × W_L2 + b_L2` is a product of two factors that move in a
correlated, reinforcing direction early in training — an update-direction/
structure problem, not a magnitude or init-distance one. **Three separate
mitigations were tried and all failed**: LR warmup, `--l2-bias-init`, and
gradient clipping (`output_warmup.md`, `l2_bias_init.md`,
`global_gradient_clipping.md`).

This describes saturation as a *fraction of L2's width*, not a fixed
neuron count — no width ablation exists in the corpus to confirm this
directly, but the mechanism predicts that **widening L2 (candidate D)
produces more saturated/dead neurons in absolute terms, not more usable
capacity**, until the underlying saturation mechanism itself is fixed.
**Recommendation: do not run D as the next experiment.** It's the
cheapest candidate by param count, which makes it tempting, but the
cheapest wrong experiment isn't the right first move.

## Recommendation: B (king-relative), starting from B-small, as the first experiment

Reasoning, weighing the table above against the project's own stated goal
(`ROADMAP.md` §0: surpass YaneuraOu/Suisho, not just improve over Sekirei's
own baseline):

1. **D is actively discouraged** by this repo's own prior research (above).
2. **C (FT 256→512)** is the safe, low-risk option — no new code path, no
   feature-semantics change — but it's a "more of the same shape" bet. It
   doesn't address the one concrete, named representational gap this repo
   has relative to its own stated competitive targets: no king-conditioning
   at all, when king-relative features are close to universal in
   HalfKP-class engines specifically because king safety/mating-net
   evaluation is exactly what a flat piece-square net structurally
   struggles to represent.
3. **B-full (81-square HalfKP-standard)** is the "do it properly" version
   top engines use, but at 94.2 MB / 47.1M params it's a ~72× jump from
   today's 1.31 MB file — a large training-cost and engineering-risk step
   for a first experiment, and this repo has zero prior data at any
   king-relative scale to de-risk that jump.
4. **B-small (9 zone buckets)** is the proposed first move: it introduces
   real king-position sensitivity — enough to test whether the *category*
   of feature (king-conditioned vs. not) helps at all before committing to
   fine granularity — at a 10.6 MB / 5.3M-param scale that's a large but not
   extreme step up, and reuses 100% of the existing teacher-label
   infrastructure. If B-small shows a clear signal, B-mid/B-full become the
   natural follow-ups; if it shows none, that's a cheap way to learn the
   *category* isn't the bottleneck before spending a 72×-larger training
   run to find out.

## Open questions (not resolved by this design pass)

- Exact zone-bucketing scheme for B-small (simple 3×3 grid over the 9×9
  board? something shaped by actual king-safety geometry, e.g. castle
  formations?) — needs its own small design pass, not decided here.
- Whether `NnueAcc`'s king-move-triggers-refresh path has an acceptable
  perf cost in this engine's actual search hot path — not benchmarked.
- Whether L2 saturation should be fixed *before* B is trained regardless
  (a saturated L2 may cap how much of B's improved input signal is usable)
  — this design pass didn't investigate mitigation options beyond the
  three already tried and ruled out; flagged as a real open question, not
  addressed here since it's a separate, deeper investigation of its own.
- This entire comparison assumes the current single-L2/single-output
  topology; it does not evaluate deeper/wider net *shapes* (e.g. an extra
  hidden layer) as a fourth category, since the user's own candidate list
  scoped this pass to B/C/D as given.
