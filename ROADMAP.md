# Sekirei — current roadmap

Internal only / gitignored. Updated 2026-10-10.

Historical detail is preserved in `tasks/archive/`, most recently:

- `roadmap-through-v0.3.66-20261010.md`
- `todo-through-v0.3.66-20261010.md`
- `lessons-through-v0.3.66-20261010.md`

Do not restart an old unchecked item without reconciling it with this file.

## Goal and evidence rules

The goal is to improve Sekirei until it can compete with leading shogi
engines while keeping the core Pure/Safe Rust.

- Search candidates are selected with Sekirei's material evaluator and
  Sekirei-generated openings.
- Suisho5 is not used for training, labels, candidate selection, defaults, or
  public artifacts. It may be used only to measure Sekirei against YaneuraOu.
- A point estimate, composite score, validation-loss reduction, or short match
  is not adoption evidence. Keep `PASS`, `FAIL`, `INCONCLUSIVE`, and
  resource-censored runs distinct.
- NNUE evidence records the exact file, successful load, SHA-256, evaluator
  mode, whole-game split manifest, and fixed gate conditions.
- Public claims come from tagged code, complete manifests, finished CI, and
  directly verified registry/Release artifacts.

## Current baseline and evidence boundary

- **Release candidate:** v0.3.68 (2026-10-10). It hardens USI game-boundary
  generation handling, option bounds, match/gate evidence, and raises the
  reusable Rust/USI coverage contract to 90%. Publication evidence is recorded
  only after the six crates and WASM archive are directly verified.
- **Runtime change from v0.3.67:** no search or evaluator default changes.
  Shutdown, `usinewgame`, and stdin closure now invalidate the active search
  generation before joining it. No new playing-strength claim is made.
- **Current defaults:** corrected quiescence stand-pat evaluation
  (`QS_STYLE=1`), non-pawn correction history (`CORR_W_NP=32`), and
  `IncrementUsePercent=75`.
- **Distributed evaluator:** the optional v0.3.38 checkpoint remains a
  compatibility artifact, not the recommended default.
- **Issue audit:** #114 is closed. Fixes for #113, #115, and #116 are complete
  on the current candidate branch and remain pending public merge.

### External evidence

- The official Floodgate listing for `sekirei-v066-gen10s` contains 10 games
  and a 7-3 record. It used v0.3.66 plus `nn_gen10s`; it is not a direct
  v0.3.67 measurement and is too small for a narrow rating claim.
- A model-based centre near 2820 with a broad 2600-3050 range is useful only
  as a planning estimate. Do not publish it as an official rating.
- WCSC36 placed Hisui first, dlshogi third, and Suisho fourth in the final;
  Sinfonietta was twentieth in the second preliminary. Tournament placement
  and architecture descriptions motivate priorities but are not controlled
  Sekirei comparisons.
- The external 82.6/100 composite assessment rates v0.3.67's reproducibility
  highly while keeping playing strength equal to v0.3.66. Treat that score as
  prioritisation input, not a measured engine metric.

### Self-NNUE evidence

- The latest private candidate is `nn_r2`, trained from `nn_r1` on 20.6M
  positions for one epoch.
- `nn_r2` vs `nn_r1`: 261-1-218 over 480 games, estimated +31 ± 30 Elo under
  the recorded private conditions. This is candidate-relative evidence, not a
  material-evaluator gate or public rating.
- The candidate remains private until its bounded-memory trainer, fresh
  whole-game hold-out, hashes, model card, and material gate are complete.
- EC2 is not justified until a self-NNUE candidate beats material evaluation
  in a preregistered colour-reversed local gate and CPU time, rather than
  correctness, teacher quality, or model design, is the remaining bottleneck.

## Dependency baseline

- Unreleased dependency update: `lineprior 0.12.3`, `shogiesa-core 0.11.2`,
  and external `shogiesa 0.11.2` wrappers.
- lineprior's prior-book schema remains v1. Its new artifact-lineage sidecars
  are optional and complement Sekirei's own opening-book manifest.
