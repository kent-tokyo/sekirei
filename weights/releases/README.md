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

## Evidence intake for `nn_r6`

The repository deliberately does not contain the candidate weight, private
hold-out positions, or raw training shards.  The NNUE owner supplies a separate
staging bundle when deciding whether to distribute this candidate.  That bundle
must contain only the following public or redacted evidence:

```text
model-card.json
NNUE-LICENSE.md
weights/nn_r6.bin
attestations/training-manifest.json
attestations/whole-game-holdout-manifest.json
gates/one-thread-result.json
gates/one-thread-raw.tar.zst
gates/four-thread-result.json
gates/four-thread-raw.tar.zst
```

The records already kept in this directory are inputs to that decision, not a
substitute for this bundle:

| Record | What it establishes | Why it cannot publish r6 |
|---|---|---|
| `sekirei-halfkp-nn_r6.json` | The artifact identity and fixed-source 1T/4T result summary. | It is a release plan, not a v2 model card with raw-evidence bindings. |
| `nn_r6-main-compatibility.json` | A 12-position native compatibility check and 4T load smoke. | It is not a colour-reversed strength gate. |
| `docs/experiments/nnue_r6_material_gate_main06490fc/` | An owner-supplied one-thread result on an older source. | It has no matching four-thread condition or current-source qualification. |

The two attestation manifests may disclose hashes, counts, generation commits,
and the whole-game split method; they must not disclose private shard or
hold-out contents.  Each current-source gate must use a separately frozen,
unspent opening corpus, material-only baseline, colour-reversed pairs, and the
same candidate bytes named by the card.  The one-thread condition must be a
`PASS`; the four-thread condition must be a predeclared `NON_REGRESSION` or
`PASS` accepted by the strict contract.

Before upload, validate the bundle against the exact full engine commit used
for those gates:

```bash
python3 scripts/validate_nnue_release_artifact.py model-card.json \
  --root /path/to/nn-r6-bundle \
  --expected-engine-commit <40-hex-commit>
```

An exit status of zero is necessary, not sufficient: it checks the evidence
envelope and hashes, while the NNUE owner remains responsible for the truth and
scope of the underlying records.  The historical records above and the
12-position compatibility receipt cannot substitute for these current-source
gates.
