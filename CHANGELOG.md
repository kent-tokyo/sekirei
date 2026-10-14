# Changelog

This file summarizes recent releases. The
[archived detailed history](CHANGELOG_ARCHIVE.md)
preserves older per-change notes.

## [Unreleased]

## [0.4.1] – 2026-10-14

Version 0.4.1 improves search correctness, behavior-preserving HalfKP update
throughput, fail-closed match evidence, and opt-in Lazy SMP diagnostics. The
playing defaults and bundled evaluator remain unchanged, so this release does
not claim a general playing-strength gain.

### Changed

- Fuse HalfKP incremental quiet-move, capture, and capture-undo updates into
  one accumulator traversal per perspective. Wrapping arithmetic, dirty/king
  handling, evaluator files, and search decisions are preserved. Add synthetic
  equivalence tests and bounded pinned-reference throughput tooling.
- Clear the shared Lazy SMP transposition table once per new game, while
  preserving every worker's evaluation-cache and move-ordering reset.
- Exclude queued new-game and position work from NPS-driver wall timing with
  a readyok barrier before each search.

### Added

- Add opt-in dominated-helper retirement (`LazyFlags` bit 8192), using exact
  completed iteration boundaries, depth and stable-move agreement, and sampled
  TT work supplied to other workers. Worker 0 and the deepest helper remain.
- Add a two-participant cooperative root-split prototype (`LazyFlags` bit
  16384, shared TT, exactly two workers), with main-worker confirmation,
  generation-stamped candidates, RAII cancellation, and a shared hard budget.
- Extend root-hint diagnostics with first-move beta cuts, aspiration retry
  nodes, superseded hints, and publication-to-use depth/generation gaps.
- Add opt-in V2 NMP diagnostics and the disabled-by-default `V2_NMP_PRIOR`
  one-ply ancestor experiment. The ordinary evaluator and defaults stay fixed.
- Add artifact-frozen per-position ABBA comparisons, exact one-worker
  fixed-depth signatures, and fresh score-blind Sekirei diagnostic positions.

- Add an opt-in Lazy SMP helper-root-LMR experiment (`LazyFlags` bit 4096).
  Worker 0 keeps the established root search while helpers remove the usual
  one-ply LMR relief for late quiet root moves. The playing default remains
  unchanged pending fixed-condition diagnostics and an independent gate.
- Add an opt-in Lazy SMP helper-root-hint experiment (`LazyFlags` bit 2048).
  Helpers publish only a fully completed iteration's move and depth; worker 0
  may try a same-or-deeper legal hint first on its next iteration. Scores,
  bounds and PVs are never imported, stale invocations are rejected by an
  atomic generation token, and the playing default remains unchanged pending
  an independent material-only gate. USI diagnostics report accepted slot
  updates, depth-qualified reads, and hints that changed the final legal root
  order.
- Extend the Lazy SMP scaling harness with fail-closed helper-root-hint transfer
  metrics and binary/corpus SHA-256 provenance.
- Add diagnostic-only per-worker TT-origin counters for Lazy SMP. Scaling
  reports now show whether the selected root worker reused sampled entries
  written by helpers, without adding counters to normal searches or changing
  the playing default.
- Add an opt-in Lazy SMP depth-consensus selector that never chooses a result
  below the maximum completed worker depth. Other workers only break ties
  between moves found at that depth; the playing default remains unchanged
  pending an independent material-only strength gate. A preregistered
  material-only diagnostic advanced the candidate with selected-depth median
  18 versus 17 and repeated-bestmove stability 75.0% versus 77.5%.
- Add an opt-in Lazy SMP root-order diversity flag for one-factor diagnostics.
  Helper workers can start from deterministically rotated root moves while the
  main worker and later iterative-deepening order remain unchanged; the
  playing default is unchanged pending an independently preregistered
  material-only strength gate.

### Changed

- Delay sorting losing tactical moves in the default v2 search until the
  search actually reaches that final move stage. Nodes that reach the stage
  retain the same ordering; earlier cutoffs avoid an unused sort.
- Update `lineprior` to 0.12.6, `shogiesa-core` to 0.11.6, and `veridict`
  to 0.19.5. The update keeps the existing product boundaries and gate
  hypotheses while adopting the upstream query, schema, and fail-closed
  statistical fixes.
- Update `lineprior` to 0.12.5 and use its allocation-reusing borrowed query
  API for opening-book lookups.
- Update `shogiesa-core` to 0.11.5 and consume position JSONL through its
  line-numbered bounded-memory reader while preserving exact input hashing.
- Update `veridict` to 0.19.4 and express SPRT execution policy through the
  typed `SprtRunOptions` API; existing hypotheses and pairing behavior remain
  unchanged.

### Fixed

