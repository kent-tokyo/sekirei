# Changelog

This file summarizes recent releases. The
[archived detailed history](CHANGELOG_ARCHIVE.md)
preserves older per-change notes.

## [Unreleased]

## [0.3.62] – 2026-10-07

- NNUE training: export feature-transformer weights and biases by rounding to
  the nearest integer instead of truncating toward zero. For finite unclipped
  values, the exported parameter error is now at most half a quantisation step;
  signed boundary cases and existing integer-checkpoint compatibility have
  regression tests.
- Existing weight files remain load-compatible and unchanged. This release
  does not adopt a new checkpoint or make a new playing-strength claim.

## [0.3.61] – 2026-10-07

- NNUE data preparation: add stable source-game identifiers to `gensfen` and
  a deterministic train/validation splitter that keeps each game in one arm,
  joins games sharing an exact SFEN, can cap positions per game, records a
  hashed split manifest, and rejects legacy rows that cannot prove isolation.
- Search research: add disabled-by-default options for an evaluation
  correction keyed by each side's pieces other than pawns and the king
  (`CORR_W_NP`) and for continuation histories split by whether the earlier
  move was made in check and captured (`V2_CONT_SPLIT`). With them off,
  search results are unchanged.
- Search research: expose the quiescence delta-pruning margin (`QS_DELTA`)
  and 24 more SEARCH_V2 constants (reduction adjustments, singular and triple
  extension margins, history offsets, full-depth thresholds) as tuning
  options. Their defaults are the previous values; moves and node counts are
  unchanged.
- Lazy SMP: add `LazyFlags` bits 32 (report worker 0's result) and 64 (among
  the deepest workers, take the lowest-numbered one instead of the highest
  score). `LazyFlags` accepts values up to 127. Defaults are unchanged.
- This change set makes no playing-strength claim.

## [0.3.60] – 2026-10-06

- Search: make SEARCH_V2 (the redesigned search below the root, with its own
  move histories, pruning, reductions, and extensions) and its root move
  ordering the default. A pre-registered, material-only paired-trinomial SPRT
  of SEARCH_V2 with root ordering against the 0.3.59 main search (0.5 s per
  move, one thread) accepted H1 at 102-34-16 over 152 games (LLR +2.95;
  H0=0 Elo, H1=+20 Elo, alpha=beta=0.05). Its tables now belong to each
  searcher (one set per Lazy SMP worker) rather than to the running thread.
- Search research: expose the remaining SEARCH_V2 constants (reductions,
  razoring, null move, ProbCut, tactical futility, double extensions,
  re-search margins) as tuning options, and use tuned values for twenty of
  its history, pruning, exchange, and reduction constants.
- Tests: the Lazy SMP agreement controls use a position with one clearly best
  move, so their result no longer depends on how equal scores are broken.
- Search research: add disabled-by-default options for a two-move evaluation
  correction (`V2_CORR_CONT_W`), quiet promotions in quiescence
  (`QS_PROMO`), a SEARCH_V2-specific quiescence (`V2_QS`), and a
  score-and-depth vote among Lazy SMP workers (`LazyFlags` bit 16).
- Release tooling: add `scripts/verify_release_publication.py`, which checks
  the published crates and WebAssembly asset against a release manifest and
  records the verified publication.
- This release does not bundle a new NNUE checkpoint or make a general
  playing-strength or external-engine superiority claim.

## [0.3.59] – 2026-10-05

- Transposition table: make the four-entry cache-line bucket layout the
  default. A pre-registered, material-only paired-trinomial SPRT against the
  direct-mapped layout accepted H1 at 283-211-40 over 534 games
  (LLR +3.00; H0=0 Elo, H1=+20 Elo, alpha=beta=0.05).
- Evaluation performance: use dense 8-bit HalfKP inference on AArch64 and
  retain per-king-square accumulators for king-move refreshes. Regression
  tests require identical integer scores.
- Search research: add an opt-in `SEARCH_V2` implementation, root ordering,
  staged move ordering, cached exchange values, and diagnostic controls.
  SEARCH_V2, FL_MOVE, adaptive time management, and the deeper null-move
  reduction remain disabled by default because their release gates are
  incomplete or inconclusive.
