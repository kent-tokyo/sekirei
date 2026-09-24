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
| Flat default | `SEKIRW01` (also legacy `JANOSW03`) | input 2420, L1 256, L2 32 | The only family with a distributed artifact. |
| King-relative B-small | `SEKIRW02` | input 20564, L1 256, L2 32 | Experimental; no released weight. |
| Flat width diagnostics | `SEKIRW01` | `nnue_l1_128` / `nnue_l1_384` and `nnue_l2_16` / `nnue_l2_64` | Unreleased experiment builds; a file must match the exact selected dimensions. |

The `king_relative_b_small` feature changes the feature layout and requires a
separately trained file. Width features likewise change the required file
length even though they retain the flat magic. `JANOSW02` is not supported.

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
  [external evaluator manifest](interop/external_sfnn_manifest_v1.md) when a
  file is used for measurements.

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

Training can use a separately operated external teacher, but its license and
the dataset terms must be assessed by the person distributing the resulting
weights. Sekirei does not convert arbitrary external NNUE formats into its
own format.

See [`design/nnue_architecture_next_candidate.md`](design/nnue_architecture_next_candidate.md)
for the historical architecture decision record and internal `ROADMAP.md` for
active unpublished work.
