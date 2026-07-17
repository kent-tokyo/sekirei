# B5 one-step shadow trace: within-step Adam interaction is sub-additive (protective), not the driver of the 32-step collapse

## Background

`l2_b5_cp_wdl_component_replay.md` found that a 32-step Blended counterfactual replay of B5 (`144-175`) produces a
deeper FT dead-unit collapse than either CP-only or WDL-only replay alone, but explicitly could not determine
whether that excess reflects genuine within-step optimizer/network interaction, cross-step threshold
accumulation, or simple 32-step trajectory divergence between the differently-updated arms — a 32-step trajectory
comparison cannot rule any of the three out. This experiment isolates a single training step: at each of B5's 32
positions, branch CP-only/WDL-only/Blended one-step counterfactual FT+L2 updates from an *identical* shared
pre-update state (weights + Adam `m`/`v` moments), evaluate each branch's outcome on the fixed 261-position probe
set, then discard every branch without ever mutating real training. Because all three branches share the same
anchor, any difference between them is attributable to that one step alone — the 32-step trajectory-divergence
confound cannot arise by construction.

**Mechanism** (`compute_shadow_trace`, `shadow_component_grad`, `apply_shadow_adam`, `ft_dead_mask`,
`l2_state_for_board`, committed `986a7cd`): within
`[diagnostic_shadow_trace_from_position, diagnostic_shadow_trace_until_position]`, every live training position
additionally computes CP-only and WDL-only gradients (each scaled by its own blend coefficient, `λ`/`1-λ`, so
`g_cp + g_wdl` is exactly the real blended gradient by construction — backprop is linear in the teacher's
contribution to `d_score`), applies one shadow Adam step to a *clone* of the pre-update FT+L2 state for each of
CP-only/WDL-only/Blended, and evaluates every clone's FT dead-unit outcome on the probe set. A fourth branch,
**linear prediction** (`linpred = anchor + Δθ_cp + Δθ_wdl`, plain vector addition, no Adam involved), serves as
"the world where Adam's own transform is linear" — the discriminator between within-step optimizer interaction
and everything else. The real backward pass and Adam update proceed completely unaffected; a dedicated test
(`shadow_trace_active_run_is_byte_identical_to_inactive`) asserts the real trained checkpoint is bit-identical
whether the shadow window is active or not, and `train_position` itself asserts the Blend branch's shadow clone
exactly reproduces the real applied update every single position (see Correctness guard below).

**Design**: same recipe as the prior B5 experiments (`--diagnostic-freeze-layer ft`, frozen `16-271`, reactivated
`144-175`, `--wdl-lambda 0.7`, `data/gateA_csa_subset`, `--shuffle-seed 11`), 3 seeds (42, 7, 123), shadow trace
active across the full `144-175` window, probe set the same fixed 261-position `l2probe.sfen`.

## Correctness guard: the shadow mechanism is verified sound, not just assumed

`blend_matches_real_ft`/`blend_matches_real_l2` — bit-exact equality between the Blend branch's shadow clone and
`self.weights` after the real update actually ran — holds at **all 32 positions, all 3 seeds (96/96)**.
`train_position` would have panicked immediately on any mismatch, so this isn't a post-hoc check that could have
silently passed on bad data. Combined with the non-perturbation test, this is the validation spine the rest of
this doc's numbers rest on.

**Precondition, verified not assumed**: the exact-linearity argument behind the whole design (`g_cp + g_wdl ==
g_blend`, and later, "any linpred-vs-blend gap is Adam-only, not FT-forward-pass nonlinearity") requires no
gradient clipping active. All 3 runs log `clipped=0/9723 (0.0%)` and `layer_clip: ft=0.0% l2=0.0% out=0.0%` —
confirmed off, not just expected to be off.

## Results

### `cos(g_cp, g_wdl) ≡ ±1` is a trivial identity, not a finding

