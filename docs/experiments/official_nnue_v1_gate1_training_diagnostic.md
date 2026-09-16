# Official NNUE v1, Gate 1 — training-diagnostic validation

Gates `candidate_seed7.best.bin` (Tier 2's user-confirmed median-seed candidate, PR #61,
`docs/experiments/official_nnue_v1_tier2_training_report.md`) per the preregistration's Gate 1 section
(`docs/experiments/official_nnue_v1_preregistration.md`, "Gate 1 — training-diagnostic validation, branch 4").

- **Phase**: Gate 1 (training-diagnostic validation only). Not Gate 2 (analysis-quality) or Gate 3 (SPRT
  strength). No strength or playing-quality claim.
- **Branch / worktree**: `docs/official-nnue-v1-validation`,
  `/Users/k_tanabe/Documents/Documents/oss_rust/sekirei-nnue-v1-gate1`.
- **Base SHA**: `412ecf1` (main tip after PR #61 merged).
- **Changed files**: this document only. No code changes.
- **Commands run**: none that mutate anything — reads `data/runs/nnue_v1_tier2/candidate_seed{7,42,123}.epoch*.meta.json`
  (all pre-existing, produced by PR #61's training runs) and, for the FT/L2/out row-variance check, a scratch
  Rust example that calls `sekirei_core::nnue::read_weights` on the real candidate file and is **not** committed
  (removed after use — see "Row variance" below). **No new training, labeling, benchmark, match, or SPRT.**

## Verdict method — reused, not invented

Per the user's explicit instruction, this gate evaluates against **existing project criteria**, not new
thresholds picked after seeing the result. The project's own deterministic training-diagnostic verdict logic
already exists in `scripts/gate_dashboard.py`'s `get_pipeline_review()` (vocabulary `HEALTHY`/`WARNING`/
`INSUFFICIENT_DATA`/`INVALID`, deliberately distinct from a gate's `PASS`/`FAIL`/`INCONCLUSIVE` so a healthy
*training diagnostic* is never read as a *strength* claim). That exact algorithm is applied here by hand against
Tier 2's meta.json series (`gate_dashboard.py` itself reads pipeline-script `manifest.json`/`checkpoints/`
layouts that Tier 2's ad hoc worktree run didn't produce, so it isn't invoked as a live tool here — but its
verdict *logic* is reused unmodified):

```
collapsed_epochs   = epochs where valid_output_range == 0.0
last_epoch_collapsed = last epoch in collapsed_epochs
cp_mse_delta       = cp_mse[last epoch] - cp_mse[first epoch]   (needs >= 2 valid readings)
growth_ratios      = output_weight_norm[i+1] / output_weight_norm[i], consecutive epochs
accelerating       = growth_ratios[-1] > growth_ratios[-2]      (needs >= 2 ratios)

if last_epoch_collapsed:            verdict = INVALID
elif cp_mse_delta is None:          verdict = INSUFFICIENT_DATA
elif cp_mse_delta > 0 or accelerating: verdict = WARNING
else:                                verdict = HEALTHY
```

## Result — seed 7 (the selected candidate)

| Signal | Value |
|---|---|
| `cp_mse` epoch1 → epoch20 | 207753.26 → 205846.05 (Δ = **−1907.22**, improved monotonically every epoch) |
| `output_weight_norm` epoch1 → epoch20 | 4.428 → 11.905 (growth ratio epoch19→20 = 1.0, epoch18→19 = 1.0 — **not accelerating**, flat) |
| Collapsed epochs (`valid_output_range == 0.0`) | **none** — final-epoch `valid_output_range` = 114.55 |

**Verdict: `HEALTHY`.**

## Full Gate 1 checklist

