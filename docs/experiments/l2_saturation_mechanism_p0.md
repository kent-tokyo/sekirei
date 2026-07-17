# L2 saturation collapse mechanism (P0a/P0b): clamping-only, saturation-driven, and a correlated FT+L2 update structure — not a single isolated layer

## Background

`docs/experiments/sample_grad_correlation_trace.md` (Stage 2) found that L2's gradient collapses to all-zero for
virtually every position by ~1/4 of the way into epoch 1 (45.7% → 99.2% → 99.6% → 100.0% across four quartiles of
the first 1024 positions), and — per the user's explicit sequencing decision — this structural question was
investigated first, before returning to the data-order question (`--shuffle-seed`), since if the L2 gradient path
is closed entirely by then, neither data-order nor loss-weighting changes downstream of that point can matter.
Two sub-investigations, run in sequence per the user's own decision framework:

- **P0a**: classify *why* each (position, neuron) gradient is zero, using the exact backward-pass identity
  `dL/dz_L2 = dL/dOutput × output_weight × ClippedReLU'(z_L2)`, aggregated by position band.
- **P0b**: for the transitions where the collapse actually happens, decompose *what moves* `L2`'s pre-activation
  on fixed probe positions: `Δz = Δh·W_old + h_old·ΔW + Δh·ΔW + Δb`, with dense probe points around the collapse
  window (`32,64,128,192,224,256,288,320,384`) rather than epoch boundaries.

## P0a: zero-gradient reason classification

Pure Python analysis against the existing `--sample-grad-trace` JSONL output — no new Rust code. `l2_gate` already
records `ClippedReLU'(z_L2)`'s two zero cases directly (`gate == -1` → `clamped_low`, `gate == 1` → `clamped_high`);
when `gate == 0` (linear zone) and the recorded gradient is exactly zero, the reason is disambiguated by
reconstructing `dL/dOutput` from the already-recorded `cp_d_output`/`wdl_d_output` (blended at `--wdl-lambda 0.7`)
into `residual_zero` (`dL/dOutput ≈ 0`) vs. `output_path_zero` (`output_weight ≈ 0`, checked by elimination).
3 seeds (7, 42, 123), 1024-position `--sample-grad-trace` runs, 32-neuron L2 layer, banded by position:

| band | linear | clamped_low | clamped_high | residual_zero | output_path_zero |
|---|---|---|---|---|---|
| seed 7: 1–64 | 29.3% | 70.7% | 0.0% | 0.0% | 0.0% |
| seed 7: 65–128 | 30.0% | 68.9% | 1.0% | 0.0% | 0.0% |
| seed 7: 129–192 | 0.9% | 68.8% | 30.4% | 0.0% | 0.0% |
| seed 7: 193–256 | 0.0% | 68.8% | 31.2% | 0.0% | 0.0% |
| seed 7: 257–512 | ~0.1% | 68.8% | ~31.1% | 0.0% | 0.0% |
| seed 42: 1–64 | 45.2% | 54.8% | 0.0% | 0.0% | 0.0% |
| seed 42: 129–192 | 0.9% | 56.2% | 42.9% | 0.0% | 0.0% |
| seed 42: 257–512 | ~0.1% | 56.2% | ~43.6% | 0.0% | 0.0% |
| seed 123: 1–64 | 39.2% | 60.8% | 0.0% | 0.0% | 0.0% |
| seed 123: 129–192 | 0.3% | 62.5% | 37.2% | 0.0% | 0.0% |
| seed 123: 257–512 | ~0.1% | 62.5% | ~37.3% | 0.0% | 0.0% |

(Full 6-band table per seed is in the analysis script output; collapsed here to the bands where the transition
happens — `65–128` is the pivot band in all 3 seeds.)

**Unambiguous across all 3 seeds**: `residual_zero` and `output_path_zero` are exactly `0.0%` in every band —
none of the zero-gradient epidemic comes from the loss residual or the output-layer weight vanishing. **It's
entirely `ClippedReLU` clamping.** Two further facts sharpen this past "it's clamping" into a specific mechanism:

- **`clamped_low` (true dead, `z ≤ 0`) is fixed by position 64 and never grows again** — 70.7%/54.8%/60.8% at
  position 64, then flat (68.8%/56.2%/62.5%) for the rest of the window in all 3 seeds. Whatever determines *which*
  neurons die happens early and locks in; it isn't a slow bleed continuing through the epoch.
