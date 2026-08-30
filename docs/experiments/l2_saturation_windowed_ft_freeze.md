# Windowed FT-freeze (position 16-128): FT movement is causally necessary for saturation — but the missing ingredient is FT-output *norm* growth, not directional alignment

## Background

The alignment-formation trace (`l2_alignment_formation_trace.md`) found `A` (`cos(Δh,w_old)`, FT-side directional
movement) building gradually over roughly positions 16-128 in the fast (saturating) trajectory while staying flat
in the slow (non-saturating) one, and proposed a direct causal test: freeze FT specifically during that window and
see whether saturation is suppressed. This experiment runs that test using the existing freeze diagnostic
(`l2_saturation_freeze_diagnostic.md`), extended with a new `--diagnostic-freeze-from-position` flag so the freeze
can be a closed window (`16-128`) rather than only "from position 1 until N."

**Design**: 3 init seeds (42/7/123), `--shuffle-seed 11` (the "fast" trajectory characterized in prior work) for
both arms. Control: normal training. Candidate: `--diagnostic-freeze-layer ft --diagnostic-freeze-from-position 16
--diagnostic-freeze-until-position 128` — FT's own Adam update is skipped for positions 16-128 inclusive; the
ordinary backward pass still computes gradient through FT's fixed weights, so L2 and Out keep receiving and
applying real gradient (not stop-gradient, same contract as the original freeze diagnostic). Both arms use
`--sample-grad-trace 512` (per-position `l2_gate`/`l2_grad_vector`, for the `clamped_high` band classification)
and `--trace-weights --trace-positions 8,16,24,32,48,64,80,96,112,128,160,192,224,256,320,384,448,512` (weight
snapshots, for the 261-position checkpoint-based saturation probe and the A/B/C alignment-formation decomposition).

## Implementation and verification

`trainer.rs`: added `diagnostic_freeze_from_position: u64` (default `0`, so freezing "from the start" — the
original behavior — is exactly `from_position=0`, byte-identical when the flag is omitted). The freeze gate is now
`from_position <= l2_sample_count <= until_position`. Wired through `main.rs` as `--diagnostic-freeze-from-position
<n>`, mirroring the existing `--diagnostic-freeze-until-position` flag's exact pattern (Args field, parsing arm,
metadata, help text, both trainer-construction call sites).