| Item | Value (seed 7, best/epoch 3 unless noted) |
|---|---|
| `valid_loss` | 178383.7306 |
| `valid_cp_mse` | 205314.7739 |
| `valid_wdl_loss` | 338471.4593 |
| `valid_calibration_error` | 0.1867 |
| `valid_output_mean` / `std` / `min` / `max` / `range` | −31.19 / 39.39 / −73.78 / 19.37 / 93.14 |
| FT/L2/out row variance | **Non-zero, non-uniform at every sampled row** — see "Row variance" below. No zero-init-style collapse. |
| `ft_active_ratio` | 1.000 (all 256 FT neurons active) |
| `ft_dead_neurons` | 0 |
| `l2_dead_neurons` | 8/32 (25%) — **stable from epoch 2 through epoch 20**, not a growing collapse (full per-epoch series checked, not just the endpoints; see "Dead-neuron trajectory" below) |
| `l2_activation_frequency_mean` (per-neuron array also inspected) | 0.393 mean; per-neuron values range 0.0 (the 8 dead neurons) to ~0.75 |
| `l2_saturation_frequency_mean` (per-neuron array also inspected) | 0.224 mean; per `tasks/lessons.md` (2026-07-14), `l2_ever_saturated_ratio` (0.75 here) means "touched the ceiling at least once," not "stuck at ceiling" — the frequency figure is the non-misleading one |
| `quantized_ft_zero_ratio` | 0.174 |
| Gradient diagnostics | `ft_grad_norm` 4311.2±8768.3, `l2_grad_norm` 905.1±1716.5, `out_grad_norm` 3866.7±2900.6 (mean±std, epoch 3); `grad_clip_count` = 0 at every epoch (no clipping triggered) |
| `param_update_norm` | 12.545 at epoch 3, decaying monotonically to 0.000042 by epoch 20 — the LR-schedule truncation's direct signature (see below) |
| Checkpoint/meta/manifest SHA consistency | Re-verified independently this gate (not just assumed from PR #61): `official_nnue_v1_candidate.bin`/`.meta.json` SHA-256 both match `selection_manifest.json`'s recorded values, exactly |
| 3-seed reproducibility | All three seeds independently verdict `HEALTHY` under the identical algorithm (seed 7: Δcp_mse=−1907.22; seed 42: Δcp_mse=−3826.98; seed 123: Δcp_mse=−1544.94 — same sign, same no-collapse, same non-accelerating-growth pattern; see below) |
| LR-schedule truncation | Present and material — named explicitly below, does not by itself change the verdict per the user's explicit instruction |

### Row variance (zero-init-style collapse check)

Loaded the real candidate via `sekirei_core::nnue::read_weights` (the actual parser, not a reconstructed byte
layout) in a scratch, uncommitted example, and sampled rows spread across each layer:

```
FT row variance (sample rows, i16 quantised): ft[0]=2.09  ft[500]=6.23  ft[1000]=1.77  ft[1500]=26.40  ft[2000]=58.04  ft[2419]=5.12
L2 row variance (sample rows, f32):           l2[0]=0.064 l2[128]=0.097 l2[256]=0.060 l2[384]=0.036 l2[511]=0.050
out weight variance: 2.278  (out_bias=0.851)
```

Every sampled row has real, non-zero, non-uniform variance — no row is a repeated-constant vector. This directly
rules out the zero-init-style symmetry-collapse bug Gate 0 (2026-07-10) found and fixed in this project's
history; the seeded-init fix from that gate is confirmed still working correctly on this run.

### Dead-neuron trajectory (not just the epoch-3 snapshot)

`l2_dead_neurons` for seed 7, all 20 epochs: `0, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8`. The
jump from 0→8 happens once, between epoch 1 and epoch 2, and is flat at 8 for the remaining 18 epochs —
consistent with `param_update_norm` collapsing to near-zero over the same window (LR-schedule truncation, below),
not an ongoing, unbounded collapse in progress. The selected checkpoint (epoch 3) sits inside this already-flat
region, not at a transient low point.

### 3-seed reproducibility cross-check

The same verdict algorithm applied to all three seeds' full epoch series, independently:

| seed | verdict | cp_mse Δ (epoch1→20) | collapsed | accelerating | l2_dead (epoch1→20) |
|---|---|---|---|---|---|
| 7   | HEALTHY | −1907.22 | no | no | 0 → 8 |
| 42  | HEALTHY | −3826.98 | no | no | 1 → 14 |
| 123 | HEALTHY | −1544.94 | no | no | 0 → 10 |

All three seeds independently land on the same verdict via the same unmodified algorithm — not a property of
seed 7 alone. `l2_dead_neurons` at epoch 20 varies more across seeds (8/14/10) than at each seed's own best
(epoch 3) checkpoint, consistent with "Training dynamics" in PR #61: post-epoch-~4, seeds are effectively frozen
at slightly different resting states rather than still actively training.

## LR-schedule truncation — named explicitly, not silently absorbed into the verdict

As documented in PR #61: the trainer's default `StepHalf` schedule halves LR every epoch, so `param_update_norm`
collapses from 35.4 (epoch 2) to 0.000042 (epoch 20) — the real, effective training budget this candidate saw
was closer to ~4 epochs than the nominal 20. `get_pipeline_review`'s algorithm does not have a dedicated check
for this (it wasn't designed with a StepHalf-truncated run in mind), so it does not by itself move the verdict
off `HEALTHY` — but it is the reason `cp_mse`'s epoch-1→20 improvement is concentrated almost entirely in
epochs 1–4 (207753 → ~205602 by epoch 4, vs. 205846 by epoch 20 — the last 16 epochs contribute a −244-unit
change out of the total −1907), and it is why `l2_dead_neurons` stabilizes rather than keeps evolving. **Per the
user's explicit instruction, this is recorded as a known, load-bearing limitation of this candidate, not treated
as grounds for a different verdict or for retraining this round.**

## Gate result

**HEALTHY.** No saturation/dead-neuron *collapse* (dead-neuron count is real but stable, not runaway), no
zero-init-style uniform-row collapse, `cp_mse` improved monotonically, `output_weight_norm` growth is flat/not
accelerating, no output collapse at the final epoch. This is a training-diagnostic verdict only — it says
seed 7's candidate is not degenerate by this project's own existing checks, not that it plays stronger than
material-only Sekirei (that is Gate 3's question, not this one).

## Known limitations carried forward

- LR-schedule truncation (above) — effective training budget ~4 epochs, not 20.
- `l2_dead_neurons` = 8/32 (25%) at the selected checkpoint — real, not collapsing further, but not zero either.
- This candidate has not been evaluated for analysis quality (Gate 2) or playing strength (Gate 3).
- `get_pipeline_review`'s algorithm was applied by hand against Tier 2's own meta.json files, not invoked as the
  live `gate_dashboard.py` tool (which expects a `manifest.json`/`checkpoints/` pipeline-run layout Tier 2's
  worktree run didn't produce) — the verdict *logic* is identical and unmodified, but this was not a script
  invocation with its own independent test coverage confirming the transcription was faithful. Spot-checked by
  hand against the source (`scripts/gate_dashboard.py` lines ~713–763) rather than assumed.

## Next operation

**Not started this round.** Gate 2 (analysis-quality comparison, branch `experiment/official-nnue-v1-analysis-gate`)
and Gate 3 (SPRT strength gate, branch `experiment/official-nnue-v1-strength-gate`) each require explicit
per-step approval per this roadmap's standing rule — this report does not request or assume it. No match,
benchmark, or SPRT run.

## Merge status

Not merged. PR to be opened from this branch, reporting only.

## Items needing approval / explicit confirmation

Whether to proceed to Gate 2 and/or Gate 3 next, and on what schedule — this report surfaces the HEALTHY verdict
and its known limitations (above) as input to that decision, not as a request to proceed autonomously.