- Diagnostics: correct node accounting so an alpha-beta leaf followed by
  quiescence is counted once, and extend cutoff, PV, move-kind, and TT probes.
- Correctness: strengthen the lock-free evaluation cache seqlock ordering so
  concurrent colliding writes cannot expose a score from another position.
- WebAssembly: expose typed bounded position analysis with explicit score,
  bound, mate, terminal, fallback, and abort metadata while preserving the
  existing browser move API.
- This release does not bundle a new NNUE checkpoint or make a general
  playing-strength or external-engine superiority claim.

## [0.3.58] – 2026-10-04

- Transposition table: add optional four-entry cache-line buckets with
  generation-aware replacement, preserve a useful stored move when a
  same-position update has no move, and expose deterministic layout tests.
  The direct-mapped layout remains the default because the clean local gate
  was positive but statistically inconclusive.
- USI: accept the standard `USI_Hash` spelling as an alias for `Hash`. The
  optional `IncrementUsePercent=75` setting was positive but inconclusive in
  the clean local gate, so the default remains zero.
- Search diagnostics: add disabled-by-default controls for history updates,
  continuation history, pruning bounds, internal iterative reduction, LMR,
  and shallow TT-bound reuse. QST, the deeper null-move reduction candidate,
  and adaptive time-management mode are not enabled in this release.
- USI: make opening books an explicit `opening-book` Cargo feature and runtime
  opt-in. The default engine no longer depends on `lineprior`, advertises book
  options, or probes a working-directory-relative file; enabled books report
  versioned provenance and fall back to normal search on load failure.
- Training: define shogiesa/Quietset runs as diagnostic-weighting pipelines.
  They validate shogiesa 0.11.0/schema 11 provenance and achieved depths,
  retain Sekirei internal search as the sole teacher, link the label manifest
  by verified SHA-256, and account for external labeling and internal training
  time separately. This is a reproducibility change, not a strength claim.
- This release does not bundle a new NNUE checkpoint or make a general
  playing-strength or external-engine superiority claim.

## [0.3.57] – 2026-10-03

- Dependencies: update `lineprior` to 0.12.1. Newly generated opening books
  now include the schema-v1 producer version and complete build configuration;
  headerless and legacy books remain readable.
- Dependencies: update `veridict` to 0.19.1. SPRT output and verdict sidecars
  now report the retained first-boundary-crossing LLR while preserving the
  full-input aggregate separately.
- Data pipeline: verify the external `shogiesa 0.11.0` CLI contract end to end
  with CSA extraction, Sekirei labeling, and quietset-input flattening. The
  training crate now consumes its versioned JSONL through the training-only
  `shogiesa-core 0.11.0` typed schema, streams records line by line, reports
  rejected metadata explicitly, and supports known `game_result` values for
  `--positions --wdl-lambda`; engine/runtime crates remain independent.
- WebAssembly: add bounded shortest-mate analysis for odd depths through 15
  plies with explicit node limits and deterministic incomplete-result handling.
- Measurement tooling: require an explicit material-only evaluator contract
  for SPSA, preserve it across resume, add per-side thread/search-mode controls
  to A/B matches, and add a thread-scaling NPS diagnostic.
- This release has no new NNUE weights and makes no general playing-strength
  or external-engine superiority claim.

## [0.3.56] – 2026-10-03

- Search: add independently switchable ordering, reduction, extension,
  transposition-table, and pruning controls. Seven history-driven defaults now
  strengthen quiet-move history, late-move reductions, and reduced-depth
  futility pruning; the remaining experimental controls stay disabled.
- Validation: a pre-fixed local self-play SPRT against the previous defaults,
  at 0.5 seconds per move, one thread, six parallel games, balanced openings,
  H0=0 Elo, H1=+10 Elo, and alpha=beta=0.05, accepted H1 at 239-139-16 over
  394 games (LLR +3.03). Four-thread evidence remains inconclusive, so this is
  not a general playing-strength claim.
- Match tooling: support Fischer base time and increment. USI adds the optional
  `IncrementUsePercent` control, which remains disabled by default.
- Training diagnostics: add self-play SFEN generation and HalfKP record packing
  examples for Sekirei's own evaluator pipeline.
