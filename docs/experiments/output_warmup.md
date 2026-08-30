# LR warmup (`--warmup-epochs`): doesn't rescue the epoch-1 output collapse

## Background

Follow-up to the gradient-clipping investigation (`docs/experiments/global_gradient_clipping.md`, closed
bucket-4), which left two output-layer failure modes on the B recipe (`--wdl-lambda 0.7 --lr-schedule cosine
--epochs 3`, seeds 42/7/123) unexplained: **(A)** epoch-2+ `output_weight_norm`/`valid_output_std` runaway growth,
and **(B)** an epoch-1 `valid_output_std≈0` collapse in some seeds. Clipping barely engages at epoch 1 (~0-2%
trigger rate) and doesn't touch either mode. The pre-existing priority queue for the next single-variable
experiment (`tasks/todo.md`, 2026-07-14) named LR warmup first.

**A correction found before this experiment could run as originally planned**: the evidence this investigation
already had for failure mode B (`B_seed7`/`B_seed123` collapsing at epoch 1) comes from `seed3_exp` (2026-07-14),
which was itself launched with `--warmup-epochs 1`. `compute_lr` (trainer.rs:122-147) assigns one LR scalar per
epoch, not a per-step ramp — with `warmup_epochs=1`, epoch 1's formula is `base_lr × 1/1 = base_lr`, identical to
no warmup at all. **`--warmup-epochs 1` cannot perturb epoch 1's dynamics by construction**; it only affects
epochs 2+ via the post-warmup schedule's re-basing. The already-collected `B_seed7`/`B_seed123` collapse data is
real, but it does not represent "no warmup" being tested against "warmup" — `warmup=1` was already the
de facto baseline throughout, and it failed. The minimal warmup value that actually reduces epoch 1's LR is
`--warmup-epochs 2` (epoch 1 → 0.5×base_lr).

**A second correction, found while pulling metrics**: `seed3_exp` also predates commit `41f0d83`
("decouple LR schedule horizon from `--epochs`"), the same schedule-compression bug `tasks/lessons.md` already
flagged and marked `INVALID_FOR_EPOCH3_COMPARISON` for a separate matched-ablation batch. `seed3_exp`'s epoch 3 LR
lands on the `min_lr` floor (`0.00001`) instead of the intended near-`base_lr` value a 20-epoch horizon would give.
This bug cannot reach epoch 1 (the warmup branch never reads `total_epochs`) and doesn't materially affect epoch 2
either (both arms land at ≈`base_lr` regardless of horizon) — so the primary comparison below is unaffected — but
it does invalidate any epoch-3 comparison in this experiment too, for the same reason as the earlier batch.

## Experiment: `--warmup-epochs 1` (control) vs `--warmup-epochs 2` (candidate)

Fixed across both arms: `--wdl-lambda 0.7 --label-depth 4 --min-rate 1500 --lr-schedule cosine --min-lr 0.00001
--epochs 3 --split-seed 42`, seeds 42/7/123 (`--init-seed`), one shared teacher-search cache
(`seed3_exp/teacher_cache.jsonl`, ≥99% cache hit rate on the new runs).

**Control was re-run rather than reused from `seed3_exp`**: the original plan intended to reuse `seed3_exp`'s
existing warmup=1 data directly (approved for compute efficiency and continuity), but that data is both an older
binary (commit `19190f8`, predating `41f0d83`/`d693028`/`de23164`/`4d583f1`) and the literal
`INVALID_FOR_EPOCH3_COMPARISON` batch — reusing it would have confounded any effect with a binary change. The 3
control runs were re-executed on the current binary (commit `d44dbd0`) instead; their epoch-1/epoch-2 numbers are
byte-identical to the original `seed3_exp` data, confirming the intervening commits didn't change unflagged
behavior — but this is now a clean single-binary, single-variable comparison rather than an assumption.

### Primary judgment: epoch 1 (the only epoch where the sole difference is LR: 1.0×base_lr vs 0.5×base_lr)

| seed | arm | `l2_dead_neurons` | `valid_output_std` | `valid_cp_mse` |
|---|---|---|---|---|
| 42 | control (warmup=1) | 2 | 19.403 | 172909.7 |
| 42 | candidate (warmup=2) | **7** | **0.000** | 173535.1 |
| 7 | control (warmup=1) | 5 | 0.000 | 173534.0 |
| 7 | candidate (warmup=2) | 5 | 0.000 | 173739.5 |
| 123 | control (warmup=1) | 3 | 0.000 | 173513.8 |
| 123 | candidate (warmup=2) | 3 | 0.028 | 173604.1 |

**Seeds 7 and 123 (the two that were already collapsing): unchanged.** Identical `l2_dead_neurons` count in both
arms (5=5, 3=3) despite halving epoch 1's LR — the dead-neuron count doesn't move at all under a 2× LR change.
`valid_output_std` stays at/near exactly zero in the candidate too (123's 0.028 is not a meaningful recovery next
to the ~19-31 std range healthy epochs show elsewhere in this data). Halving the first update's step size does not
rescue either previously-collapsing seed.

