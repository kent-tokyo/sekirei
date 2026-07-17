# Alignment formation trace: fast/slow FT-L2 co-movement diverges by position 8-16 — over 100 positions before saturation is visible — B's magnitude is near-maximal in both trajectories (only its sign differs), A's magnitude itself is the emergent signal

## Background

The state-swap probe (`l2_saturation_state_swap_probe.md`) established two prior facts this trace builds on
directly:

1. **Categorical fact**: only `FF` (fast FT × fast L2) ever saturates; `FS` and `SF` read `0.0%` at every one of
   24 tested seed×position combinations. Saturation needs fast's own co-evolved FT+L2 pairing — neither
   component's endpoint state alone, recombined with the other trajectory's counterpart, reproduces it.
2. **Mechanism, split by failure mode**: `SF` (slow FT × fast L2) fails on raw FT-output-norm insufficiency —
   slow-FT's `‖h‖` is ~10× too small to reach the saturation threshold even at `cos(θ)=1`, arithmetic, no
   alignment argument needed. `FS` (fast FT × slow L2) fails predominantly on alignment collapse — `cos(θ)` drops
   from `~0.90` (`FF`) to `~0.01-0.19` (`FS`) for the same neurons, a 10-20× collapse against only a ~2× norm
   drop.

That result compares two *endpoints* (position 320). This trace asks the temporal question directly: **when**,
and from **which side** (FT movement, L2-row movement, or their joint co-movement), does the fast trajectory's
alignment actually form — and does it precede or follow saturation, which prior work (P0a/P1) placed around
position 128-192?

## Method

New `crates/sekirei-core/examples/l2_alignment_formation_probe.rs`, extending `l2_delta_z_probe.rs`'s existing
`Δz = A + B + C + Δb` decomposition (`A = Δh·w_old`, `B = h_old·Δw`, `C = Δh·Δw`) with, per neuron per probe
position: `‖h_old‖`, `‖Δh‖`, `‖w_old‖`, `‖Δw‖`, `cos(Δh, w_old)` [direction `A` pushes in], `cos(h_old, Δw)`
[direction `B` pushes in], `cos(Δh, Δw)` [direction `C` pushes in], `cos(h_new, w_new)` [alignment *at* the new
checkpoint], and each term's fractional contribution to `Δz`.

**Verified before trusting**: the identity `Δz ≈ A + B + C + Δb` holds to `~3×10⁻⁴` (float precision only); `A`/`B`/
`C` match `l2_delta_z_probe.rs`'s already-verified `term1`/`term2`/`term3` for the same checkpoint pair to
`~3×10⁻⁵` (independent code path, same result). **Additional cross-tool check found in this pass**: `cos(h_new,
w_new)` at the final transition (`256→320`) matches `l2_state_swap_probe.rs`'s independently-computed `cos_FF` at
position 320 exactly to 4 decimals for all 3 seeds (`0.9037`/`0.9087`/`0.9101`) — two different tools, two
different runs, same quantity, same answer.

