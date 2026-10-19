# `nn_r6` release evidence

`nn_r6` is not attached to v0.4.5. It is separate from source crates and
executables; material remains the fallback when no `EvalFile` is set. The
[release plan](sekirei-halfkp-nn_r6.json) binds the fixed-source 1T/4T gates
and compatibility receipt to the exact weight identity, but it is not a
validator-accepted public artifact card.

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

## Publication requirement

Publication requires a release card that passes
`validate_nnue_release_artifact.py`, binds the owner attestation, licence,
artifact and raw gate evidence, and is verified again after upload. This record
does not switch the default evaluator or make an external-rating claim.
