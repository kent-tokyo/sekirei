# Fast/slow state-swap probe: saturation needs the fast trajectory's *specific, co-aligned* FT+L2 pairing — FS fails on alignment, SF fails on arithmetic norm-starvation

## Background

The freeze diagnostic (`l2_saturation_freeze_diagnostic.md`) established that FT movement and L2-weight movement
are each individually *necessary* for saturation — freezing either alone, while the other keeps adapting within
the same run, fully suppresses it. That's a necessity result about *movement*. It doesn't say whether saturation
needs the two layers' *specific, co-evolved endpoint states* to line up, or whether "big enough movement in both"
is sufficient regardless of which trajectory produced it. The order-sensitivity finding (`l2_saturation_order_
sensitivity_p1.md`) gives exactly the material to test this directly: shuffle-11 ("fast") reaches full saturation
by position ~128-192; shuffle-33 ("slow") stays mostly healthy through position 320, same init seed, same
architecture, same recipe otherwise. Cross-connecting FT output and L2 weights between the two checkpoints at
matching positions — without building any new sampler — asks the question directly: does *fast's own L2*, paired
with *slow's FT output* (or vice versa), still saturate?

## Method

New `crates/sekirei-core/examples/l2_state_swap_probe.rs`, structured as a sibling of the already-verified
`l2_delta_z_probe.rs` (reuses its `ft_output()` unchanged). Given two checkpoints sharing the same `--init-seed`
(neuron index `o` only corresponds across checkpoints because SGD never permutes indices — swapping across
different init seeds would be meaningless and the tool never does this), computes all 4 combinations of
FT-source × L2-source at each probe position:

```
FF = FT_fast × L2_fast   (the fast checkpoint itself)
FS = FT_fast × L2_slow
SF = FT_slow × L2_fast
SS = FT_slow × L2_slow   (the slow checkpoint itself)
```

