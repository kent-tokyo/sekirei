# Eight-thread CPU speed candidates

These independent experiments did not establish an 8-thread speed improvement. Both integrated search parameters remain **0 (disabled)**: `V2_QUIET_LMP_SKIP` and `V2_SEE_BOUND_CACHE`. The private EvalCache candidate was tested on an older source revision and is not integrated here. This is an evidence record, not a default-adoption or playing-strength claim.

## Independent own-evaluator results

Each row compares one candidate against its own same-workspace native baseline. Ratios are candidate/baseline raw NPS; positive percentages mean faster. The rows use separate held-out corpora and are not pooled or multiplied.

| Candidate | Source base | Accepted calls / eligible positions | NPS change | Bootstrap 95% interval | Independent H1 |
| --- | --- | --- | --- | --- | --- |
| Worker-private Relaxed EvalCache | `2499c485d3fbab231e1695e2ea1537705b1b5245` | 128 / 32 | +0.2764% | −0.6158% to +1.1514% | Failed; code not integrated |
| Skip preparation of quiet moves already covered by LMP | `a76962e316f3fd9389bab98c54f6538227b5b0a8` | 128 / 32 | +0.0071% | −0.8778% to +0.8971% | Failed; default 0 |
| Retain a proven non-negative SEE bound | `a76962e316f3fd9389bab98c54f6538227b5b0a8` | 128 / 30 | −1.1628% | −1.9835% to −0.2162% | Failed; default 0 |

The first two intervals include no change. The SEE candidate was slower within this experiment's interval. These results do not establish a general hardware-independent effect. None passes the preregistered gate: point ratio at least 1, **95% interval lower bound strictly greater than 1**, and at least eight eligible positions.

## Conditions and interpretation

- Apple M4 with 10 logical CPU cores (4 performance and 6 efficiency cores); `Threads=8` and `RAYON_NUM_THREADS=8`. Eight threads do not imply eight performance cores or fixed core affinity.
- CPU-only, fresh process and one `go movetime 1000` per call. Each independent H1 uses 32 own legal-playout positions, one baseline→candidate→candidate→baseline quartet per position: 128 accepted calls. Host-rejected attempts are retained separately and not pooled into accepted timing.
- Own deterministic synthetic HalfKP evaluator, SHA256 `60db2e5a2888d4fb72ff45431e009f504ae0c86e8b7c8a14fb69a4b10b11c20e`; `FV_SCALE=16`, `Hash=64 MiB`, `SearchMode=Auto`, `SpecTopN=0`, `MoveOverhead=0`, diagnostics **off**. Synthetic random weights exercise CPU work; they provide no strength evidence.
- Normal native ThinLTO builds, identical build path/target/compiler/flags per pair, no tuning feature or PGO. Quiet/SEE compare only their parameter's compiled default 0 versus 1, with LazyFlags 15 in both arms. The private-cache row compares LazyFlags 15 versus 65551 on its frozen source-2499 pair; it is not a current-main result.
- The per-arm arithmetic mean NPS within each quartet forms a candidate/baseline ratio; the aggregate is the geometric mean across eligible positions. The 95% interval uses 5,000 position-bootstrap draws with seed 20261014.
- A whole quartet is excluded from timing if any elapsed time is below 700 ms. All 128 calls were retained for each capture; SEE positions 9 and 21 were filtered by that predeclared rule, leaving 30 eligible positions. No retry is chosen by speed, score or depth.
- Corpora exclude the relevant development/training and earlier hold-outs by board/hand/turn. Position generation uses own legal play, not evaluator scores or previous timing results. External weights, evaluator outputs and derived labels are not used for candidate selection.

Raw NPS, completed depth/useful search work, and game results remain separate. This document establishes neither an advantage over YaneuraOu nor an Elo change. No new external comparison was run for these failed gates.

## Integrated mechanisms and correctness scope

`V2_QUIET_LMP_SKIP` preserves legal quiet generation and filtering, terminal-detection bookkeeping, excluded/TT move handling and the count passed to later tactical moves. It skips scoring/check/SEE/sorting only when the first non-excluded quiet already reaches the existing LMP threshold. Empty/all-excluded lists and an already-skipped quiet stage retain their original state. Diagnostic counters keep their definitions; avoided scoring can change their values.

`V2_SEE_BOUND_CACHE` uses bit 3 of the existing Entry flag byte, without increasing its layout. Only the existing fast tactical classification may retain the proven lower bound SEE ≥ 0. A comparison against a non-positive threshold can then return false without exact SEE; a positive threshold or unknown value requires exact SEE, while an already cached exact value is reused. TT and quiet entries do not gain an unsupported proof. Move ordering and pruning thresholds are preserved.

The quiet-LMP and SEE candidates each separately passed fixed-depth identity checks and a fresh 72-process native correctness capture: 31 single-thread exact pairs, four isolated 8-thread pairs with exact selected-worker-0 signatures, and lifecycle cases for both arms. The older private-cache candidate has its own source-2499 proof: four isolated 8-thread pairs plus four persistent/nonpersistent lifecycle cases (12 fresh processes); it is not substituted for the a769 proofs. Total helper/worker nodes in shared parallel searches are not required to match. Legal PV replay, cancellation/reset/quit, evaluator activation and normal process teardown are checked. The two integrated candidates each have an ignored tune integration test that compares material first and own `HalfKpNetwork::random(11)` second in an isolated process, with fresh histories/TT/cache and exact root restoration. Coverage invokes these tests explicitly in separate processes while retaining the existing 98% threshold and exclusion regex.

The merged integration separately passed 484 core/USI tests (four ignored), both isolated tune integration tests, formatting, and core/USI release Clippy with and without tune. That is 486 successful test executions; the ignored stress tests were not run. The local correctness runs used unoptimized builds with debug assertions and overflow checks, with debug information disabled to limit disk use. Candidate-native correctness was checked separately in the optimized binaries. The full coverage job and hosted CI were not run locally or confirmed here; the 98% coverage contract and exclusion regex are unchanged. The combined enabled configuration is not speed-qualified; defaults remain disabled.

## Local evidence bindings

Complete raw reports and frozen input manifests are retained in ignored local experiment artifacts. They are not uploaded with this document, and their presence is not hosted CI proof. The aggregate above is bound to these report SHA256 values:

| Independent capture | Report SHA256 |
| --- | --- |
| Private EvalCache, source 2499 | `576af3c0a7a5b58dd1a33e26fcccec4b109689422adbaa7a731872bed6e01dbd` |
| Quiet-LMP, source a769 | `d0bb8cccd4168f25066c4f4b7733a165958a3e025f61e6201979f1f9603b9cb3` |
| SEE-bound, source a769 | `25a191ed7fbcb622712916ff5b449e9e4008279c0f8cda3dbe170dbcd63f16f1` |
