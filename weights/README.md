# Sekirei NNUE artifacts

`sekirei-nnue-v0.3.38.bin` is an available **optional** evaluator originally
published for Sekirei 0.3.38. It is not part of any crate package or
executable: keep the file beside the engine, verify its checksum, then load it
through `EvalFile` or the first command-line argument.

```bash
shasum -a 256 sekirei-nnue-v0.3.38.bin
# 154ad1e4c8335b5e51a87af946d50a6789fbaf2d2568af58e85c6431e9d3797a

sekirei sekirei-nnue-v0.3.38.bin
```

For USI GUIs, set `EvalFile` to the file's absolute path before `isready`.
Use `NnueOutput=absolute`, which is the format declared in the adjacent model
card. If the weight cannot be loaded, the engine reports the error and remains
on material evaluation; it never silently substitutes a different checkpoint.

The historical 0.3.38 paired gate remains recorded in the model card. It does
not establish an advantage over the current material-only evaluator. In a
current-engine local diagnostic, B scored 1/32 at 1 second per move and 2/32
at 5 seconds per move against material-only evaluation. This small comparison
does not formally select a default, but the checkpoint remains available for
compatibility and experimentation rather than as a blanket strength
recommendation.

The binary and its model card are versioned release artifacts. They are
licensed under [CC BY 4.0](../NNUE-LICENSE.md), separately from Sekirei's
MIT OR Apache-2.0 source code. Retain the Sekirei / Kentaro Tanabe attribution
when redistributing or adapting the artifact.

## Current in-house candidate

`nn_r3` is the current local HalfKP candidate. Its exact identity and evidence
boundary are recorded in
[`candidates/sekirei-halfkp-r3.json`](candidates/sekirei-halfkp-r3.json).
The binary is not stored in Git, bundled, selected by default, or recommended
for distribution. The recorded 480-game result is against `nn_r2`, not against
material evaluation; the chained estimate against `gen9` is not a direct
measurement.

If the local artifact is available, verify it before any diagnostic:

```bash
python3 scripts/validate_nnue_candidate_card.py \
  weights/candidates/sekirei-halfkp-r3.json \
  --artifact data/selfplay/nnue/claude061/nn_r3.bin
```