- WebAssembly: add the browser-only `sekirei-wasm` package with stateless SFEN
  parsing, legal USI moves, validated move application, structured errors, and
  deterministic material-only search bounded to depth 8 and 100,000 nodes;
  expose fixed worker capabilities and complete legal mate-in-one validation.
- CI: run the API in headless Chrome and build a reproducible `wasm-pack`
  package containing the MIT, Apache-2.0, and NOTICE files.

## [0.3.55] – 2026-10-01

- USI: make `SearchMode=Auto` select sequential search for one thread, Lazy
  SMP for multiple threads, and the root-candidate backend for `MultiPV>1`;
  default `SpecTopN` to zero.
- Parallel search: keep Lazy SMP workers on their sequential inner search path
  to avoid nested YBW pool contention.
- CSA: parse standard `,T<seconds>` move suffixes, reject malformed server
  moves, and add bounded `--max-games` operation with durable attempt and stop
  reason records that exclude credentials.
- Diagnostics: complete the bounded shared-TT write-topology and Lazy SMP
  provenance audits; reject evaluator replacement before reading another
  weights file.
- The release has no new NNUE weights and no general playing-strength or
  external-engine superiority claim.

## [0.3.54] – 2026-09-29

- Search: skip selected non-promotions below the root, and distinguish safe
  checks from hanging checks when applying shallow pruning and reductions.
- Search: add capture, continuation, follow-up, and correction histories, a
  per-ply search stack, and depth-aware pruning and extension controls.
- Tuning: centralize search constants in `search::params`; the optional
  `tune` feature exposes bounded `T_<NAME>` USI options, and `scripts/spsa.py`
  drives reproducible local parameter searches.
- The release has no general strength or external-engine superiority claim.

## [0.3.53] – 2026-09-29

- Search: detect an immediate mating move before pruning can hide it at eligible
  main-search nodes and the quiescence entry. Positions already represented in
  the transposition table skip the scan.
- Move generation: add a lower-cost direct-check mate-in-one scan, plus
  `mate_bench` and `mate_scan` examples for local solver-cost and game-record
  diagnostics.
- A pre-registered local S29 SPRT against v0.3.52, using 0.5 s per move, 200
  balanced openings with colours swapped, one thread, and a pinned external
  HalfKP evaluation, accepted H1 at 793-672-59 over 1,524 games (LLR +2.99).
  This is a local, scoped diagnostic, not a general strength claim.

## [0.3.52] – 2026-09-27

- Refactored alpha-beta search into focused helpers for move application, late
  move pruning, TT probing and storage, root mate safety, null move, ProbCut,
  and PVS child search.
- Moved move-ordering heuristic tables into `search::heuristics`, reducing the
  size of `alpha_beta` without changing its search contract.
- Fixed-node probes on 50 positions and depth-12 node counts on 20 positions
  were identical on x86 and Arm. This is a behavior-preservation check, not a
  speed or playing-strength result.

## [0.3.51] – 2026-09-27

- Search: use geometric root aspiration-window recovery, defer full move ordering
  until a transposition-table move fails to cut, and retain a completed move
  from an interrupted iteration.
- Search: represent repetition history as parent-linked search frames rather
  than copying the whole game history at every node; preserve adjudication.
- Performance: reuse qsearch and move-ordering scratch storage, replace a
  quadratic large-list ordering path, and skip zero HalfKP input pairs.
- A pre-fixed local S16 SPRT against v0.3.50 accepted H1 at 524-410-23
  (LLR +3.00). S17 crossed H1 at 462-354-48 (LLR +2.96), but ten in-flight
  games completed during shutdown moved it back below the boundary. These
  are local, scoped diagnostics, not general strength claims.

## [0.3.50] – 2026-09-26

- Quiescence search skips captures that lose material in the exchange, using
  a new bitboard static exchange evaluation (`movegen::see_swap`) that does
  not change the board. Main-search capture ordering and ProbCut use the same
  evaluator; shallow non-PV nodes also prune clearly losing non-check captures.
  A pre-registered local SPRT against v0.3.49, with a pinned external HalfKP
  file, one thread, 0.5 s/move, 200 balanced openings, H0=0 Elo, H1=+10 Elo,
  and alpha=beta=0.05, accepted H1 at 206-108-7 (LLR +3.04). This is not a
  general playing-strength or external-engine superiority claim.

