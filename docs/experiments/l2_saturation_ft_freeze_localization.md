# FT-freeze localization (16-64 vs 64-128): saturation onset tracks the freeze's *release point*, not which half was frozen — the two halves are non-additive, ruling out a cumulative-dose story

## Background

The windowed FT-freeze result (`l2_saturation_windowed_ft_freeze.md`) showed freezing FT for the full `16-128`
window suppresses saturation through position 256 and relapses by 320, with the missing ingredient identified as
`‖h‖` (FT-output norm) growth, not alignment. That leaves open which part of `16-128` is doing the work: early
direction-setting, late amplification, distributed/cumulative growth across the whole interval, or neither half
alone. This experiment splits the window in half and tests each independently.

**Design**: 3 arms (Control: no freeze; Early: `--diagnostic-freeze-layer ft --diagnostic-freeze-from-position 16
--diagnostic-freeze-until-position 64`; Late: same flags with `64`/`128`), 3 seeds (42/7/123), `--shuffle-seed 11`,
everything else identical to the prior experiment. `--sample-grad-trace 512` and `--trace-weights --trace-positions
0,16,32,48,64,80,96,112,128,160,192,224,256,320,384,512` (position `0` is a documented no-op, matches prior
experiments' behavior). 9 runs total, self-verified metadata against requested config before use.

## Results

### 1. `‖h‖` at position 128 is again the discriminator; `‖w‖` and `cos(h,w)` stay roughly matched across arms

| seed | `‖h‖`@128 control | early | late | `‖w‖`@128 control | early | late | `cos(h,w)`@128 control | early | late |
|---|---|---|---|---|---|---|---|---|---|
| 42 | 49.18 | 35.98 | 29.10 | 2.806 | 2.792 | 2.766 | 0.866 | 0.863 | 0.851 |
| 7 | 48.16 | 34.95 | 27.17 | 2.820 | 2.774 | 2.723 | 0.869 | 0.864 | 0.846 |
| 123 | 49.30 | 35.24 | 28.17 | 2.802 | 2.738 | 2.707 | 0.867 | 0.858 | 0.845 |

Reconfirms the mechanism from the full-window experiment: `‖w‖` and `cos(h,w)` are nearly identical across all 3
arms (within ~7% and ~0.02 respectively) while `‖h‖` differs substantially (control's is `1.4-1.7×` larger than
either half-freeze arm's) — norm-starvation, not alignment, remains the discriminator. (The `A+B+C+Δb ≈ Δz`
identity underlying `cos(h,w)`'s definition was independently cross-checked against `l2_delta_z_probe.rs` in the
alignment-formation trace; re-confirmed here to `~6×10⁻⁵` on this run's own data, not re-derived from scratch.)

### 2. Saturated fraction and linear-neuron count at 128/256/320

| seed | arm | sat%@128 | sat%@256 | sat%@320 | linear%@128 | linear%@256 | linear%@320 |
|---|---|---|---|---|---|---|---|
| 42 | control | 13.3% | 40.9% | 43.7% | 30.4% | 2.8% | 0.1% |
| 42 | early (16-64) | 0.0% | 8.1% | 45.6% | 50.0% | 41.9% | 4.4% |
| 42 | late (64-128) | 0.0% | 0.3% | 40.8% | 43.8% | 43.6% | 3.0% |
| 7 | control | 8.6% | 23.2% | 27.6% | 19.6% | 5.0% | 0.5% |
| 7 | early (16-64) | 0.0% | 4.9% | 28.7% | 31.2% | 26.4% | 2.6% |
| 7 | late (64-128) | 0.0% | 0.0% | 26.2% | 28.1% | 28.1% | 1.9% |
| 123 | control | 11.4% | 34.7% | 37.5% | 26.1% | 2.8% | 0.0% |
| 123 | early (16-64) | 0.0% | 5.2% | 36.8% | 40.6% | 35.4% | 3.8% |
| 123 | late (64-128) | 0.0% | 0.0% | 37.5% | 40.6% | 37.5% | 2.5% |

Both half-window freezes suppress saturation to `0.0%` at position 128, matching the full-window result. **Late**
stays at essentially `0.0%` through position 256 too (matching the full-window experiment's suppression duration
almost exactly); **Early** already shows small but nonzero saturation by 256 (`4.9-8.1%`). Both relapse to
control-comparable or slightly-exceeding levels by 320. `clamped_high` bands (`--sample-grad-trace`, same method
as prior experiments) and the checkpoint-based `l2_saturation_probe` agree throughout (not tabulated separately
here — same qualitative and quantitative picture as this table).

### 3. The key check: aligning onset to each arm's own *release point* collapses the apparent Early/Late difference

Read against absolute position, Late looks like the "stronger" freeze (0% through 256 vs Early's 256 already
showing leakage). But Early and Late release FT at different absolute positions (`65` and `129` respectively), so
comparing them at a fixed absolute checkpoint compares different points in each arm's own recovery trajectory.
Re-reading onset relative to each arm's *own release position*:

| arm | release position | last-observed 0%-saturated checkpoint | first-observed nonzero checkpoint | positions-after-release bracket |
|---|---|---|---|---|
| Early (16-64) | 65 | 192 (0.0%, all 3 seeds) | 256 (4.9-8.1%) | `192-65=127` to `256-65=191` |
| Late (64-128) | 129 | 256 (0.0-0.3%, all 3 seeds) | 320 (26.2-40.8%) | `256-129=127` to `320-129=191` |
| Full 16-128 (prior experiment) | 129 | 256 (0.0%, all 3 seeds) | 320 (27.7-44.8%) | `256-129=127` to `320-129=191` |

**All three arms bracket to the identical `127-191` positions-after-release window**, despite freezing different
absolute windows, different durations (`48` positions for each half vs `112` for the full window), and releasing
at different points. Read at the resolution these coarse checkpoints (no `288` snapshot) allow, saturation onset
tracks *when FT resumes moving*, not which specific sub-window was frozen or for how long.

### 4. Non-additivity: Late alone reproduces the full-window result almost exactly; Early alone gets most of the way there too

If the mechanism were cumulative dose (norm growth accumulating in proportion to how much of `16-128` is frozen),
the `112`-position full freeze should delay onset roughly twice as long as either `48`-position half. It doesn't:
**Late alone (`0.0%` through 256, only `48` frozen positions) reproduces the full `16-128` freeze's suppression
duration almost exactly** (same `127-191`-after-release bracket), and **Early alone gets most of the way there
too** (suppressed through `192`, only modest leakage by `256`). The two halves are not additive — freezing either
one alone captures nearly all of the delaying effect the full window produces. This rules out simple cumulative
dose as the story.

### 5. One position-locked, unexplained anomaly — flagged, not chased

`‖h‖` dips at the same *absolute* position band (`192-224`→`224-256`) in both Early and Late, despite those bands
landing at very different points in each arm's own post-release recovery (Early: `127-191` positions post-release;
Late: `63-95` positions post-release). A release-timing story alone would not obviously produce a dip locked to
the same absolute position regardless of release point — this looks more consistent with something in that
specific stretch of training data (game/position content) than with the freeze mechanism itself, but this wasn't
investigated further here (per the standing "don't drill into game content until the window is settled" discipline
carried over from the alignment-formation trace).

## Decision-table mapping: none of the 4 pre-registered branches fit cleanly — stated explicitly rather than forced

- **"16-64 alone delays substantially (early direction-setting matters)"**: technically true (Early does delay
  substantially) but the *reason* isn't "early direction-setting is special" — see non-additivity above.
- **"64-128 alone delays substantially (late amplification matters)"**: also technically true (Late reproduces the
  full effect) but for the same reason as above, not because the late window carries unique causal weight.
- **"Both partially delay — distributed/cumulative growth matters"**: this is the branch the data actively
  contradicts — cumulative dose predicts Full ≈ 2× either half; instead Late ≈ Full and Early gets most of the way
  there too. Non-additive, not cumulative.
- **"Neither alone works — only the full 1-320-equivalent freeze matters"**: clearly false, both halves work
  strongly alone.

**The pattern the pre-registration didn't anticipate**: saturation onset ≈ release position + a roughly constant
(`~127-191` position) recovery interval, regardless of which sub-window was frozen or how long it was frozen for.
Freezing FT doesn't remove a specific causal trigger localized to either half of `16-128` — it pauses a resumable
`‖h‖`-growth process, and what matters is when that process resumes, not which particular positions were skipped.
Stated at the confidence 3 coarse-checkpoint arms actually support, not as a precise law: this is a triangulation
across 3 arms landing on the same bracket, not a proof that the recovery interval is exactly constant for any
window/duration.

**One more precision, worth stating explicitly rather than leaving implicit**: saturation onset more likely
corresponds not to the *release event itself* but to the point at which FT's trajectory has accumulated a certain
amount of progress *after* resuming updates — release is necessary (nothing accumulates while frozen) but the
current data doesn't distinguish two different clocks that would both produce the observed `127-191` bracket:
(1) a **sample-count clock** — literally `~127-191` positions of elapsed training, regardless of how much FT
actually moved in that span, or (2) a **learning-progress clock** — a roughly fixed amount of cumulative `‖h‖`
growth or FT-parameter movement, which happens to take `~127-191` positions under this recipe's learning rate and
data but would take a different number of positions under a different LR or a different game-order density of
"useful" gradient. This experiment's arms don't separate these two clocks (all three ran under the same LR/data/
order), so both remain live explanations — this is exactly the question the next re-coordination pass (re-analyzing
these same 9 runs on release-relative *and* trajectory-relative axes) is aimed at resolving.

**Summary of the corrections and caveats worth keeping visible together**:
- absolute-position comparison across arms is confounded by each arm's own release timing — release-relative
  comparison is required, and changes the reading (Late no longer looks intrinsically "stronger" than Early).
- release-relative, all 3 arms (Early, Late, and the prior full `16-128` freeze) land in the same `127-191`-
  positions-after-release relapse window.
- the delay is not additive with respect to freeze duration (Late alone ≈ Full; Early alone gets most of the way
  there) — ruling out a simple cumulative-dose story.
- neither `16-64` nor `64-128` individually is supported as a distinct, uniquely-necessary critical window — both
  work, for what looks like the same underlying reason (release timing), not because either sub-window carries
  special causal content.
- saturation is not eliminated by any of these freezes, only deferred to after FT resumes updating — consistent
  with the full-window result.
- the shared `‖h‖` dip around position `192-256` (same *absolute* band in both Early and Late) may be a separate
  data-band effect (something about that stretch of training positions/games) rather than a freeze-mechanism
  effect, but this is unverified, not investigated further here.

## Scope

3 seeds, 2 half-windows plus the prior full-window result, one trajectory (`shuffle-11`). Onset brackets are only
as tight as the available checkpoints (`192/256/320`, no `288`) — the `127-191` bracket is a resolution limit, not
a measured precise value. Not tested: whether an even shorter freeze (e.g. `16` positions) still reproduces
close to the same delay, which would further test the "release-point, not duration" reading; whether releasing at
the *same* absolute position with *different* freeze durations changes the recovery interval (this experiment
varied duration and release position together across arms, not independently).

## Status

- 9 runs (control/early/late × 3 seeds), self-verified metadata, complete. Raw data and scripts in scratch
  (`ft_localize_exp/`: `run.sh`, `verify_meta.py`, `classify_freeze_arm.py`, `analyze_localize.py`,
  `satprobe_out/`, `align_out/`).
- No code changes this experiment — reuses the `--diagnostic-freeze-from-position`/`--diagnostic-freeze-until-
  position` flags already committed.
