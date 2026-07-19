# quietset falsification test against the teacher-conflict-masking gate: Outcome D (historical data insufficient)

## Status: retrospective audit only. P3 (matched-size ablation) does not proceed from this result.

## Hypothesis

quietset added 10 new optional `Observation` fields plus `block-score`/`trajectory-audit` commands intended to
flag training instability (pathological blocks, seed/order/checkpoint sensitivity) from training-dynamics
signals alone, before any match-play evidence exists. The teacher-conflict-masking gate (rejected 2026-07-19,
159W/0D/237L, Elo −69.3, 95% CI [−104.2, −34.4], all training-dynamics/validation metrics having favored the
candidate) is the strongest available real counterexample: a case where "looks good on validation" and "loses
badly in matches" diverged sharply. The question this audit was commissioned to answer:

> Could quietset's new block/trajectory metrics have flagged this candidate as risky using only
> training-dynamics data available *before* the match result — without looking at match results, and without
> retrofitting thresholds after seeing the outcome?

## Frozen criteria (fixed before generating or inspecting any quietset output)

- quietset commit: `d0a199d` (main, `origin/main` at time of audit)
- Default `BlockThresholds`/`ScoreWeights` — no threshold or weight tuned for this test
- New component weights left at their default `0.0`
- No new quietset profile created
- No post-hoc threshold adjustment to make the candidate detectable
- This is explicitly a **retrospective study** — the gate result was already known before this audit began

## Outcome: D — historical data insufficient to run the falsification test as designed

**This is not "the new metrics failed to catch it."** It is: the specific observations the new metrics need to
discriminate risk were never recorded for this experiment, so the test cannot be run without inventing data.
Per the frozen protocol, unavailable fields are `null`/not computed — never guessed, never zero-filled, and
substitute keys are not used in place of real block identity.

Two candidate substitute `block_id` schemes were considered (`"{arm}:epoch{N}"` for a 3-seed cross-check;
`"{arm}:seed{N}"` for a within-arm epoch1→epoch7 diff) and **rejected**. Both would make `block-score`/
`trajectory-audit` run and produce numbers, but neither satisfies what quietset's `block_id` actually means:
*the same training block observed under different conditions*. An epoch number or an arm+seed pair is not a
training block — it's a point in a training run. Using either as a stand-in `block_id` would launder "we don't
have block data" into "quietset says this block is stable/pathological," which is a worse failure mode than
reporting D. See "Why no substitute `block_id` was used" below.

## What is actually recoverable: field-by-field matrix

