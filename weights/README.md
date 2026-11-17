# NNUE downloads and loading

Weights are optional and separate from the executable. Without a loaded weight,
Sekirei uses material evaluation. Run the commands below from the repository
root. Format and release-evidence rules are in [NNUE weights](../docs/nnue_weights.md).

## HalfKP r6

Download the v0.5.0 [compressed weight](https://github.com/kent-tokyo/sekirei/releases/download/v0.5.0/sekirei-halfkp-r6-v0.5.0.bin.xz)
and [evidence bundle](https://github.com/kent-tokyo/sekirei/releases/download/v0.5.0/sekirei-halfkp-r6-v0.5.0-bundle.tar.xz).
Verify both against the [release card](sekirei-halfkp-r6-v0.5.0.json):

| File | SHA-256 |
|---|---|
| `.bin.xz` | `01623e1146293a3aa30a299d250865750d7b166a416cc46f2d1b8db3a48cbb8f` |
| Decompressed `.bin` | `634bdf527e7306045c8278df1d5311777561dfb09c1ebe6829ff3f4d883cc469` |
| `-bundle.tar.xz` | `b9e8c05a5a277e52c742a0461d26714a276c5d23b03ca264072aa3f6855a026c` |

Configure the USI GUI with the decompressed file's absolute path and
`FV_SCALE=24`. The equivalent commands to send to a fresh engine are:

```text
usi
setoption name EvalFile value /absolute/path/sekirei-halfkp-r6-v0.5.0.bin
setoption name FV_SCALE value 24
isready
```

Confirm the weight-load acknowledgement and `info string FV_SCALE 24`.
`NnueOutput` does not change HalfKP scores. The [qualification record](releases/README.md)
applies to its pinned 1T/4T native builds and conditions; r6 is not the default
and has no general external-rating or WASM/mobile qualification.

## Historical flat weight

The repository contains [sekirei-nnue-v0.3.38.bin](sekirei-nnue-v0.3.38.bin)
and its [model card](sekirei-nnue-v0.3.38.json):

```bash
shasum -a 256 weights/sekirei-nnue-v0.3.38.bin
# 154ad1e4c8335b5e51a87af946d50a6789fbaf2d2568af58e85c6431e9d3797a
sekirei weights/sekirei-nnue-v0.3.38.bin
```

For a GUI, set `EvalFile` to the absolute path and `NnueOutput=absolute`
before `isready`. This artifact is retained for compatibility and experiments;
its historical results do not make it a recommended current weight.

An initial load failure reports an error and leaves material evaluation active.
To switch an already loaded weight, restart the engine. It never automatically
selects a different checkpoint.

## Licensing and historical candidates

Both distributed weights are [CC BY 4.0](../NNUE-LICENSE.md), separate from the
MIT OR Apache-2.0 source. Preserve each model card's attribution when
redistributing or adapting its artifact.

The local `nn_r3` binary is not distributed in Git. Its
[candidate card](candidates/sekirei-halfkp-r3.json) and
[historical gate](../docs/experiments/nnue_r3_material_gate_v0.3.68/README.md)
retain its identity and limited evidence; they do not authorize a new trial,
distribution, or default adoption.
