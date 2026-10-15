# Quiet TT staged generation (default off)

`V2_QUIET_TT_STAGE` remains **0**. This experiment reduces work spent generating
and ordering moves that an earlier cutoff makes unnecessary. It changes no
pruning rules, evaluator, or Entry/Frame/Board layout.

## R1: correctness passed, speed gate rejected

R1 used source base `eea038febf94843deff606aa2449e37e62f352f7`.
An independently validated quiet TT hint outside check could use staged
generation; its quiet siblings were generated only when needed, with the hint
excluded from their list. Tactical generation stayed before TT search for
ProbCut. Invalid hints and `SKIP_NONPROMO` exclusions kept the previous path.

Completed local checks:

- 403 ordinary core tests passed; 6 ignored. Formatting and default/tune
  release clippy passed.
- 10,000,000 deterministic legal-playout position visits compared every legal
  move and 16 mutated/random candidates against full generation, including
  tactical/quiet subset order and restoration. These are visits, not unique
  positions; seed `0x6a09e667f3bcc909`.
- 144 fixed-depth searches compared material and own random11 HalfKP,
  both colors, stage generation 0/1, `SKIP_NONPROMO` 0/1/2 and warmed or invalid
  quiet TT hints. Bestmove, score, PV, nodes, depth and bounds matched; root
  SFEN/hash/accumulators were restored.
- Fresh same-path native controls differed only in compiled parameter 0/1.
  The 72-process correctness check covered 31 exact 1T pairs, four isolated
  4T worker-zero pairs and both arms' 4T lifecycle/PV checks.

The first own4T H1 attempt stopped after ENOSPC with **0 accepted calls**.
A separate fresh run completed **128 calls, all 32 positions eligible**:

| Own4T metric | R1 result |
| --- | --- |
| Candidate/baseline aggregate nodes per wall second | 0.9812500414613538 |
| Position-bootstrap 95% interval | [0.9673090186838624, 0.9950027925975671] |
| Speed gate | Rejected |

The default stays off. No partial timings were pooled and no external
comparison was run. Correctness passing does not establish a speed gain.

## R2: correctness passed, speed gate rejected

R2 uses source base `a2619c52b9bba72ec8dcac48adc3eae825833cd4`.
It limits quiet-TT staging to **non-PV nodes**. Tactical generation remains
before ProbCut and TT search. Ordering keys are saved before the TT child or
singular search can update history; SEE classification and winning-tactical
sorting are delayed until after the quiet TT move. Original good-list input
order/length and bad-list order are preserved. `V2_EAGER_LOSING_SORT=1` retains
the eager classification path.

R2 also reuses valid, same-side, empty-checker king constraints after TT
validation. Missing or unsuitable cached constraints use the established
outside-check scan. Ordinary generation and cache invalidation remain
unchanged. Single-move validation shares movement, promotion, pins/evasions,
king safety, nifu and uchifuzume predicates with full generation.

R2 completed 417 ordinary core tests (6 ignored), the 144-search
material/own-HalfKP identity matrix, 10,000,000 legality differential visits
and fresh native 72-process correctness. These are R2-specific checks; R1 proof was
not substituted.

Its fresh own4T H1 completed all 128 accepted calls. The whole-quartet 700ms
filter excluded 2 of 32 positions, leaving 30 eligible positions:

| Own4T metric | R2 result |
| --- | --- |
| Candidate/baseline aggregate nodes per wall second | 0.9610156454176877 |
| Position-bootstrap 95% interval | [0.9498141602729777, 0.9724387164561369] |
| Speed gate | Rejected |

The default remains 0. No external comparison was run and no default
adoption or playing-strength claim follows. The conditional external adapter
rejects this completed failed-gate report before model access or engine
startup.

## Required evidence and reproduction

Pure-speed qualification requires fixed-depth bestmove/score/PV/nodes/depth/
bounds identity and root restoration. The integration stage counter is a
stage boundary, not a quiet-TT execution count. R2 adds a test-only cache-hit
counter in the actual non-PV generator; it is not used in NPS measurements.

Each revision needs its own native controls, correctness proof and fresh
own32 H1, excluding prior corpora. Counter-free 4T searches use Hash64/FV16,
1 second and per-position ABBA. All 128 accepted calls are required; the
whole-quartet 700ms filter must leave at least 8 positions. The gate requires
ratio >=1 and a 5,000-draw position-bootstrap 95% lower bound >1
(seed 20261014). The host protocol retains five quiet samples, 600s waits,
three whole-quartet retries and a 512MiB disk floor. Incomplete runs are not
pooled. All source/binary/model/proof inputs are frozen.

Run tuning/network tests in separate processes because their state is global:

```sh
cargo test --release -p sekirei-core --features tune --test quiet_tt_stage \
  -- --ignored --nocapture --test-threads=1
SEKIREI_LEGALITY_POSITIONS=10000000 cargo test --release -p sekirei-core \
  --lib movegen::single_move_tests::single_move_legality_10m_position_differential \
  -- --exact --ignored --nocapture --test-threads=1
cargo test --release -p sekirei-core --lib \
  movegen::single_move_tests::single_move_checking_pawn_drop_restores_active_halfkp \
  -- --exact --ignored --nocapture --test-threads=1
cargo test --release -p sekirei-core --features tune --lib \
  search::v2::tests::quiet_tt_stage_selects_only_enabled_legal_quiet_hints \
  -- --exact --ignored --nocapture --test-threads=1
```

Raw proof files remain local. Correctness, raw NPS and playing strength are
separate claims; this document makes no YaneuraOu or playing-strength claim.
