# NNUE weights

Sekirei uses material evaluation unless a weight is explicitly loaded. Weight
files are separate from the MIT OR Apache-2.0 source. Each needs its own license,
checksum, provenance, and evidence. Private data and unpublished weights stay
outside Git.

## Available weights

| Artifact | Format and setting | Evidence and use |
|---|---|---|
| `sekirei-halfkp-r6-v0.5.0.bin` (`nn_r6`) | HalfKP(Friend) 256x2-32-32, `FV_SCALE=24` | Optional v0.5.0 asset; independent 1T/4T gates against material evaluation on a pinned native build. |
| `sekirei-nnue-v0.3.38.bin` | Flat `SEKIRW01`, `NnueOutput=absolute` | Historical optional artifact for compatibility and experiments. |
| `nn_r3` | HalfKP 256x2-32-32, `FV_SCALE=24` | Historical local candidate; binary not distributed in Git. |

Use [weights/README.md](../weights/README.md) for downloads, checksums, and
loading commands. Both distributed weights are CC BY 4.0. The r6
[qualification record](../weights/releases/README.md) gives exact source,
settings, and results. Its material gates do not establish an external rating,
a default-selection decision, or WASM/mobile qualification.

The flat weight's [model card](../weights/sekirei-nnue-v0.3.38.json) records a
2026-09-21 diagnostic: 1 win in 32 games at one second per move and 2 wins in
32 at five seconds against material evaluation. These separate historical
settings are not measurements of the current checkout. The
[r3 card](../weights/candidates/sekirei-halfkp-r3.json) and
[gate record](experiments/nnue_r3_material_gate_v0.3.68/README.md) likewise
apply only to their recorded builds and conditions.

## Loading and compatibility

Set an absolute `EvalFile` path before `isready`, or pass the path as the first
command-line argument. Confirm the load acknowledgement. An initial load
failure leaves material evaluation active; changing an already loaded file
requires a new engine process.

| Build family | Header | Required layout |
|---|---|---|
| Default flat | `SEKIRW01`, legacy `JANOSW03` | 2420 inputs, L1 256, L2 32 |
| Experimental `king_relative_b_small` | `SEKIRW02` | 20564 inputs, L1 256, L2 32; no released weight |
| Experimental flat widths | `SEKIRW01` | Exact selected `nnue_l1_128` / `nnue_l1_384` and `nnue_l2_16` / `nnue_l2_64` dimensions |

The loader checks the header and exact length. `JANOSW02` is unsupported;
feature-layout and width changes require a matching trained file.
The trainer exports feature-transformer parameters as i16 at scale 64, rounded
halfway away from zero and clamped to ±32767; L2/output parameters remain f32.
Record the training commit when comparing exports.

## External HalfKP networks

`EvalFile` also accepts HalfKP(Friend) 256x2-32-32. The version word
`0x7AF32F16`, structural hashes, and exact length must match. `FV_SCALE`
divides the integer output (default 16, range 1–128), then scores are clamped
to ±30,000. `NnueOutput` does not affect HalfKP; it always supplies a full score.
Only one evaluator can be active per process.

External files retain their own licenses and are not bundled. Record measurement
provenance with [validate_external_eval_manifest.py](../scripts/validate_external_eval_manifest.py).
The `halfkp_oracle` example generates deterministic random test networks and
checks incremental accumulators against full recomputation; compare its output
with a separate reference using [check_halfkp_oracle.py](../scripts/check_halfkp_oracle.py).
This checks interoperability, not strength.

## Release evidence

A HalfKP release bundle uses a `sekirei.nnue-model-card.v2` card with artifact
hash, license, training and whole-game hold-out manifests, exact engine identity,
and raw 1T/4T gate records. Validate the assembled bundle against its source:

```bash
python3 scripts/validate_nnue_release_artifact.py model-card.json \
  --root /path/to/release-bundle \
  --expected-engine-commit <full-40-hex-commit>
```

The validator checks schema, references, hashes, and declared results. It does
not replay SPRT, prove source-to-binary identity, or authorize adoption. Keep
raw match data, frozen openings, and terminal receipts; use
[export_nnue_gate_evidence.py](../scripts/export_nnue_gate_evidence.py) to derive
summaries. A gate must meet its original paired decision rule. An experimental
release with missing conditions requires an explicit record of those gaps and
remains optional; distribution requires separate authorization.

## Retained data status

The original r6 private manifest has SHA-256
`e916768450de96e6562a691e08cb4a5278de76ec1b8ab38d0bf243ec4e0661a6` and names
32 raw shards. The 2026-11-14 [recovery audit](https://github.com/kent-tokyo/sekirei/issues/284#issuecomment-6872041961)
found no verified Mac/SSD copy or usable locator for the old cloud workspace.
This records unavailable inputs, not deletion, and does not change the completed
r6 qualification. Regenerated data cannot inherit those shard identities.

The reserved `sekirei-selfplay-teacher-20261114-v1` corpus was not generated.
Its proposed work is closed by the 2026-11-16 plan reset. Any future corpus needs
its own recipe, hashes, independent split, and tested restoration receipt or an
explicit `not retained` status. The
[architecture note](design/nnue_architecture_next_candidate.md) is historical.