## [0.3.49] – 2026-09-26

- Fixed the root mate-safety filters running out of time in positions with
  many legal moves. To reject root moves that allow a mate in one, they played
  every legal reply to every root move. With large hands, about 400 moves by
  350 replies, this used a whole 100 ms move before depth 1 finished, and the
  engine then played its first legal move. Found in a won position that was
  drawn by repetition.
  - Both filters now play only moves that can give check: direct checks, or
    moves of a piece that may uncover a slider. The result is unchanged,
    since only a check can mate.
  - In the reported position the search now reaches depth 4 and finds a mate
    in 5 instead of stopping at depth 0.
- `scripts/run_ab_match.py` gains `--sprt ELO0,ELO1`. The match stops once a
  generalized SPRT on the game results (logistic Elo, alpha = beta = 0.05)
  accepts H0 (Elo <= ELO0) or H1 (Elo >= ELO1), and `--games` becomes the
  upper limit. Unit tests run in CI.

## [0.3.48] – 2026-09-26

- Added `mate::solve_mate`, a check-only depth-first proof-number (df-pn)
  mate solver with node, ply and deadline bounds, and
  `movegen::discovered_check_candidates`. Unlike the exhaustive
  `dfpn` foundation it expands only checking moves for the attacker and
  selects the most-proving child under thresholds; it proved mates the
  search had missed in local games in 1,000-18,000 nodes (under 30 ms).
  It is not yet called by the search: running it at the root with a small
  budget did not improve local self-play or node-limited external-engine
  results.

## [0.3.47] – 2026-09-25

- Root search now uses principal-variation search: after the first move,
  root moves get a null-window probe and are re-searched with the full window
  only when they may raise alpha, and late quiet root moves are probed at a
  reduced depth first. Previously every root move was searched with the full
  window at full depth, so all of them were treated as PV nodes. Nodes to
  reach depth 9 on 20 fixed positions fell from 3.25M to 1.39M. Local
  self-play against 0.3.46 (external HalfKP network, one thread, 200
  balanced openings): 284-183-9 at 0.1 s/move and 126-95-5 at 0.3 s/move;
  not a formal gate.

## [0.3.46] – 2026-09-25

- Reverse futility pruning uses a smaller margin (90 instead of 120 per
  ply) when the static evaluation improved on the one two plies earlier.
  Local self-play against 0.3.45 (external HalfKP network, one thread, 200
  balanced openings): 168-96-4 at 0.1 s/move and 128-108-5 at 0.3 s/move; not
  a formal gate. Tightening the late-move and futility limits on
  non-improving nodes as well won at 0.1 s/move but not at 0.3 s/move.

## [0.3.45] – 2026-09-25

