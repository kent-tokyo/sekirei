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

## Optional `nn_r6` artifact

`nn_r6` is an optional HalfKP(Friend) 256x2-32-32 artifact. It uses
`FV_SCALE=24`, is not embedded in a crate, and is not selected automatically.

The v0.5.0 evidence bundle is bound to source commit
`a60be2cdf2f17a2c8bfb9980536f452e8c19958e`. It passed independent,
colour-reversed one-second material-only paired-trinomial gates: 1T
57-1-4 over 62 games (LLR 3.023878) and 4T 59-0-5 over 64 games (LLR
3.032299). Its model card, redacted training and whole-game hold-out
manifests, and raw gate evidence pass
`validate_nnue_release_artifact.py` against that exact commit.

These native fixed-condition gates are not an external rating, a default-choice
decision, or a WebAssembly/mobile qualification. Verify the release-asset
checksum and the CC BY 4.0 terms before use. The [weight card](../weights/sekirei-halfkp-r6-v0.5.0.json)
names both v0.5.0 release assets and their checksums.

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

## Evidence for a release weight

The B-small implementation passed mechanical load and inference checks, but
has no successful paired strength gate. Unreleased diagnostics do not make a
checkpoint distributable or stronger than material evaluation.

A proposed HalfKP release weight needs a `sekirei.nnue-model-card.v2` card.
The [validator](../scripts/validate_nnue_release_artifact.py) checks its
artifact hash, license, training and whole-game hold-out manifests, exact
engine identity, and raw 1T/4T current-engine gate references. Each gate must
pin its opening corpus, material baseline, candidate load acknowledgement,
paired-trinomial result, and terminal receipt. A `PASS` requires the
preregistered bounds and complete colour-reversed pairs; a 4T
`NON_REGRESSION` also requires a margin declared before play.

The validator checks the bundle's schema, references, hashes, and declared
results. It does not replay SPRT, prove the source-to-binary relationship,
audit every raw log claim, or authorize default adoption. Preserve the
original records for review. A short depth diagnostic cannot replace a
current-engine gate. Validate against the exact release source commit:

```bash
python3 scripts/validate_nnue_release_artifact.py model-card.json \
  --root /path/to/release-bundle \
  --expected-engine-commit <full-40-hex-commit>
```

Use [`export_nnue_gate_evidence.py`](../scripts/export_nnue_gate_evidence.py)
on each retained terminal receipt to create the 1T and 4T summaries. Keep
the raw match JSON, opening corpus, manifests, and receipts in the bundle;
calculate the card's reference hashes only after assembling it. The exporter
cannot create a gate or infer a license.

An experimental opt-in asset may be released only with an explicit record of
its missing v2 conditions. It must stay non-default and must not be described
as a recommended checkpoint until a complete v2 bundle validates against the
engine commit being released.

Training can use a separately operated external teacher, but its license and
the dataset terms must be assessed by the person distributing the resulting
weights. Sekirei does not convert arbitrary external NNUE formats into its
own format.

See [`design/nnue_architecture_next_candidate.md`](design/nnue_architecture_next_candidate.md)
for the historical architecture decision record and internal `ROADMAP.md` for
active unpublished work.