- **`clamped_high` (saturation, `z ≥ 127`) is what actually grows** — 0% at position 64 → 30–44% by position 192,
  mirroring `linear`'s collapse from 29–45% down to ~0% in the *same* window. The neurons that survive the early
  death cutoff aren't staying healthy — most of them saturate instead, in the 65–192 window specifically.

This directly explains Stage 2's 45.7%→99.2%→99.6%→100.0% all-zero-gradient quartile progression: it isn't a
gradually accumulating death count, it's a fixed dead fraction (locked by position 64) *plus* a saturation wave
that eats nearly all of the remaining healthy neurons by position ~192–256.

## P0b: forward-side Δz decomposition

New example `crates/sekirei-core/examples/l2_delta_z_probe.rs`: given two full-precision weights checkpoints and a
fixed probe set (SFEN positions via stdin), computes each checkpoint's FT output manually (`ft_output`, mirroring
`NnueAcc`'s saturating-i16-accumulate math against an explicitly-passed `&NnueWeights`, not the global `weights()`
singleton — needed because two checkpoints must be probed in one process) and decomposes the algebraically-exact
identity `(h_old+Δh)·(W_old+ΔW) − h_old·W_old = Δh·W_old + h_old·ΔW + Δh·ΔW` into per-neuron `term1`/`term2`/`term3`,
plus `term4 = Δb` (board-independent). New `--trace-weights` flag on `sekirei-train` dumps a full `TrainWeights`
snapshot (not the quantized checkpoint — avoids quantization noise) at each `--trace-positions` marker.

**Verification**: (a) identity `z_new − z_old ≈ term1 + term2 + term3 + term4` holds to ~1e-5 across all probed
positions/neurons/transitions (float-precision only, exact algebraically); (b) `z_old`'s value matches the
independently-implemented `l2_saturation_probe.rs`'s computation exactly for the same checkpoint/position.

Dense probe points around the collapse window (`32,64,128,192,224,256,288,320,384`), 3 seeds, fixed probe set
(the same 32 positions each seed's own epoch-1 trace visited in `0..32`).

**Timing confirms P0a exactly, from a completely independent measurement (forward-side movement, not
backward-side gradient)** — mean `|Δz|` pooled across all neurons/positions, per transition:

| transition | seed 7 | seed 42 | seed 123 |
|---|---|---|---|
| 32→64 | 3.99 | 6.32 | 5.41 |
| 64→128 | 35.19 | 47.76 | 41.67 |
| 128→192 | 15.63 | 16.47 | 15.46 |
| 192→224 | 0.25 | 1.31 | 1.09 |
| 224→256 | 0.035 | 0.16 | 0.15 |
| 256→288 | 0.002 | 0.007 | 0.007 |
| 288→320 | ~0.000 | ~0.000 | ~0.000 |

Movement peaks at `64→128`, is still substantial at `128→192`, and is essentially zero from `224→256` onward.
**Three independent measurements now agree on the same window**: P0a's band classification (saturation completes
by ~192), Stage 2's all-zero-gradient quartiles (>99% by position ~300), and this Δz magnitude decay (<1% of peak
by position 224). The collapse is not a slow drift — it is essentially complete by position ~224–256, and any
intervention has to act inside the first ~200 positions of epoch 1 to matter.

**Term decomposition for the neurons that end up saturated** (summed across all 8 transitions, i.e. the full
32→384 span), signed sums (not absolute — tests reinforcement vs. cancellation) for every neuron that's saturated
in >50% of the final-transition probe positions:

| seed | saturated neurons | neurons where term1/term2/term3/term4 are *all* same-signed as total Δz |
|---|---|---|
| 7 | 10 | 10/10 |
| 42 | 14 | 14/14 |
| 123 | 12 | 12/12 |

**36/36 saturating neurons across all 3 seeds: every nonzero term reinforces the total movement, none cancel.**
Two further robust (checkpoint-spacing-independent) facts:

- **`term4` (bias) is uniformly negligible** — typically 15–30 vs. term1/term2/term3 in the 10,000–22,000 range,
  i.e. under 0.2% of the total movement, in every single saturating neuron, every seed. Bias update is not a
  driver of the collapse.
