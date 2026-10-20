# `nn_r6` release evidence

`nn_r6` is attached to v0.5.0 as an optional GitHub Release asset. It remains
separate from source crates and executables; material remains the fallback when
no `EvalFile` is set. The asset is licensed under [CC BY 4.0](../../NNUE-LICENSE.md)
and is not selected by default.

## Qualification

The strict release bundle is bound to engine commit
`a60be2cdf2f17a2c8bfb9980536f452e8c19958e`, Hash 64, `SpecTopN=0`, material
baseline, one second per move, and separate frozen colour-reversed opening
corpora. It validates with `validate_nnue_release_artifact.py`.

| Threads | Search mode | Games | Wins-draws-losses | LLR | Verdict |
|---:|---|---:|---|---:|---|
| 1 | Speculative | 62 | 57-1-4 | 3.023878 | PASS |
| 4 | LazySMP | 64 | 59-0-5 | 3.032299 | PASS |

Both gates use paired-trinomial SPRT with H0=0, H1=+20, alpha=beta=0.05 and
bounds +/-2.944439. The archive contains the model card, license, redacted
training and whole-game hold-out manifests, frozen opening manifests, raw
match records, terminal receipts, and the weight. Verify the release-asset
checksums before use.

These results are native fixed-condition material comparisons. They are not an
external rating, a default-adoption decision, or a WebAssembly/mobile-quality
qualification.
