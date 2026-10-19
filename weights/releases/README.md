# Planned optional nn_r6 release asset

`sekirei-halfkp-nn_r6-qualified-native.tar.gz` is ready for the proposed
0.4.5 GitHub Release. It has not been uploaded or published. The
[release plan](sekirei-halfkp-nn_r6.json) binds the exact archive (25,738,866
bytes), model card and weight file. The archive includes the weight, CC BY 4.0
notice, training/split provenance, both native gates and raw match evidence.
Weights are separate from source crates and executables; material remains the
fallback when no `EvalFile` is set.

## Verification and use

After obtaining the release asset, verify its SHA-256 before extraction:

```bash
shasum -a 256 sekirei-halfkp-nn_r6-qualified-native.tar.gz
# 97cd46116e479671f00fbde604e743c4afb7e3ccd7a65d14f92cb0bf591cfd6c
tar -xzf sekirei-halfkp-nn_r6-qualified-native.tar.gz
cd sekirei-halfkp-nn_r6
shasum -a 256 -c SHA256SUMS
```

In a native USI GUI, set these options before `isready`:

```text
setoption name EvalFile value /absolute/path/to/sekirei-halfkp-nn_r6/weights/nn_r6.bin
setoption name FV_SCALE value 24
setoption name NnueOutput value absolute
isready
```

Require `info string NNUE weights loaded from ...` followed by `readyok`.
The `FV_SCALE=24` setting is part of this model's operating conditions;
the engine's HalfKP default scale is 16. Retain the attribution and license
from the bundle when redistributing it.

## Recorded qualification

The model passed fixed-source material gates on
`311ff1221064e5807d045bcf2c71badff0442671`, using one second per move,
Hash 64, `SpecTopN=0`, and colour-reversed pairs on separate 1T/4T subsets.
1T used `SearchMode=Speculative`; 4T used `SearchMode=LazySMP`.

| Threads | Games | Wins-draws-losses | LLR | Verdict |
|---|---:|---|---:|---|
| 1 | 76 | 64-0-12 | 2.945103 | PASS |
| 4 | 70 | 61-1-8 | 3.047240 | PASS |

Paired trinomial SPRT used H0=0, H1=+20, alpha=beta=0.05 and bounds
+/-2.944439. The shared-host protocol retains every valid completed pair
regardless of CPU telemetry. A user-authorized continuation retained all 22
saved games and preserved the unrecorded interrupted attempt; its addendum
and unknown interruption cause remain in the evidence. This is a native
fixed-condition material comparison, not an external rating or a default
selection. Browser/WASM/mobile use is not qualified by these matches.

## Compatibility on current main

On `c3f335848a700a32717debedc406fe3107651df5`, the ordinary native build
loaded the same weight and matched the qualification binary on all 12
already-spent own roots at depth 10: best move, score, nodes and PV were
identical. A separate 4T loading/search smoke also passed. The
[compatibility receipt](nn_r6-main-compatibility.json) binds both executables
and the corpus. These checks establish compatibility on that source only;
they are not a new strength gate. Recheck the actual release build.

## Release procedure

1. Verify the archive SHA-256, size and extracted `SHA256SUMS` against the
   release plan. Keep the qualified bundle unchanged.
2. Validate its model card with the recorded gate source, not a substituted
   new commit:

   ```bash
   python3 scripts/validate_nnue_release_artifact.py \
     /path/to/sekirei-halfkp-nn_r6/model-card.json \
     --root /path/to/sekirei-halfkp-nn_r6 \
     --expected-engine-commit 311ff1221064e5807d045bcf2c71badff0442671
   ```

3. Check loading and scale on the actual release build. Record any source
   difference and its compatibility checks separately. Changes to evaluation
   or search require fresh strength evidence before extending the gate claim.
4. Once the release is authorized, attach this exact archive to that GitHub
   Release, alongside the release's other assets. Record its name, SHA-256,
   byte size and original gate source in the release manifest; download it
   again and verify the published bytes before declaring it shipped.

The proposed version must match the actual release decision. This plan does
not bump crate versions, publish an asset, or switch the default evaluator.
