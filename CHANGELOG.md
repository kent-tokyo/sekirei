# Changelog

This file summarizes recent releases. The
[archived detailed history](CHANGELOG_ARCHIVE.md)
preserves older per-change notes.

## [Unreleased]

- USI: advertise the package version as `id version <version>` during the
  `usi` handshake, matching `sekirei --version` for dataset provenance.
- Dependencies: update `lineprior` to 0.12.3 and `shogiesa-core`/the verified
  external `shogiesa` CLI contract to 0.11.2. The opening-book schema remains
  v1 and the shogiesa position schema remains v11.
- Coverage: add behavioral tests for search diagnostics, bounded teacher
  search, position/game training filters, epoch-stat reset, gradient clipping,
  and CSA metadata/result boundaries. The engine/library gate now requires 92%
  line coverage with the same exclusion list.
- NNUE training: keep every HalfKP training and validation shard as a separate
  read-only memory map, and generate deterministic shuffle indices one batch at
  a time. Multi-file corpora no longer require a full in-memory concatenation
  or an all-position permutation. New checkpoints record this data-order
  contract and reject unsafe mid-epoch resumes from the former ordering.

## [0.3.68] – 2026-10-10

- Coverage: exercise isolated tunable-search paths, native WASM APIs, CSA
  conversion, export, hashing, and the USI child-process lifecycle in CI. The
  measured engine/library line coverage is 90.08%, and the local and Codecov
  gates now require at least 90% without widening the exclusion list.
- USI: `usinewgame`, `quit`, and stdin closure now invalidate the active
  search generation before joining it, preventing a previous game's ponder
  result or a shutdown-time `bestmove` from crossing the command boundary.
  Extreme increment values now use saturating time-budget arithmetic.
- Gate tooling: reject path-traversing run names and result-artifact paths
  that would overwrite the raw match JSON or text log. Evidence collection now
  records unreadable input identities without losing the terminal result.
- Tests: run the deep transposition-table search fixture with the same 8 MiB
  recursive-search stack contract used by the engine, avoiding debug-only
  test-harness stack overflow.

- Gate tooling: `run_ab_match.py --result-json` now atomically records a
  versioned, fail-closed A/B result with the exact configuration, W/D/L and
  Elo inputs, optional SPRT state, binary/evaluation/opening hashes, command,
  timing, and explicit completed, stopped, inconclusive, partial, or failed
  state. Self-play and external-engine runs share the same top-level schema.
- Evidence: match runner now sends USI `gameover` to both engines so opt-in
  decision logs retain one terminal record per completed game. Archive a
  hash-verified v0.3.67 opening-book ON/OFF diagnostic and a zero-row gate-
  model readiness report; neither artifact is a playing-strength claim.

- Search: avoid walking the parent search line for fourfold-repetition checks
  when a conservative 64-bit history filter proves that the current hash has
  not occurred there. Hash collisions still use the exact comparison, so
  move choice, score, node count, and PV remain unchanged.
- Diagnostics: expose quiescence exit/work counters in
  `sekirei-search-diagnostic` and summarize pooled qsearch rates in the Q26
  fixed-budget report without presenting overlapping timers as additive.
- Added a loopback-only Denryu rehearsal that runs the real CSA client across
  a forced 7+7 process boundary, resumes the cumulative game ceiling, records
  an append-only status journal, hashes all evidence, and verifies that both
  child processes are reaped.
- Added explicit `sekirei-csa --completed-attempts` and `--status-journal`
  options for audited bounded-run recovery.

- CSA: bound incoming lines to 64 KiB, retain partial bytes across a socket
  timeout, reject invalid UTF-8 and partial EOF explicitly, and drain an
  oversized line before failing closed so its suffix cannot become a command.
- CSA: validate game identifiers, side assignment, CSA time units and Fischer
  fields before agreeing; preserve `%KACHI`/repetition events, stop on a
  duplicate final result, and refuse a move beyond the 512-ply tournament
  boundary. The Denryu 3 min/10 min + 2 s clock has an explicit fixture.
- USI: cover repeated `isready` during infinite search and
  `isready`/`ponderhit` during ponder with process-level regression tests, and
  abort/join an active search when an adapter closes stdin without `quit`.
- Lazy SMP diagnostics: identify the worker that supplied the returned result
  and report its node share, the workers' best-move agreement, and approximate
  worker stop-lag spread; `LazyFlags` bit 128 provides an opt-in isolated-TT
  causal control, and the thread
  scaling script preserves these fields in JSON output.
