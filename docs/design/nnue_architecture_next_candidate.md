# NNUE architecture decision record

Status: **historical design record**. This document records what was decided
and tested for the first architecture exploration; it is not a current model
recommendation or a strength result. Active, unpublished work belongs in the
internal roadmap and run manifests.

## Baseline and options considered

The original flat evaluator uses piece-square and hand features, an
input width of 2420, `L1=256` per perspective, and `L2=32`. It has no
king-relative board feature.

| Option | Intended effect | Main cost or risk |
|---|---|---|
| B-small | Add the own king's 3×3 zone to board features. | Much larger feature table; a king move refreshes the accumulator. |
| Wider L1 | Increase feature-transformer capacity. | Larger file and incremental-update work. |
| Wider L2 | Increase output-side capacity. | Earlier diagnostics indicated L2 saturation, so width alone was not a good first bet. |

The first implementation chose B-small: it adds nine king-zone buckets while
leaving hand features unconditioned. It uses `SEKIRW02`, input width 20564,
`L1=256`, and `L2=32`; it is incompatible with the flat released artifact.

## Outcome

B-small passed format, loading, inference, and differential-update checks.
Across its matched three-seed validation, CP MSE improved in all three seeds,
while WDL loss and calibration error regressed in all three. No paired
playing-strength gate established an advantage.

**Verdict: `MECHANICAL_PASS / EXPERIMENTAL_HOLD`.** The implementation remains
available behind `king_relative_b_small`, but no B-small checkpoint is
distributed and it is not a production recommendation.

## Durable lessons

1. A feature or capacity change needs a fresh matching weight file; magic and
   exact-length checks must reject an incompatible file.
2. Offline validation metrics may disagree. Do not promote a checkpoint until
   an independent, fixed-condition playing-strength gate passes.
3. Separate representational content from inference cost. A faster or larger
   evaluator is not automatically a stronger engine.
4. Change one declared factor at a time, preserve source/teacher/checkpoint
   hashes, and keep resource-censored runs distinct from failures.

For the supported formats and the published flat artifact, see
[`../nnue_weights.md`](../nnue_weights.md). The historical detailed evidence
is retained in experiment artifacts; it should be consulted only when
reopening this specific decision.