**Verified before trusting**: 2 new unit tests — `diagnostic_freeze_from_position_unset_is_byte_identical_to_no_
freeze` (mirrors the existing "unset" no-op test) and `diagnostic_freeze_window_only_freezes_between_from_and_
until_positions` (proves position 1 updates normally, positions 2-3 inside `[from=2,until=3]` stay frozen, position
4 resumes) — both pass, alongside all 102 pre-existing tests (104 total). `fmt`/`clippy` clean. **Additional
end-to-end check, not just unit tests**: ran one real training smoke-test with `--diagnostic-freeze-from-position
16 --diagnostic-freeze-until-position 128`, then diffed the raw FT weight matrices between weight-snapshot
checkpoints directly (independent of the trainer's own self-reported diagnostics) — FT is confirmed byte-identical
between every pair of checkpoints strictly inside `[16,128]` (pos16 vs pos32, pos16 vs pos128), confirmed
*different* between pos8 and pos16 (only pos16 itself is frozen; positions 9-15 update normally) and between
pos128 and pos160 (release at position 129), while L2 and Out differ at every checkpoint pair throughout —
confirming the window boundaries are exact and gradient still flows to the unfrozen layers.

## Results

### 1. `clamped_high` stays at exactly 0.0% through position 256 — over 100 positions past the freeze's own release at 129

Per-position `l2_gate` classification (`--sample-grad-trace`, same method and fixed bands as the original freeze
diagnostic):

| seed | band | control `clamped_high` | frozen `clamped_high` |
|---|---|---|---|
| 42 | 129-192 | 31.4% | **0.0%** |
| 42 | 193-256 | 39.7% | **0.0%** |
| 42 | 257-320 | 42.4% | 10.3% |
| 42 | 321-384 | 43.8% | 50.0% |
| 7 | 129-192 | 17.2% | **0.0%** |
| 7 | 193-256 | 16.1% | **0.0%** |
| 7 | 257-320 | 25.5% | 5.0% |
| 7 | 321-384 | 28.1% | 31.2% |
| 123 | 129-192 | 25.0% | **0.0%** |
| 123 | 193-256 | 26.5% | **0.0%** |
| 123 | 257-320 | 35.1% | 3.9% |
| 123 | 321-384 | 37.5% | 40.6% |

Cross-checked independently with the checkpoint-based 261-position saturation probe (`l2_saturation_probe.rs`,
same tool used throughout this investigation): saturated fraction reads **exactly 0.0% at every checkpoint from
position 16 through 256** in all 3 seeds, then jumps sharply between 256 and 320 (seed 42: 0.0%→44.8%; seed 7:
0.0%→27.7%; seed 123: 0.0%→35.6%), converging by 384-512 to levels matching or very slightly exceeding control's
own terminal saturation (seed 42: 47.1% vs control 43.7%; seed 7: 29.6% vs 27.9%; seed 123: 37.9% vs 37.5%). Both
measurement methods (per-training-position `l2_gate` and per-checkpoint fixed-probe-set classification) agree.

**This answers the primary pre-registered question directly: yes, freezing FT during 16-128 stops the 128-192
`clamped_high` surge.** But the suppression window is *longer* than the freeze itself — it holds through position
256, roughly 127 positions past the release at 129, not just through the frozen window.

### 2. Relapse bracket: onset is in (256, 320], not the pre-registered 64-128-after-release window

Position 129 is release; saturated fraction is still exactly 0.0% at 256 (127 positions after release) and has
already reached double digits by 320 (191 positions after release). The relapse onset is therefore in `(256,
320]` — later than the pre-registered "64-128 positions after release" guess, not earlier. Stated precisely rather
than rounded to fit the pre-registration. The trace positions used here don't include `288`, so this bracket
can't be narrowed further without an extra re-run (~17s × 6, cheap, not done since no conclusion depends on
tightening it past "127-191 positions after release").

Also observed, not interpreted: frozen's terminal saturation (`47.1%`/`29.6%`/`37.9%`) is slightly but
consistently higher than control's own (`43.7%`/`27.9%`/`37.5%`) in all 3 seeds — small and same-signed, reported
as-is rather than explained.

### 3. Mechanism, corrected: the freeze blocks FT-output *norm* growth, not directional alignment — alignment forms fine without saturating