- Make local A/B and SPRT gates fail closed on missing or inconsistent raw
  match JSON/JSONL, kifu gaps, artifact-write failures, and invalid games.
  Sequential SPRT decisions are replayed from the durable JSONL, including
  colour-pair order, and must exactly match the streamed stop decision.
  `--games` remains a hard cap in cover-all mode; terminal evidence hashes the
  JSONL and every non-empty kifu. Gate verdict sidecars are atomic and a write
  failure is a gate error rather than a printed warning. Git provenance probes
  are time-bounded so a completed match cannot hang indefinitely while writing
  its terminal result.
  `sekirei-match` now emits each machine-scraped game result only after its
  enabled durable artifacts and incremental snapshot have been committed; a
  write failure stops the runner, so an early SPRT boundary cannot accept an
  unrecorded terminal game. Early stopping terminates the runner and its engine
  process group, escalates surviving children after a grace period, and does
  not let an unexpected child exit code hide behind a numerical verdict.
- Serialize Lazy SMP abort reset and TT lifecycle mutations with active search
  invocations. A conflicting mutation requests cooperative stop before waiting
  for worker join, so it cannot alter a running group or deadlock behind
  `go infinite`; concurrent lifecycle waiters cannot re-arm a stale abort
  after another waiter resets it.
- Attribute Lazy SMP diagnostics to the exact selected worker index, including
  selectors whose deterministic tie rule picks a later identical result.
- Serialize overlapping searches on one `LazySmpSearcher`; the new root-hint
  `try_*` diagnostic API instead returns `None` without panicking or mixing
  invocation generations.
- Prefer a completed exact winning-mate result from any ordinary Lazy SMP
  worker over a deeper heuristic score. The explicit worker-0-only diagnostic
  mode keeps its existing result-selection contract.
- Reset root-move ordering at each search boundary so Rayon scheduling cannot
  leak a previous logical Lazy SMP worker's order into a later `go` command.
- Invalidate an in-flight search as soon as any `position` command arrives,
  including malformed replacements, so an old-position `bestmove` cannot
  cross the USI command boundary while the new position is being parsed.

## [0.4.0] – 2026-10-12

Version 0.4.0 is a compatibility and operations milestone. It preserves the
public `sekirei-core` API and the default material evaluator while making the
mate-analysis, release-evidence, Floodgate supervision, and browser contracts
explicit. No general playing-strength gain is claimed for this release.

### Changed

- Update the match runner to `veridict` 0.19.2. Existing Elo and pentanomial
  SPRT gates keep their current behavior; the new paired-Elo fixed-sample
  metric remains opt-in and is not used by Sekirei's existing gate commands.

### Fixed

- Preserve the exact mating move found by one-ply mate detection in normal,
  V2, and quiescence searches so completed mate PVs reach the terminal move.
- Keep bounded DFPN outcomes distinct at the USI boundary: proven mates with
  unknown distance use `score mate +`, while bounded disproofs and incomplete
  searches omit a centipawn score and report outcome, limit, abort state, and
  best-move provenance through `info string` diagnostics.

### Added

- Add an idempotent bounded-Floodgate LaunchAgent helper that chooses a safe
  start sequence from `RunAtLoad`, refuses registered labels, records the
  runtime directory and rejects `launchctl` evidence with `runs != 1`.
- Add explicit Rust and WebAssembly mate-analysis entry points for tsume
  diagrams that omit the attacking king, while preserving the existing strict
  two-king APIs. Mate results now identify their position policy and expose the
  all-legal-defender-reply proof-distance rule separately from Japanese
  puzzle-convention move counts that may omit futile interpositions.
- Add a fail-closed HalfKP release model-card contract that verifies the exact
  weight, license, training-shard and whole-game hold-out manifests, raw
  current-engine 1/4-thread gate evidence, and engine commit without bundling
  or selecting an NNUE candidate.
- Add a v0.4 public Rust API compatibility gate for `sekirei-core`, checked
  against v0.3.73 before merge and again on the exact release tag before any
  crates.io upload. The check forces patch-level compatibility so the 0.4
  version bump cannot silently permit a breaking API change.

## [0.3.73] – 2026-10-12

- Search diagnostics: attribute a bounded sample of committed TT entries to
  the worker that wrote them, report confirmed local/foreign-worker reuse,
  and separate all discarded root-result nodes from the divergent subset.
  The counters remain opt-in and do not change the playing defaults.
- Documentation: align the mobile contract with the existing one-shot WASM
  evaluator loader and bounded MultiPV API, and define a preregistered mobile
  analysis gate that keeps training, internal reference, and final evidence
  separate.
- CI: keep the 98% Rust coverage gate for Rust/Cargo pull requests, every
  main update, and release tags while skipping documentation-only and ordinary
  script-only pull requests. Codecov upload now runs as the final step of the
  same coverage job, without a second runner or artifact hand-off.
