# Issue #283: Fischer clock candidate (2026-11-14)

## Contract frozen before implementation

The old 300+10F rule requests 18,000 ms at a 300,000 ms bank. Historical public CSA accounting shows 433 Sekirei moves at mean 11.67 s and 169–306 s calculated bank after the last own move in ten games. These are observed CSA `T` values, not search-only times. The existing S18 500/1500 ms comparison changed two moves in eight early positions but did not establish quality.

Candidate `bank-reserve` is opt-in. For Fischer time only, request the increment less a transport reserve (at least 20% of the increment and 200 ms), plus one twentieth of the bank above 60 seconds. At 300+10F this requests 20 seconds initially, 13.5 seconds at 170 seconds bank, and 8 seconds at 60 or zero seconds bank. Non-Fischer and default `legacy` remain unchanged. The candidate is a testable hypothesis, not a default or strength claim.

| Bank before this move | Legacy request | Candidate request | Calculation |
| ---: | ---: | ---: | --- |
| 300 s | 18 s | 20 s | 10 s increment - 2 s transport + (300-60)/20 s |
| 170 s | 13.666 s | 13.5 s | 10 - 2 + (170-60)/20 s |
| 60 s | 10 s | 8 s | no surplus above bank reserve |
| 0 s | 8 s | 8 s | 10 s increment minus 2 s transport reserve |

`time_left_ms` is the bank before this move's increment. The CSA echo update adds the increment before subtracting the server's `T` usage, so the candidate uses the same clock convention. In the 200-move simulation each move consumes its full search budget plus the 2 s transport allowance, and the calculated bank stays at or above 60 s. Actual network delay and server adjudication have not been measured; delays above the allowance can still exhaust the bank in a long game.

GitHub `main` was `2ea51086` at the read-only check; its `protocol.rs` blob `15cfc1b6` matched the local base `149dd3f` blob. The current public source has no competing Fischer policy implementation.

## Public CSA replay

Run `python3 docs/experiments/issue283_clock_trace.py SOURCE_DIR docs/experiments/issue283-clock-trace.jsonl` with the verified v0.5.1+r6 trial directory as `SOURCE_DIR`. The script requires results manifest SHA-256 `fba23cb84d664e21eb09e70494929d253ce6874890d0e686e0329414ecf75998` and checks each public CSA against its manifest hash. The [433-move trace](issue283-clock-trace.jsonl) has SHA-256 `985361c8a04cf40274964931320e91c0fe2c536218eae5220c971cf951d843c5`. Its first line records the schema and source hashes, followed by one JSON object per move. It records the bank before and after each observed move, the server `T` value, and the old and candidate *requested* budgets. All ten move counts, used-time totals, and final calculated banks match the retained S16 audit. On those observed bank states, the candidate requests more on 243 moves, less on 185, and the same on 5. These candidate values are counterfactual and do not predict the future bank or the actual used time after changing the policy.

Smallest check: unit cases at bank 300, 170, 60, and zero seconds, plus deterministic 200-move full-budget clock simulation. Require each candidate budget to be at most increment minus reserve plus one twentieth of bank surplus, leave the transport allowance on the current turn, and retain a 60 s bank after every simulated move including the transport allowance. Keep the full `sekirei-csa` test suite and Clippy under two minutes. Stop on candidate-specific compile or test failure; no match or training.

Observed checks: the two new targeted unit tests and the run-manifest contract test passed; `cargo clippy -p sekirei-csa --all-targets -- -D warnings`, `cargo fmt --all -- --check`, and `git diff --check` passed. The full CSA test command compiled and ran 51 passing tests, but 28 pre-existing loopback-server tests failed at socket creation with sandbox `Operation not permitted`; this environment cannot validate those tests. No search or match was run.

Before any default change, freeze fresh CSA positions outside the S18 sample and an independent move-quality judge. Compare legal moves at the old and candidate time budgets with the same source, r6 weights, verified 4-worker Rayon setting, Hash 256 MB, history, and host controls. The CSA client has no `--threads` argument: it calls `Searcher::search`, whose young-brothers work can use Rayon workers. Verify the actual launch environment before calling a run 4T. Require improved and worsened counts and outlier sensitivity, then a separate independent strength qualification. This work supplies no such move-quality or strength result.