The first read of the A/B/C decomposition leaned on `A`'s suppression (exactly `0.000` during the freeze, by
construction: `Δh=0` when FT is frozen forces `A=Δh·w_old=0` and `C=Δh·Δw=0` identically) as "alignment formation
is blocked." **That's not what the data shows, and it matters that it's wrong**: `cos(h_new,w_new)` — the actual
directional alignment between FT's output and L2's weight column — still rises substantially during the freeze,
driven by `B` alone (`h_old·Δw`, L2's own update against a temporarily-fixed `h`):

| seed | `cos(h,w)` @ pos32 (frozen) | @ pos64 (frozen) | @ pos128 (frozen, end of window) | control @ pos128 |
|---|---|---|---|---|
| 42 | 0.496 | 0.670 | **0.818** | 0.866 |
| 7 | 0.453 | 0.642 | **0.820** | 0.869 |
| 123 | 0.477 | 0.657 | **0.810** | 0.867 |

Alignment reaches `~0.81-0.82` under freeze — only modestly behind control's `~0.87` — yet saturated fraction is
exactly `0.0%` at every one of these checkpoints. This is the actual finding the pre-registered decision table
couldn't distinguish in advance: it only had a single "`A` (and saturation) stop" branch, treating `A`'s magnitude
and alignment-formation as one thing. The freeze separates them cleanly — `A` is forced to exactly `0` by
construction (`Δh=0`), but `cos(h,w)` climbs to `0.82` regardless via `B` alone, and saturation is still fully
suppressed. **If alignment this high produces no saturation, alignment is not the blocked ingredient here.** What
the freeze actually pins is `‖h‖`, FT's own output norm, at its pre-freeze value:

| seed | `‖h‖` @ position 128, control | `‖h‖` @ position 128, frozen | ratio | `‖w‖` (saturating neurons, control) | `‖w‖` (frozen) | max achievable `z` (`cos=1`), control | frozen |
|---|---|---|---|---|---|---|---|
| 42 | 49.18 | 13.48 | 3.65× | 2.806 | 2.620 | 138.0 | **35.3** |
| 7 | 48.16 | 13.00 | 3.71× | 2.820 | 2.614 | 135.8 | **34.0** |
| 123 | 49.30 | 13.10 | 3.76× | 2.802 | 2.561 | 138.1 | **33.6** |

`‖w‖` (the L2 weight column norm, for control's saturating neurons) is nearly unaffected (within ~7%) since L2 is
never frozen. `‖h‖` differs by `~3.7×`. The maximum achievable `z` even at perfect alignment (`cos=1`) comes to
only `33.6-35.3` under freeze — far short of the `127` saturation threshold — while control's own max-achievable
`z` (`135.8-138.1`) is already past it. **This is arithmetically the same signature the state-swap probe found
for its `SF` combo (slow-FT × fast-L2): saturation fails on raw norm insufficiency, not alignment.** The freeze
experiment reproduces that finding as a controlled causal intervention rather than an endpoint counterfactual: FT
movement's causal contribution to saturation is predominantly through growing `‖h‖`, not through rotating into
better alignment with L2's weights (which `B` alone accomplishes regardless of whether FT moves at all).

### 4. `B`'s sign/cosine commitment persists essentially unchanged through the freeze — it doesn't need reciprocal FT movement

`cos(h_old,Δw)` (`B`'s direction) stays high and positive throughout the frozen window, close to control's own
values despite FT not moving at all: seed 42 at `64→128`: control `0.943`, frozen `0.938`; seed 7: control `0.926`,
frozen `0.934`; seed 123: control `0.934`, frozen `0.930`. `B`'s raw magnitude is reduced (frozen `~11.6` vs
control's `~30.6` at `64→128`) — consistent with the norm-starvation reading: `B=h_old·Δw`, and `h_old` is
smaller under freeze, so the same directional update produces a smaller dot product — but the *direction itself*
is essentially untouched by whether FT is moving. This corroborates and sharpens the alignment-formation trace's
own reading of `B` as largely a property of L2's own gradient (structurally tied to whichever `h` it currently
sees), not a signal that depends on FT co-moving.

(Note: frozen and control end up with different dead-neuron sets by position 320, since the frozen run's own
trajectory diverges — every A/B/C comparison in this doc is paired on *control's* saturating-neuron set specifically,
not each arm's own, so the comparison stays apples-to-apples.)

### 5. Relapse mechanism: `A` and `‖h‖` both resume strongly at release, with `A` overshooting control before saturation catches up

Once FT resumes updating (position 129+), `A` and `‖h‖` both recover, and `A` doesn't just resume at control's
pace — it overshoots it. At `256→320` (exactly the transition where saturated fraction jumps from 0% to
double-digits): `A_frozen = 55.3-58.6` vs `A_control = 10.4-17.0`, roughly `3-5×` larger. `‖h_old‖` at the same
point: `21.5-23.2` (frozen) vs `55.5-57.0` (control) — frozen's `‖h‖` is still substantially behind control's even
at this point, but growing fast (`‖Δh‖` at this single transition: `24.0-25.0`, more than doubling `‖h_old‖` in
one step) — the combination of a large, still-recovering-but-rapidly-growing `‖h‖`, an already-large `‖w‖` (L2
never stopped growing), and the persistently high `cos(h,w)` established during the freeze is what crosses the
threshold. One shared, unexplained transient also reproduces here, shifted earlier by roughly one transition
compared to the un-frozen trajectory: `A_frozen` swings sharply negative at `160→192` (`-33.7` to `-35.3`, deeper
than control's own `160→192`/`192→224` swings) before recovering — consistent with the same kind of anomaly
flagged in the alignment-formation trace, evidently shifted in time by the freeze-induced delay, not investigated
further here.

## Decision-table mapping, kept time-keyed rather than forcing one branch

The pre-registered branches describe different things depending on which window they're read against — reporting
both rather than picking one:

- **During the frozen window (positions 16-256, i.e. including ~127 positions past release)**: closest to
  "`A`(and saturation) stop while `B`'s sign persists" — except the mechanism is now known to be norm-starvation
  of `‖h‖`, not a blocked alignment-formation process; alignment (`cos(h,w)`) actually forms substantially (`~0.82`)
  under freeze, it just isn't sufficient without the accompanying norm.
- **After the window (positions 256-512)**: matches "freeze doesn't remove the cause, only delays it" — full
  relapse, terminal saturation matching or slightly exceeding control, same qualitative outcome as the original
  1-320 full-window freeze diagnostic.
- **Ruled out**: "saturates on schedule regardless of the freeze" (saturation is clearly suppressed for over 100
  positions) and "alignment formation itself is blocked" (alignment reaches `~0.82`, not blocked, just insufficient
  paired with a starved `‖h‖`).

## Relation to the alignment-formation trace's own framing

This refines, not contradicts, `l2_alignment_formation_trace.md`. That trace correctly identified `A`'s *magnitude*
as the emergent, diagnostically meaningful signal (vs. `B`'s magnitude, discounted as structural) — this
experiment doesn't change that reading of the *correlational* trace. What it adds is the *causal* attribution for
*why* FT movement matters: not because FT's movement is what completes a mutual alignment loop (alignment forms
fine from `B` alone), but because FT's movement is what grows `‖h‖` — the same norm-growth ingredient the
state-swap probe's `SF` failure mode already identified as necessary. Read together with state-swap: saturation
needs both `‖h‖` growth and alignment, and this experiment shows those two ingredients are separable by direct
intervention, not just by endpoint comparison — freezing FT removes the norm-growth ingredient specifically while
leaving alignment formation intact, and that alone is sufficient to suppress saturation.

## Scope

3 seeds, one window (`16-128`), one trajectory (`shuffle-11`, "fast"). Not yet tested: whether an even narrower
window (`16-64` or `64-128`) reproduces the same suppression, which would localize the necessary norm-growth
period further — this is the natural next step, and was pre-registered as conditional on `16-128` showing a clear
effect, which it did.

**Necessity claim, stated precisely**: this experiment demonstrates `‖h‖`-growth is *necessary* — removing it
(via freeze) suppressed saturation. It does **not** test alignment's necessity: alignment was never removed here
— it reached `~0.82` in both the frozen (non-saturating) and control (saturating) conditions, so within this
experiment alone, alignment doesn't discriminate the outcome at all. Alignment's own necessity rests on the
state-swap probe's `FS` result separately, not on this experiment. The accurate combined statement is "both
ingredients are necessary, and this experiment shows they're separable by direct intervention" — not "alignment
is dispensable." Worth noting the asymmetry: this freeze is a *cleaner* single-variable test for norm than
state-swap's `FS` was for alignment — `‖w‖` here moved only ~7% (L2 was never frozen), whereas `FS` itself carried
a ~2× `‖w‖` drop alongside its alignment collapse, so `FS`'s "predominantly alignment" reading always had a minor
norm confound. Norm-necessity is, by this measure, the better-isolated of the two findings.

## Status

- `trainer.rs`/`main.rs`: `--diagnostic-freeze-from-position` implemented, tested (104/104 unit tests pass),
  `fmt`/`clippy` clean. **Uncommitted.**
- 6 runs (control/frozen × 3 seeds), self-verified metadata, complete. Raw data and scripts in scratch
  (`ft_window_freeze_exp/`: `run.sh`, `verify_meta.py`, `classify_freeze_arm.py`, `analyze_freeze_effect.py`,
  `satprobe_out/`, `align_out/`).
