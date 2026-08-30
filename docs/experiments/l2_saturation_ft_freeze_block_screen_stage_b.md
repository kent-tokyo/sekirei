# Stage B (3 seeds): B2 and B7 replicate cleanly; B4 replicates only on propagation; B5 is a genuine anti-growth candidate

## Background

Stage A (`l2_saturation_ft_freeze_block_screen.md`, 1 seed) identified B2 as the strongest necessity-candidate
block, B7 as the strongest sufficiency-candidate block, and B4 as a candidate for dissociating local firing from
probe-wide propagation. Stage B confirms these at 3 seeds (`42`/`7`/`123`), plus an added `B5` negative-control arm
and a new `all-frozen` sufficiency baseline (freeze the entire `16-271` window, zero active positions — never run
before this experiment).

**Design**: same 8-block layout as Stage A. Necessity series = freeze one block only (`224` active positions
elsewhere); baseline = control (no freeze at all). Sufficiency series = reactivate one block inside an otherwise
fully-frozen `16-271` window (`32` active positions); baseline = `all-frozen` (`0` active positions in-window).
`4` necessity arms (control/B2/B4/B5/B7 — control reused from `ft_intermittent_exp/`, metadata-verified exact
match) × `4` sufficiency arms (all-frozen/B2/B4/B5/B7) × `3` seeds; `9` of `24` unique (arm, seed) cells already
existed from Stage A (seed `123`) or via control reuse, `19` new runs. 108/108+ `sekirei-train` tests unaffected
(no new code this stage), block placement empirically validated using the same chained-`‖Δθ_FT‖` method as Stage A.

## Results

### Necessity series (vs. control)

| arm | seed 42 | seed 7 | seed 123 |
|---|---|---|---|
| control `T_any` / `T_probe_25` | 109 / 144 | 114 / 288 | 114 / 144 |
| **B2** `T_any` / `T_probe_25` | **161** / 288 | **161** / 320 | **161** / 304 |
| B4 `T_any` / `T_probe_25` | 109 / 256 | **161** / 304 | 119 / 288 |
| B5 `T_any` / `T_probe_25` | 109 / 144 | 114 / 288 | 114 / 144 |
| B7 `T_any` / `T_probe_25` | 109 / 144 | 114 / 304 | 114 / 144 |

- **B2 replicates cleanly**: `T_any=161` in all 3 seeds (vs. control's `109-114`), `T_probe_25` delayed in all 3
  (`288-320` vs. control's `144-288`). Both the local-firing delay and the propagation delay hold at 3/3 seeds.
- **B4 replicates only on propagation**: `T_probe_25` is delayed in all 3 seeds (`256-304`), consistently in the
  same direction as B2. `T_any`, predicted to stay near control, does so in `2/3` seeds (`109`, `119`) but jumps
  to `161` (matching B2's own value) in seed `7`. **Not smoothed over — this is a `2/3` seed replication of the
  "local firing preserved" half of the B4 hypothesis, not a clean `3/3`.**
- **B5 replicates as a clean negative control**: matches control almost to the digit in all 3 seeds, including
  reproducing control's own seed-specific noise pattern (e.g. seed `7`'s `T_probe_25=288` outlier value appears
  identically in B5's own seed `7` row).
- **B7 stays close to control**: `T_any` matches control exactly in `2/3` seeds; `‖h‖@271` stays far closer to
  control (`58-65`) than to the frozen floor (`~14`, see below).

### Sufficiency series (vs. all-frozen, weighted input added)

| arm | seed 42 | seed 7 | seed 123 |
|---|---|---|---|
| all-frozen `‖h‖@271` / mean weighted input | 14.1 / 13.5 | 13.8 / 7.3 | 13.8 / 10.0 |
| B2 `‖h‖@271` / mean weighted input | 28.8 / 32.9 | 27.6 / 17.8 | 28.2 / 24.8 |
| B4 `‖h‖@271` / mean weighted input | 24.0 / 21.0 | 23.6 / 11.3 | 23.7 / 15.5 |
| **B7** `‖h‖@271` / mean weighted input | **47.1 / 62.6** | **45.4 / 34.8** | **46.0 / 47.2** |
| B5 `‖h‖@271` / mean weighted input | **2.2 / 4.4** | **2.3 / 2.1** | **2.3 / 3.2** |

**The predicted ranking `B7 > B2/B4 > all-frozen` replicates exactly in all 3 seeds**, on both `‖h‖@271` and mean
weighted input. B2 is consistently (not just on average) slightly ahead of B4 in every seed.

**Unpredicted finding**: B5 active alone falls *below* the all-frozen floor (`‖h‖≈2.2-2.3` vs. `~14`) in all 3
seeds — not merely "weak," but consistently *below* the zero-active-movement baseline. Since `all-frozen`'s
`‖h‖@271` matches `‖h‖` at position `16` (the pre-window release point) almost exactly — i.e. genuinely zero net
FT movement over the whole window — B5's active updates are pushing `‖h‖` net *downward* relative to that already-
frozen floor, not merely failing to grow it.

## Terminology, further restricted per this stage's results

- **B2**: control軌道における必要性候補 (necessity-candidate under control's own trajectory) — unchanged from
  Stage A, now 3-seed-replicated.
- **B7**: single-active条件での十分性候補 (sufficiency-candidate under the single-active condition, i.e. with
  every other block frozen) — unchanged from Stage A, now 3-seed-replicated.
- **B4**: probe-wide propagationを一貫して遅らせる傾向は3/3 seedで再現したが、局所発火(`T_any`)への影響は
  seed依存 (consistently delays probe-wide propagation in 3/3 seeds; effect on local firing is seed-dependent,
  2/3) — not classified as a clean "propagation-only" block pending further investigation (deferred, see Status).
- **B5**: negative controlではなく、FT normを縮小する可能性のあるanti-growth候補 (not a negative control — an
  anti-growth candidate, since active updates here consistently push `‖h‖` below the zero-movement floor across
  3/3 seeds). Deferred to after Stage C.

## Scope

Same as Stage A (necessity/sufficiency series use different active-position budgets and are not directly
comparable in absolute terms across series; `T_any` in the sufficiency series is confounded by position-`272`
unconditional resumption; checkpoint-boundary attribution carries the same small leak; data content and
trajectory state remain entangled). B4's seed-`7` divergence is not chased in this stage — deferred per priority
ordering below.

## Status

- No new code this stage (reused the Stage A reactivate-window flags).
- 19 new runs + `9` reused (arm, seed) cells from Stage A/control, complete, self-verified.
- Priority ordering going forward: (1) this doc, (2) B2×B7 interaction (Stage C — tests whether B2 primes B7's
  amplification or whether the two are independent/additive/interfering), (3) B5's anti-growth mechanism, (4) B4's
  seed-7-specific local-firing divergence.