**Seed 42 (previously stable): collapses under the candidate.** `l2_dead_neurons` 2→7, `valid_output_std`
19.4→0.000 — a clean, non-confounded (same binary, single-variable) observation that the *gentler* start triggers
a collapse that wasn't present under the stronger one. This is reported as an observation, not a mechanism: seed42
already showed exactly this kind of small-perturbation-triggered epoch-1 flip in both prior clipping experiments
(`global_gradient_clipping.md`'s Experiment 1 and 2, both n=1, both self-recovering) — it's a known-bistable seed,
and the collapse here is transient too (see epoch 2 below). Read as "seed42 is on a knife-edge that different
epoch-1 LR values can tip either way," not as "warmup causes collapse."

### Secondary/auxiliary: epoch 2 (schedule-horizon bug doesn't reach here either — both arms land at ≈base_lr)

| seed | arm | `l2_dead_neurons` | `valid_output_std` | `valid_cp_mse` |
|---|---|---|---|---|
| 42 | control | 8 | 56.931 | 161521.1 |
| 42 | candidate | 2 | 18.553 | 171838.2 |
| 7 | control | 7 | 3.917 | 173413.3 |
| 7 | candidate | 5 | 16.122 | 171486.4 |
| 123 | control | 0 | 21.119 | 171235.7 |
| 123 | candidate | 0 | 33.722 | 168719.5 |

Every collapsed epoch-1 state recovers by epoch 2 in both arms — failure mode B is transient within a 3-epoch
window regardless of warmup, not a permanent kill. Beyond that, the epoch-2 pattern doesn't support a clean
narrative either way (seed42's candidate arm, despite collapsing harder at epoch 1, has *fewer* dead neurons at
epoch 2 than its own control; seed7's candidate has better `valid_cp_mse` but seed123's is worse) — not read as
evidence for or against warmup, just recorded. Epoch 3 is excluded from analysis (schedule-horizon bug, see above).

## Applying the pre-registered decision framework

Framework (fixed before running, `tasks/lessons.md`/user's design instructions):
1. **Promote**: `l2_dead_neurons` drops in all 3 seeds under warmup=2.
2. **Scale-only**: dead-neuron count unchanged, but output runaway measurably reduced.
3. **Robustness-only**: seed7/123 improve, seed42 unchanged.
4. **No change**: none of the above.

Dead-neuron count doesn't drop in any seed (flat in 2/3, worse in 1/3) — bucket 1 fails outright. Seed7/123 show
zero movement, not improvement — bucket 3 fails (backwards from what it describes). No clean scale-only
improvement either (epoch-2 pattern is mixed, not directional). **Result: bucket 4 — no change**, with an
additional negative data point (a new, transient collapse in a previously-stable seed) that doesn't change the
bucket but is worth carrying into the next investigation.

## Conclusion

**`--warmup-epochs 2` does not rescue the epoch-1 output collapse.** The two seeds that were already collapsing
under `warmup=1` (7, 123) show *zero* change in dead-neuron count under a 2× reduction in epoch 1's LR — this is
the more informative result than it first appears: if these neurons died because the first update was simply too
strong, halving that update should have saved at least some of them. It didn't save any. **This is more consistent
with these specific neurons being dead (or effectively dead-on-arrival) at initialization than with an
update-magnitude-driven death** — though this sits in some tension with the earlier epoch-0 probe
(`tasks/lessons.md`, 2026-07-14) which found the majority of a *different* run's dead neurons were created during
epoch 1's execution, not already dead at init. Reconciling that tension is not attempted here; it's flagged for
the next investigation rather than resolved by inference.

Failure mode B is transient in every case observed here (both arms, all 3 seeds) — collapsed epoch-1 states
recover by epoch 2 regardless of warmup. Given a long enough schedule this may be a self-limiting nuisance rather
than a permanent failure; that hasn't been tested past epoch 3 here and isn't claimed.

## Status

- `--warmup-epochs` stays in the codebase as-is (pre-existing flag, not modified by this experiment).
- **Not promoted.** The warmup line is shelved per the bucket-4 outcome — it doesn't move the metric it was aimed
  at (epoch-1 dead-neuron count) in any seed, and offers no compensating win elsewhere.
- **Priority queue update** (was: (1) warmup, (2) lower initial LR, (3) gradient clipping [closed]): warmup is now
  closed alongside clipping. Next candidate: **(2) initialization/L2-bias investigation** — directly motivated by
  this experiment's own finding that the collapsing neurons are insensitive to a 2× epoch-1 LR change, which
  points at init rather than update magnitude. A lower-`base_lr` experiment (the other original priority-2 option)
  is a weaker candidate now, since this data suggests the mechanism isn't LR-magnitude-sensitive at all.
- Full per-epoch data: `warmup2_exp/` (candidate + control rerun) and `seed3_exp/` (original control, superseded
  for this comparison but left in place) in scratch.
