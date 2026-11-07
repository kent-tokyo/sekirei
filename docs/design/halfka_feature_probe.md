# King-feature prototype

Status: **synthetic mechanical prototype only**. This experiment is separate from
the production evaluator, trainer, existing weight files and B-small decision.
Its format cannot be selected through `EvalFile`.

The [B-small record](nnue_architecture_next_candidate.md) remains
`MECHANICAL_PASS / EXPERIMENTAL_HOLD`. B-small adds nine own-king zones to the
older flat feature family and uses `SEKIRW02`. Its historical three-seed record
reports improved CP MSE and worse WDL loss/calibration. The original raw run
artifacts were not available in this worktree for a new analysis. The existing
`select_king_relative_checkpoint.py` CP-MSE gate treats those other metrics as
diagnostics; a mechanical or offline PASS does not establish playing strength.

This prototype instead starts from the current HalfKP feature mapping and
changes **one factor: adding both kings as input features**. It is inspired by
HalfKA, but does not claim compatibility with a standard HalfKA implementation
or weight format. It adds no buckets, changes no widths, and introduces no new
activation or training objective.

## Feature and arithmetic contract

All 125,388 existing HalfKP feature IDs remain unchanged. Let `k` be the own
king square and `s` a king's square in the existing perspective-oriented
file-major format. White rotates both squares by 180 degrees. Append:

```
own king:      125388 + k * 162 + s
opponent king: 125388 + k * 162 + 81 + s
```

There are 13,122 addressable added rows and exactly two added active features per
perspective. Some rows are unreachable: an own-king feature always has `s=k`,
and the two kings cannot share a square. This prototype keeps a uniform index
formula without compacting those rows. Both kings must be present exactly once. Non-king inventory cannot
exceed the physical shogi supply. Hand and promoted-piece mapping remains the
existing HalfKP mapping. Input validation checks representation and inventory;
it does not certify that an arbitrary SFEN arose from a legal game.

Each perspective retains 256 FT channels, followed by concatenation in
side-to-move/opponent order and layers `512 → 32 → 32 → 1`. FT addition wraps
as signed i16. FT activation clips to `[0,127]`; each hidden layer uses arithmetic
division by 64 followed by the same clipping. Output division truncates toward
zero using the file's explicit FV_SCALE, followed by the existing ±30000 clamp.
Stored integer biases are already quantized; the loader does not add another
rounding offset. Conservative i32 headroom checks protect every affine partial
sum, including weights of -128.

## Diagnostic image version 1

All numbers are little-endian. Magic is ASCII `SKXK0001`; this is deliberately
different from `nn.bin`, `SEKIRW01` and `SEKIRW02`.

| Field | Representation |
|---|---|
| Header | 8-byte magic, then eight u32 values |
| Header values | version=1, feature kind, input count, FT=256, hidden1=32, hidden2=32, FV_SCALE, stored row count |
| Feature kind | 0: baseline HalfKP; 1: added king features |
| Input count | 125388 or 138510, matching the kind |
| FT bias | 256 i16 |
| Sparse FT rows | strictly increasing unique u32 ID, then 256 i16 values per row |
| Hidden layer 1 | 32 i32 biases, then 32×512 i8 weights in output-major order |
| Hidden layer 2 | 32 i32 biases, then 32×32 i8 weights in output-major order |
| Output | one i32 bias, then 32 i8 weights |

An omitted FT row means exactly zero. At most 4,096 rows may be stored; the
complete image is at most 2,131,788 bytes. Exact length, magic, version, kind,
dimensions, scale, row ordering/range and bias headroom are validated. Loading
requires the expected kind explicitly, so a baseline file cannot silently become
a king-feature file. There is no production checkpoint conversion or resume path.

Sparse storage exists to keep synthetic tests small. It is not a proposed
compression scheme for a trained evaluator. Integrity hashes are recorded in the
private smoke report rather than embedded in this experimental binary header.

## Mechanical checks and cost accounting

The standalone safe-Rust example uses the engine's existing non-king feature
functions. The independent Python reference derives all indices from SFEN and
computes affine sums with arbitrary-precision integers. It reduces the complete
FT sum modulo 65536; Rust wraps after each addition. Their layer values and
scores must match exactly.

The smoke has three synthetic images sharing every baseline parameter:

1. HalfKP baseline.
2. Added king features with all new weights zero: every activation and score
   must match the baseline.
3. The same extension with seeded synthetic king weights: the new feature
   paths must affect the output without changing any baseline weight.

For consecutive SFENs, the example checks feature-set difference updates against
a full refresh, then applies the inverse and verifies the prior accumulator.
This is a generic delta/undo invariant, **not** integration with `Board::do_move`,
Finny, lazy accumulation, search threads or production rollback tokens.

| Analytical quantity | Baseline | King extension |
|---|---:|---:|
| Logical dense FT bytes | 64,198,656 | 70,917,120 |
| Maximum active features per perspective | 38 | 40 |
| Full-refresh FT additions | 256 × total active features | baseline + 1,024 |
| Dense affine multiply-accumulates per forward | 17,440 | 17,440 |
| Two i16 accumulators | 1,024 bytes | 1,024 bytes |

Trace fields beginning `logical_` count hypothetical dense vector work. The
sparse fixture code may skip absent rows and also performs map lookups; those
counts are neither measured CPU instructions nor timings. Dense storage counts
exclude allocator/metadata/cache overhead. No RSS, memory bandwidth, SIMD speed,
training throughput, calibration or strength improvement is established.

## Reproduce without training

Build only the isolated example, then run bounded synthetic inference:

```sh
cargo build --offline -p sekirei-core --example halfka_feature_probe
SEKIREI_HALFKA_PROBE=target/debug/examples/halfka_feature_probe \
  python3 -W error::ResourceWarning -m unittest discover -s scripts \
  -p 'test_halfka_feature_prototype.py' -v
python3 scripts/halfka_feature_prototype.py \
  --native target/debug/examples/halfka_feature_probe \
  --output-dir /PRIVATE/NEW-ATTEMPT-DIRECTORY --seed 20261107
```

Keep images and raw SFEN/trace evidence private. The smoke requires a new output
directory and a free-space floor; it never reads external weights or starts
training/search/games. All existing NNUE and hold-out pauses remain in effect.

A later architecture trial needs owned labels with the same teacher, game-level
split and hold-out exclusions; matching fixed-width initialization/training
budgets; current-source integer parity; memory and throughput measurements; and
fresh independent strength gates. The diagnostic format and sparse fixtures do
not supply those inputs or qualify a model for adoption.
