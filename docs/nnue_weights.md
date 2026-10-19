# NNUE weights: use, compatibility, and licensing

An NNUE weight file is a data artifact, separate from Sekirei's source code.
The source is available under MIT OR Apache-2.0; every distributed weight must
carry its own model card, checksum, provenance, strength-gate scope, and
license. Local training data and unpublished checkpoints remain outside Git.

## Distributed artifact

[`weights/sekirei-nnue-v0.3.38.bin`](../weights/sekirei-nnue-v0.3.38.bin) is
the optional flat evaluator published for 0.3.38. Its adjacent model card
pins the SHA-256, training provenance, output mode, strict-health result, and
historical local gate.

It is retained for compatibility and experimentation, not as a blanket
strength recommendation. On the current engine, a small local comparison
against material evaluation scored 1/32 at 1 second per move and 2/32 at
5 seconds per move. Those settings were separate and are not a default-choice
gate, external rating, or comparison with another engine.

Without `EvalFile` or a command-line checkpoint, Sekirei uses its material
fallback. It does not silently select or substitute a model.

## Unpublished `nn_r6` candidate

`nn_r6` is an unpublished HalfKP(Friend) 256x2-32-32 candidate. It uses
`FV_SCALE=24`, is not embedded in a crate, and is not selected automatically.

On fixed source `311ff1221064e5807d045bcf2c71badff0442671`, `nn_r6` passed
one-second material-only paired-trinomial gates at 1T (64-0-12, 76 games,
LLR 2.945103) and 4T (61-1-8, 70 games, LLR 3.047240). The
[release evidence](../weights/releases/README.md) and
[compatibility receipt](../weights/releases/nn_r6-main-compatibility.json)
bind the conditions and hashes. The actual v0.4.5 build is load-smoked with
the same file and scale; that compatibility check does not extend the older
strength result.

The retained owner record also attests a whole-game hold-out, hashed training
shards, and byte-identical uninterrupted/checkpoint-resume export. The
current public evidence does not yet form a card accepted by
`validate_nnue_release_artifact.py`, so no r6 model file is attached to this
release. These native fixed-condition gates are not an external rating, a
default-choice decision, or a WebAssembly/mobile qualification.

## Historical `nn_r3` candidate

`nn_r3` is a historical in-house HalfKP 256x2-32-32 candidate used with
`FV_SCALE=24`. It was trained for one epoch from `nn_r2` on gen33--40
self-play. Its recorded candidate comparison does not establish current-main
compatibility, an external rating, or a default-adoption decision.

## Loading a weight

```bash
sekirei /absolute/path/to/sekirei-nnue-v0.3.38.bin
```

For a USI GUI, set `EvalFile` to an absolute path and
`NnueOutput=absolute` before `isready`. Verify the checksum and CC BY 4.0
terms in [`weights/README.md`](../weights/README.md) first.

## Exact build compatibility

The loader verifies both an 8-byte magic value and the exact expected byte
length. A mismatched architecture or width is rejected rather than parsed
silently.

| Build family | Magic | Dimensions | Status |
|---|---|---|---|
| Flat default | `SEKIRW01` (also legacy `JANOSW03`) | input 2420, L1 256, L2 32 | Historical optional 0.3.38 artifact. |
| King-relative B-small | `SEKIRW02` | input 20564, L1 256, L2 32 | Experimental; no released weight. |
| Flat width diagnostics | `SEKIRW01` | `nnue_l1_128` / `nnue_l1_384` and `nnue_l2_16` / `nnue_l2_64` | Unreleased experiment builds; a file must match the exact selected dimensions. |

The `king_relative_b_small` feature changes the feature layout and requires a
separately trained file. Width features likewise change the required file
length even though they retain the flat magic. `JANOSW02` is not supported.

## Training export precision

The in-house trainer exports feature-transformer weights and biases as i16
at scale 64, rounded to the nearest integer (halfway cases away from zero).
Values retain the existing symmetric clamp at +/-32767. For finite values
within that range, the parameter error is at most 1/128 in training units.
L2 and output parameters remain f32.

This affects newly exported files and their checksums; existing files load
as before. It does not establish a playing-strength improvement. Record the
training commit when comparing checkpoints across export implementations.

## External HalfKP networks

`EvalFile` (and the first command-line argument) also accepts the common
external `HalfKP(Friend) 256x2-32-32` format. A file is treated as HalfKP when
it starts with the version word `0x7AF32F16`; its structural hashes and exact
byte length must then match, or it is rejected. Only one evaluator can be
active per process.

- The score is the integer network output divided by `FV_SCALE` (USI option,
  default 16, range 1..=128), clamped to +/-30,000. Some published networks
  recommend a different divisor.
- `NnueOutput` does not apply: HalfKP output is always a complete score.
- The implementation is written independently from the format description.
  Sekirei neither bundles nor redistributes any external network; each file's
  own license governs its use. Record provenance with the
  [`external_eval_manifest_v1` validator](../scripts/validate_external_eval_manifest.py)
  when a file is used for measurements.