Every recorded value is within floating-point tolerance of exactly `+1` or `-1` (e.g. `-0.999999999999965`).
This is expected, not novel: CP-only and WDL-only share the identical forward pass (same board, same
`active_us`/`active_them`, same `w.out`), and only the scalar residual (`score - eval_teacher` vs.
`score - wdl_target`) differs — so the entire multi-dimensional gradient vector is a scalar multiple of the same
underlying shape for both branches, at any single position. Reported for completeness, not interpreted further.

### The decisive result: Adam's own transform is sub-additive at single-step granularity — the reverse of what a "synergy" story would predict

Summed across all 32 positions, comparing the actual applied Blended step's newly-dead-unit count against the
linear-prediction branch's newly-dead-unit count (both measured from the identical anchor):

| seed | actual Blend newly-dead | linpred newly-dead | ratio (actual / linpred) |
|---|---|---|---|
| 42 | 61,998 | 120,190 | **0.516** |
| 7 | 69,834 | 134,162 | **0.521** |
| 123 | 66,449 | 128,483 | **0.517** |

The real Adam-applied Blended step kills roughly **half** as many FT units as the plain vector sum of the
independently-Adam-applied CP-only and WDL-only deltas would predict — consistently, to within 1%, across all 3
seeds. This is mechanistically explainable: Adam's `m̂/√v̂` normalization gives each branch's own delta close to
a *characteristic* step size regardless of that branch's raw gradient magnitude, so `Adam(g_cp) + Adam(g_wdl)`
(the linpred sum) can approach twice a single characteristic step, while `Adam(g_cp + g_wdl)` (the actual blended
step) is normalized as one combined gradient and stays closer to one characteristic step. Under a linear optimizer
(plain SGD) this ratio would be exactly `1.0` by construction — the gap is attributable entirely to Adam's own
nonlinear transform, not to the FT forward pass (which is affine in FT parameters, so the linpred null is exact
there) and not to trajectory divergence (both sides share the identical single-step anchor).

**This directly resolves the deferred question, and in the opposite direction from what a "blend is destructively
synergistic" story would need.** Within a single step, Adam's own nonlinearity is *protective*, not amplifying.
The deep 32-step Blended collapse (`43-48%` dead, `l2_b5_cp_wdl_component_replay.md`) cannot be explained as
within-step super-additive interaction, because the within-step Adam effect measured here runs the other way.
The 32-step collapse is cross-step accumulation, not a single-step interaction effect.

### The naive "neither component alone kills it, but Blend does" count is small and mostly linpred-explained too

For context, the raw contingency (dead/alive under CP-only × WDL-only × Blended, among probe-board/unit pairs
alive at the anchor, summed over all 32 positions):