- Search: preserve the primary PV captured at the last completed speculative
  iteration when a later node- or time-limited iteration is interrupted. The
  returned move, score, depth, and PV now remain one consistent snapshot.
- Diagnostics: add a fresh-process fixed-node tool that records first-move,
  reply, and response observations separately. Human reference moves remain
  metadata rather than correctness assertions, and material and explicit
  NNUE runs retain separate executable and evaluator provenance.
- WebAssembly: add bounded sequential MultiPV analysis for up to four unique
  root moves under one shared node budget. Ranked candidates expose typed
  score, bound, depth, node, cutoff, and legal PV provenance only from fully
  completed iterations; interrupted and terminal requests never fabricate
  lines.
- WebAssembly: accept one caller-supplied SEKIRW or HalfKP evaluator byte
  image after fail-closed validation, expose its exact SHA-256 provenance in
  load and search results, and document Web Worker recreation for selecting a
  different immutable evaluator. No weight file is bundled.
- Floodgate supervision: when a bounded CSA child exits non-zero, persist its
  exit code and terminal reason to runtime status, run manifest, and the
  one-shot completion record. A live game is classified as incomplete and is
  never added to the completed-game count.
- Dependencies: update the training-only `shogiesa-core` contract to 0.11.4
  and verify label manifests, input positions, and output observations through
  its typed verifier before Quietset or training consumes them. Complete
  provenance is required unless an explicit compatibility mode is selected;
  the default USI engine remains independent of shogiesa.
- NNUE evidence: record the pinned v0.3.68 material-gate PASS results for the
  local `nn_r3` candidate at 500 ms and 1 second per move, including engine,
  opening-corpus, candidate, and local raw-archive hashes. The weight remains
  unbundled and the material evaluator remains the default.
- Search: advance TT generations for speculative searches and for every
  persistent isolated Lazy SMP worker. Four-slot replacement now ages entries
  consistently across all alpha-beta backends instead of retaining stale
  equal-depth entries indefinitely in those modes.
- Match runner: gracefully reap engine children at normal shutdown, force and
  reap engines that ignore `quit`, and kill a launched child before exiting on
  a failed USI initialization. The thread-scaling diagnostic also waits after
  its forced-kill fallback. Match completion, startup failure, and diagnostic
  timeout paths no longer leave orphan or zombie engine processes. Opening
  corpora also accept the documented `startpos` token instead of validating
  every line as raw SFEN.
- Search diagnostics: estimate within-worker and cross-worker repeated
  position work with a bounded, opt-in Lazy SMP sampler, report it through the
  thread-scaling tool, and add a disabled depth-ladder experiment. Shared TT
  reduced sampled overlap substantially; the first depth-ladder match was
  neutral, so no playing default changed.

## [0.3.72] – 2026-10-11

- Search diagnostics: add an opt-in Lazy SMP counter path and extend the
  thread-scaling tool with worker TT hit/store counts, shared-table collision
  topology, repeated-position best-move stability, and explicit warning that
  counter-enabled runs are not timing baselines. The default search path is
  unchanged.
- Refactoring: centralize Lazy SMP worker construction and separate its USI
  diagnostic rendering from the normal search-result response path without
  changing the diagnostic wire format or search policy.
- CSA: let bounded multi-game runs reconnect up to three times after a
  post-game transport close while preserving completed-game progress. Failed
  recovery is recorded as `post_game_reconnect_exhausted`; mid-game, protocol,
  and recording failures remain terminal.

## [0.3.71] – 2026-10-11

- CSA: terminate an authenticated bounded client generation with a non-zero
  exit after `run_error`. The one-shot supervisor records the terminal reason
  and completed-attempt count without restarting or inventing a result.
- Coverage: raise the reusable Rust engine/library line-coverage contract from
  95% to 98% without widening the exclusion list. Add behavioral coverage for
  search, rules, protocol, trainer, and native WASM paths; browser-only
  JavaScript conversions remain covered by the WebAssembly browser job.
- Refactoring: separate CSA framing and time control, USI search lifecycle and
  `go` parsing, match clock rules, search path state, move ordering, pruning,
  TT probes, and atomic trainer output into smaller tested units. Fixed-budget
  search results remain unchanged by the search refactors.
- Documentation: align the English and Japanese entry points, current contract
  index, WebAssembly API, analysis benchmark, mobile notes, and script index
  with v0.3.71 and the 98% coverage gate. Historical experiment and release
  records retain their original scope.

## [0.3.70] – 2026-10-11

- Match runner: enforce one absolute deadline for each USI handshake, readiness
  barrier, and `go` response. Repeated `info` output can no longer extend a
  move indefinitely when an engine never returns `bestmove`.
- Match runner: reject overflowing Fischer clock fields and use saturating
  arithmetic when applying elapsed time and increments, preventing untrusted
  clock values from wrapping or panicking.

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