- NNUE training: reject output paths that alias input data or initial states
  through direct paths, hardlinks, or symlinks; publish network, float-state,
  and resume-checkpoint outputs by atomic replacement.
- Dependencies: update `lineprior` to 0.12.2 and `shogiesa-core`/the verified
  external `shogiesa` CLI contract to 0.11.1. The opening-book schema remains
  v1 and the shogiesa position schema remains v11.

## [0.3.67] – 2026-10-10

- Documentation: shorten the English/Japanese entry READMEs and script index,
  add a current documentation index, separate historical experiment records,
  and move releases through v0.3.58 to the changelog archive.
- Training provenance: position datasets now use byte-level SHA-256 identities
  in checkpoint metadata and resume fingerprints, with optional expected-hash
  checks for both training and explicit validation inputs.
- Opening books: `--build-book` now writes an adjacent reproducible manifest;
  the opt-in USI book feature can emit structured decision and terminal JSONL
  records for held-out paired evaluation.
- Gate tooling: add a fail-closed exporter and lineprior-schema validator for
  independent historical gate observations.
- WASM packaging: render the current package version into the bundled README
  and reject stale README, package, or archive version combinations in CI.

## [0.3.66] – 2026-10-09

- Search: the static-evaluation correction now also uses one table per side
  keyed by that side's pieces other than pawns and the king (`CORR_W_NP`
  default 0 → 32; `T_CORR_W_NP=0` in a `tune` build restores the old
  behaviour). Selected with Sekirei's built-in material evaluator and
  Sekirei-generated openings, and checked in self-play with a Sekirei-trained
  HalfKP network at 1 and 4 search threads; this entry makes no
  playing-strength claim.
- USI time management: `IncrementUsePercent` now defaults to 75, so each
  move's base time adds three quarters of the Fischer increment instead of
  spreading the increment over the remaining-moves estimate (0 restores the
  old rule). Selected with Sekirei's built-in material evaluator and
  Sekirei-generated openings at 10 s + 0.1 s with 1 and 4 search threads;
  this entry makes no playing-strength claim.

## [0.3.65] – 2026-10-09

- Search: quiescence search now stands pat on the static evaluation plus the
  correction-history value, as the main search already does for its static
  eval (`QS_STYLE` default 0 → 1; `T_QS_STYLE=0` in a `tune` build restores
  the old behaviour). Selected with Sekirei's built-in material evaluator and
  Sekirei-generated openings at 1 and 4 search threads, and checked in
  self-play with a Sekirei-trained HalfKP network; this entry makes no
  playing-strength claim. `IncrementUsePercent=75` and `CORR_W_NP=32` remain
  experimental and are not enabled by this release.

## [0.3.64] – 2026-10-08

- Search (SEARCH_V2): a node no longer re-searches a move after the node or
  time budget has run out. The aborted child's 0 could pass for a fail-high
  and the re-search only counted one more node past the limit; the returned
  values are unchanged.
- Search tests: reproducibility checks now isolate thread-local root ordering,
  use a position with an unambiguous best move where required, and run the
  one-worker Lazy SMP case in a dedicated pool.
- The default evaluator, NNUE checkpoint, quiescence style, and increment-time
  allocation are unchanged. This maintenance release makes no new
  playing-strength claim.

## [0.3.63] – 2026-10-08

- NNUE training: add `scripts/train_halfkp.py`, the PyTorch trainer for the
  in-house HalfKP 256x2-32-32 `nn.bin`, with by-game validation files
  (`--val-data`), an optional king-independent factorizer folded at export
  (`--fact`), and a single-epoch default for continuing on new self-play data.
  It does not add a checkpoint or make a playing-strength claim.
- WebAssembly: position analysis now exposes one legal primary principal
  variation from the same completed iteration as its score, depth, and bound.
  Fallback, pre-iteration abort, and terminal results keep the PV empty; the
  capability contract explicitly reports that MultiPV is not supported.
- CSA/Floodgate operations: bounded one-shot supervision now records a durable
  terminal reason, completed-attempt count, and requested game limit, then
  refuses to relaunch the completed child. The launchd template uses the
  client's own `--loop --max-games` cap with `KeepAlive` disabled.
- This release does not change the default evaluator, adopt a new checkpoint,
  or make a new playing-strength claim.

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

## Older releases

See [CHANGELOG_ARCHIVE.md](CHANGELOG_ARCHIVE.md) for v0.3.58 and earlier.