| quietset field | Recoverable? | Source | Conversion rule |
|---|---|---|---|
| `block_id` | **No** | — | No position/training-block identifier (e.g. `block_00042`) exists anywhere in the repo — confirmed by a repo-wide grep for `block_[0-9]`/`block_id`, zero hits. The only "block" concept in the codebase (`B1`-`B8`, 32-position windows in `docs/experiments/l2_saturation_ft_freeze_block_screen*.md`) belongs to an unrelated, earlier single-seed diagnostic probe — different checkpoints, not this experiment's arms. |
| `layer_id` | Yes | `data/runs/20260717_longrun_conflict_mask/{arm}_seed{seed}.epoch{N}.meta.json`: `ft_*`/`l2_*`/`out_*` key prefixes | Reshape wide→long: one `Observation` row per `layer_id` ∈ {`ft`, `l2`, `out`} per epoch, reading the correspondingly-prefixed fields. Lossless. |
| `seed` | Yes | `.meta.json: "init_seed"` | Direct copy. Verified as a `REQUIRED_IDENTICAL_FIELDS` invariant between paired control/candidate runs in `scripts/check_longrun_meta.py`. |
| `shuffle_seed` | Yes, but zero discriminating signal | `.meta.json: "shuffle_seed"` | Direct copy. Constant `11` across all 6 runs in this longrun (`--shuffle-seed 11` fixed in `scripts/run_longrun_conflict_mask.sh`) — the field is genuinely present and correct, but was never varied, so `shuffle_direction_consistency`/`order_sensitive` can never fire on this data regardless of what quietset computes. |
| `loss_recipe` | Partial | `manifest.json: "arm"` (`"control"`/`"conflict_ft"`) or `meta.json: "diagnostic_conflict_mask"` (`null`/`"ft"`) | Direct copy, for **Control and Conflict-mask-FT only**. Conflict-mask-FT+L2 and Rate-matched have **zero raw per-run data anywhere in the repo** — only hand-summarized numbers in `docs/experiments/teacher_conflict_masking.md`'s prose tables (a 3-epoch diagnostic run, never re-run at 20 epochs, no saved artifacts). Populating those two `loss_recipe` values from real records is not possible. |
| `gradient_sign` | No | — | `gradient_sign_consistency`-style fields exist as Rust struct fields (`diagnostics.rs`) but are only populated when `--cp-wdl-grad-trace`/`--trace-positions` is passed, writing to a separate `<output>.epochN.trace.json`. `run_longrun_conflict_mask.sh` never passes these flags; a repo-wide search for `*.trace.json` returns zero files. Nothing to convert. |
| `update_cosine` | No | — | Same reasoning as `gradient_sign`. The one relevant fact — `cos(g_cp, g_wdl) ≡ ±1` at every position — is a proven analytic property stated in doc prose (`l2_b5_shadow_trace.md`), never logged as a value. A different cosine (`cos_h_old_delta_h`, hidden-state movement direction) exists only in the unrelated B5-window probe tool, applied to different checkpoints, whose trace files also weren't retained. |
| `teacher_residual` | Partial, with caveats | `.meta.json: conflict_group.{cp,wdl}_residual_abs_mean`, `nonconflict_group.{cp,wdl}_residual_abs_mean` | Present for **both** arms, all seeds, epoch 7 (and every other epoch). Caveats: (a) two separate residuals exist (CP-teacher, WDL-teacher) — no single unified value; (b) per-epoch, per-group (`conflict_group`/`nonconflict_group`) **aggregate mean**, not per-position; (c) **absolute value only** (`_abs_`) — a signed version exists in `trainer.rs` but is only serialized to the never-produced trace.json files. |
| `trajectory_effect` | Partial, with caveats | `.meta.json: param_update_norm`, `{ft,l2,out}_update_norm_mean` | A genuine per-epoch weight-update-magnitude metric, saved for both arms, all epochs. Caveat: **magnitude only, unsigned** — cannot distinguish "growth" from "shrink" the way `trajectory_effect`'s sign is meant to. The directional decomposition (`cos_h_old_delta_h`, radial/orthogonal split) exists only in the unrelated B5-window probe, different checkpoints, trace files not retained. |
| `dead_unit_count` | Yes (per layer) | `.meta.json: l2_dead_neurons`, `ft_dead_neurons` | Direct copy, one row per `layer_id`. No output-layer dead-unit concept exists (no ReLU-style saturation there). |
| `saturated_unit_count` | Partial, weaker than dead_unit_count | `.meta.json: l2_saturation_frequency_per_neuron` (32-entry array), `l2_ever_saturated_ratio`, `ft_saturation_ratio` | No literal saturated-unit **count** exists (unlike dead neurons, a clean whole-epoch integer). A count requires an undictated threshold choice — e.g. `l2_ever_saturated_ratio × 32` — which is a materially looser definition ("ever saturated at least once") than `dead_unit_count`'s strictness ("dead the entire epoch"). Not a direct field read. |

### Why no substitute `block_id` was used

`trajectory-audit` requires the *same* `block_id` to appear in both the before-file and the after-file for a
block to be diffed at all (see `quietset::group::group_by_block_id`, and quietset's own documented `block_id`
identity contract as of commit `138eabc`: unique within one dataset / one before-after pair). Two schemes were
drafted and rejected:

- `block_id = "{arm}:epoch{N}"`, `seed` varying 42/7/123 within it — would let `block-score` compute
  `seed_effect_consistency` across the 3 seeds at a fixed epoch (exactly a Track-A-style 3-seed check). Rejected
  because "the training run at a given epoch, across 3 seeds" is a real repeated-measurement design, but it is
  not a *block* in quietset's sense — there is no shared underlying training unit (a specific set of
  positions/parameters) being re-observed, only "the whole network, at the same point in three separate
  1-in-a-million-different training runs." Reusing block-score's machinery here would silently redefine what
  "block" means without the redefinition being visible in the output.