- Quiet-move ordering: history scores now use a gravity update instead of
  saturating at ±9,000; drops get their own history slots and now earn
  history, killer, and countermove credit; a continuation history (reply to
  the opponent's previous move) is added to the quiet-move key. Local
  self-play (external HalfKP network, one thread, 0.1 s/move): 113-83-4 over
  200 games against 0.3.44; not a formal gate.
- Late-move (move-count) pruning is removed. Widening it lost heavily in
  local self-play, and removing the remaining depth-1/2 limit scored
  110-87-3 over 200 games at 0.1 s/move (21-18-1 over 40 at 0.3 s/move)
  against the ordering change above; not a formal gate.
- Shallow pruning package: razoring at depth ≤ 2, non-PV move-count and
  static-eval futility pruning of quiet moves up to depth 6 that skips moves
  giving direct check (new `movegen::move_gives_direct_check`), and checks
  are no longer extended (only protected from reductions beyond one ply).
  Local self-play against the ordering and late-move changes above (external
  HalfKP network, one thread): 125-68-3 over 196 games at 0.1 s/move and
  50-20 over 70 at 0.3 s/move; not a formal gate. Razoring alone scored
  110-85-1 at 0.1 s/move.
- Search internals: king-move undo restores the saved HalfKP perspective
  instead of rebuilding it, and branch repetition histories allocate once
  per child with an allocation-free common case. Node counts are unchanged.
- Added `scripts/run_ab_match.py` for fixed-protocol A/B self-play and
  node-limited YaneuraOu ladders.

## [0.3.44] – 2026-09-25

- `EvalFile` now detects and loads external `HalfKP 256x2-32-32` networks
  (`nn.bin`) with incremental Pure Rust inference and a new `FV_SCALE` USI
  option. Static scores matched an external reference implementation exactly
  on 23,000 positions using self-generated random networks. No external
  evaluation file is bundled, and this adds no playing-strength claim.
- Search: late non-first moves now use a principal-variation null-window
  probe, and drops count as quiet moves for late-move pruning and depth-1
  futility pruning (previously every drop was exempt). In a local 100-game
  self-play diagnostic at 0.2 s/move with one external HalfKP network this
  scored 63-35-2 against the previous search; it is not a formal gate.
- HalfKP inference regroups the hidden-layer weights by input pair so safe
  Rust vectorizes them; scores remain bit-identical.
- Production USI searches no longer record per-node diagnostic counters or
  cost timers; only the reported root mate-safety counters remain.
- Byoyomi-only time control now uses the whole byoyomi minus
  `MoveOverhead`; previously it stopped at about 65-80% of it.
- Single-worker searches skip the young-brothers probe pass, which searched
  up to six siblings before checking any of them for a cutoff, and
  quiescence no longer generates and plays every legal move to look for
  quiet checks. Local self-play (external HalfKP network): 112-86-2 over 200
  games at 0.1-0.2 s/move against the previous search; not a formal gate.
- Recursive foreground and speculative search workers now use an explicit
  8 MiB stack budget. This prevents valid deep ponder searches from aborting
  with a stack overflow; it is a correctness fix, not a strength claim.

## [0.3.43] – 2026-09-23

- Updated the direct `lineprior` dependency to 0.12.0 and verified the
  external data scripts with `shogiesa` 0.10.0.
- Hardened CSA record names and dashboard path handling; credentials no longer
  pass through the generic protocol logger. These changes do not publish a
  checkpoint or make a new playing-strength claim.
- Condensed public guidance and retained detailed historical evidence in the
  changelog archive and dated experiment records.

## [0.3.42] – 2026-09-22

- Added output-equivalent four-row NNUE L2 accumulation, exact-evaluation
  caching, and fixed-contract component profiling. These are component-level
  performance changes, not an overall engine-speed claim.
- Added explicit frozen validation inputs plus audited pairwise and listwise
  root-ranking objectives, activation diagnostics, and teacher/source contract
  checks to the NNUE trainer.
- Preserved the Q21 diagnostic chain with deterministic corpus, teacher-depth,
  transfer, and independent-coverage checks, and connected all Q21 tests to CI.
- No Q21 candidate passed the preregistered adoption screens. The optional
  v0.3.38 checkpoint is unchanged, and this release makes no new
  playing-strength claim.

## [0.3.41] – 2026-09-21

- Reduced the fixed-fixture NNUE L2 forward-pass cost with an
  output-equivalent two-row accumulation path. This is a component diagnostic,
  not an overall search-speed or playing-strength claim.
- Added `NnueResidualScalePermille` and fixed-contract residual/cost
  diagnostics, including explicit loaded-weight acknowledgement in gate
  records.
- Added deterministic, source-capped pilot-corpus selection by game phase and
  hand-aware side-to-move material stratum. It records sampling provenance but
  does not select a model.
- The residual-scale candidate did not pass its selection screen and remains
  unadopted. No new playing-strength claim is made by this release.

## [0.3.40] – 2026-09-20

- Fixed the USI ponder state machine so a completed ponder search is emitted
  exactly once when `ponderhit` or `stop` arrives. Added release-binary
  transcripts for stop, ponder, mate-score, MultiPV, and quit boundaries.
- Added a diagnostic-only legal-move binary plus a versioned small boundary
  corpus. Its static rule expectations and optional `cshogi==1.0.5` oracle
  must both match; cshogi is not a runtime dependency.
- Hardened root-mate-safety diagnostics, match-runner handling of incomplete
  bounds, and durable self-play contracts. Normal self-play now requires an
  explicit NNUE checkpoint and opening source; material start-position runs
  are limited smoke tests and never NNUE or strength evidence.
- Added a planned-versus-verified release-manifest state. A manifest can be
  checked before publishing without claiming that crates.io publication has
  occurred; only a workflow-backed manifest is verified after publication.
- No new playing-strength, external-GUI compatibility, or overall-speed claim
  is made by this release.

## [0.3.39] – 2026-09-18

- Hardened bounded positions-mode teacher labels: only completed exact search
  iterations may enter a cache, and positions now use the dedicated teacher
  search route. Earlier cache entries remain isolated by a new cache identity.
- Added an opt-in positions-mode diagnostic to exclude mate-scale labels from
  ordinary CP regression, with resume/checkpoint provenance and accounting.
- Added deterministic teacher-label strata tooling for phase, material balance,
  and mate/non-mate calibration diagnostics. These tools do not select a
  checkpoint or make a playing-strength claim.

## [0.3.38] – 2026-09-18

- Added the optional A-flat NNUE checkpoint
  `weights/sekirei-nnue-v0.3.38.bin`, with a CC BY 4.0 model card, SHA-256,
  training provenance, strict-health status, and artifact validation.
- The checkpoint passed a pinned local 1-thread, color-reversed paired-SPRT
  gate against its NNUE baseline: 80–14 over 94 games. The +302.8 gate-only
  estimate is not a Floodgate, human, or external-engine rating.
- Preserved only completed primary-PV iterations in match and self-play
  records; added opening-SFEN validation, durable per-game manifests, balanced
  opening/color reuse, and exact-game deduplication metadata.
- Added a narrow formal-preflight exception for small macOS swap allocations:
  at most 1 GiB total, 512 MiB used, and at least 6 GiB reclaimable memory.
  Unknown inputs and all other resource checks remain fail-closed.
- Condensed the bilingual READMEs and active changelog while preserving the
  detailed historical notes in `CHANGELOG_ARCHIVE.md`.

## [0.3.37] – 2026-09-16

- Corrected singular-extension TT reuse and added history-aware SFEN/search,
  root-ranking, Floodgate replay, holdout, and budget-ladder diagnostics.
- Hardened candidate and gate contracts around evaluator mode, weight hash,
  source identity, resume state, and diagnostic-only evidence.
- Added opt-in B-small NNUE experiments, trainer observability, static Explorer,
  USI analysis export, and correct `score mate` output. B-small remains on
  `EXPERIMENTAL_HOLD`; no new strength claim was made.
- Simplified public documentation and removed unreferenced load-test wrappers.

## [0.3.36] – 2026-09-12

- Synchronized workspace versions, lockfile, release metadata, and public
  documentation. Engine behavior and measurement claims were unchanged.

## [0.3.35] – 2026-09-12

- Corrected the cross-library roundtrip fixture and added legality, canonical
  move-set, SFEN, hash, NNUE-restoration, corpus, and measurement-contract
  preflights. Earlier v6 roundtrip timings are invalid.
- Separated initialization, board/NNUE updates, inference, and output conversion
  in component diagnostics. A clean ten-session A/A run established the local
  noise floor; the three-case rsshogi comparison remained component-scoped.
- Reduced SFEN and root-search overhead while preserving correctness checks.
  No overall speed or playing-strength claim was made.

## [0.3.34] – 2026-09-11

- Added strict positions-mode SFEN validation and connected resume,
  release-manifest, candidate-readiness, and documentation checks to CI.

## [0.3.33] – 2026-09-10

- Fixed interrupted DFPN fallback and hardened CSA/Floodgate position, color,
  hand, result, and record handling.
- Added bounded external SFNN header and provenance validation. This does not
  establish SFNN inference compatibility.

## [0.3.32] – 2026-09-10

- Accelerated rules-only Perft and legal move generation while preserving
  incremental state. Measurements remained machine-local diagnostics.

## [0.3.31] – 2026-09-10

- Separated Adam/full-resume checkpoint persistence and retained atomic writes,
  schema validation, and exact mid-epoch continuation.

## [0.3.30] – 2026-09-06

- Refreshed release metadata and public documentation after the documentation
  cleanup. The internal roadmap remained unpublished.

## Older releases

See the [archived detailed changelog](CHANGELOG_ARCHIVE.md)
and repository tags for versions 0.1.0 through 0.3.29.