**Data**: 6 re-runs (fast=shuffle-11, slow=shuffle-33 × init seeds 42/7/123) with a denser `--trace-positions
8,16,24,32,48,64,80,96,112,128,160,192,224,256,320` (finer near the collapse window than the swap probe's
32-spaced points, so term ranking isn't a checkpoint-interval artifact) — 14 consecutive transitions per
trajectory per seed, same 261-position probe set, same self-verification. Neurons grouped 3 ways per seed via
`l2_saturation_probe.rs`: **fast-saturating** (>50% saturated by position 320 in the fast trajectory),
**fast-non-saturating** (neither saturated by 320 nor dead at 8), **initially-dead** (>90% dead by position 8,
excluded from the trace — they never receive meaningful gradient either way).

## Results

### 1. Fast-saturating neurons: `A` and `B` both diverge fast-vs-slow from the first transition, all 3 seeds

At `8→16`, the very first transition measured — over 100 positions before saturation is visible — fast and slow
already disagree in sign on both terms, for the neurons that will later saturate:

| seed | `A_fast` | `A_slow` | `B_fast` | `B_slow` | `cos(Δh,w_old)_fast` | `cos(Δh,w_old)_slow` | `cos(h_old,Δw)_fast` | `cos(h_old,Δw)_slow` |
|---|---|---|---|---|---|---|---|---|
| 42 | 0.915 | -0.144 | 2.067 | -0.567 | 0.241 | -0.036 | 0.954 | -0.945 |
| 7 | 0.983 | -0.122 | 1.806 | -0.581 | 0.282 | -0.030 | 0.848 | -0.842 |
| 123 | 0.915 | -0.009 | 1.956 | -0.416 | 0.245 | -0.003 | 0.950 | -0.864 |

`C` stays comparatively minor throughout the entire window in all 3 seeds (e.g. seed 42 peak `C_fast=4.198` at
`64→80`, versus `A_fast`/`B_fast` peaks of `11.2`/`13.8` in the same transition) — this rules out the
"joint/cross-movement-dominant" branch of the pre-registered decision table.

### 2. Which term diverges first: `B` leads by raw divergence and by cosine, in all 3 seeds — but this needs a caveat before it's read as "B causes A"

Measuring divergence directly (`|fast − slow|` at the first transition, and the first transition each cosine
crosses `0.5` in the fast trajectory):

| seed | `\|A_fast−A_slow\|` @ 8→16 | `\|B_fast−B_slow\|` @ 8→16 | ratio B/A | `cos_A` first > 0.5 | `cos_B` first > 0.5 |
|---|---|---|---|---|---|
| 42 | 1.060 | 2.634 | 2.49× | `48→64` | `8→16` |
| 7 | 1.105 | 2.386 | 2.16× | `48→64` | `8→16` |
| 123 | 0.924 | 2.371 | 2.57× | `48→64` | `8→16` |

By every measure, `B`'s fast/slow divergence is both larger and earlier than `A`'s, identically in all 3 seeds:
`B`'s cosine is already beyond `0.5` (in fact `0.84-0.95`) at the very first transition; `A`'s cosine doesn't
cross `0.5` until 4 transitions later (`48→64`, roughly position 48-64).

**This is not safe to read directly as "L2's update direction leads and FT follows."** The tell is in the *slow*
column, not just the fast one: `cos(h_old,Δw)` is already large in magnitude on **both** sides at `8→16` —
`+0.95`/`-0.94` (seed 42), `+0.85`/`-0.84` (seed 7), `+0.95`/`-0.86` (seed 123) — the L2 update is nearly
(anti-)parallel to the current FT output *regardless of whether the neuron goes on to saturate*. Only the *sign*
differs between fast and slow; the magnitude is close to maximal on both sides from the first transition on. A
quantity that's already near-extreme in both the trajectory that saturates and the one that doesn't isn't telling
us much by its size — only its sign carries the outcome-relevant information. (This is loosely consistent with
what a linear layer's `dL/dw_j = dL/dz_j · h` update would produce if the per-sample `dL/dz` sign inside the
window were fairly one-sided, but that's an aside, not the load-bearing argument — the window mixes many distinct
`h_t` across samples and Adam rescales per-coordinate, so the identity doesn't force this outcome on its own; it's
an empirical observation, not a proven mechanical necessity.)

`A`'s cosine shows the opposite signature, and that contrast is what marks it as the emergent term: in slow, it
stays small and flat throughout the whole window (`-0.12` to `+0.13`, never far from zero); in fast, it starts
small (`0.24-0.28` at `8→16`, comparable to slow's noise floor) and *grows*, reaching `0.79-0.84` by `128→160`
(all 3 seeds within `0.03` of each other at every transition). Unlike `B`, `A`'s magnitude itself is informative —
small-and-flat in the trajectory that stays healthy, small-then-large in the one that saturates. This is FT's
representation rotating, gradually, into alignment with L2's existing weight direction — real co-adaptation, not
a quantity that's already saturated at its ceiling regardless of outcome.

**Read together**: `B`'s sign commits essentially immediately (by position 16), but its magnitude was already
near-maximal on both sides and so isn't independent evidence of anything beyond that sign; `A`'s magnitude is
smaller at first, builds over roughly the next 100 positions, and is the piece that actually reflects FT-side
representational change. Neither is safely called the sole "leading" term — the honest summary is *reciprocal*,
term-magnitude-wise (both substantial, `C` minor), with `B`'s sign-commitment slightly leading `A`'s magnitude
build-up.

### 3. Temporal precedence: alignment divergence clearly precedes saturation, in all 3 seeds

Prior work (P0a, P1) placed `clamped_high` onset in the 128-192 position window. This trace's earliest measured
transition is `8→16` — and both `A` and `B` already disagree in sign between fast and slow at that point, in all
3 seeds. The pre-registered "cosine rises only after saturation → alignment is merely a consequence" branch is
therefore rejected: whatever sets the fast/slow difference is acting at least 100+ positions before saturation
becomes visible, not something that appears only once neurons start clamping. This is a real, falsifiable,
pre-registered check that came back negative — it does not by itself demonstrate that early alignment *causes*
the later collapse (see Conclusion, scope).

One shared, unexplained pattern worth flagging rather than silently smoothing over: in all 3 seeds, `A_fast` and
`cos(Δh,w_old)_fast` show a sharp transient sign reversal at the `192→224` transition (seed 42: `A_fast=-4.030`,
`cos=-0.821`; seed 7: `A_fast=-5.827`, `cos=-0.593`; seed 123: `A_fast=-7.261`, `cos=-0.743`), before recovering
to strongly positive by `224→256`. This falls entirely *after* the 128-192 saturation-onset window (i.e. after
many of these neurons are already clamped), so it doesn't bear on the precedence question above, but it's a
consistent 3-seed anomaly with no explanation yet — plausibly a post-saturation regime change (once `z` pins at
the ceiling, the specific feature-activation pattern behind `Δh` for these neurons may behave differently), not
investigated further here.

### 4. Common across all 3 seeds vs. seed-specific

**Common to all 3 seeds** (the load-bearing claims): fast/slow sign divergence in both `A` and `B` present by
`8→16`; `B`'s cosine `>0.84` immediately, `A`'s cosine crossing `0.5` at the identical transition (`48→64`) in
every seed; `C` minor throughout; the `192→224` transient reversal; `cos(h_new,w_new)` converging to `0.89-0.91`
(fast) by `256→320`, matching the independently-computed state-swap `cos_FF` to 4 decimals in every seed.

**Seed-specific**: neuron counts and identities differ (fast-saturating: 14 neurons in seed 42, 9 in seed 7, 12 in
seed 123 — expected, different init seeds produce different weight assignments). Seed 42 has **zero**
fast-non-saturating neurons (every neuron is either fast-saturating or dead by position 8) — the
fast-non-saturating contrast below is only testable in seeds 7 and 123. Absolute magnitudes of `A`/`B` in the
late window (`224→256`, `256→320`) vary more between seeds (e.g. seed 123's `A_fast=16.957` vs. seed 42's
`A_fast=10.382` at `256→320`) than the cosines do — read as aggregation/neuron-count noise on the raw dot
products, not a qualitative mechanism difference, since the normalized (cosine) picture is consistent to within
`0.03` at every transition across all 3 seeds.

### 5. Fast-non-saturating contrast (seeds 7, 123 only): sign, not magnitude, is what separates outcomes

At `8→16`, within the *same* fast trajectory: fast-saturating neurons show `cos(h_old,Δw)_fast ≈ +0.85` to
`+0.95`; the fast-non-saturating neurons in the same run show `cos(h_old,Δw)_fast ≈ -0.46` to `-0.94` — the
opposite sign, at the same early position, in the same trajectory, with the *same* near-maximal magnitude on both
sides. Seed 123's single fast-non-saturating neuron: `cos_B(fast)=+0.95` (saturating group) vs. `-0.94`
(non-saturating neuron) at `8→16`. This is the same "high magnitude on both sides, sign is what discriminates"
signature as the fast-vs-slow comparison in §2 — just applied per-neuron within one trajectory instead of across
trajectories. It confirms sign, not magnitude, is what separates outcomes at two different granularities
simultaneously: which neurons end up saturated is already legible by this sign at the very first transition.

## Conclusion

**FF-only-saturates fact (from state-swap, restated for context)**: saturation requires fast's own co-evolved
FT+L2 pairing; recombining either component with the other trajectory's counterpart eliminates it completely at
every tested checkpoint.

**SF/FS split (from state-swap, restated for context)**: `SF` fails on FT-output-norm insufficiency (arithmetic);
`FS` fails predominantly on alignment collapse (directional).

**Which term diverges first (this trace's new result)**: by both raw magnitude and cosine, `B` (`cos(h_old,Δw)`,
the L2-row-update direction) diverges from slow earlier and more sharply than `A` (`cos(Δh,w_old)`, the FT-output-
movement direction) — consistently across all 3 seeds (`B` crosses cosine `0.5` at the first transition measured,
`A` four transitions later). This is a solid, reproducible empirical ordering. **It is not safe to read as "L2
leads FT causally"**: `B`'s magnitude is already large on *both* the fast and slow side from the first transition
(`+0.95`/`-0.94`, `+0.85`/`-0.84`, `+0.95`/`-0.86` across the 3 seeds) — near-maximal regardless of whether the
neuron goes on to saturate, so only its sign carries outcome-relevant information, not its size. `A` shows the
opposite signature — small and flat in slow throughout the window, small-then-growing in fast (`0.24→0.83`) —
meaning `A`'s magnitude, not just its sign, is informative, and that pattern is what marks it as the emergent
term rather than a quantity already pinned near its ceiling on both sides. (A linear layer's `dL/dw_j=dL/dz_j·h`
update is loosely consistent with `B`'s near-maximal early magnitude but doesn't force it — the window mixes many
distinct `h_t` and Adam rescales per-coordinate — so this is offered as a consistent-with note, not the load-
bearing argument.) The fast-non-saturating contrast (§5) shows the identical "large-magnitude-both-sides,
sign-discriminates" signature *within* one trajectory, at the per-neuron level — the same fact recurring at two
different granularities. Read together with the term-magnitude comparison (both substantial, `C` minor), the fair
summary is **reciprocal co-movement with an early sign-commitment on the L2 side (already near its ceiling in
both outcomes) and a slower, more diagnostically meaningful directional build-up on the FT side** — not a clean
single-term-dominant story.

**Precedes or follows saturation**: precedes, clearly and consistently. Divergence in both terms is present at
the earliest transition measured (`8→16`), more than 100 positions before the previously-established 128-192
saturation-onset window, in all 3 seeds. This rejects the pre-registered "alignment is a downstream consequence
of saturation" branch. It adds temporal-formation detail to the state-swap result; it does not itself add a new
causal proof — the freeze diagnostic and state-swap probe already carry the causal load (necessity and
endpoint-sufficiency respectively).

**Common vs. seed-specific**: the qualitative pattern (early sign divergence in both terms, `B` leading `A` by
the same 4-transition gap, `C` minor, the `192→224` transient reversal, converged endpoint alignment `≈0.90`) is
identical across all 3 seeds. Seed-specific variation is limited to neuron counts/identities and raw-magnitude
scale in the late window — not to the qualitative timing or sign story. Seed 42's complete absence of
fast-non-saturating neurons means the sign-contrast check in §5 is only 2-seed-confirmed, not 3-seed.

**Does this justify the next causal experiment**: yes, two different next steps follow, and they answer different
questions — worth keeping separate rather than collapsing into one:

1. **A genuine causal test, buildable now from existing infrastructure**: `A`'s gradual `8→16`-through-`128→160`
   build-up is what this trace identifies as the emergent, diagnostically meaningful term — that specifically
   motivates a **windowed FT-freeze**, using the freeze diagnostic's existing `--diagnostic-freeze-layer ft
   --diagnostic-freeze-until-position <n>` flags, but bounded to roughly the `16-128` window (freeze FT
   *specifically* during the window this trace shows `A` accumulating, then release) rather than the freeze
   diagnostic's original 1-320 freeze-then-release design. If suppressing FT movement specifically during that
   window prevents saturation as effectively as the original full-window freeze did, that's direct causal
   evidence the `A`-buildup window is where the mechanism actually operates, not just correlated with it. This is
   answerable without touching game content at all.
2. **The deferred, correlational step**: attributing *which* game-order difference between shuffle-11 and
   shuffle-33 produced the early (position ≤16) sign-commitment in `B`. This is narrower than a blind 0-320
   search — the window this trace identifies is roughly positions 1-16 — but it's still correlational
   game-content attribution, not a causal test, and per the user's own explicit sequencing
   ("まだゲーム内容には降りない") it stays deferred, not run here. It's also worth noting the current dense-trace
   floor is position 8: this data doesn't establish whether the divergence is present even earlier (positions
   1-8); a tighter trace at the very start of the window would be needed before treating "position 1-16" as a
   confirmed lower bound rather than "no later than 16."

## Status

- `l2_alignment_formation_probe.rs`: implemented, verified (identity check, cross-check against
  `l2_delta_z_probe.rs`'s term1/term2/term3, and a new independent cross-tool match against `l2_state_swap_probe.
  rs`'s `cos_FF` at position 320), `fmt`/`clippy` clean. **Uncommitted.**
- 6 dense re-runs (`--trace-positions 8,16,24,32,48,64,80,96,112,128,160,192,224,256,320`, `--trace-weights`) ×
  14 transitions each, complete. Raw data and scripts in scratch (`p1_shuffle_exp/`: `align_seed{42,7,123}_
  {shuf11,shuf33}_*.jsonl`, `analyze_alignment_formation.py`, `classify_seed{42,7,123}_pos{8,320}.txt`).
- Game-content attribution for the position-1-16 divergence window: not started, next per the user's own plan —
  narrow to that window specifically, not a broad search.
