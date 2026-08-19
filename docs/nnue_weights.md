# NNUE weights — model card and licensing

This document exists because it's a real adoption question, not a
hypothetical one — see [issue #44](https://github.com/kent-tokyo/sekirei/issues/44),
from a prospective commercial mobile integrator.

## Software license vs. weights license — these are separate things

Sekirei's **source code** is dual-licensed MIT / Apache-2.0 (`LICENSE-MIT`,
`LICENSE-APACHE`), same as most of the Rust ecosystem — permissive, no
copyleft, no GPL. A **trained NNUE weight file** is a separate artifact:
it's data derived from training runs, not code, and this repository does
not currently ship one. Nothing in this repo grants a license to a weight
file, for the simple reason that no weight file is distributed as part of
it — the entire `data/` directory (where any weights would live) is
`.gitignore`d (see `.gitignore`), so no `.bin` weight file has ever been a
tracked, released, or distributed artifact of this project.

## Currently distributed weights: none

**No production-recommended trained weight file exists yet.** Without one,
`sekirei` runs on a genuine material-count fallback
(`crates/sekirei-core/src/eval.rs::evaluate`, dispatches to
`material_score` whenever `nnue::weights_active()` is false) — correct
shogi play, but not the strength-relevant evaluation NNUE-class engines are
built around. This is a real, current limitation, not a hedge: this
project's own backlog (`tasks/todo.md`) still lists "deploy NNUE to
floodgate once it beats material eval baseline" as **not done**, and no
weight file has cleared this project's own strength gate against that
baseline as of this writing.

If/when a production-quality weight file is published, its license terms
(including whether commercial redistribution is permitted) will be
specified explicitly at that time — do not assume the code's MIT/Apache-2.0
terms extend to it by default.

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
| A (flat, default) | Shipping default since this project's earliest NNUE work | No weight file has cleared the material-eval strength gate yet (see above) — the architecture itself is stable, but no specific trained checkpoint is currently recommended |
| B-small (king-relative, opt-in) | Experimental. Phase 3 validation: `valid_cp_mse` improved in 3/3 seeds, but `valid_wdl_loss`/`valid_calibration_error` regressed in 3/3 seeds against the same baseline — status is **MECHANICAL_PASS / EXPERIMENTAL_HOLD** (`docs/experiments/king_relative_b_small_phase3_diagnostic.md`, `docs/experiments/king_relative_scale_contract_static_audit.md`). No paired Elo/SPRT strength gate has been run. **Not recommended for production use at this time.** |

Neither architecture currently has a published, production-recommended
checkpoint. For app-size-insensitive integrators (per issue #44's own
framing), B-small is the more representationally interesting long-term
direction — but "interesting" and "validated" are different things here,
and it is explicitly not the latter yet.

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

`docs/experiments/king_relative_b_small_phase3_diagnostic.md` §1 is a
worked example of this template applied to a real (non-production) run.
