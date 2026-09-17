# Sekirei NNUE artifacts

`sekirei-nnue-v0.3.38.bin` is the recommended **optional** evaluator for
Sekirei 0.3.38. It is not part of any crate package or executable: keep the
file beside the engine, verify its checksum, then load it through `EvalFile`
or the first command-line argument.

```bash
shasum -a 256 sekirei-nnue-v0.3.38.bin
# 154ad1e4c8335b5e51a87af946d50a6789fbaf2d2568af58e85c6431e9d3797a

sekirei sekirei-nnue-v0.3.38.bin
```

For USI GUIs, set `EvalFile` to the file's absolute path before `isready`.
Use `NnueOutput=absolute`, which is the format declared in the adjacent model
card. If the weight cannot be loaded, the engine reports the error and remains
on material evaluation; it never silently substitutes a different checkpoint.

The binary and its model card are versioned release artifacts. They are
licensed under [CC BY 4.0](../NNUE-LICENSE.md), separately from Sekirei's
MIT OR Apache-2.0 source code. Retain the Sekirei / Kentaro Tanabe attribution
when redistributing or adapting the artifact.