- `block_id = "{arm}:seed{N}"`, `layer_id` (ft/l2/out) varying within, before=epoch1/after=epoch7 — would let
  `trajectory-audit` diff dead/saturated-unit rates and classification between two checkpoints of the same run.
  Rejected for the same reason: this conflates "the whole network's per-layer state at two points in training"
  with "a training block," and — more concretely — mixes "training progressed" with **no held-fixed sample set**,
  which is exactly the confound quietset's own `loss_recipe` mismatch guard (commit `138eabc`) was built to
  reject when the *recipe* changes between before/after. Here the recipe is the same, but the analogous
  confound (no fixed observation unit) is structurally identical.

Per the user's explicit instruction: **a shuffle-sensitivity design must hold the block's sample set fixed and
vary only in-block order** via `shuffle_seed`. If block composition itself is allowed to vary (as either scheme
above would require, since there is no real recurring sample set here), an observed "order effect" cannot be
distinguished from a "different samples were included" effect. Neither substitute key can satisfy this, which
is the deciding reason both were rejected rather than adjusted.

## Primary endpoints: which ones are even computable here

Of the endpoints named for this audit:

| Endpoint | Computable on this data? | Why |
|---|---|---|
| pathological block rate | No | requires `block_id`; not recoverable |
| trajectory_sensitive block rate | No | requires `block_id` *and* a meaningful `model_id`/checkpoint axis (none exists — `checkpoint_reproducibility` would be `None` even with a substitute `block_id`) |
| block_stability median | No | requires `block_id`; and even hypothetically, collapses to `seed_effect_consistency` alone (`shuffle_direction_consistency` is always `None` since `shuffle_seed` never varies; `checkpoint_reproducibility` is always `None`) |
| early→late rank stability | No | requires a per-sample_id or per-block ranking across a repeated set of comparable units; no such set exists at this granularity |
| `high_teacher_conflict` count (active-review) | No | requires per-sample repeated observations to compute `teacher_residual_stability`'s variance; only one epoch-level aggregate residual exists per (arm, seed, epoch) — no repetition to take a standard deviation over |
| `high_gradient_instability` count (active-review) | No | requires `gradient_sign`; not recoverable |

**All six of the audit's named primary/active-review endpoints are non-computable from what survives.** This
is stated plainly rather than worked around.

## Conclusion

**Outcome D.** The new-metric hypothesis is **neither confirmed nor denied** by this historical run. What can
be said, using only the metrics that *do* survive (see descriptive postmortem below), is the same fact already
on record in `tasks/lessons.md`: the existing validation/training-dynamics metrics (CP MSE, WDL loss, L2
dead-neuron count) supported the candidate, and real match strength went decisively the other way. quietset's
new fields were never in a position to add or contradict that, because the specific signals designed to catch
this class of divergence (`gradient_sign`, `update_cosine`, a real block/position identifier, varied
`shuffle_seed`) were not recorded for this experiment.

**A structural observation worth stating plainly, not just as a footnote:** the fields that *do* survive
(dead-neuron count above all) are precisely the validation-side signals already known to have pointed the
wrong way. The fields that might have caught the divergence (per-position gradient sign, update-direction
cosine, true block identity) are exactly the ones not recorded. This is not evidence the new metrics don't
work — it's evidence this experiment was never instrumented to test them. Whether they would have worked is
open until a properly-instrumented run exists.

**P3 (matched-size A/B/C ablation) does not proceed from this result.** Proceeding would mean building an
ablation on top of a falsification test that never actually ran.

## Descriptive postmortem (separate from the above — not a quietset block-score/trajectory-audit result)

The following uses only real, already-existing per-epoch aggregates (no quietset command was run to produce
this section; it is a plain re-statement of what training logged). It answers a narrower, different question:
*does a simple before/after look at the surviving training-dynamics aggregates, for the actual gate pair
(`control_seed123` vs `conflict_ft_seed123`, epoch 7), show what the existing conclusion already says?* It is
not a test of quietset's new metrics and must not be cited as one.

Epoch 7, 3-seed values (42 / 7 / 123) and mean, both arms:

| Metric | Control (42/7/123, mean) | Conflict-mask-FT (42/7/123, mean) |
|---|---|---|
| `l2_dead_neurons` | 8 / 10 / 12, **mean 10.0** | 0 / 0 / 0, **mean 0.0** |
| `ft_dead_neurons` | 0 / 0 / 0, mean 0.0 | 0 / 0 / 0, mean 0.0 |
| `l2_ever_saturated_ratio` | 0.750 / 0.688 / 0.625, mean 0.688 | 0.938 / 1.000 / 1.000, mean 0.979 |
| `ft_saturation_ratio` | 0.0 / 0.0 / 0.0 | 0.0 / 0.0 / 0.0 |
| `param_update_norm` | 1.271 / 0.911 / 0.876, mean 1.019 | 0.899 / 0.780 / 0.866, mean 0.848 |
| `ft_update_norm_mean` | 0.0022 / 0.0019 / 0.0018, mean 0.0019 | 0.0014 / 0.0012 / 0.0013, mean 0.0013 |
| `l2_update_norm_mean` | 0.0006 / 0.0005 / 0.0005, mean 0.0006 | 0.0005 / 0.0004 / 0.0005, mean 0.0005 |
| `conflict_group.cp_residual_abs_mean` (seed123) | 230.83 | 235.74 |
| `conflict_group.wdl_residual_abs_mean` (seed123) | 566.26 | 558.77 |
| `nonconflict_group.cp_residual_abs_mean` (seed123) | 365.50 | 328.78 |
| `nonconflict_group.wdl_residual_abs_mean` (seed123) | 555.14 | 516.54 |

Honest nuance not previously called out: **`l2_ever_saturated_ratio` favors Control, not the candidate**
(Control 0.625-0.750 vs. Conflict-mask-FT 0.938-1.000 — the candidate has *more* L2 neurons that were
saturated at least once during the epoch). This doesn't overturn the recorded conclusion (the pre-registered
selection rule used `l2_dead_neurons`, `valid_cp_mse`, `valid_wdl_loss` — not `l2_ever_saturated_ratio` — and
"dead" and "ever-saturated" measure different failure modes: permanently off vs. sometimes pinned high), but
it means the honest statement is "the pre-registered validation metrics favored the candidate," not "every
conceivable training-dynamics metric favored the candidate."

**Bounded conclusion of this section only**: existing validation/training-dynamics metrics supported the
candidate; the real match result went the opposite direction. Nothing here is a block-score or
trajectory-audit output, and none of it should be read as validating or invalidating quietset's new fields.

## Required instrumentation for the next run (pre-registered now, before any new training starts)

To make a real, prospective falsification test possible next time, the training harness must save, per run:

- A **real** `block_id` — not a synthetic key. The block must be a fixed, identifiable set of samples (e.g. a
  named position group or shard) that is *re-observed* under different conditions, the same way `sample_id`
  identifies a re-evaluated sample today.
- `seed`, at least 3 distinct values (as already done: 42, 7, 123)
- `shuffle_seed`, at least 3 distinct values (currently fixed at `11` for every run — this is the single
  biggest instrumentation gap found in this audit; `order_sensitive` cannot exist as a signal without it)
- At least 2 checkpoints of the same recipe, for within-recipe `trajectory-audit` (already available: any two
  epochs of the same arm+seed — no new instrumentation needed here, only the block_id gap blocks using it)
- `loss_recipe`, explicit and consistent (already available: `arm`/`diagnostic_conflict_mask`)
- `gradient_sign`, at block or sample granularity — **not currently produced by default**; requires
  `--cp-wdl-grad-trace`/`--trace-positions` (or equivalent) to be part of the standard run, not an opt-in flag
- `update_cosine` — same requirement as `gradient_sign`; both come from the same trace mechanism
- `teacher_residual`, with CP and WDL kept **distinct** (not collapsed into one field), signed (not
  absolute-value only)
- `trajectory_effect`, **signed** (the existing `param_update_norm`/`*_update_norm_mean` are unsigned
  magnitudes only — insufficient to distinguish growth from shrink)
- `dead_unit_count` / `saturated_unit_count`, per layer, with saturated defined as an explicit, pre-registered
  threshold (not left to be reconstructed after the fact from a frequency array)

