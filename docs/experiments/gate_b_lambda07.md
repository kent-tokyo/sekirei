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
neuron death that neither A nor B exhibit at all. This is the "C is
clearly worse" branch of the pre-registered decision tree: **WDL
blending (λ=0.7) has real generalization value; the isolated problem is
running cosine for a fixed 20-epoch budget without early stopping, not
λ or the CSA data source.** C is excluded from further comparison.

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

**A/B/C conclusion**: WDL blending (λ=0.7) is worth retaining — Run C
showed it has a real generalization effect, not just a training-time
artifact. Cosine run for a fixed 20 epochs is not adopted — not because
of λ (Run C shows the overfitting isn't a WDL problem), but because it
overfits hard regardless of λ once the LR-starvation ceiling is lifted.
step-half stays the control despite wasting roughly half its epoch
budget once the LR decays — B did not demonstrate a real playing-strength
edge over it. **Valid loss alone is not sufficient for checkpoint
selection**: B's best-valid-loss checkpoint did not produce a
measurable playing-strength improvement over A's. Next candidates (not
started): early-stopping/`--patience` (saves compute, does not itself
fix B's overfitting), investigating why `l2_active == l2_sat` persists
across all three runs, and a single-variable regularization experiment
(weight decay, gradient clipping, or output-scale target — one at a
time) re-tested against A as control. See `tasks/todo.md` for the full
backlog with rationale.
