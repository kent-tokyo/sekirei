# `nn_r6` optional release evidence

`sekirei-halfkp-r6-v0.4.5.bin.xz` is the optional GitHub Release asset for
v0.4.5. It is separate from source crates and executables; material remains
the fallback when no `EvalFile` is set. The [release plan](sekirei-halfkp-nn_r6.json)
binds the fixed-source 1T/4T gates and compatibility receipt to the exact
weight identity.

## Verification and use

After obtaining the release asset, verify its checksum before extraction:

```bash
shasum -a 256 sekirei-halfkp-r6-v0.4.5.bin.xz
# 92e2b2ce67f5829b17752d4dba37cc278d7ae96e4e25d880fc68f7f9552ec38f
xz -d sekirei-halfkp-r6-v0.4.5.bin.xz
shasum -a 256 sekirei-halfkp-r6-v0.4.5.bin
# 634bdf527e7306045c8278df1d5311777561dfb09c1ebe6829ff3f4d883cc469
```

In a native USI GUI, set these options before `isready`:

```text
setoption name EvalFile value /absolute/path/to/sekirei-halfkp-r6-v0.4.5.bin
setoption name FV_SCALE value 24
setoption name NnueOutput value absolute
isready
```

Require `info string NNUE weights loaded from ...` followed by `readyok`.
The `FV_SCALE=24` setting is part of this model's operating conditions;
the engine's HalfKP default scale is 16. Retain the attribution and license
when redistributing it.

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

1. Verify the compressed and decompressed SHA-256 values against the release
   manifest and model card.
2. Check loading and scale on the actual release build. Record any source
   difference and its compatibility checks separately. Changes to evaluation
   or search require fresh strength evidence before extending the gate claim.
3. After publication, download the asset again and verify its bytes before
   declaring it shipped.

This evidence does not switch the default evaluator or make an external-rating
claim.