Output layer never enters `z = h·w_L2 + b_L2`, so it's not read. Per neuron, also decomposes `z − b = ‖h‖ · ‖w_col‖
· cos(θ)` (`h` = concatenated 2×L1-wide FT output, `w_col` = that neuron's L2 weight column) — `‖w_col‖` only
depends on the L2 source, reported once per checkpoint pair, not per position.

**Verification (required before trusting FS/SF)**: FF and SS are literally the unmodified fast/slow checkpoints,
so their dead/linear/saturated classification must match `l2_saturation_probe.rs`'s independent, already-
established implementation exactly. Confirmed on `seed42_shuf11`/`seed42_shuf33` at position 320: FF matches
`l2_saturation_probe`'s per-neuron output exactly (56.2%/0.1%/43.7% dead/linear/saturated, every one of 32
neurons identical), SS matches exactly (32.9%/67.1%/0.0%). A forward-math bug would have corrupted all four cells
identically; this cross-check rules that out for at least the two directly-checkable ones.

**Data**: 6 re-runs (shuffle 11 + 33 × init seeds 42/7/123) of the P1 recipe with `--trace-weights --trace-
positions 0,32,64,96,128,160,192,256,320` added (P1 itself didn't need weight snapshots) — same `verify_meta.py`
self-check, same shared teacher cache. Position 0 doesn't produce a snapshot (documented no-op — the first
opportunity is after position 1 completes), so 8 of the 9 requested positions are probed. Fixed 261-position probe
set (`l2probe.sfen`, reused from P0b).

## Results: the categorical finding

Saturated fraction (pooled over all 32 neurons × 261 probe positions), all 4 combos, positions 96–320 — this is
the window where fast is already collapsing and slow is still mostly healthy:

| seed | pos | FF | FS | SF | SS |
|---|---|---|---|---|---|
| 42 | 128 | 13.3% | 0.0% | 0.0% | 0.0% |
| 42 | 192 | 40.8% | 0.0% | 0.0% | 0.0% |
| 42 | 320 | 43.7% | 0.0% | 0.0% | 0.0% |
| 7 | 128 | 8.6% | 0.0% | 0.0% | 0.0% |
| 7 | 192 | 19.7% | 0.0% | 0.0% | 0.0% |
| 7 | 320 | 27.6% | 0.0% | 0.0% | 0.0% |
| 123 | 128 | 11.4% | 0.0% | 0.0% | 0.0% |
| 123 | 192 | 30.7% | 0.0% | 0.0% | 0.0% |
| 123 | 320 | 37.5% | 0.0% | 0.0% | 0.0% |

**`FS` and `SF` read exactly `0.0%` at every single one of the 24 seed×position combinations probed (32 through
320), in all 3 seeds — only `FF` ever saturates.** Per the pre-registered decision table, this is squarely the
first branch: *fast's own FT-and-L2 pairing is what carries the collapse; neither component alone, recombined
with the other trajectory's counterpart, reproduces it.* This is trajectory-specific co-adaptation — genuinely
more specific than "both layers must move" (the freeze result): it's not enough for both to have moved a lot: it
has to be *this* FT state with *this* L2 state, evolved together.

## Results: FS and SF fail for two different reasons — this matters, don't conflate them

The first pass read this as "alignment/cosine dominates." **That's not fully supported and was caught before
writing it as a conclusion.** `mean‖h‖` differs enormously by FT source: fast-FT's output norm is ~50-63 at
position 320, slow-FT's is only ~5-13 — roughly a **10× gap**. That single fact is already enough to explain `SF`
on its own, with no alignment story needed:

**SF is arithmetic norm-starvation.** For the neurons that actually saturate in FF, the *maximum possible* `z` for
SF — computed assuming perfect alignment, `cos(θ)=1` — using slow-FT's actual mean `‖h‖` and fast-L2's actual
`‖w_col‖`, comes to only **13.6–22.0** across all 3 seeds (need `≥127` to saturate). SF cannot saturate regardless
of alignment; the FT-output norm alone is an order of magnitude too small.

**FS is a genuine alignment effect, not primarily a norm effect.** Restricting to the neurons that saturate in FF
by position 320, and comparing `FF`'s and `FS`'s actual norm/cosine terms for exactly those neurons (both use the
*same* `h_fast`, isolating the L2-source contribution):

| seed | avg `‖w_fast_col‖` | avg `‖w_slow_col‖` | norm ratio | avg `cos_FF` | avg `cos_FS` |
|---|---|---|---|---|---|
| 42 | 3.289 | 1.543 | 0.469 | **0.9037** | **0.0160** |
| 7 | 3.363 | 1.487 | 0.442 | **0.9087** | **0.0626** |
| 123 | 3.372 | 1.501 | 0.445 | **0.9101** | **0.0108** |

`‖w_slow_col‖` is about half of `‖w_fast_col‖` (ratio ~0.44-0.47) — a real but moderate difference that alone
would not prevent saturation (halving `z≈181` still leaves `≈90`, close to the wall). **The dominant term is
`cos(θ)`**: `cos_FF ≈ 0.90-0.91` — fast-FT's output direction is *near-perfectly aligned* with fast-L2's
corresponding weight columns for exactly the neurons that saturate — while `cos_FS ≈ 0.01-0.19`, **near-orthogonal
to slow-L2's columns for the same neuron indices**, in all 3 seeds. Alignment collapses roughly 10-20×; norm only
drops ~2×. Fast training doesn't just grow FT output and L2 weights in magnitude — it grows them *into alignment
with each other*, specifically for the neurons that end up saturated, and that alignment is what a different
(slow) L2 trajectory doesn't share even though it's the same architecture from the same init.

## Conclusion

**Categorical result (solid, 3-seed, tool-verified)**: saturation requires fast's *own* co-evolved FT+L2 pairing —
swapping either component's source to the slow trajectory eliminates it completely, at every checkpoint tested.
Same-trajectory FT+L2 co-adaptation was necessary for saturation to reappear under swap — that's the load-bearing
categorical claim, independent of mechanism.

**Mechanism, explicitly split, not left as a single unattributed "alignment/norm" claim, and not reduced to
"alignment alone"**: the two swap directions fail for *different* reasons. `SF` fails on raw FT-output-norm
insufficiency — arithmetic, no alignment argument needed. `FS` fails predominantly because slow-L2's weight
columns, for the specific neurons fast's L2 has driven toward saturation, are only weakly (near-zero, sometimes
slightly negative) aligned with fast's FT output direction — the alignment collapse (10-20×) is larger than the
accompanying norm drop (~2×), so alignment is the dominant, not the only, factor in `FS`'s case specifically. `FF`
itself saturates on the combination of both large norms *and* ~0.9 alignment together — norm growth alone (matched
by `SS`'s own within-trajectory norm growth, which never gets an aligned partner) doesn't produce saturation
either. Read together: it's norm growth **and** trajectory-specific directional co-adaptation acting jointly, not
either one alone.

**Scope, stated explicitly**: this is a counterfactual result on the specific checkpoints (3 seeds × 8 positions,
fast=shuffle-11/slow=shuffle-33) and the fixed 261-position probe set actually tested here — it demonstrates that
*for these trajectories*, neither component's endpoint state alone (recombined with the other's) reproduces
saturation. It is not a general mathematical necessity proof that FT+L2 co-adaptation is *always* required for
this architecture to saturate under any training trajectory whatsoever.

This directly motivates the next-branch question the user posed for the "FF only" outcome: which part of the
*fast* trajectory builds this alignment, and does it build early or accumulate gradually? (`cos(ΔFT, W_old)`,
`cos(FT_old, ΔW)`, `cos(ΔFT, ΔW)` compared fast vs. slow, per the user's own follow-up plan — not run here.)

## Status

- `l2_state_swap_probe.rs`: implemented, verified (FF/SS exact match against `l2_saturation_probe.rs`), `fmt`/
  `clippy` clean. **Uncommitted.**
- 6 re-runs (shuffle 11/33 × 3 seeds) with `--trace-weights` needed regenerating — P1 itself didn't dump weight
  snapshots. Raw data and scripts in scratch (`p1_shuffle_exp/`: `l2_state_swap_probe` binary output
  `swap_seed{42,7,123}_pos{32..320}.jsonl`, `analyze_swap.py`, `discriminate_fs.py`).
- Not yet run: the `cos(ΔFT,W_old)`/`cos(FT_old,ΔW)`/`cos(ΔFT,ΔW)` fast-vs-slow comparison that would show *when*
  and *from which term* the alignment forms — the user's own pre-registered next branch for this outcome.
