# Gate A/B: WDL-blended teacher (λ=0.7), Gate B not-promoted, and the A/B/C follow-up

## Gate A: selecting λ=0.7 over λ=1.0 (eval-only)

`sekirei-train --wdl-lambda <λ>` blends a search-based eval teacher with
the game's own win/draw/loss result: `teacher = λ·eval_teacher +
(1-λ)·wdl_target`, `wdl_target` mapping loss/draw/win to ∓600/0/±600 cp
from the side-to-move's perspective. Gate A compared `λ=0.7` against
`λ=1.0` (pure eval, the pre-existing default) on the same 500-file/
337-game CSA subset (`data/gateA_csa_subset`), same epochs, same seed
— isolating the WDL blend as the only variable.

Pre-registered with a loosened SPRT bound (`elo0=0, elo1=10`) on the
reasoning that WDL's true effect might be modest. The observed effect
was much larger initially (+125 elo after sprint 1, regressing toward
+56 by sprint 4, as expected) — pooled result after 8 sprints (396 games):
`elo_diff=+7.9±34`, SPRT still technically `INCONCLUSIVE` (LLR=0.095,
inside the ±2.944 bounds) once the sprint budget was exhausted. A
heterogeneity check (χ²=20.11, dof=7, p≈0.005 across sprints) was run
before trusting this — one sprint (14-38) diverged sharply from the
otherwise-consistent trend — but didn't change the conclusion enough to
withhold the decision. Per the pre-registered rule (adopt the point
estimate's sign once the budget is exhausted), **λ=0.7 was adopted**;
`weights_gateA_lambda0.7.epoch3.bin` proceeded to Gate B.

## Gate B: not promoted

Gate B compared `weights_gateA_lambda0.7.epoch3` against `v010`
(baseline), pre-registered as a proper early-stopping SPRT (`elo0=0,
elo1=20, α=β=0.05`, `MAX_GAMES=3200`, `Trinomial`+`paired_by_id`).

Stopped early by engineering decision at sprint 32/65 (1728 games), short
of the pre-registered cap, after a background-process kill interrupted
the run a second time. `elo_diff` ranged -1.7 to -8.4 across sprints
15-32, never once positive after sprint 16; LLR oscillated between
roughly -0.4 and -2.9 without a clean drift toward either bound, closest
approach to the FAIL bound (-2.944) being -2.876. Distance to the PASS
bound never dropped below ~5.2 after sprint 15 — practically unreachable
given the observed effect size. **Verdict: not promoted** (engineering
decision, not a formal SPRT closure — the remaining ~1400 games to reach
a clean INCONCLUSIVE weren't judged worth the wall-clock given the
direction was already unambiguous).

## Candidate-identity correction

Mid-investigation, the Gate B candidate was informally referred to as
"v012_loss_mined." **That's wrong.** `weights_v012_loss_mined.bin` (built
via the separate loss-position-mining pipeline,
`scripts/train_with_loss_mining.sh`) and
`data/weights_gateA_lambda0.7.epoch3.bin` (the actual Gate B candidate)
are two unrelated files from two unrelated pipelines — confirmed by file
mtimes and by `data/runs/gateA_lambda0.7/train.log` showing a plain CSA
run with no loss-mining or quietset involvement. A 5-point investigation
checklist floated at the time the stop decision was made (loss-mined
held-out test set, quietset over-filtering, stability-weighting) targets
`v012_loss_mined` specifically and mostly doesn't apply to the real
candidate — recorded here so it isn't mistakenly reused against the wrong
checkpoint later.

## Root-cause hypothesis: undertrained, not necessarily wrong-approach

Two facts, read together, point somewhere more specific than "the
training data was bad":

1. `v010` (Gate B's baseline) has collapsed capacity — trained before the
   zero-init symmetry-collapse fix (see `docs/training_lessons.md`),
   effectively a linear evaluator despite its declared architecture.
2. `weights_gateA_lambda0.7.epoch3.bin` was trained *after* that fix, with
   real capacity.

Gate B was therefore a structurally favorable matchup for the candidate
— and it still didn't clear +20 elo. That's a stronger anomaly than "the
data selection went wrong somewhere." The training recipe itself is the
more specific suspect: `weights_gateA_lambda0.7.epoch3.bin` is exactly
what the filename says — epoch 3 of a **planned 20-epoch schedule**. The
training log stops at the epoch-4 header; epochs 4-20 were never run
against this recipe. By epoch 3, the hardcoded LR schedule
(`0.001 * 0.5^(epoch-1)`, no floor, no alternative) had already decayed
to 0.00025, and `avg_loss` moved only ~0.7% across those 3 epochs
(155691.58 → 154587.13).

**Open question this experiment exists to answer**: is `gateA_lambda0.7`
underperforming because it's undertrained (3/20 epochs, an already-decayed
LR, near-flat loss) rather than because WDL-blending or the CSA data
source is the wrong idea?

## Why A/B/C, and what changed to make it trustworthy

Before re-training anything, the trainer itself needed to be able to
answer that question rather than produce another unexplainable result:
a configurable LR schedule (the hardcoded step-half formula was itself a
suspect), a validation split on the CSA path (previously 100% train, no
held-out set, every epoch), per-epoch diagnostics (the only prior check
of this kind — finding the capacity collapse — was a one-off manual
weight-file read), and a teacher-search cache (without it, three 20-epoch
runs would have cost ~120h/5 days total, working against the intent of
this phase). See `docs/training_lessons.md` for the durable design notes
on each, and `tasks/lessons.md`'s 2026-07-13 entries for the
session-local implementation detail.

**Important caveat**: introducing the validation split means the CSA
ablation runs (A/B/C below) train on *fewer* positions than the original
`weights_gateA_lambda0.7` run did (some fraction now held out for
validation). **Run A is not a bit-for-bit reproduction of the original
Gate A recipe** — it's a control using the old step-half LR schedule,
under the new leakage-safe validation protocol. A, B, and C are fair to
compare against each other (identical data/split/seed/architecture,
varying only LR schedule and λ), but none of them is identical-condition
to the checkpoint that actually went through Gate B.

A second known simplification, accepted for this comparison only: a
single `--seed` value drives both weight initialization and the
validation split. Held fixed across A/B/C, so it doesn't compromise the
comparison — but it means "same seed, different split" and "different
seed, same everything else" can't be tested independently yet. Splitting
this into separate `--init-seed`/`--split-seed` flags is deferred until
before a 50k/200k-scale run (not needed at this dataset size).

## The three runs

Fixed across all three: CSA dataset (`data/gateA_csa_subset`), group-aware
validation split (`--validation-ratio 0.15`, `--seed 42`), 20 epochs,
architecture (`INPUT=2420 L1=256 L2=32`), `--label-depth 4`,
`--min-rate 1500` (both assumed to match the original Gate A recipe —
not recorded verbatim anywhere from that run, since it predates
per-run `.meta.json` on the CSA path; well-evidenced from the original
`train.log` header and every pre-Gate-A verification run in
`tasks/lessons.md`, but not a certainty).

| Run | `--wdl-lambda` | `--lr-schedule` | Purpose |
|---|---|---|---|
| A | 0.7 | step-half | Control: old schedule, new leakage-safe protocol |
| B | 0.7 | cosine | Isolates LR-schedule effect vs. A |
| C | 0.0 | cosine | Isolates WDL-lambda effect vs. B |

A and B deliberately omit `--min-lr`/`--warmup-epochs` where they'd be
inconsistent with reproducing "the old schedule" (A only) — B and C, both
cosine, use `--min-lr 0.00001 --warmup-epochs 1`.

Sequencing: run A alone first, review its diagnostics (best-valid-loss
epoch, train/valid gap, update-norm and LR trend, active/saturation
ratios, output std, quantized-zero rate, differences across epochs
{1,3,5,10,20}) before running B or C — not all three launched blind. If
A's best checkpoint lands early (epoch 2-3) with vanishing later update
norms, that's itself the strongest argument for B (cosine) over assuming
WDL is the problem. Only after comparing all three runs' best checkpoints
does this proceed to `opening_sanity.sh` and a small paired quick gate
(100-200 games) on the top 1-2 candidates — not directly to a long SPRT
gate.

## Results

All three runs completed 2026-07-13/14. `split_hash`/`dataset_hash`/`seed`/
`architecture` verified identical across all three before comparing.

| | A (λ0.7/step-half) | B (λ0.7/cosine) | C (λ0.0/cosine) |
|---|---|---|---|
| best epoch | 8 | 3 | 1 |
| best valid loss | 140077.83 | **138595.82** | 350921.23 |
| epoch 20 valid loss | 140089.03 | 162591.60 | 421551.85 |
| train/valid gap @ best | 1.127 | 1.111 | n/a (never improves) |
| update_norm @ best | 0.53 | 55.6 | n/a |
| l2_active/l2_sat @ epoch20 | 0.719/0.719 | 0.812/0.812 | 0.969/0.969 |
| ft_active @ epoch20 | 1.000 | 1.000 | **0.934** (FT neuron death) |

**A**: best checkpoint lands at epoch 8, but `update_norm` has already
collapsed toward zero by then (halves every epoch with the unfloored
LR) — confirms the "undertrained epoch-3 checkpoint" root-cause
hypothesis: step-half burns out roughly half the 20-epoch budget.

**B**: best checkpoint (epoch 3) is genuinely healthy on its own terms —
tighter train/valid gap than A's own best, still-substantial
`update_norm`, not every active L2 neuron saturating yet. Past epoch 3,
valid loss rises every single epoch while train loss keeps falling and
output scale runs away (`out_std` 82→321). Reading: cosine fixes the
LR-starvation problem A has, but exposes that the model overfits hard
once genuinely free to keep learning — an argument for early stopping
on valid loss, not evidence cosine itself is the wrong schedule.

**C**: dramatically worse in every dimension, not just similarly
overfit — degrades from epoch 1 onward, never improves, and shows FT
neuron death that neither A nor B exhibit at all.

**Correction (2026-07-14)**: the paragraph below originally read this as
"WDL blending has real generalization value." That claim doesn't hold up.
Per `--wdl-lambda`'s actual formula (`teacher = λ·eval + (1-λ)·wdl`,
`trainer.rs`), **λ=0.0 is pure game-outcome (±600/0 cp step function),
not pure eval** — the opposite polarity from what was assumed here. More
importantly, A/B's `valid_loss` is MSE against a 70%-eval-blended teacher
and C's is MSE against a pure-outcome teacher — two different-scale
objectives. C's raw `valid_loss` being ~2.5× A/B's does not, by itself,
quantify a WDL-generalization effect; it may just reflect that a coarse,
nearly-binary per-game target is intrinsically harder to fit than a
smooth search-eval target, independent of any "blending helps
generalization" claim. What C's own trajectory *does* support on its own
terms: valid loss degrades monotonically from epoch 1 (A/B both improve
before overfitting), FT neuron death appears that A/B never show, and
output scale runs away — training this architecture against a
pure-game-outcome target is unstable, a real finding independent of the
cross-run comparison. Whether blending eval in *specifically* aids
generalization (vs. just being a lower-variance target) is not
established by these numbers and needs a common-scale metric — see
`valid_cp_mse`/`valid_wdl_loss` in the follow-up investigation below. C
remains excluded from the playing-strength comparison (its checkpoints
are training-unstable on any reading), but the "C is clearly worse"
branch is no longer read as WDL-specific evidence.

**Common-metric back-apply, resolved (2026-07-14)**: added `valid_cp_mse`
(MSE vs. the raw search eval, computed identically regardless of a run's
own `wdl_lambda`) and `valid_wdl_loss` (MSE vs. the raw game-outcome
target) to `eval_game`, then re-scored A(epoch8)/B(epoch3)/C(best=epoch1)
against the *same* held-out validation split via a new `--eval-only
<checkpoint>` flag (loads a checkpoint's weights for forward-scoring only,
without touching the teacher search — see `tasks/lessons.md`'s 2026-07-14
entry for a real bug this caught and fixed along the way: an earlier
version of `--eval-only` silently redirected the teacher-generating search
itself onto the checkpoint being scored, via a shared global that
`load_weights` sets and `Searcher`'s leaf evaluation reads).

**Correctness check on `--eval-only` itself**: each run's own-objective
`valid_loss` (row below) reproduces that same checkpoint's original
best-epoch training-time `valid_loss` almost exactly — A: 139980.15 vs.
140077.83 (0.07% off), B: 138631.86 vs. 138595.82 (0.03%), C: 350950.35
vs. 350921.23 (0.008%). All three are real, previously-trained checkpoints
(not the small smoke-test checkpoint used during development), so this
confirms `read_weights`/`from_nnue_weights`/`--eval-only` faithfully
reproduce a *trained* checkpoint's scoring, not just a freshly-initialised
one — the residual is ordinary FT i16 quantisation rounding.

| | A (epoch8) | B (epoch3) | C (epoch1, best) |
|---|---|---|---|
| `valid_cp_mse` (common) | 160513.20 | 159164.41 | 173445.50 |
| `valid_wdl_loss` (common) | 348290.48 | 347129.64 | 356420.68 |
| `valid_output_mean` | 32.257 | 51.574 | 48.559 |
| `valid_output_std` | 64.480 | 101.660 | **6.744** |
| own-objective `valid_loss` (not comparable across rows) | 139980.15 | 138631.86 | 350950.35 |

On the common yardstick, **C is only ~9% worse than A/B at predicting the
search eval** (`cp_mse` ratio 1.09×), not the ~2.5× the raw `valid_loss`
comparison implied — confirming that gap was mostly a different-objective
scaling artifact, not a real 2.5× quality gap. `valid_wdl_loss` is nearly
identical across all three (~3% spread) — none of them predicts game
outcome noticeably better than the others, including C, which nominally
trains on nothing else.

The metric that *does* show a real, large difference is `valid_output_std`:
C's is **9.6× smaller than A's and 15× smaller than B's**. C's best
checkpoint isn't discriminating positions much at all — its output sits
close to a near-constant ~48.6cp regardless of position, rather than
tracking real positional differences. That's a distinct failure mode from
the *later* output-scale blowup this same run develops by epoch 20 (see
the L2 saturation probe findings above, `out_std=453`): epoch 1 is
under-differentiated/collapsed, not yet blown up. A near-constant
predictor can still post a middling MSE if the target itself has modest
variance in this small validation slice — which is exactly why MSE alone
under-states how unusable this checkpoint would be in practice, and why a
playing-strength gate (not attempted for C, given the instability already
established above) remains the real test, not just a validation-metric
comparison.

**A(epoch8) vs B(epoch3) paired quick gate (198 games) — INCONCLUSIVE**:
`elo_diff=-17.56`, 95% CI=[-66.02, +30.89], LOS=23.84% — the CI is wider
than the effect itself, so this cannot distinguish "A is stronger" from
"no real difference." **Statistical conclusion**: undetermined.
**Engineering decision**: retain A as control, do not promote B, do not
extend to a 400-game gate — B's 1.06% valid-loss edge did not translate
into any measurable playing-strength advantage at this sample size, and
the point estimate leans the wrong way, so there's no basis to invest
further compute chasing it. This is not "A won." Full manifest:
`docs/experiments/ablation_lr_schedule_a8_vs_b3.md`.

**A/B/C conclusion**: cosine run for a fixed 20 epochs is not adopted —
it overfits hard once the LR-starvation ceiling A suffers from is
lifted (see B above). step-half stays the control despite wasting
roughly half its epoch budget once the LR decays — B did not demonstrate
a real playing-strength edge over it (see quick-gate result above).
**Valid loss alone is not sufficient for checkpoint selection**: B's
best-valid-loss checkpoint did not produce a measurable playing-strength
improvement over A's, and A/B/C's `valid_loss` values aren't even on a
comparable scale across differing `wdl_lambda` (see the common-metric
back-apply above). On the resolved common yardstick, C is not "2.5×
worse" at eval-prediction — it's ~9% worse, far below what the raw
comparison implied. Whether that remaining ~9% is a meaningful gap or
within this setup's run-to-run variation is not established from a
single deterministic run each — that question is exactly what the 3-seed
sensitivity experiment (P1, not yet run) would answer, not something to
presume either way here. Its real, clearly-established problem is a
different one: `valid_output_std` collapsed to ~1/10-1/15th of A/B's,
meaning C's best checkpoint barely discriminates between positions at
all. Whether that specific collapse is caused by training on a
game-outcome-heavy target (low λ) or is a training-instability artifact
unrelated to λ is also not established — resolving that needs the same
3-seed experiment or a dedicated λ-sweep, neither of which this
investigation covers.

**Follow-up investigation (2026-07-14)**: `l2_active_ratio ==
l2_saturation_ratio` was root-caused — a set-membership artifact of a
subset of L2 neurons going permanently dead, not saturation intensity
(A: 9/32 dead, fixed from epoch 1, never recover under step-half; B: same
9 at epoch 1 but 3 recover by epoch 20 under cosine's sustained LR; C:
almost no dead neurons but a different pathology, output-scale runaway).
A follow-up epoch-0 probe found L2 neuron death is dominated by epoch 1's
first-update magnitude, not initialization: only 3 of A's 9 dead neurons
were already dead post-init, 6 died specifically during epoch 1 (a
~100-300× pre-activation scale jump across the whole L2 layer in one
epoch, no warmup). Full detail in `tasks/lessons.md`'s 2026-07-14 entries.
This points the next single-variable experiment at LR
warmup/lower-initial-LR/gradient-clipping, not weight decay or init
adjustment — not yet run. The common cross-λ validation metric is now
done (see above).

## 3-seed sensitivity experiment (corrected, 2026-07-15)

Ran the short 3-seed sensitivity experiment referenced above (B and C,
seeds 42/7/123, `--epochs 3` at the real 20-epoch cosine schedule). A real
bug surfaced mid-analysis: `compute_lr`'s cosine schedule shaped its curve
using `total_epochs`, and every call site passed `args.epochs` directly —
so the first `--epochs 3` attempt compressed the *entire* 20-epoch decay
into 3 epochs instead of reproducing its first 3 epochs (epoch 3's LR
landed at the `min_lr` floor instead of the correct, barely-decayed
value). Epochs 1-2 were unaffected (LR is `0.001` either way under
1-epoch warmup). Fixed with a new `--lr-schedule-epochs <n>` flag
(decouples the schedule's horizon from the run length; see `CHANGELOG.md`)
and all 6 runs re-executed from scratch — SHA-256 confirms epoch 0-2
checkpoints are byte-identical to the invalidated first attempt, so only
epoch 3 changed. Full table, per-seed detail, and the invalidated-run
record: `tasks/lessons.md`'s 2026-07-14/15 entries; artifact:
<https://claude.ai/code/artifact/acf56cda-86d2-4066-86b0-da63b0a5bb76>.

**Result — the λ-vs-collapse question above gets a qualified answer, not
a settled one.** Same-seed `valid_cp_mse` at epoch 2 (unaffected by the
bug) was mixed — B ahead in seeds 42/123, C ahead in seed 7. At the
corrected epoch 3, **B (λ=0.7) has lower cp_mse than C (λ=0.0) in all
3 seeds**, including seed 7, which crosses over between epoch 2 and 3
rather than trending toward B monotonically. Looking at what drives that:
every B seed *improves* its own cp_mse from epoch 2 to 3, while C
*worsens* in 2 of 3 seeds and only marginally improves in the third. The
two C seeds that worsen (42, 7) show early **output-scale runaway** in
`train_std` — the same pathology the full A/B/C run above found in C by
epoch 20, already visible here by epoch 3 of a short run. The third C
seed (123) instead looks **stuck**, not runaway (`train_std` barely
moves), consistent with never fully breaking its own epoch-1 output
collapse.

**This does not resolve to a clean single story.** The epoch-1 dead-L2
neuron count reproduces cleanly (C=0 in every seed, B=2-5 in every
seed) but stops tracking anything by epoch 3 — C's dead-neuron count
exceeds B's own-seed count in 2 of 3 seeds by then, inverting the epoch-1
pattern. And the `output_std≈0` collapse this doc flagged as C's
signature pathology is **not λ=0-specific**: the corrected re-run's new
`valid_output_range` diagnostic (exact min/max, immune to the variance
formula's catastrophic-cancellation rounding) confirms a genuine,
exact-zero collapse at epoch 1 in `B_seed7`, `B_seed123`, *and*
`C_seed123` — two B runs, one C run.

**Revised conclusion**: λ=0.7 does show a real, seed-consistent
generalization edge on the common cp_mse yardstick by epoch 3 of this
short run — stronger support than this doc previously had for keeping
the WDL blend. But the mechanism is not "fewer dead L2 neurons," and
init-sensitivity is layered on top of it (two distinct C failure modes —
runaway vs. stuck — across only 3 seeds). Still a 3-seed, 3-epoch
direction-reproduction check, not statistical certainty — it justifies a
longer matched B/C run or a mechanism investigation, not a final verdict
on λ. Next candidate: a single-variable fix (warmup/LR/clipping) from the
epoch-0 probe above, re-tested against A as control, or a matched
20-epoch B/C rerun to see whether B's epoch-3 edge holds up over a full
schedule. See `tasks/todo.md` for the full backlog with rationale.