**Design constraint on the block/shuffle-seed experiment specifically**: the block's *sample set* must be held
fixed across `shuffle_seed` variants — only in-block order may change. If block composition is allowed to vary
between shuffle_seed values, an observed effect cannot be attributed to order vs. to a different sample mix
being included; the two are confounded and no conclusion about order-sensitivity would be possible.

## Related, separate fix: L2 layer width was recorded as 16, not the actual 32

While reading the saved diagnostics for this audit, found `scripts/select_longrun_checkpoint.py`'s `L2 = 16`
constant and this file's own prior "0/16 … 12/16" phrasing were both stale (actual architecture: `L1=256
L2=32`, confirmed by every `.meta.json`'s `"architecture"` field and the 32-entry
`l2_saturation_frequency_per_neuron` arrays). Fixed in a separate commit — see `tasks/lessons.md`'s 2026-07-20
entry for the correction and verified blast radius (none: the actual gate selection is unaffected, since no
epoch in this longrun ever had `l2_dead_neurons` in the affected `[16,31]` range). Raw artifacts (the
`.meta.json`/`.bin`/`.log` files themselves) were not modified.

## Source artifacts consulted (read-only; nothing in `data/` or `results/` was modified)

| Artifact | SHA-256 |
|---|---|
| `docs/experiments/teacher_conflict_masking.md` (pre-correction) | `bc0c85d529dbc4f183577ca98ab712922b8db7f76dfe6607c03bffd672e35d52` |
| `results/20260719_190910_conflict_ft_seed123.epoch7_vs_control_seed123.epoch7.json` | `815fb992060c9f3b7011552b48763684ea5a9e1d11002f4bb51b2c714c71ef42` |
| `results/20260719_190910_conflict_ft_seed123.epoch7_vs_control_seed123.epoch7.verdict.json` | `b86bfebf423e976eb78a74cf49a4d1e50339e4120ba713333f159b5e1cc135ad` |
| `results/20260719_190910_conflict_ft_seed123.epoch7_vs_control_seed123.epoch7.jsonl` | `57ddb2875570c2b180b26cad669f71a0ce1ff9ccc480f2556255cbda8a0cf7ae` |
| `tasks/lessons.md` (pre-correction) | `dc1763964d047881ee3af95fdbc3c3473ef4de5b7bda15d30872b845767a9ff1` |
| `scripts/select_longrun_checkpoint.py` (pre-fix) | `4f8433202061eb12937802e51814a30fa5f6c7f65c8d77b2cd7e3cfc0466c240` |
| `scripts/check_longrun_meta.py` | `af6d5b65924e07de0ae1f5b88e294abdfbe16c608d1f26512afd9f8b6b9be6b2` |
| `scripts/run_longrun_conflict_mask.sh` | `3512df9802bbad040da0c8ac9017db33e1c2c2d800fc23cc5d2396c789bcf41d` |
| `scripts/flatten_label_to_quietset.py` | `d9613d49bae16dd17b417c67c7b84f987f396352c1ceafd44404dd793fa4ffc7` |
| `data/runs/20260717_longrun_conflict_mask/conflict_ft_seed123.epoch7.meta.json` | `c8206025b6211f2f130f9147165e0ea715ac8134456f3fd105c1319cbbf1d7bd` |
| `data/runs/20260717_longrun_conflict_mask/control_seed123.epoch7.meta.json` | `5223ff2daa371ef6671868039b4e202ed9b9e77993136d1da240ee91ba6325e0` |
| `data/runs/20260717_longrun_conflict_mask/conflict_ft_seed123.manifest.json` | `4867bf576a6452d9ee3e7f0677e15619cf497feca37c675cc282c9c5c0323559` |
| `data/runs/20260717_longrun_conflict_mask/control_seed123.manifest.json` | `ccca7c1009eb4576fb68685516f650cfb3ab00682cdbc42df6612dd9c670cbad` |

sekirei commit at time of audit: `730b8ce0e7723f4994064c93b63f5ce2f418a148` (2026-07-19).

Descriptive-postmortem table values were read directly from the epoch-7 `.meta.json` files listed above via a
one-off Python read (no persistent script added; the exact fields read are named in the table).