| seed | alive/alive/**Blend-dead** (naive headline) | of which: linpred-alive too (genuine super-additive interaction) |
|---|---|---|
| 42 | 478 | 47 |
| 7 | 568 | 45 |
| 123 | 538 | 47 |

Even the naive "neither component alone would kill this unit, but Blend does" count is tiny relative to the
~3.1-3.3M alive-at-anchor pairs per seed, and cross-checking against the linpred branch shows most of it (~90%)
is explained by simple linear accumulation of the two components' deltas (the sum crosses the dead threshold even
without any optimizer nonlinearity) — only `45-47` pairs per seed are true super-additive Adam interaction, and
that count is dwarfed by the sub-additive effect above (which spans tens of thousands of pairs per seed, in the
opposite, protective direction).

### The interaction signal concentrates at the reactivation onset

`blend_dead_linpred_alive` (genuine super-additive interaction) is heavily concentrated at position 144 — the
first step after FT reactivates — in every seed: `16`/`17`/`25` at position 144, decaying to single digits by
147 and to zero by 148 in all 3 seeds. A small secondary bump appears around position `164-166` (`2-9` per seed)
in every seed — consistent in *location* across seeds but small in magnitude; reported as an observation, not
built into a structural claim.

### FT-vs-L2 inversion: not resolved by this trace, deferred with reason

The user's original request for this trace included probing `l2_b5_cp_wdl_component_replay.md`'s FT-vs-L2
inversion (WDL-only leaves the most FT units alive but wrecks L2 the most) via per-unit `cos(h, w_L2)`. This
trace only recorded L2 dead-fraction and mean weighted input per branch, not the requested per-unit alignment —
and those two fields move too little in a single step to be informative (L2 dead-fraction differs by a few
percentage points at most between CP/WDL/Blend at any given position, swamped by L2's own position-to-position
drift from `~0.5` down to `~0.15-0.3` over the window as reactivated FT starts feeding it fresh gradient). The
inversion is a property of the fully-trained 32-step endpoint state, not a single step from a shared anchor — it
needs to be probed on the trained checkpoints directly (FT-vector·L2-row dot products / `cos(h, w_L2)` on
`l2_b5_cp_wdl_component_replay.md`'s own WDL-only/CP-only/Blended checkpoints), not on this trace's one-step
branches. Left open.

## Conclusion

**The 32-step Blended collapse is cross-step accumulation, not within-step optimizer or network interaction.**
Measured from an identical shared anchor at every position (eliminating trajectory divergence by construction),
Adam's own nonlinear transform is *sub-additive* at single-step granularity — the actual blended step kills
roughly half as many FT units as the linear sum of the two components' individually-Adam-applied deltas would
predict, consistently across all 3 seeds (ratio `0.52`, ±1%). The super-additive direction (neither component
alone kills a unit, but Blend does, beyond what linear summation explains) is real but small (`45-47` pairs per
seed, concentrated almost entirely at the reactivation onset) and dwarfed by the dominant sub-additive effect.
Since FT's forward pass is affine in FT parameters, the linpred null is exact for FT, and the observed gap is
attributable to Adam's optimizer transform alone — not to network nonlinearity, which this design rules out for
the FT dead-set specifically.

This closes the open question from `l2_b5_cp_wdl_component_replay.md`: not "synergy confirmed," and not even
"inconclusive" — the within-step mechanism runs opposite to what a synergy story requires, so the deeper 32-step
collapse must come from accumulation across positions, not from a single step's optimizer or network behavior.

## Scope

All metrics are pooled/summed over the fixed 261-position probe set and over all 32 positions in `144-175`,
3 seeds. The linpred exactness argument holds only because no gradient clipping was active in any run (verified,
not assumed — see Correctness guard). L2-side metrics (dead fraction, weighted input) are observational only,
per `l2_b5_cp_wdl_component_replay.md`'s own note that L2's pre-activation is bilinear in FT-output × L2-weight,
so linear prediction is not an exact null there — no linpred branch was computed for L2. The FT-vs-L2 inversion
remains unresolved; see above.

## Status

- `compute_shadow_trace` / `shadow_component_grad` / `apply_shadow_adam` / `ft_dead_mask` / `l2_state_for_board`,
  `diagnostics::ShadowTraceRecord`, `--diagnostic-shadow-trace-from-position`,
  `--diagnostic-shadow-trace-until-position`, `--diagnostic-shadow-trace-probe-set`: committed (`986a7cd`).
  3 new unit tests, including the non-perturbation guard
  (`shadow_trace_active_run_is_byte_identical_to_inactive`) and a full-partition contingency check
  (`shadow_trace_records_pass_the_blend_correctness_guard_and_a_full_contingency`). 119/119 `sekirei-train` tests
  passing, `fmt`/`clippy` clean.
- 3 runs (32 positions × 3 seeds) complete, self-verified against requested config, correctness guard passing
  96/96 (32 positions × 3 seeds).
- Next: B5 can now be closed out as a documented pathology case (the causal chain is: CP pushes FT backward, WDL
  reverses sign, Blended's deep 32-step collapse is cross-step accumulation of the CP-dominant direction, not a
  single-step interaction effect). Lower-priority follow-up remains open: B4's seed-7-specific `T_any`
  divergence, and the FT-vs-L2 inversion (needs the trained-checkpoint alignment probe noted above, not this
  trace).