- **`term1` (FT-output movement) and `term2` (L2-weight-update movement) are both large** — comparable order of
  magnitude in the large majority of neurons (e.g. seed 42: every one of 14 neurons has `term1`/`term2` within
  ~10% of each other), with a small consistent minority per seed showing `term2` more clearly ahead (seed 7:
  neurons 8, 12; seed 123: neuron 13, where `term2` is roughly 1.3–3.6× `term1`).

**One reading is explicitly *not* supported and is left out of the conclusion**: a clean term1-vs-term2 *ranking*
per neuron. `term3` (the `Δh·ΔW` cross/interaction term) is itself substantial — roughly 70–80% of `term1`'s
magnitude in the peak transitions — which means these 32-position-wide checkpoint steps are coarse relative to how
far the weights actually move per step; a second-order cross term that large is a sign the ratio between term1 and
term2 would shift under finer-grained checkpointing, not a property of the underlying dynamics. The per-neuron
"term2 leads" outliers noted above are reported as an observation, not as evidence those specific neurons are
"L2-weight-driven" — that attribution isn't safe from a frozen-weights decomposition at this checkpoint spacing.

## Conclusion

**Both P0a and P0b land on the same reading, from independent measurements**: the epoch-1 L2 collapse is not a
single-layer failure. FT output movement (`term1`) and L2 weight movement (`term2`) are both large and both push
every saturating neuron in the *same* direction as its actual saturation, with a substantial reinforcing
interaction term (`term3`) on top — the bias term (`term4`) is ruled out cleanly as negligible everywhere. This
matches the pre-registered "複数項が同方向なら、相関した更新構造が本命" (multiple terms moving the same direction
→ a correlated update structure is the primary suspect) branch, not a clean single-term-dominant result.

**This decomposition is correlational, not causal, and can't be pushed further on its own** — it only shows that
FT and L2 move together and in the saturating direction; it cannot say what would happen if only one of them
moved (freezing a layer lets the other layer keep adapting, which a frozen-weights decomposition structurally
cannot simulate — the same fixed-weights-counterfactual limitation `sample_grad_correlation_trace.md`'s Stage 2
was explicit about). **This is exactly the condition the freeze-diagnostic (Control / Arm FT / Arm L2 / Arm Out,
~512 positions, freeze first 320) was pre-registered to resolve**, and P0a+P0b's signal — clean saturation-only
mechanism, timing pinned to positions ~65–192, correlated multi-term reinforcement — is a clear enough result to
warrant running it.

One specific reading to pre-register **before** running the freeze arms, since the large `term3` cross-term
predicts it: freezing FT alone *and* freezing L2 alone may **each** reduce the collapse somewhat (because the
interaction needs both layers moving), which would confirm joint structure rather than mean either one is *the*
sole cause read in isolation. The **Out-freeze arm is the one tied to the standing output→L2 hypothesis** from
`sample_grad_correlation_trace.md` — if freezing the output layer's updates stops the correlated FT+L2 saturation,
that implicates the output-layer gradient as the common driver feeding both, which is the exact backprop-structure
thread this P0a/P0b work was launched to chase.

**Not run yet.** Per this investigation's standing discipline (the same scope-down that split the sample-shuffle
work into stages before touching production code), the freeze arms are new *training-intervention* code, not
another read-only diagnostic — held for explicit user sign-off before implementation, same as Stage 3's
micro-replay harness was held pending Stage 2's result.

## Status

- P0a: complete, decisive, 3/3 seeds consistent, pure analysis script against existing trace data — no new code.
- P0b: `l2_delta_z_probe.rs` + `--trace-weights` built and verified (identity check, cross-check against
  `l2_saturation_probe.rs`), run across 3 seeds × 8 transitions. **Uncommitted** — `TrainWeights: Clone`, the
  `weight_snapshot_trace`/`weight_snapshots` `Trainer` fields, `save_weight_snapshots`, the `--trace-weights` flag,
  and the new example are all pending a commit decision.
- Freeze-diagnostic (task tracked as #91): not started, awaiting explicit go-ahead — this doc's finding is the
  trigger condition, but the decision to implement new training-intervention code is not this doc's to make.
- Analysis scripts and raw data: `sample_grad_exp/` in scratch (`classify_zero_reason.py`,
  `analyze_delta_z.py`, `signed_sums.py`, `deltaz_seed{7,42,123}_*.json`).
