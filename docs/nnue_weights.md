# NNUE weights — model card and licensing

This document exists because it's a real adoption question, not a
hypothetical one — see [issue #44](https://github.com/kent-tokyo/sekirei/issues/44),
from a prospective commercial mobile integrator.

## Software license vs. weights license — these are separate things

Sekirei's **source code** is dual-licensed MIT / Apache-2.0 (`LICENSE-MIT`,
`LICENSE-APACHE`), same as most of the Rust ecosystem — permissive, no
copyleft, no GPL. A **trained NNUE weight file** is a separate artifact:
it is data derived from training runs, not code. Project training runs remain
under ignored `data/`, while each distributed checkpoint lives in the tracked
`weights/` directory with its own model card, SHA-256, and CC BY 4.0 notice.
The source-code license does not extend to a weight artifact.

## Currently distributed weights

[`weights/sekirei-nnue-v0.3.38.bin`](../weights/sekirei-nnue-v0.3.38.bin) is
the versioned recommended optional A-flat checkpoint for 0.3.38. Its SHA-256,
training provenance, output mode, strict-health result, and local
strength-gate scope are pinned in its adjacent
[model card](../weights/sekirei-nnue-v0.3.38.json). It cleared a local,
color-reversed paired-SPRT comparison against the pinned baseline. That does
not establish a Floodgate rating, a human rating, or superiority to another
engine.

The crate and executable do not embed a model. Without an explicit `EvalFile`
or command-line checkpoint, `sekirei` runs on a genuine material-count fallback
(`crates/sekirei-core/src/eval.rs::evaluate`, dispatches to
`material_score` whenever `nnue::weights_active()` is false) — correct
shogi play, but not the checkpoint-backed evaluation measured above.

## Weight file format compatibility

| Magic | Architecture | Compatible builds |
|---|---|---|
| `SEKIRW01` | Flat piece-square ("A", default) | Default build (`king_relative_b_small` feature off) |
| `SEKIRW02` | King-relative 9-zone ("B-small") | `--features king_relative_b_small` build only |
| `JANOSW03` (legacy) | Flat piece-square, same layout as `SEKIRW01` | Default build only, accepted for backward compatibility |
| `JANOSW02` (legacy) | Different layout | **Not accepted by any current build** |

A binary refuses to load the wrong variant's file: the magic string is
checked first, and `read_weights` requires an *exact* byte-length match
(not just a minimum), so a wrong-architecture file fails with a clear
error instead of silently misparsing (`crates/sekirei-core/src/nnue.rs`,
module doc and `read_weights`). Binary layout (identical shape in both
variants, `INPUT` differs per architecture):

```
Offset        Size           Content
0             8              Magic
8             INPUT*L1*2     ft_weights: INPUT × L1 × i16
+L1*2         L1*2           ft_bias: L1 × i16
+2*L1*L2*4    2*L1*L2*4      l2_weights: (2×L1) × L2 × f32
+L2*4         L2*4           l2_bias: L2 × f32
+L2*4         L2*4           out_weights: L2 × f32
+4            4              out_bias: f32
```

`L1=256`, `L2=32` in both variants. `INPUT=2420` (flat) / `INPUT=20564`
(king-relative). Total file size: ≈1.24 MB (flat) / ≈10.0 MB
(king-relative).

## Architecture status

| Architecture | Status | Recommended for production use? |
|---|---|---|
| A (flat, default) | Shipping default since this project's earliest NNUE work | `sekirei-nnue-v0.3.38.bin` is the recommended optional checkpoint; its claim boundary is the pinned local paired-SPRT result above |
| B-small (king-relative, opt-in) | Experimental. Phase 3 validation: `valid_cp_mse` improved in 3/3 seeds, but `valid_wdl_loss`/`valid_calibration_error` regressed in 3/3 seeds against the same baseline — status is **MECHANICAL_PASS / EXPERIMENTAL_HOLD** (see [`design/nnue_architecture_next_candidate.md`](design/nnue_architecture_next_candidate.md)). No paired Elo/SPRT strength gate established an improvement. **Not recommended for production use at this time.** |

Only the A-flat checkpoint above is recommended for 0.3.38, and only within
its recorded local paired-SPRT scope. For app-size-insensitive integrators
(per issue #44's own framing), B-small is the more representationally
interesting long-term direction — but "interesting" and "validated" are
different things here, and it is explicitly not the latter yet.

## Training your own weights

`sekirei-train` is the training crate; nothing about it requires this
project's own teacher data specifically. If you use a strong external USI
engine as a teacher to generate evaluation/WDL labels for your own position
dataset, note:

- The resulting weight file's licensing is yours to determine, informed
  by whatever license terms attach to the teacher engine's own output and
  the position dataset used — this repo makes no claim about that, since
  it isn't the origin of either.
- The trained file must match the exact binary format (§ above) and,
  architecture-wise, one of the two supported feature configurations —
  there is no format-conversion tool for arbitrary external NNUE weights.
- `sekirei-train` never calls `nnue::load_weights` during training itself
  (confirmed: label generation runs on the fixed-depth search/material
  path, independent of any NNUE weights) — so training does not require a
  pre-existing Sekirei weight file to bootstrap from.

## Model-card template for a specific checkpoint

If you produce and want to track a specific weight file's provenance,
record at minimum:

```
checkpoint_sha256:     <shasum -a 256 output>
architecture:           A-flat-ps | B-small-king9zone
magic:                  SEKIRW01 | SEKIRW02
training_commit:        <sekirei git commit the training run was built from>
dataset_hash:           <from the .meta.json sidecar, if trained with sekirei-train>
teacher_cache_sha256:   <if applicable>
validation_summary:     valid_cp_mse / valid_wdl_loss / valid_calibration_error
strength_gate_status:   not run | SPRT PASS (H0/H1, N games) | SPRT FAIL | INCONCLUSIVE
license:                <explicit statement -- do not assume it inherits the code's MIT/Apache-2.0>
```

The outcome in
[`design/nnue_architecture_next_candidate.md`](design/nnue_architecture_next_candidate.md)
shows how a real non-production validation verdict remains separate from a
strength or release claim.
