# Changelog

This file summarizes recent releases. The
[archived detailed history](CHANGELOG_ARCHIVE.md)
preserves older per-change notes.

## [Unreleased]

- Documentation: shorten and align the English and Japanese entry READMEs,
  make the documentation index the single map for current contracts and
  historical evidence, and move releases through v0.3.66 into the archive.
- Dependencies: update `lineprior` to 0.12.4 and `shogiesa-core` plus the
  verified external `shogiesa` CLI contract to 0.11.3. The opening-book schema
  remains v1 and the position schema remains v11; diagnostic manifests now
  require the independent `manifest_schema_version: 1` contract.
- NNUE evidence: register the exact `nn_r3` HalfKP candidate identity,
  fresh-only self-play lineage, candidate-relative 480-game result, and missing
  release evidence. Add a fail-closed card/artifact validator without bundling
  the 61 MiB network or changing the default material evaluator.

## [0.3.69] – 2026-10-11

- Search: retain every non-capturing pawn, bishop, and rook promotion when the
  optional quiescence promotion search is enabled; promotion-heavy positions
  are no longer silently truncated after 16 candidates.
- CSA: reject time echoes and game-id clock fallbacks that overflow the
  client's millisecond representation instead of panicking or wrapping.
- Opening-book gates: freeze disjoint CSA game subsets for book training and
  held-out evaluation, preregister actual-selection coverage, and stop before
  a strength match when support is too sparse. The first 20-position run made
  0 book selections and is archived as `not_ready / INCONCLUSIVE`, not as a
  book failure or strength result.
- Gate evidence: validate a versioned, hashed GateObservation declaration
  before an A/B run, carry it into the terminal result, and quarantine changed
  schemas, missing groups, retries, and incomplete results during export. The
  first two-game pilot proves the contract only; fitting remains disabled.
- CSA: consume one delayed decisive result after an aborted game before the
  next game summary, while retaining fail-closed handling for duplicate or
  ambiguous terminal lines.
- USI: advertise the package version as `id version <version>` during the
  `usi` handshake, matching `sekirei --version` for dataset provenance.
- Dependencies: update `lineprior` to 0.12.3 and `shogiesa-core`/the verified
  external `shogiesa` CLI contract to 0.11.2. The opening-book schema remains
  v1 and the shogiesa position schema remains v11.
- Coverage: add behavioral public-path tests for move containers, search and
  pruning controls, evaluation modes, SFNN safety boundaries, CSA protocol
  handling, USI lifecycle, and a bounded trainer smoke. The engine/library
  gate now requires 95% line coverage with the same exclusion list.
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

## Older releases

See [CHANGELOG_ARCHIVE.md](CHANGELOG_ARCHIVE.md) for v0.3.66 and earlier.