Interoperability is checked with deterministic random networks that anyone can
regenerate, so no third-party file is needed:

```bash
cargo run --release -p sekirei-core --example halfkp_oracle -- write-net /tmp/hk/nn.bin 1
cargo run --release -p sekirei-core --example halfkp_oracle -- dump /tmp/hk/nn.bin 3000 1 > /tmp/hk/sekirei.tsv
python3 scripts/check_halfkp_oracle.py --engine /path/to/reference-engine \
  --option EvalDir=/tmp/hk --tsv /tmp/hk/sekirei.tsv
```

`dump` also asserts that the incremental accumulator equals a full rebuild at
every visited node. The reference engine must print `eval = <int>` for the
`eval` command.

## Evidence boundary

The B-small implementation passed mechanical load/inference checks, but its
validation signals disagreed and it has no successful paired strength gate.
It remains experimental. Unreleased candidate runs and their diagnostics do
not make a new file distributable or stronger than material evaluation.

Any checkpoint considered for distribution needs, at minimum:

```text
checkpoint_sha256:     <sha256>
architecture/build:    <features and dimensions>
training_commit:       <commit>
dataset/teacher hashes: <hashes>
validation summary:    <metrics and held-out input>
strength gate status:  not run | PASS | FAIL | INCONCLUSIVE
license:               <explicit artifact license>
```

For a HalfKP artifact proposed as a recommended release candidate, use the fail-closed
`sekirei.nnue-model-card.v2` contract checked by
`scripts/validate_nnue_release_artifact.py`. In addition to the fields above,
it requires hashed training-shard and whole-game hold-out manifests, exact raw
evidence for current-engine one- and four-thread gates, and the full engine
commit. This validator only checks whether the evidence bundle is complete and
internally consistent; it does not create evidence or turn an unpublished
candidate into a strength claim.

Each gate condition declares `threads`, `byoyomi_ms`, `games`, `verdict`,
and hashed `evidence` / `raw_evidence` references. `current_engine_gate` also
pins `engine_sha256`. The evidence reference must be a UTF-8 JSON object
(up to 1 MiB), schema `sekirei.nnue-engine-gate-evidence.v1`, containing:

- Matching `engine_commit`, `engine_sha256`, `candidate_sha256`,
  `opening_corpus_sha256`, `raw_evidence_sha256`, `threads`, `byoyomi_ms`,
  `fv_scale`, `games` and `verdict`.
- `wins`, `draws`, `losses` summing to the even, positive game count;
  `status="complete"`, `artifact_write_failures=[]`, `invalid_games=[]`.
- `baseline_evaluator="material"`, `candidate_load_acknowledged=true`,
  `baseline_weights_loaded=false`, as supported by the retained raw logs.
- `sprt`: `method="paired-trinomial"`, `elo0`, `elo1`, `alpha=0.05`,
  `beta=0.05`, `require_complete_pairs=true`, `analyzed_pairs=games/2`,
  `llr`, `lower_bound` and `upper_bound`. Bounds must match +/-log(19),
  and LLR must reach the upper bound. Numerical fields must be finite.

A `PASS` requires nonnegative `elo0` and `elo1>elo0`. A four-thread
`NON_REGRESSION` additionally requires the condition's explicitly declared
`non_regression_margin_elo>0`, with `elo0=-margin` and `elo1=0` in the
accepted test. Set that margin before play; this contract supplies no default
margin and does not authorize retrospective changes.

These identity and result checks reject a hash-correct summary for another
model/build or an unaccepted test. They do not independently replay SPRT,
prove the claimed source-to-binary correspondence, attest hold-out scope
completeness, or verify that logs support every declared field. The owner must
assemble this small metadata envelope from the original records; preserve
those records unchanged for review. A 12-position depth check remains a
bounded regression check, not a replacement for a current-engine gate.

Validate the assembled release bundle against the exact source commit:

```bash
python3 scripts/validate_nnue_release_artifact.py model-card.json \
  --root /path/to/release-bundle \
  --expected-engine-commit <full-40-hex-commit>
```

All referenced files must remain inside the bundle root; absolute paths,
parent-directory escapes, missing files, and hash mismatches fail validation.

An experimental opt-in asset may be distributed with a release only when its
card explicitly records every missing v2 condition. It must remain
non-default and may not be described as a recommended checkpoint until its
complete v2 bundle validates against the engine commit being released.

Training can use a separately operated external teacher, but its license and
the dataset terms must be assessed by the person distributing the resulting
weights. Sekirei does not convert arbitrary external NNUE formats into its
own format.

See [`design/nnue_architecture_next_candidate.md`](design/nnue_architecture_next_candidate.md)
for the historical architecture decision record and internal `ROADMAP.md` for
active unpublished work.
