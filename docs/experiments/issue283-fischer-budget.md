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

## D01: next development diagnostic, registered before position selection

Use the SHA-verified public losses numbered 6, 7, and 8 from the old r6 trial, excluding sailfish2 game 5. Reuse the existing `sekirei-history-replay --export-json ... --player sekirei-v051-r6` outputs and S17 receipt SHA-256 `8edfa88ab0263d5544f50f7ce106e560bf95a975ff1c6b8fb76b4e72a56cf551`, plus the per-move clock trace SHA-256 `985361c8a04cf40274964931320e91c0fe2c536218eae5220c971cf951d843c5`. For each game, select the first Sekirei-to-move position with zero-based CSA `ply >= 30` and candidate search budget at least 500 ms above legacy. Match the nth Sekirei-to-move position to the nth own-move clock row. Stop input preparation if any game has no eligible position, hash mismatch, missing legal history, or an overlap with S18's eight positions (plies 12/14, 9/13, 9/11 for games 6/7/8). The selection uses only game, ply, clock, and player side, never the played move, later result, or evaluation.

The next run, if separately admitted, uses one source-identical normal USI binary with r6 SHA-256 `634bdf527e7306045c8278df1d5311777561dfb09c1ebe6829ff3f4d883cc469`, `FV_SCALE=24`, absolute NNUE, one worker, Hash 64 MB, no book, `SpecTopN=0`, and full `initial_sfen` plus `history_before_usi` from the replay. For each of the three positions, cold-start one legacy-budget and one candidate-budget search, each with exactly the milliseconds calculated from the observed pre-move CSA bank. The binary and source hashes, loader response, complete search metadata, legal best move, and actual wall time must be recorded. A technical failure or overrun stops the diagnostic; no time or position is changed after observing a move. The maximum is six searches, one worker, 180 seconds total, 35 seconds per call, 32 MiB output, with no concurrent engine/training job. Check free disk, memory, and other jobs immediately before starting.

Move quality is assessed only if the two budgets return different legal moves. On those positions, use fresh processes of the same frozen r6 source to force each selected root move at a common 1,000,000-node limit, at most six further calls / six million nodes / 90 seconds. Compare root-perspective scores only when both forced-root results have valid completed exact bounds and legal PVs. Predefine candidate improvement as at least +50 cp, worsening as at most -50 cp, and otherwise a tie; incomplete or non-exact pairs remain unresolved. Report counts and each case, including the largest-change removal sensitivity. This is a development teacher with the same evaluator, so it is not an independent strength qualification. The two phases together have a 270-second wall ceiling. No Suisho5 executable, weight, score, or derived label is used for input selection or judgment. No default change follows from this three-game diagnostic alone.

### D01 input result and D02 rule

The D01 read-only input check found no eligible `ply >= 30` position in game 6. It stopped before freezing a corpus or launching a search. This is an input eligibility failure, not a policy verdict; D01's rule remains above.

For D02, keep games 6/7/8, the same public hashes, clock-difference threshold, and S18 exclusions. Select the first Sekirei-to-move position with zero-based `ply >= 16` and candidate budget at least 500 ms above legacy in each game. This lower bound stays beyond the S18 plies 12/14, 9/13, and 9/11. Stop if any game has no eligible position. The search budgets, quality rule, and resource ceilings above apply to D02 unchanged; D01 data are not pooled with D02. Select by clock and replay metadata before reading move scores or running search.

### D02 input result (read-only; no search)

The [frozen inputs](issue283-d02-inputs.json) have SHA-256 `0d77cc6290f9336e69c201f8d95d5cbc34e114eb8960d2e9d81aa5b271144b6d`. The [selection script](issue283_freeze_input.py) checked the results manifest, all three public CSA hashes, the S17 replay receipt and replay JSON hashes, the clock trace hash, legal replay completion, own-move/clock-row counts, and zero SFEN overlap with S18. It did not inspect scores or search results.

| Public loss | CSA ply / own move | Bank before move | Legacy / candidate budget |
| --- | ---: | ---: | ---: |
| [AobaZero, game 6](https://wdoor.c.u-tokyo.ac.jp/shogi/x/2026/10/21/wdoor+floodgate-300-10F+sekirei-v051-r6+AobaZero_w4778_n_p800+20261021090001.csa) | 16 / 9 | 241 s | 16,033 / 17,050 ms |
| [Panda, game 7](https://wdoor.c.u-tokyo.ac.jp/shogi/x/2026/10/21/wdoor+floodgate-300-10F+Panda+sekirei-v051-r6+20261021100003.csa) | 17 / 9 | 246 s | 16,200 / 17,300 ms |
| [Gokaku, game 8](https://wdoor.c.u-tokyo.ac.jp/shogi/x/2026/10/21/wdoor+floodgate-300-10F+Gokaku-v2.4-b10c512-v500+sekirei-v051-r6+20261021103001.csa) | 17 / 9 | 246 s | 16,200 / 17,300 ms |

These are distinct replayed positions, but they come from three games already represented in S18, only 2–6 plies after S18's selected positions. They are a development sample, not an independent game-level confirmation set. The candidate budget difference drove eligibility prospectively, so the sample measures the policy where it acts; no later move or quality outcome entered selection. The displayed budgets are requested search times calculated from historical CSA clocks, not observed candidate search times. D02 has not been executed.