- shogiesa's typed position schema remains v11; the canonical fixture is byte
  identical to 0.11.0. Version 0.11.1 adds stricter validation, measurement
  harnesses, and a reused-USI command-order barrier.
- lineprior remains outside the default runtime unless opening-book support is
  enabled. shogiesa remains a training-data dependency/tool, not a search
  dependency. Neither update is itself a strength claim.

## Priority phases

### T0 — Denryu tournament readiness (P0, deadline 2026-10-24)

The [seventh Denryu tournament](https://denryu-sen.jp/denryusen/dr7_prd/dr1_live.php)
opens registration on 2026-10-25, closes it on 2026-11-08 23:59, and runs
online on 2026-11-14/15. Its asymmetric clock is Black 3 minutes, White 10
minutes, with a two-second Fischer increment. Treat the tournament as the
nearest operational gate; do not wait for a strength candidate before fixing
protocol correctness.

- [x] Make CSA receive fail closed with a 64 KiB line ceiling, retained partial
  bytes across a read timeout, explicit invalid-UTF-8/EOF errors, and complete
  draining of an oversized line before any subsequent command can be parsed.
- [x] Add `go infinite -> isready x2 -> stop` and
  `go ponder -> isready -> ponderhit` USI regression tests. `isready` must not
  stop search or emit a stale `bestmove` when no pending configuration change
  requires a barrier.
- [x] Prove search-worker shutdown on `stop`/`quit` for every USI backend and
  on adapter stdin EOF for multi-thread Lazy SMP. The CSA path searches
  synchronously; read/protocol failures are terminal before a new search can
  start. The input-loop EOF path now uses the same abort-and-join barrier as
  `quit` instead of detaching its `JoinHandle`.
- [x] Add an asymmetric 3 min/10 min + 2 s clock fixture; validate CSA 1.2.1
  time units, IDs, side assignment and START agreement; retain `%KACHI`,
  repetition and perpetual-check event lines; fail closed on disconnect,
  malformed summaries and duplicate final results; and refuse a move beyond
  the 512-ply boundary. Existing core tests continue to distinguish ordinary
  repetition from one-sided perpetual check; final adjudication remains the
  server's responsibility.
- [x] Run at least 14 consecutive local CSA games with restart injection and
  retain binary/model/config/source hashes, records, status transitions, and
  proof that no child process remains. On 2026-10-10 the release-profile
  candidate binary completed 7+7 loopback games across PIDs 84148 and 84149,
  resumed 0 -> 7 -> 14, wrote 14 hashed CSA records and 316 status events,
  and left no matching process after both children were reaped. Evidence is
  retained under ignored
  `data/rehearsals/denryu-20261010-codex/rehearsal-manifest.json`; it records
  binary SHA-256 `ba3276c...5ad51`, configuration SHA-256
  `89783268...0e16`, and the exact dirty-source identity. This is an
  operational candidate result, not tagged-release or strength evidence.
- [x] Produce a one-command offline launch bundle and recovery checklist;
  Slack notification is supplementary and never the sole failure detector.
  `scripts/run_denryu_rehearsal.py` is loopback-only, uses material evaluation,
  writes a self-contained evidence manifest, and documents fail-closed resume.

Completion: the exact tournament configuration survives the full fault matrix
and continuous-run rehearsal with no illegal result, clock inversion, stale
state, unbounded input, or orphan process.

### R0 — Validate dependency and provenance contracts

- [x] Build and test `lineprior 0.12.3` with book schema v1 compatibility.
- [x] Build and test `shogiesa-core 0.11.2` and the exact-version diagnostic
  wrapper against the unchanged schema-v11 fixture.
- [ ] Exercise expected-SHA checks on one train/validation pair, including a
  controlled byte-mutation failure.
- [x] Build one opening book from a frozen 80-game subset and verify the
  lineprior header, Sekirei sidecar manifest, and actual decision log from a
  fresh directory. A disjoint 20-game hold-out produced 0/20 selections, so
  the preregistered full gate was correctly not opened.
- [x] Export one prospective terminal gate observation and validate it with
  `validate_gate_observations`. The one independent group is contract-only;
  model fitting remains disabled until 20 groups exist.

Completion: all paths reproduce from fresh directories and fail closed after
an identity, schema, or lineage mismatch.

Validated on 2026-10-10 with the opening-book feature tests, the seven Python
diagnostic-contract tests, all release workspace tests, and clippy across all
targets and features. The unoptimised workspace run alone overflowed the test
thread stack in the deep-search `tt_reduces_nodes` case; the same test passed
in the release workspace run.

### R1 — Make one self-NNUE candidate release-ready

Owner boundary: Claude owns self-NNUE training, weights, and candidate
selection. Codex must not modify this lane concurrently; it may only preserve
the evidence contract needed by the search and release gates.

- [ ] Integrate and test bounded-memory/memory-mapped HalfKP training.
- [x] Refuse direct-path, hardlink, and symlink input/output aliases in the
  HalfKP trainer, and atomically replace network, float-state, and resumable
  checkpoint outputs only after a complete sibling-file write.
- [ ] Freeze a fresh whole-game hold-out before candidate selection; never
  reuse it for tuning.
- [ ] Record training hashes, float checkpoint, resume metadata, exported
  network SHA-256, `FV_SCALE`, and engine revision.
- [ ] Compare fixed-node judgement first, then run a fixed-time colour-reversed
  SPRT against material evaluation.
- [ ] Publish a model card only after a decisive PASS; otherwise retain the
  candidate as private/inconclusive.

Completion: a reproducible candidate decisively beats material evaluation or
is rejected without contaminating the hold-out.

### R2 — Reduce search cost and improve thread scaling

- [x] Repair the Q26 measurement contract: fixed-node runs no longer pass a
  depth cap that silently disables the node limit. Material-only and direct
  SFEN corpora are explicit, and stdlib tests cover fixed-node/fixed-time
  command construction.
- [x] Add opt-in SEARCH_V2 loop/re-search/qsearch/stage counters without
  charging production searches. On 20 standard material-only positions at
  depth 9, qsearch was about 40% of node calls and 93% of beta cutoffs were on
  the first move; only 57% of move-loop nodes reached the quiet stage.
- [x] Measure short material-only LazySMP scaling with a pinned 200 ms,
  Hash=64, no-book, `SpecTopN=0` contract: median NPS was 2.15M / 4.08M /
  8.01M at 1/2/4 threads, or 1.00x / 1.90x / 3.73x. This clears the old 3.1x
  diagnostic target but is not NNUE or playing-strength evidence.
- [ ] Re-measure move generation, do/undo, NNUE, SEE, move ordering, TT, and
  quiescence under one pinned 1/2/4-thread contract with an explicitly named
  evaluator and longer samples.
- [ ] Measure TT contention, worker-local history memory, useful-worker node
  share, cancellation latency, and discarded speculative nodes. Lazy SMP now
  marks the worker that supplied the returned result and reports its node
  share plus best-move agreement. An initial material-only 4-thread sample
  over eight Sekirei-generated openings at 300 ms found a selected-worker
  share of 24.4%..25.7% (median 24.85%); four positions agreed 4/4 on the move
  and four agreed 2/4. This establishes balanced worker cost, not 75% waste:
  helper TT contributions still need separate attribution before changing the
  parallel policy. A follow-up build recorded each worker's elapsed time,
  abort state, and abort source. On the same eight-position, four-thread,
  300 ms material-only sample, the last-worker duration exceeded worker 0 by
  0 ms in every position at millisecond resolution. There is no evidence of
  millisecond-scale stop lag in this sample; this duration spread is only an
  approximation, not a timestamp measured at abort-flag publication. A
  diagnostic-only `LazyFlags` bit 128 now divides the same total `Hash` budget
  among private worker TTs. In a 20-position ABBA measurement (40 paired
  observations per side, 300 ms, four threads, material-only), shared TT
  reached a greater selected depth in 20 pairs, tied 15, and lost 5, with a
  mean depth advantage of 0.45 ply (median 19 versus 18). A helper supplied
  the selected result in 28/40 shared-TT searches and 0/40 isolated searches.
  This supports retaining TT sharing; it is a depth diagnostic, not an Elo
  result, and does not yet attribute individual cross-worker TT hits. A second
  20-position ABBA compared the existing one-ply helper depth skew with no
  skew. The default was effectively tied: 15 deeper, 14 equal, 11 shallower,
  mean +0.05 ply (median 19 versus 18.5), while no-skew had slightly higher
  move agreement (3.075 versus 2.95 workers). Do not open a strength gate for
  removing the skew from this result.
- [x] Complete the full gate for `V2_STAGE_GEN=1`, which defers full quiet
  move generation. It keeps all decisions and diagnostic counters identical
  over 99 standard positions, matches filtered legal moves over four 128-ply
  deterministic playouts, and reduced a depth-11 material-only ABBA median
  from 380.45 ms to 365.15 ms (about 4.0%). The first material-only,
  Sekirei-opening, colour-reversed 100 ms screen was 19-1-12 (+77 ± 123 Elo),
  with 32/32 unique games and no invalid outcomes: positive but INCONCLUSIVE.
  A follow-up quiet-only generator stopped regenerating captures at the quiet
  stage while retaining the exact 99-position depth-10 tree; its 20-position
  depth-11 ABBA median improved a further 354.48 ms -> 348.11 ms (about
  1.8%). Core tests (226 passed, four ignored), workspace clippy, fmt, and
  diff checks pass. The preregistered 300 ms, one-thread, material-only gate
  against `V2_STAGE_GEN=0` finished 207-175-18 over 400 games, estimated
  +28 +/- 33 Elo, LLR +0.80 for SPRT [0, 10]: positive but INCONCLUSIVE.
  The A/A control was 11-9 over 20 games; the gate produced 398 unique games,
  two duplicates, no invalid outcomes, and no remaining process. Retain the
  exact-tree speed improvement, but do not claim a proven Elo gain.
- [x] Make the A/B wrapper material-only by default, omit `EvalFile` and
  `FV_SCALE` in that mode, and support repeatable per-side USI options. A
  two-game real-binary smoke passed with both option values recorded in the
  match manifest, two unique games, no invalid outcomes, and no remaining
  engine process. This validates the experiment path, not the candidate.
- [ ] Optimise only the largest measured cost, one factor at a time; require
  deterministic correctness plus a preregistered paired gate. The
  qsearch-mate-after-TT-miss experiment was rejected: identical trees and
  about 0.35% slower in its diagnostic ABBA run. Reusing the tactical-stage
  pin context in the deferred quiet stage was also rejected: its tree was
  identical but the 20-position ABBA median changed only 347.03 ms ->
  346.71 ms (about 0.09%), below a useful signal for the added coupling. A
  qsearch change that skipped `CheckSquares` construction when the move list
  was empty also preserved the 99-position depth-10 tree, but a 20-run-each
  ABBA was slightly slower (median -0.48%, mean -0.32%; faster in only 3/10
  paired cycles), so it was reverted. Caching the qsearch victim/history value
  already computed during ordering also preserved the exact tree, but its
  20-run-each ABBA was indistinguishable from zero (median and mean about
  +0.02%; faster in 5/10 paired cycles), so it was reverted too. `V2_SORT=1`
  changed the tree substantially: a 99-position depth-10 probe agreed on only
  2/98 decisions and used 1.5% more nodes, while a separate 20-position
  depth-11 corpus used about 4.6% fewer nodes with essentially unchanged wall
  time. Its 100 ms material-only screen was 35-27-2, but the preregistered
  300 ms confirmation was 61-62-5 over 128 unique games (estimated -3 +/- 60
  Elo) with no invalid outcomes. One incremental JSON snapshot hit a transient
  ENOSPC condition; the final JSON/JSONL and all 128 per-game records were
  saved, and the failure remains explicit in the final manifest. The
  short-screen direction did not reproduce, so keep the default `V2_SORT=0`
  and reject this candidate. A fresh eight-position, fixed-100k-node material
  profile selected TT probing as the largest independently timed leaf cost:
  median 6.10 ms for about 80,040 probes (about 76 ns/probe), with qsearch
  reported separately because it overlaps leaf spans. Explicitly expanding
  the fixed four-slot bucket probe preserved the search output but reproduced
  a slowdown: TT probe +0.62% and uninstrumented wall time +0.59% in the repeat
  comparison. The compiler-generated loop is retained. Q26 now exports its
  existing qsearch path counters. Across 16 profiled fixed-100k-node runs,
  qsearch searched 0.47 moves per call; 34.5% of calls ended at stand-pat,
  3.5% were in check, and top-level TT cutoffs occurred on 5.1% of top-level
  qsearch calls. A conservative 64-bit filter now skips the exact parent-line
  scan when fourfold repetition is impossible; Bloom collisions fall back to
  the prior exact check. It preserved best move, score, node count, bound, and
  PV in 160/160 depth-11 ABBA searches and 15/15 additional depth-10 positions.
  On the depth-11 ABBA sample it was faster in 65/80 pairs, reducing median
  wall time by about 2.4% and mean time by about 1.2%. Core tests passed
  (229 passed, four ignored) and release clippy was clean. This is an exact-tree
  local speed result, not an Elo claim.

Completion: one change improves fixed-time results and 4-thread scaling without
illegal moves, hash/PV regressions, or a same-node judgement regression.

### R3 — Turn opening-book logs into a controlled gain

- [x] Freeze the 100-game Sekirei self-play source into disjoint 80-game book
  training and 20-game hold-out groups, with source/run hashes and one unique
  replayed position per held-out game.
- [x] Preregister a one-ply actual-selection preflight before a colour-reversed
  book-on/book-off A/B gate. The result was 0/20 book selections, all
  `unseen_state`; status is `not_ready`, the full 400-game budget was not
  opened, and the only valid interpretation is `INCONCLUSIVE`.
- [ ] Improve book support on a new training-only corpus, then rerun the same
  coverage contract. Only a ready preflight may open a full gate that records
  score, time used, exit evaluation, fallback reason, and duplicate rate.
- [x] Keep opening-book generation and gate observations content-addressed;
  evaluate whether lineprior's optional lineage sidecar should be linked from
  or replace any duplicate Sekirei metadata.

Completion: a book policy passes without overlap between build and evaluation
games and retains a reproducible manifest/log chain.

### R4 — Narrow the external-strength estimate

- [ ] Run 30-50 v0.3.67 games with a frozen binary, `nn_gen10s` SHA-256,
  Hash, Threads, options, time control, and supervisor manifest.
- [ ] Include opponents around the observed lower, centre, and upper rating
  bands rather than extrapolating from one class of opponent.
- [ ] Save terminal status, parsed outcomes, CSA records, and proof that no
  client remains running before reporting completion.
- [ ] Treat the run as measurement and failure analysis, never as candidate
  selection.

Completion: report the observed record and uncertainty separately from any
model-derived rating estimate.

### R5 — Demonstrate the Rust-specific parallel-search advantage

- [ ] Define useful speculative work, cancellation delay, discarded nodes,
  task-creation cost, and synchronisation overhead as first-class counters.
- [ ] Validate sequential equivalence and 1/2/4-thread behaviour before
  scaling to 8/16/32/64 threads.
- [ ] Attempt the original under-5% synchronisation-overhead target only after
  correctness and local scaling gates pass.

Completion: a reproducible high-core run shows a fixed-time depth or score gain
and attributes the gain to useful speculative work rather than raw task count.

## Release gate

Release only after workspace tests, clippy, docs, opening-book feature, WASM
native/browser/package checks, USI smoke, strict manifest validation, six-crate
publication, registry checksums, GitHub Release asset digest, and final-main CI
all pass. A maintenance/reproducibility release must not imply a strength gain.

## Explicitly not current work

- Re-running historical v0.3.40-and-earlier candidates.
- Using external evaluation output as training labels.
- Treating same-engine self-play Elo as Floodgate or human rating.
- Treating the 7-3 Floodgate sample as v0.3.67 evidence without a pinned
  v0.3.67 run.
- Moving to EC2 before the local self-NNUE gate and provenance conditions are
  satisfied.
