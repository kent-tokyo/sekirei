# A(epoch8) vs B(epoch3) paired quick gate — step-half vs cosine playing strength

Follow-up to `docs/experiments/gate_b_lambda07.md`'s A/B/C matched-ablation
training comparison. That comparison picked each run's best-valid-loss
checkpoint (A=epoch8, B=epoch3) and found B's valid loss modestly better;
this gate asks whether that translates into playing strength.

## Purpose

Confirm whether step-half's and cosine's best-valid-loss checkpoints
differ in actual playing strength, not just validation loss.

## Candidate

- Run B: λ=0.7, cosine, epoch3
- path: `data/runs/ablation_B/weights.epoch3.bin`
- SHA-256: `5fcde6042f28be8cf8f6bd8b806bea0da0274e756b8cc8a9668345d8bbe622a7`
- valid_loss: 138595.82

## Baseline

- Run A: λ=0.7, step-half, epoch8
- path: `data/runs/ablation_A/weights.epoch8.bin`
- SHA-256: `67fa752a2ccac8cb668cabcc8cdd3d1bd93515c318eade2f2e43f9e80cf2d6b8`
- valid_loss: 140077.83

## Controlled variables

Verified identical between candidate and baseline via each checkpoint's
`.meta.json` before the gate ran — only `lr_schedule` (and `min_lr`,
inert for `step-half`) differs:

| field | value |
|---|---|
| dataset_hash | 11756567284176478750 |
| split_hash | 15885596499200304103 |
| seed | 42 |
| architecture | INPUT=2420 L1=256 L2=32 |
| wdl_lambda | 0.7 (both sides) |
| label_depth | 4 |
| min_rate | 1500.0 |
| validation_ratio | 0.15 |
| teacher config | build-time-fixed `Searcher`, no separate weights (see `docs/training_lessons.md`) |
| git_commit | 4f0d3c6 |

## Match

| field | value |
|---|---|
| usable openings | 99 (see note below) |
| games_per_position | 2 |
| total games | 198 |
| Threads | 1 / 1 |
| time control | byoyomi 1000ms (hardcoded in `scripts/sprint_gate.sh`) |
| opening suite | `data/gate/openings_standard.sfen`, SHA-256 `e4cfde1fc8b6346542f0bff2af1fe375f117b4abf8be8e3991622568bd342dbe` |
| diversity_ratio | 1.0 (every game's 20-ply prefix unique) |

**Why 99, not the header's claimed 100**: `openings_standard.sfen` has
exactly 100 lines total; one is the file's own comment header (`# one
SFEN per line -- standard strength-gate opening suite (100 positions,
...)`), leaving 99 real SFEN lines. `scripts/sprint_gate.sh` filters
comment lines (`grep -vc '^#'`) before counting, so it correctly used 99.
No position was dropped for being terminal, malformed, illegal, or
duplicate — the header's "100 positions" is a stale off-by-one in the
comment text (it appears to count itself as one of the 100), not a
data-quality issue. All 99 remaining lines are well-formed SFEN.
`README.md`'s reference to this file has been corrected to match.

## Result

| metric | value |
|---|---|
| B wins | 94 |
| A wins | 104 |
| draws | 0 |
| elo_diff | -17.56 |
| 95% CI | [-66.02, +30.89] |
| LOS | 23.84% |
| verdict | **INCONCLUSIVE** |

veridict's own warning: *"the measured effect is smaller than the CI's
own half-width: it could plausibly be noise around zero, even though the
sample isn't tiny."*

## Decision

**Statistical conclusion**: the A/B playing-strength difference is
undetermined at 198 games. The point estimate leans toward A, but the CI
comfortably spans zero and even a modest positive effect for B — this
result does **not** establish that A is the stronger player.

**Engineering decision**: retain A as the working control; do not
promote B; do not extend to a 400-game gate. This is not "A won" — it's
"B produced no positive evidence at this sample size, and the point
estimate trends the wrong direction, so there is no strong basis to
invest further compute chasing it." B's 1.06% valid-loss improvement
over A did not translate into a measurable playing-strength edge here.
