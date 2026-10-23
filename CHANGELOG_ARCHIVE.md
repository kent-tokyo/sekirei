# Archived detailed changelog

This file preserves releases through v0.4.3 after the public changelog was
condensed. It is historical evidence; use the root `CHANGELOG.md` for current
and recent releases.

## [0.4.3] – 2026-10-16

Version 0.4.3 improves bounded CSA supervision, records independently
qualified eight-thread PGO throughput, and adds fail-closed qualification for
disabled search experiments. The default evaluator and ordinary search policy
remain unchanged, so this release does not claim a general playing-strength
gain.

### Changed

- Record an independent Apple M4 eight-thread PGO comparison against the
  pinned YaneuraOu binary. The result applies only to the recorded profile,
  evaluator, corpus, and host; ordinary release builds do not embed that PGO
  profile.
- Extend the 98% Rust coverage contract to the isolated tune-only search
  qualification paths added in this release.

### Added

- Add deterministic activity and correctness qualification for the existing
  V2 fail-high blend and transposition-table depth-retention policies. Neither
  result changes a production default or constitutes an Elo decision.
- Add a default-off correction pawn-key reuse experiment. Exact one-thread
  search equivalence passed, but timing did not qualify it for adoption.
- Add default-off helper-hint refresh at aspiration retries and cooperative
  parent-budget batching experiments. The former was declined by the 4T/8T
  depth screen; the latter stopped before timing after a diagnostic validator
  error. Both defaults remain off.

### Fixed

- Preserve one-shot Floodgate completion and finalization across bounded
  supervisor I/O failures, without counting failed sessions as completed
  games or starting an unbounded restart loop.

## [0.4.2] – 2026-10-16

Version 0.4.2 improves search-boundary correctness, bounded training-data
ingestion, reproducible performance evidence, and disabled-by-default search
experiments. The default evaluator and ordinary search policy remain
unchanged, so this release does not claim a general playing-strength gain.

### Changed

- Bind native YaneuraOu comparisons to the actual `nn.bin`, reject evaluator
  option overrides, and invalidate measurements or H1 after input drift.

- Borrow the completed root-move prefix for V2 history learning, removing
  a growing vector without changing move order or learning updates.

- Keep the public changelog focused on the 0.4 series by moving v0.3.67
  through v0.3.73 into the history archive. Shorten the script index, record
  the terminal depth-consensus evidence, and align the WebAssembly capability
  text with its existing four-line MultiPV implementation.
- Poll empty cooperative root-candidate slots without an atomic swap, retaining
  consuming synchronization and late-publication draining. The root-split mode
  remains opt-in; no measured speed or strength gain is claimed.

### Added

- Add default-off deferred quiet-HalfKP and refresh-column fusion experiments.
  Independent 4T engine gates did not establish a speed gain, so
  `V2_LAZY_QUIET_HKP=0` and `NNUE_REFRESH_TILE=0` remain unchanged. Exact
  conditions and raw-result hashes are retained in dated experiment records.
- Add default-off quiet-TT staging and continuation-row scoring experiments.
  Their 4T speed evidence was negative or incomplete, so both defaults remain
  unchanged.
- Add default-off cooperative root prestart and initial-work cap experiments.
  Fixed-node diagnostics did not improve completed depth; qualified timing was
  blocked before search, so no speed or strength conclusion was adopted.
- Add a default-off `repetition_filter2` build feature that uses two bits in
  the existing search-line repetition filter. Exact repetition and perpetual
  check adjudication remain authoritative; this is an unqualified CPU candidate.

- Add independently gated Pure Rust gzip/zstd JSONL decoding with physical and
  decoded hashes, complete-stream integrity checks, and compression/storage
  equivalence tests. Preserve the physical-file SHA contract for resume.
- Add opt-in disk-backed JSONL position storage and durable ingestion audit
  manifests with exact input hashes, rejection counts and bounded row evidence.
  Preserve source selection, shuffle and resume semantics; avoid retaining an
  additional epoch-sized clone of accepted positions. Add audit-only operation
  and reproducible ingestion RSS/time comparisons without changing search or
  the evaluator.
- Add an opt-in highest-ready/rescan cooperative confirmation policy and
  arrival-phase outcome diagnostics. Worker 0 still verifies every accepted
  score at full depth; defaults remain disabled pending independent H1.

- Add opt-in cooperative-root advice latency diagnostics using private main-node
  accounting and phase epochs, with compatible snapshots and USI evidence.
  Defaults and search policy are unchanged; diagnostics are not strength proof.

- Document a native PGO build qualified with own-only fixed-depth and four-worker
  throughput gates, retaining the compiler-profile corpus and provenance.
  Search source, evaluator weights and ordinary release defaults are unchanged.
- Add a safe Rust PGX shogi environment comparison adapter with dense legal
  masks, timed action decoding, full observations and probe output. Document
  pinned-reference CPU timing and correctness requirements separately from
  search NPS, accelerator throughput and playing strength.
- Add fresh material-opening generation with full move histories and mandatory
  owner identity exclusions before independent gate qualification. Freeze and
  replay generator/validator provenance; optionally bind corpus and executable
  identities before and after a match, revoking H1 if inputs change.

### Fixed

- Saturate packed TT depths at 127 without corrupting scores, and compare
  requested depths without narrowing wrap in legacy and speculative probes
  and singular eligibility. Preserve conservative depth on deep root stores.

- Accept finite fractional wall times and explicitly unavailable optional opening
  books in the pinned YaneuraOu throughput comparator, while retaining strict
  artifact, option, counter, and positive-value checks.
- Preserve match-opening repetition history from legal USI move prefixes in
  runner adjudication and recorded USI/CSA games. Reject prefixes that already
  ended by repetition and avoid duplicate moves keywords. Bare openings remain
  supported; search defaults and NNUE are unchanged.

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
  Opening-book lookups use lineprior's borrowed query API, position JSONL uses
  shogiesa-core's bounded reader, and SPRT policy uses veridict's typed API.

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

- Published the optional A-flat NNUE checkpoint `sekirei-nnue-v0.3.38.bin`
  with a CC BY 4.0 model card, SHA-256, training provenance, strict-health
  status, and a machine-validated artifact contract.
- The pinned candidate cleared the local 1-thread, color-reversed paired-SPRT
  gate against its pinned NNUE baseline: 80 wins, 14 losses, 0 draws over 47
  completed pairs (94 games), with a gate-only estimate of +302.8 Elo. This is
  neither a Floodgate/human rating nor a claim against external engines.
- Added a small-swap formal-preflight exception only for hosts with at most
  1 GiB total swap, at most 512 MiB used swap, and at least 6 GiB reclaimable
  memory. Unknown values and all other resource checks remain fail-closed.
- Preserved only completed primary-PV iterations in match/self-play records,
  added explicit opening-SFEN validation, and expanded durable local self-play
  provenance for NNUE experiments.

- Corrected engine/color assignment for arbitrary starting SFENs, including
  White-to-move positions, and record structured USI score, mate, bound,
  depth, nodes, and PV data for each move.
- Made self-play JSONL, summary, and CSA manifests durable after every
  completed game. The wrapper now preserves stop intent, rejects missing
  required artifacts, balances opening/color reuse, and emits an exact-game
  deduplication index for downstream training.

## [0.3.37] – 2026-09-16

- Simplified the English and Japanese READMEs, added a categorized script
  index, and condensed historical gate/audit notes without changing their
  verdicts.
- Removed two unreferenced throwaway load-test wrappers. No engine runtime,
  NNUE format, or playing-strength behavior changed.
- Prevented singular-extension verification searches from reusing a TT cutoff
  that belongs to the unverified parent search.
- Added history-aware SFEN/search diagnostics, root-ranking audits, and
  Floodgate replay, holdout, budget-ladder, manifest, and supervisor tooling.
- Hardened candidate/gate execution contracts so evaluator mode, weight hash,
  source identity, resume state, and diagnostic-only results remain explicit.
- Added the opt-in king-relative B-small NNUE architecture and calibration
  diagnostics. Its offline validation remains `EXPERIMENTAL_HOLD`; this
  release makes no new playing-strength claim.
- Improved trainer observability and reproducibility with progress logging,
  patience-based early stopping, ranking diagnostics, and corrected
  `--checkpoint-dir` handling for CSA-game training.
- Added reproducible USI analysis export, an experimental static Explorer,
  and correct USI `score mate` output for mate-distance scores.
- Updated the component benchmark capture contract and retained A/A noise-floor
  requirements. Diagnostic results are not an overall speed or Elo ranking.

## [0.3.36] – 2026-09-12

- Updated workspace versions, `Cargo.lock`, release metadata, and public
  documentation for the 0.3.36 distribution.
- Added an explicit v0.3.36 release badge to both README files.

## [0.3.35] – 2026-09-12

- Corrected the cross-library six-ply roundtrip fixture: apply all moves before
  undoing in reverse. Added legality, canonical-move-set, SFEN, hash and NNUE
  restoration preflights. Old v6 roundtrip timings are invalid.
- Separated warm initialization, board/NNUE updates, inference and move-output
  conversion in component diagnostics with raw samples and frozen-binary
  capture. v7 move generation no longer includes a raw-encoding checksum.
- Added isolated NNUE accumulator-refresh cases for the startpos, midgame, and
  drop-only fixtures, expanding the fixed component contract to 66 cases.
- Expanded the SP0 speed smoke corpus to 128 verified positions (eight
  categories, four original and four mirrored positions per category, both
  sides to move). Generated-history coverage remains a separate gate.
- Added an explicit `required_case_ids` contract so a missing or renamed smoke
  case fails validation instead of passing a count-only check.
- Stored the sorted expected USI move set for every smoke position and compare
  the complete set during the Rust/rsshogi preflight.
- New component captures now require a clean worktree by default; diagnostic
  captures from dirty trees require `--allow-dirty` and preserve their status.
- Added verified Perft(2) divide data for every SP0 smoke position; validation
  now checks the complete divide and its sum, not only the aggregate count.
- Added a canonical SHA-256 over the corpus cases so fixture content changes
  cannot be hidden behind an unchanged record count.
- Added a deterministic 64/64 tuning/hold-out split manifest that keeps each
  base position and its side-to-move variant in the same partition.
- Connected the verified one-move sequences to the 128-position preflight;
  longer six-ply nested roundtrips remain a separate component check.
- Added a deterministic twelve-ply generated-sequence roundtrip for every corpus
  SFEN, checking both engines and restoring board state, hash, and NNUE data.
- Fixed the generated-sequence length and LCG seed in the speed contract so the
  long-history preflight is reproducible and not biased to the first legal move.
- Added a runner `--build` mode that performs the offline release build and
  records its command/profile before capturing the binary.
- Corrected that build mode to pass `--release` before selecting the release
  binary; the interrupted dev-profile capture is not valid evidence.
- Recorded one clean release-profile component capture from the corrected build
  path, with binary hash and selected p50 rows; no formal ranking claim follows.
- Recorded ten same-binary A/A captures: overall geomean 0.9989x with a
  0.9943–1.0036x pair-geomean 95% interval inside the 0.98–1.02 noise floor.
- Added a separate ten-session comparison for three common legal-generation
  cases: combined rsshogi/Sekirei ratio 1.1596x (95% CI 1.1364–1.1833x).
  The scope excludes full-state updates, NNUE, search, and strength.
- Added explicit-weight evaluation, rules-only-to-NNUE evaluation, and Board
  clone diagnostics with setup work kept outside the timing window.
- Centralized component timing parameters in the checked-in speed contract and
  made the capture validator reject mismatched measurement headers.
- Removed temporary field/rank vectors from SFEN initialization and return a
  parse error before constructing an out-of-range square on overfull ranks.
  No new engine-strength or overall speed claim is made.
- Refactored root-search safety stages, unified alpha-beta beta-cutoff
  bookkeeping, and reduced quiet-move derived-bitboard updates to one XOR
  mask. These are correctness/readability changes, not a new speed claim.

## [0.3.34] – 2026-09-11

- Added strict SFEN validation for positions-mode training and connected the
  resume, release-manifest, candidate-readiness, and documentation checks to CI.
- Refreshed public and internal documentation references for the current
  release metadata. No new strength or heavy-measurement claim is included.

## [0.3.33] – 2026-09-10

- Fixed a DFPN time-limit path that could return `bestmove resign` when the
  bounded search was merely `Unknown`; the USI layer now falls back to a legal
  move whenever an auxiliary search is interrupted and moves remain.
- Hardened CSA/Floodgate state handling: consume the server-provided position
  (including `PI` and hand declarations), validate move color against the
  internal side to move, and wait for the final `#WIN`/`#LOSE`/`#DRAW` result
  after an intermediate `#RESIGN` marker.
- Corrected CSA record color headers and avoided treating a non-check terminal
  position as checkmate at the search root.
- Added regression coverage for CSA position parsing, hand declarations,
  wrong-side moves, DFPN time-limited moves, and the existing USI race suite.
- Verified the workspace with Clippy warnings denied, formatting checks, and
  the full workspace test suite. Existing Floodgate records are historical
  evidence and were not rewritten.

- Added bounded external SFNN header inspection and a provenance/compatibility
  manifest validator. Header acceptance does not claim inference compatibility;
  feature mapping and numerical inference remain a separate adapter gate.

## [0.3.32] – 2026-09-10

- Reduced Perft overhead by using the rules-only do/undo path without NNUE
  accumulator updates; added a state-preservation regression test.
- Replaced legal-move king-capture checks with an inlined opponent-king
  bitboard test and inlined the main move-generation helpers.
- Recorded reproducible local Criterion results: legal move generation reached
  361.24 ns and Perft(3) reached 315.72 µs on the release benchmark host.
- Kept performance measurements diagnostic only; no playing-strength or Elo
  claim is attached to this release.

## [0.3.31] – 2026-09-10

- Separated Adam and full-resume checkpoint persistence from the training
  implementation into a dedicated module without changing checkpoint schemas
  or the training API.
- Retained atomic checkpoint writes and schema, shape, and finite-value
  validation while reducing the trainer implementation by about 250 lines.
- Verified the training crate with 154 tests, Clippy with warnings denied, and
  formatting/whitespace checks.

## [0.3.30] – 2026-09-06

- Refreshed the documentation and release metadata after the documentation
  cleanup; implementation and measurement claims remain unchanged.
- Preserved the internal-only roadmap boundary and the MIT OR Apache-2.0
  attribution policy.

## [0.3.29] – 2026-09-06

- Added deterministic fixed-budget SharedMcts diagnostics with explicit
  transposition-sharing controls and comparison summaries.
- Added release-manifest and artifact-audit tooling for reproducible diagnostic
  evidence without making a playing-strength claim.
- Corrected transposition-table publication on aborted searches so incomplete
  nodes do not publish full-node bounds.
- Kept external-engine comparisons and heavyweight strength gates separate from
  this release.

## [0.3.28] – 2026-09-05

- Added single-check evasion prefiltering while retaining make/unmake legality
  confirmation for correctness.
- Added fixed pin/evasion and branched legality differential corpora, including
  the 10,000,000 random Perft/mated validation gate.
- Reused thread-local move buffers in alpha-beta and quiescence search without
  shared mutable state.
- Recorded fixed-condition local performance diagnostics; mixed results remain
  diagnostic only and do not imply a strength or Elo claim.
- Updated release metadata and the public attribution/reproducibility records.

## [0.3.27] – 2026-09-04

- Added an opt-in USI `SearchMode=LazySMP` backend. Workers keep private board
  and heuristic state while sharing the lock-free transposition table and a
  common cancellation flag; `Threads` selects the worker count.
- Added deterministic Lazy SMP correctness coverage for board preservation,
  one-worker equivalence, legal result selection, and USI stop/quit joining.
- Added a pinned, documentation-only rshogi capability audit and CI validator;
  it deliberately makes no speed, feature-equivalence, or Elo claim.
- Aligned the USI `id author` value with the public attribution name,
  `Kentaro Tanabe`.
- Accelerated legality probes by skipping temporary NNUE accumulator updates,
  added a capture-only generator for quiescence search, pre-unioned attack
  bitboards, cached the bit-exact LMR formula, and removed transient YBW
  allocation and duplicate child-check work.
- Extended the Criterion suite with explicit NNUE, do/undo, and tactical
  capture-generation cases. A local fixed-depth diagnostic measured depth-4
  start-position search at 7.659 ms versus 22.544 ms before this optimization
  pass; this is a machine-local performance result, not a strength or Elo claim.
- Updated the documented shogiesa pipeline baseline from 0.9.0 to 0.9.2;
  shogiesa remains an external data-generation tool rather than a Cargo
  dependency of Sekirei.

## [0.3.26] – 2026-09-04

- Added bounded candidate-versus-teacher NNUE analysis diagnostics and
  reproducible self-distillation tooling.
- Added top-level quiescence-search TT probing, TT-move ordering, guarded
  depth-zero bound storage, and abort/deeper-entry regression coverage.
- Updated the documented shogiesa pipeline baseline to 0.9.2.
- Added an explicit fixed-NNUE teacher mode for training, with teacher-hash
  isolation across caches, complete-resume fingerprints, and checkpoint
  metadata, plus a six-position CI fixture.
- Added a deterministic teacher-search node budget (`--label-nodes`) and bound
  it into cache identities, complete-resume fingerprints, and metadata.
- Added top-level quiescence-search TT probing, TT-move ordering, and guarded
  depth-zero bound storage; recursive qsearch and aborted searches do not
  publish reusable entries.

## [0.3.25] – 2026-09-03

- Added release-manifest-shaped evaluator diagnostic output without mutating
  the source manifest.
- Added training-only Adam checkpoints with raw parameters, all first/second
  moments, validation, atomic writes, and `--resume-adam` restoration.
- Added a validated diagnostic release-manifest fixture and epoch-boundary
  `--resume-checkpoint` files carrying epoch, data cursor, recipe fingerprint,
  weights, and Adam state.
- Connected manifest validation to CI and added optional atomic mid-epoch
  checkpoints with a data cursor and teacher-cache snapshot.
- Added a controlled stop-after-checkpoint CLI mode and documented resume
  rejection conditions.

## [0.3.24] – 2026-09-02

- Made singular-extension verification-search TT write exclusion explicit and
  added a regression test for the protected parent entry.
- Recorded the Phase 5 correctness checks and Phase 7 release-audit artifacts.
- Aligned English and Japanese documentation with deterministic controls,
  inference-compatible NNUE checkpoints, and the current attribution policy.

- Restored the Rust-standard MIT OR Apache-2.0 source-code licensing and added
  a NOTICE with Kentaro Tanabe attribution and recommended product wording.
- Documented CC BY 4.0 as the separate license for project-produced NNUE
  weight artifacts; third-party training data remains subject to its own terms.
- Added reproducible NNUE calibration and SFEN outlier-classification
  diagnostics, and corrected the training pipeline to use a short early-stop
  run with validation-best checkpoint selection and an explicit cosine
  schedule horizon.
- Corrected USI info output to report forced mates as `score mate N` instead
  of exposing internal mate scores as centipawns.
- Added `sekirei --version` and `sekirei -V` for release and packaging checks.
- Added `sekirei --help` and `sekirei -h` with a concise usage summary.
- Fixed the USI `ponderhit` abort/restart race and added an immediate-command
  regression test.
- Added `nnue_probe --strict` to reject constant-output or non-deterministic
  checkpoint probes automatically.
- Added a near-constant-output threshold (8 cp) to the strict NNUE probe so
  collapsed checkpoints do not pass the basic health screen accidentally.
- Included the strict threshold and pass/fail result in machine-readable probe
  output for auditable candidate selection.
- Extended the default NNUE probe set with king-placement sensitivity cases.
- Added `--cache-only` training mode and `analysis_record_v1` teacher-cache
  compatibility for bounded pilots that must not launch fallback searches.

## [0.3.23] – 2026-09-02

- Updated the `lineprior` dependency from 0.9.0 to 0.11.1 in the USI and
  training crates; workspace compilation and tests pass with the new API.

## [0.3.22] – 2026-09-01

- Improved NNUE candidate-health diagnostics and bounded teacher-cache
  training controls.

## [0.3.20] – 2026-08-31

- Promoted the official A-flat NNUE v1 candidate after a preregistered Gate 3
  SPRT pass against the material baseline: 56 wins, 2 losses, 0 draws in 58
  games, with an estimated relative advantage of +578.9 Elo.
- Added reproducibility records for the Gate 3 configuration, artifact hashes,
  and the limitation that this result is relative to the material baseline,
  not an absolute Floodgate rating.

## [0.3.19] – 2026-08-30

- Added deterministic reload verification to `nnue_probe` output.

## [0.3.18] – 2026-08-30

- Added an explicit `constant_output` flag to `nnue_probe` diagnostics.

## [0.3.17] – 2026-08-30

- Clarified `nnue_probe` help text to include its mean and variance diagnostics.

## [0.3.16] – 2026-08-30

- Added score mean and variance to `nnue_probe` diagnostics for detecting
  constant-output checkpoints cheaply.

## [0.3.15] – 2026-08-30

- Documented `nnue_probe` JSON and custom-SFEN usage in the English and
  Japanese README files.

## [0.3.14] – 2026-08-30

- Added regression coverage for `nnue_probe` JSON rendering, including score
  ranges, reference deltas, and escaped string values.

## [0.3.13] – 2026-08-30

- Added machine-readable JSON output to `nnue_probe` for automated evaluator
  diagnostics and checkpoint comparisons.

## [0.3.12] – 2026-08-30

- Added focused argument-parsing tests for `nnue_probe`, covering default
  probe names, custom SFEN ordering, and malformed options.

## [0.3.11] – 2026-08-30

- Improved `nnue_probe` with named default probes and score deltas from the
  reference position for clearer lightweight evaluator diagnostics.

## [0.3.10] – 2026-08-30

- Added the lightweight `nnue_probe` diagnostic for checkpoint score range and
  material-sensitivity checks without process-global weight state.

## [0.3.9] – 2026-08-30

- Added side-effect-free NNUE checkpoint evaluation via explicit weight
  instances, so diagnostics and candidate comparisons do not depend on the
  process-global `EvalFile` load order.
- Added a regression test proving explicit evaluation preserves the board and
  incremental accumulator state.

## [0.3.8] – 2026-08-30

- Hardened NNUE reload and search-abort result attribution.
- Added TT depth-preference and immediate-deadline legality regression tests.
- Improved release, publish, and test-harness reproducibility checks.

## [0.3.7] – 2026-08-30

- Hardened NNUE checkpoint loading and saving against malformed, non-finite,
  truncated, and colliding artifacts.
- Made teacher-cache, diagnostic sidecar, opening-book, and dataset-export
  outputs atomic, durable, deterministic, and error-propagating.
- Added lightweight evaluator and diagnostic probes without running heavy
  training or strength measurements.
- Search now returns a legal fallback move when a hard deadline arrives before
  the first iterative-deepening result completes.

## [0.3.6] – 2026-08-30

- Aligned all workspace crate versions and `Cargo.lock` with the `v0.3.6` release.
- Updated the public documentation and release metadata for the current distribution.

## [0.3.5] – 2026-08-19

Safer search contracts and clearer integration documentation. No search
behavior change for the default build; no strength claim.

### Search correctness

- Fixed #6: none of `alpha_beta`'s 5 `store_tt` call sites were guarded on
  `skip_move`, so the singular-extension verification search (which
  excludes `tt_mv`) could write its result into the shared TT under the
  position's ordinary hash — indistinguishable at probe time from a
  genuine unrestricted search. `Tt::store`'s depth-preferred replacement
  rule incidentally blocked this in practice, but that was never an
  enforced invariant. `skip_move` is now threaded into `store_tt` itself
  (single guarded call site), with a regression test that removes the
  depth-preference confound by storing at matched depth.

### Testing / resource predictability

- Fixed #9: added the one missing piece of `SpecTopN`'s USI-option
  exposure — a fixture pinning `SpeculativeSearcher`'s dedicated thread
  pool size to the configured `top_n`, so the `Threads + SpecTopN`
  capacity-planning formula (`docs/design/pr5_pool_isolation_static_audit.md`)
  has regression coverage on the `SpecTopN` side.

### Documentation

- `docs/design/shared_tt_write_topology_audit.md` updated with the issue
  #36/PR #37 abort-driven-TT-store fix outcome (the audit's own Finding 4
  follow-up).
- New `docs/nnue_weights.md`: NNUE weight model card and licensing —
  clarifies that no production-recommended weight file is currently
  distributed, that the software license (MIT/Apache-2.0) does not
  automatically extend to any future weight file, the `SEKIRW01`/
  `SEKIRW02`/`JANOSW03` format compatibility table, the king-relative
  ("B-small") architecture's current `MECHANICAL_PASS / EXPERIMENTAL_HOLD`
  status, and a model-card template for tracking a specific checkpoint's
  provenance.
- New `docs/mobile_integration.md`: current on-device/mobile integration
  surface (USI binary only, no official FFI layer), the small
  mobile-relevant dependency footprint, resource-planning USI options
  (`Threads`, `SpecTopN`, `Hash`, `EvalFile`), and explicit "not yet true"
  limitations — written for the real integration questions raised in #44.

### Engineering

- New `sekirei --build-info`: prints this build's name/version/NNUE
  architecture/weight-format-magic/expected-weight-size/`SpecTopN`
  default as JSON, without starting a USI session — lets a caller detect
  a weight-file/binary architecture mismatch before attempting to load
  one, rather than only at `read_weights`'s runtime error.

## [0.3.4] – 2026-08-12

Groundwork for the next strength-lever experiment (king-relative NNUE
features), landed as opt-in infrastructure. **No default-build behavior
change**: `king_relative_b_small` is a Cargo feature flag, default OFF —
`sekirei`/`sekirei-train` built without it are unaffected in every way
(same byte layout, same weight-file format, same search behavior). This
release makes no Elo or playing-strength claim for the new feature; see
"Known limitations."

### NNUE (opt-in, default OFF)

- New `king_relative_b_small` Cargo feature (`sekirei-core`, forwarded from
  `sekirei`/`sekirei-train`): king-relative board features, bucketed by a
  3×3-region zone (`Square::king_zone`) around each perspective's own king
  (`INPUT` 2420 → 20564 when enabled; weight file magic bumps to
  `SEKIRW02`, no legacy-format fallback for that variant). `Board::do_move`/
  `undo_move` gain a feature-gated hook that fully rebuilds the NNUE
  accumulator on a king move (every board feature for that perspective
  depends on the moved king's zone); `NnueAcc.king_sq` is tracked
  unconditionally (both build configurations) so this path is exercised by
  default CI regardless of the flag. `Board::startpos()` switched from a
  hand-rolled per-piece accumulator loop to a full refresh — a real
  correctness fix for the flag-enabled build, a no-op under the default one.
- Structural sanity, not strength: a fixed-depth A/B comparison (base vs.
  `king_relative_b_small`, same commit, deterministic LCG-default
  evaluation, 21 positions) found zero correctness issues (no
  panic/timeout/illegal-move/incomplete-output either side).

### Training pipeline

- New calibration metric (`diagnostics::expected_calibration_error`):
  Expected Calibration Error over predicted-win-probability deciles, using
  the same linear cp↔probability mapping the WDL training target itself
  uses. Wired into the `.meta.json` sidecar, `--eval-only`, and the
  per-epoch console output, alongside the existing `valid_cp_mse`/
  `valid_wdl_loss` metrics.
- New `scripts/run_king_relative_phase2.sh` (multi-seed training
  orchestration) and `scripts/select_king_relative_checkpoint.py`
  (validation-metrics-only pass/fail gate, ≥2-of-3-seeds bar with
  collapse/saturation-regression hard stops) — not yet run against real
  training data.

### CI / gate tooling

- `.github/workflows/fixed-depth-ab.yml`: new `base_features`/
  `candidate_features` inputs, so a run can compare one commit's Cargo
  feature flag on vs. off, not just two different commits.
- `scripts/sprint_gate.sh`: new `ENGINE1_BIN`/`ENGINE2_BIN` env overrides
  (both default to the existing single auto-built binary — an unset
  invocation is unchanged), for comparing two different binaries rather
  than only two weight files.
- New CI job (`king-relative-feature`) builds/clippies/tests the workspace
  with `king_relative_b_small` enabled on every push/PR — previously that
  code path was verified only by local runs.

### Housekeeping

- Removed 8 small `results/` files left tracked in git from before that
  directory was gitignored (2026-07-01–07-04 debris, nothing referencing
  them).

## [0.3.3] – 2026-08-12

A search-correctness patch release, plus documentation/evaluation-provenance
cleanup. One real engine-behavior fix (below); everything else is docs,
dependency, and process hygiene on top of v0.3.2.

### Search correctness

- **Fixed (issue #36, PR #37): abort-driven TT store could persist a
  mislabeled bound, or a corrupted `0` score, from a truncated search.**
  `root_search_inner` and two loops in `alpha_beta` (`crates/sekirei-core/src/search.rs`)
  used to `break` on a search-deadline abort but then fall through to an
  unconditional final TT store, persisting a partial result as if the
  search had completed. A second, more severe path: the abort check only
  ran at the top of each loop iteration, not immediately after that
  iteration's own recursive `alpha_beta` call — if the deadline landed
  *during* that recursive call, its abort-sentinel return value (`0`)
  could be used as a real score, corrupting `best_score`/`best_move`, or
  even firing a fabricated `Bound::Lower` TT store mid-loop. Fix: an
  `aborted` flag now gates every final store, and a `should_abort()`
  recheck runs immediately after each loop's recursive call, before its
  result is used — matching the ordering `spec_alpha_beta` already used
  correctly. 3 new deterministic regression tests (no timing dependence),
  each independently confirmed to fail without the fix.

### Dependencies

- `lineprior` 0.9.0 → 0.10.0. The dependency's own CHANGELOG confirms the
  only source-breaking change (3 new `TuneParam` variants) is never
  referenced by Sekirei; diff scoped to `sekirei-train`/`sekirei-usi`/
  `Cargo.lock`.

### Documentation / evaluation provenance

- **Corrected an evaluation-provenance issue in a prior internal
  experiment.** An internal B-vs-C strength gate comparing `UseYBW=true`
  vs. `UseYBW=false` had been run against a commit that turned out **not
  to be an ancestor of `main`** — an unmerged side branch that predates
  several of this release's own correctness fixes and adds
  measurement-only toggles that don't exist on `main`. That gate's result
  is **not** treated as evidence about `main`'s YBW implementation and is
  not cited as such anywhere in this project's planning docs; see
  `docs/experiments/gate_redesign_low_load.md` and internal roadmap notes
  for the corrected framing. No code or default behavior is affected by
  this correction — it's a documentation/process fix.
- New: `docs/design/nnue_architecture_next_candidate.md` — a first
  research pass comparing candidate next-step NNUE architecture changes
  (king-relative input features vs. widening the feature transformer vs.
  widening the second hidden layer) on parameter count, inference cost,
  and weight/training-data compatibility. No implementation in this
  release.
- New: `docs/design/shared_tt_write_topology_audit.md` (issue #32) —
  static audit of the shared transposition table's write topology under
  YBW/speculative-search concurrency. Confirmed `Tt::store`'s equal-depth
  replacement is unconditional last-writer-wins with no CAS, and that the
  XOR-trick storage format can't produce cross-writer field mixing.
  Classified 3 plausible-but-unconfirmed concurrency races; found this
  release's issue #36 as a byproduct. No fix applied yet — recommended
  next step (an instrumented live replay) is not started.
- Recorded: PR #17's (quiescence-search TT integration, issue #8)
  `repeats`-mode re-evaluation came back unfavorable — worse bestmove-
  variance and node-swing than an already-lower-noise baseline. Not a
  confirmed regression, but not merge-recommended either. PR #17 stays
  draft.

## [0.3.2] – 2026-08-10

A distribution/reproducibility patch release — **no engine behavior,
search algorithm, or public API changes**. Everything here is CI,
dependency sourcing, and packaging, on top of the v0.3.1 code as
published to crates.io.

### Dependencies

- `lineprior` switched from a git tag pin (`{ git = "...", tag = "v0.9.0" }`)
  to a plain crates.io version requirement (`"0.9.0"`) in both
  `sekirei-train` and `sekirei` (the `sekirei-usi` crate) — `lineprior`
  0.9.0 is now published to crates.io. No source changes beyond the
  manifests; the registry version resolves to the exact commit these
  crates were already pinned to. This was the last blocker preventing
  `sekirei-train`/`sekirei` from being published to crates.io at all
  (cargo refuses to publish a crate with a git-sourced dependency that
  has no version requirement).

### Packaging / crates.io

- All 6 workspace crates (`sekirei-core`, `sekirei-bench`, `sekirei-csa`,
  `sekirei-match-runner`, `sekirei-train`, `sekirei`) are now published
  to crates.io. `sekirei-core`/`bench`/`csa`/`match-runner` were
  previously published at 0.3.1; `sekirei-train`/`sekirei` are newly
  published starting with this release, now that the `lineprior`
  blocker above is resolved.
- Publishing moved from a manually-run `cargo publish` with a long-lived
  API token to crates.io's **Trusted Publishing** (GitHub Actions OIDC):
  a new `.github/workflows/cargo-publish.yml` authenticates per-crate via
  a short-lived (30-minute), automatically-revoked token requested
  through `rust-lang/crates-io-auth-action`. No `CARGO_REGISTRY_TOKEN`
  repository secret exists.
- That workflow is hardened against a real publish going wrong in ways
  that can't be undone (a bad crates.io version can be yanked, never
  deleted): it pins to an exact release tag rather than whatever ref
  happened to trigger it, verifies every requested crate's `Cargo.toml`
  version matches that tag before publishing anything, validates its
  own crate-name input strictly, and polls the crates.io index for
  `sekirei-core` to actually appear before publishing anything that
  depends on it — instead of a fixed sleep.

### CI

- Fixed a `rustc` crash (`SIGILL`, illegal instruction) that could occur
  compiling proc-macro dependencies on GitHub-hosted runners. Cause:
  `.cargo/config.toml` sets `-C target-cpu=native` workspace-wide,
  intentional for local release builds tuned to a known machine, but
  GitHub Actions runners aren't guaranteed identical hardware between
  runs — "native" CPU feature detection isn't reliable in that
  environment. CI workflows (`ci.yml`, `cargo-publish.yml`,
  `fixed-depth-ab.yml`) now override `RUSTFLAGS` to disable this for CI
  specifically; local dev/release builds are unaffected.

## [0.3.1] – 2026-08-10

This is the first published release since 0.2.4. A
`v0.3.0` tag exists in git history (2026-07-16) but no GitHub Release was
ever published from it — see the note on the 0.3.0
section below. Accordingly, everything in this section covers the full
`v0.2.4...v0.3.1` range, including some pre-existing work that landed
before the `v0.3.0` tag but was never previously written up in this file
(the [0.3.0] section below only ever documented training-pipeline changes).
Training-pipeline changes from before the tag are **not** repeated here —
see the [0.3.0] section for those.

### Search

- **Fixed a structural search-depth stall** (PR #5, headline fix of this
  release). `SpeculativeSearcher` submitted its background tasks to rayon's
  shared *global* pool — the same pool the main alpha-beta search's own
  parallel (YBW) dispatch depends on. Since speculative tasks are unbounded
  and never cancelled, they could occupy every worker thread and starve the
  main search of a thread to run on, freezing search depth regardless of
  remaining time budget. Confirmed via profiling (main search thread blocked
  in `pthread_cond_wait` while a worker computed an unbounded speculative
  line) and reproducible depth-scaling tests. Fix: `SpeculativeSearcher` now
  gets its own dedicated thread pool, structurally isolated from the main
  search's pool.
- Fixed speculative-search TT mate-score corruption (issue #7, PR #13):
  speculative search's internal TT stores didn't apply the ply-relative
  mate-score encoding every other read/write site already used, so a
  mate-adjacent score computed by a speculative task could be misread later
  from a different ply via the shared TT.
- Fixed a same-parent-hash race in speculative search (issue #14, PR #16):
  every candidate move's speculative task independently stored its result at
  the same shared parent TT entry, so the surviving entry was whichever task
  finished last rather than the best candidate — a real source of
  `SpecTopN>0` search nondeterminism, confirmed and measured (not just
  theorized) via a purpose-built gate `repeats` mode: across 21 positions run
  3 times each at `SpecTopN=3`, the number of positions with an unstable
  bestmove dropped from 9/21 to 3/21 after this fix. Node-count variance
  improved on the primary (median) measure but the picture is genuinely
  mixed at the tail — see Known limitations below; not claimed as a full fix
  for issue #14.
- A search/gate-hang fix predating the `v0.3.0` tag: bounded qsearch depth,
  time-bounded speculative tasks, and an OS-thread watchdog on the hard
  deadline, plus a fix for the TT not being cleared on `usinewgame` and CPU
  oversubscription in self-play from a missing `Threads` `setoption`.
  Also from this era: recursive SEE, a quiet-check safety filter with a YBW
  sibling cap, and NMP/LMR/delta-pruning refinements.
- Two related fixes remain **implemented but not merged**, excluded from
  this release — see Known limitations: quiescence-search TT integration
  (issue #8, PR #17) and singular-extension verification search (issue #6,
  PR #4).

### USI / Runtime

- Fixed a `setoption Hash` output-ordering race (issue #10, PR #15):
  `setoption Hash` now aborts and joins any in-flight search before
  rebuilding the searcher's TT and thread pool, closing a window where a
  stale `bestmove` from the old search could arrive interleaved with later
  protocol output.
- New `SpecTopN` USI option (issue #9, PR #18): exposes speculative search's
  top-N candidate count (default 3, matching prior hardcoded behavior — no
  behavior change unless set), including `0` to disable speculation
  entirely, without a rebuild. The abort-and-join fix from PR #15 was
  generalized into a shared helper applied at all 7 places engine state gets
  rebuilt (`go`, `usinewgame`, `setoption Hash`, `setoption SpecTopN`,
  `stop`, `ponderhit`, `quit`).
- New runtime safety invariants (direct commits, no PR): the engine now
  hard-aborts with a full diagnostic dump instead of silently continuing
  whenever a computed bestmove is actually illegal, or whenever an
  independently-replayed shadow reconstruction of a `position` command's
  move history disagrees with the engine's own incrementally-maintained
  board. Runs unconditionally on every `position` command. This is a
  detection safety net for an intermittent, still-unreproduced
  position-replay corruption observed during a training gate — see Known
  limitations; the underlying root cause remains open.
- Predating the `v0.3.0` tag, not previously documented: pondering, MultiPV,
  a soft-limit instability extension, `binc`/`winc`/`movestogo` time
  control, and a fix for `setoption EvalFile` never actually activating NNUE
  when set via GUI (only worked from the CLI).

### Training

Training-pipeline work in this range that predates the `v0.3.0` tag —
including the `TrainWeights::new_seeded(seed)` symmetry-collapse fix — is
documented in the 0.3.0 section below, not repeated
here. New since that tag:

- Extensive off-by-default diagnostic instrumentation investigating an
  epoch-1 L2/FT neuron-collapse and CP/WDL gradient-target dynamics:
  per-neuron per-position tracing, a shuffle-seed control (found data
  order — not initialization — materially affects whether collapsed L2
  neurons recover), CP/WDL gradient decomposition, and configurable WDL
  target scale. All opt-in and off by default.
- A "teacher-conflict masking" training strategy (halting the FT/L2 update
  wherever the CP and WDL teachers disagree) was implemented and evaluated
  in a paired gate against a rate-matched control. **Rejected, not
  adopted** — underperformed the control.
- Fixes: shuffle seed is now recorded in checkpoint metadata (previously
  missing); validation now emits the same progress heartbeat training
  already had; the teacher-search cache now writes via temp-file+atomic
  rename instead of truncating in place, and filters entries by
  `label_depth` so caches from different depths can't silently blend;
  `scripts/select_longrun_checkpoint.py`'s hardcoded L2 width was corrected
  to the actual architecture (verified zero effect on prior checkpoint
  selections).

### Match / Gates / Validation

- Phase A2 launch-readiness hardening (PR #2): a fixed-seed, resumable
  Fisher-Yates permutation of the B1-vs-A opening corpus with a real
  diversity/contamination gate, `TimeForfeit` added as its own
  distinguishable end reason (previously collapsed into generic engine
  errors, so it could never actually be counted by the contamination
  check), and an immutable, append-only, hash-verified manifest. Bundled in
  the same PR: a match-runner protocol hardening (`stop` →
  `usinewgame` → `isready`/`readyok` draining, illegal-move/timeout process
  retirement) closing a stale-`bestmove`-leak-across-game-boundaries bug,
  plus per-move transcript logging.
- Read-only resource preflight checker (PR #12,
  `scripts/gate_resource_preflight.py`): inspects host load/swap/memory/disk/
  contending-process state before a gate or match launch and refuses, rather
  than silently proceeding, whenever any check can't be confirmed safe.
- Remote fixed-depth A/B tool (PR #19): an opt-in, `workflow_dispatch`-only
  GitHub Actions workflow that builds two engine binaries and drives them
  through a small fixed-position corpus at one fixed search depth, entirely
  on a GitHub-hosted runner. Explicitly a correctness/node-count structural
  pre-filter, not an Elo/SPRT/strength gate — CI has no NNUE weights
  available, so both binaries run the deterministic default evaluation.
- Hardened the same tool (PR #20), after dogfooding it against PR #17
  surfaced two real tooling bugs: a config-mismatch guard (fails closed if
  `Threads`/`SpecTopN` aren't both advertised by a binary, instead of
  silently comparing incompatible configurations) and a rewrite of the USI
  driver from a single blocking all-at-once command send to an interactive
  driver that waits for a real `bestmove` before ever sending `quit` (the
  engine's `go` is asynchronous; the old driver could race its own `quit`
  ahead of an in-flight search and misreport a real position as
  `bestmove resign`). Also added a `repeats` mode (run each side N times,
  report within-binary bestmove/node-count variance) — necessary because
  `SpecTopN>0` search was separately discovered, via this same dogfooding,
  to be measurably nondeterministic even for an identical binary run
  against itself (see Known limitations); a single-shot diff at
  `SpecTopN>0` has no resolving power, and `repeats` mode is now the
  standard way to evaluate any such candidate.
- `spread_ok`'s decile-keying semantics in the B1-vs-A gate design were
  audited and documented (PR #3) — read-only, no gate behavior changed.
- Predating the `v0.3.0` tag, not previously documented: the CI-based Elo
  gate (`sekirei-match gate`, later extended to `gate --sprt` — documented
  in [0.3.0] below) replaced an earlier ad-hoc point-estimate+LOS check, and
  a required opening-diversity check was added to strength gates after a
  350-game run was found to collapse into a handful of repeated games.
- The intermittent position-replay-desync investigation (see USI/Runtime
  above) originated from, and was closed out against, a training gate: the
  shadow-replay invariant closed the detection gap that let the original
  corruption go unflagged, and a follow-up gate run completed cleanly with
  it enabled. The root cause itself is still unresolved.

### Tooling

- `scripts/gate_resource_preflight.py` and the remote fixed-depth A/B
  tooling (both above).
- Predating the `v0.3.0` tag, not previously documented: the gate dashboard
  (`scripts/gate_dashboard.py`) gained live external-gate-run watching, an
  AI chat assistant panel, dark mode, a responsive layout, and pipeline
  visualization, ahead of the embedded-review-panels work already
  documented in [0.3.0] below.
- CI/build hygiene: fixed a `cargo doc -D warnings` bare-URL lint failure;
  assorted `cargo fmt` fixups.

### Dependencies

- `lineprior` bumped to 0.9.0 from its crates.io release; required adding two
  new required fields
  (`observed_at_unix_seconds`, `source`) to an `Observation` construction
  site.
- `veridict` bumped to 0.15.0 and switched from a pinned git revision to a
  published crates.io version now that one exists — required adapting 3
  breaking API changes (`MetricConfig::Elo` becoming a struct variant with a
  `FailurePolicy`, `compare_one` gaining a `cluster_by_id: bool`,
  `sprt::run` gaining 4 trailing parameters), all pinned to values that
  reproduce prior behavior exactly.
- `shogiesa`/`quietset` version-pin comments updated to 0.9.0/0.16.0
  (docs-only; verified compatible end-to-end against a hand-written
  schema-v11 fixture).

### Known limitations

- Issue #14 (speculative-search parent-hash TT race) is **partially**
  fixed by PR #16 in this release: bestmove nondeterminism at `SpecTopN=3`
  measurably improves (9/21 → 3/21 unstable positions across 3 repeats in
  the gate corpus used for evaluation), but a second, still-unidentified
  source of node-count variance remains. Tracked as a follow-up: a static
  audit of the remaining shared-TT write topology.
- `SpecTopN>0` search (the production default) is measurably
  nondeterministic even for an identical binary run against itself — this
  is intrinsic scheduling-order variance in `SpeculativeSearcher`'s
  concurrent background workers, not specific to any one change. Any future
  fixed-depth A/B evaluation at `SpecTopN>0` needs the gate's `repeats` mode
  (see above), not a single-shot diff.
- Issue #8 (quiescence-search TT integration, PR #17) is implemented,
  CI-green, and structurally clean at `SpecTopN=0`, but shows no
  distinguishable effect from the `SpecTopN=3` production-default noise
  floor. Not merged, not included in this release; no strength or
  node-reduction claim is made for it.
- Issue #6 (singular-extension verification search, PR #4) was not
  re-evaluated this release cycle and is not included.
- An intermittent position-replay desync (see USI/Runtime above) has a
  detection safety net in place but an undiagnosed root cause.
- This release makes no Elo or playing-strength claims. No comparison
  against other engines is made or implied.

## [0.3.0] – 2026-07-16

_Tag created, but no GitHub Release was published from it._

### Added
- `sekirei-train --lr <f> --lr-schedule constant|step-half|cosine --min-lr <f> --warmup-epochs <n>` — replaces the previously hardcoded `0.001 * 0.5^(epoch-1)` schedule (both `--games` and `--positions` paths). `step-half` remains the default and reproduces the old formula exactly. `min-lr` floors every schedule, not just `cosine` — unfloored `step-half` decays toward zero (~2e-9 by epoch 20), which made an early-stopped checkpoint hard to interpret (undertrained, or already past the point where the schedule mattered?).
- `sekirei-train --validation-ratio <f>` now also works on the `--games` (CSA) path, not just `--positions` — held-out split by **game index** (leakage-safe: every sampled position from a game lands on one side, never split across train/valid). Validation loss uses a new `Trainer::eval_game`, sharing the exact training objective (including the `--wdl-lambda` blend) via a shared `position_teacher` helper — routing through the positions path's `eval_positions` would have silently validated against a different (eval-only) objective whenever `--wdl-lambda` was set.
- Per-epoch training diagnostics (`diagnostics.rs`): FT/L2 "ever active"/"ever saturated" ratios (epoch-scoped — a dead neuron is one that never fires across a whole epoch, not a single-sample zero read), output mean/std, whole-parameter-vector update norm between epochs, post-quantization FT zero ratio. Printed per epoch and written to checkpoint `.meta.json`.
- Checkpoint `.meta.json` (both paths) now also records `git_commit`, `dataset_hash` (path+size fingerprint), `split_hash` (fingerprint of which positions/games landed in validation — distinguishes two different splits of the same dataset, which `dataset_hash` alone can't), `train_games`/`valid_games` (game-level counts, CSA path only), `architecture`, and the new LR-schedule fields. The `--games` (CSA) path writes `.meta.json` for the first time — previously only `--positions` did.
- CSA-path teacher-search caching: `position_teacher` (shared by `train_game`/`eval_game`) now caches each position's raw search score across epochs, mirroring the fix `eval_positions` already had on the positions path. Previously every epoch re-ran a full label-depth search on every sampled position — on a 20-epoch run this made epochs 2-20 pure repeat work (~2h/epoch observed, flat, zero speedup). `cache_hits`/`cache_misses` are logged per epoch and recorded in `.meta.json`.
- `sekirei-train --lr-schedule-epochs <n>` — shapes the LR curve for a horizon independent of `--epochs`, so a short run can reproduce the first N epochs of a longer schedule (e.g. `--epochs 3 --lr-schedule-epochs 20`). Defaults to `--epochs`, reproducing prior behavior exactly when omitted. Rejects `schedule_epochs=0`, `warmup_epochs > schedule_epochs`, and `schedule_epochs < epochs` outright instead of silently clamping (an earlier attempt at this reproduction always passed `--epochs` as the schedule horizon, compressing the whole cosine decay into the short run instead of reusing the long run's curve — this flag is the fix). `.meta.json` records both `epochs` and `lr_schedule_epochs`.
- Validation-set output stats (`ValidStats`, `.meta.json`, per-epoch log line): `valid_output_min`/`valid_output_max`/`valid_output_range`, computed directly (no variance-formula cancellation) alongside the existing `valid_output_std`. `std`'s cancellation can round a genuinely nonzero spread down to an exact `0.000` near total output collapse; `range` disambiguates "truly constant" (`range == 0.0`) from "collapsed but not literally frozen."
- `sekirei-match gate --sprt [--elo0 0] [--elo1 20] [--alpha 0.05] [--beta 0.05] [--sprt-variant wald|trinomial]` — sequential (SPRT) gate verdict alongside the existing CI-based one, using veridict's `sprt::run`. Reaches PASS/FAIL as soon as the log-likelihood ratio crosses a Wald boundary, often well before a fixed game count.
- `scripts/sprint_gate.sh SPRT=1` — opt-in early stopping: checks `gate --sprt` after every sprint and stops as soon as it's decisive. `MAX_GAMES` (default 1600) is a hard compute-budget cap, independent of `N_SPRINTS`, for the case where the true effect sits between `elo0`/`elo1` and SPRT would otherwise keep going indefinitely.
- `sekirei-train --wdl-lambda <f>` (`--games`/CSA path only) — blends the game's own result into the training teacher: `teacher = λ·eval_teacher + (1-λ)·wdl_target`. Positions from `GameResult::Unknown` games (aborted, timed out, illegal move, ...) fall back to eval-only, since there's no result signal to mix in for those.
- `csa.rs`: `GameResult` now recognizes `%SENNICHITE` (repetition → draw) and `%KACHI` (27-point declaration → win for the side that just moved) — previously both silently fell into `Unknown` (a combined ~13.9k games, ~3.9% of the current floodgate corpus).
- `scripts/cleanup_runs.sh` — prunes `data/runs/*/stage1`-`stage3` intermediates (raw extracts/observations/scored jsonl, often multi-GB) once a run has a `manifest.json` and is older than `MIN_AGE_DAYS` (default 3); skips runs referenced by name in `scripts/*.sh` (live cross-run dependencies) and runs with no manifest (still running or ad-hoc). Dry run by default, `APPLY=1` to delete. Wired into `redo_quietset_bc.sh`/`train_with_loss_mining.sh`/`train_with_shogiesa_quietset.sh` so old runs get pruned automatically each time a new one starts.
- `sekirei-train --grad-clip-norm <f>` (global) and independent per-layer `--ft-clip-norm`/`--l2-clip-norm`/`--out-clip-norm` — optional gradient-norm clipping controls.
- `sekirei-train --l2-bias-init <f>` — tunable L2 layer bias at initialization (default `0.5`).
- `scripts/gate_dashboard.py`: embedded review panels (training-pipeline, individual gate result, project-wide trend) with deterministic Python-computed numbers/verdicts and an optional, strictly descriptive LLM narrative (never allowed to originate a number or override a verdict). Three distinct, deliberately non-interchangeable verdict vocabularies — gate: `PASS`/`FAIL`/`INCONCLUSIVE`; pipeline: `HEALTHY`/`WARNING`/`INSUFFICIENT_DATA`/`INVALID`; project trend: `IMPROVING`/`MIXED`/`FLAT`/`REGRESSING`/`INSUFFICIENT_EVIDENCE` with an explicit confidence level and positive/negative evidence lists, never a bare pass/fail — so a numerically healthy training run is never conflated with a playing-strength claim.

### Fixed
- **`TrainWeights::new()` → `new_seeded(seed)`: broke a symmetry-collapse bug present in every trained network to date.** `ft`/`l2`/`out` were zero-initialized; with no source of asymmetry, every unit within a layer receives an identical gradient every step (backprop through a uniform downstream weight is itself uniform), so the whole net converges to and stays at effective width 1 per layer forever, no matter how much data or how many epochs. Confirmed by parsing real trained weights (`v007` through `v012`): every FT row, every L2 row, and `out` were each a single repeated scalar, variance exactly 0.0 — the declared 256/32-wide architecture was training as a linear (KP-style) evaluator. Fix: seeded He/Kaiming-uniform init for `ft`/`l2`/`out` (biases unchanged — they solve a narrower, unrelated dead-ReLU problem). `Trainer::new(seed)` / `--seed` (already existed for validation split and source_cap) now also seeds weight init, so training stays fully reproducible for a fixed seed. Verified: post-init variance > 0 in every layer, stays > 0 after training, and two identical `--seed`-fixed runs (`--label-depth 1` and `--label-depth 4`, the latter exercising the rayon-parallel search path) produced byte-identical output weight files despite differing wall-clock schedules.

## [0.2.4] – 2026-06-28

### Added
- `sekirei-train --positions <jsonl>` — accept a [shogiesa](https://github.com/kent-tokyo/shogiesa) `positions.jsonl` file as an alternative to `--games`; skips CSA parsing and trains from pre-extracted SFENs with phase/side/source metadata.
- `PositionSample`: carries `phase`, `side_to_move`, `ply`, `source` from shogiesa tags for training control.
- `--phase-weights <spec>` — per-phase loss multipliers (e.g. `opening=0.5,middlegame=1.0,endgame=1.2`).
- `--side-balance` — equalise black/white sample weights based on training-split distribution.
- `--source-cap <N>` — deterministic hash-based per-source sample cap (seed-reproducible, order-independent).
- `--validation-ratio <f>` / `--seed <n>` — hold-out split via SFEN hash; logs `loss_raw` and `loss_weighted` per epoch.
- `--checkpoint-dir <dir>` — save epoch checkpoints to a custom directory with `.meta.json` (training params + sample counts).
- `--teacher-cache <path>` / `--reuse-teacher-cache` — cache teacher scores (sfen → score_cp) to JSONL; epoch 2+ skips search entirely on cache hits.

## [0.2.3] – 2026-06-28

### Added
- `sekirei-train --label-threshold-cp <n>` — configurable adv/equal/disadv boundary (default: 120 cp).
- Epoch stats log: `missing_rate`, `avg_weight`, `matched` counts printed per epoch when `--scored` is active; `missing_rate > 50%` triggers a SFEN-mismatch warning.
- `Trainer::reset_epoch_stats()` — resets `total_loss / total_count / total_weight / dropped_missing` between epochs so per-epoch stats are correct.

### Fixed
- `avg_loss` now divides by `total_weight` (sum of stability weights) instead of `total_count`; previously under-reported loss when `--stability-weighted` was active.
- `scored.rs`: duplicate SFENs in the scored JSONL are now averaged (previously last-wins, which made results dependent on file ordering); switched JSON parsing from manual string scan to `serde_json`.

## [0.2.2] – 2026-06-28

### Added
- `setoption MoveOverhead` (default 50 ms) — subtracts network latency from time budget.
- `setoption Ponder` option declaration; `go ponder` treated as infinite search.
- `ponderhit` command — aborts ponder search; GUI follows with a real `go`.
- `sekirei-train --export <path>` — exports observation JSONL for quietset stability filtering.
- `sekirei-train --depths <list>` — comma-separated search depths for export (default: `4,6,8`).
- `sekirei-match-runner --games-per-position <n>` — cover-all mode: play N games per SFEN entry.
- `sekirei-train --quiet`, `--min-ply`, `--label-depth` — quiet position filtering based on "Study of the Proper NNUE Dataset".
- `setoption Threads` — configure rayon global thread pool from GUI.

### Fixed
- **`go depth N` time cap**: pure depth search (no clock args) no longer capped at 50 ms.
- **TT size**: `Tt::new` now uses floor-power-of-two; previously halved capacity for power-of-2 inputs (e.g. 64 MB → 32 MB).
- **Root TT bound**: stores `Bound::Lower` on fail-high instead of always `Bound::Exact`.
- **USI search thread race**: `JoinHandle` now stored and joined on `stop`/`usinewgame`/`go`/`quit`; prevents stale `bestmove` output.
- Time control: tighter divisor (÷15) when < 30 s remain; panic mode when < 5 s and byoyomi exists.
- CSA client: `dotenvy` loads `.env`; env vars renamed `FLOODGATE_ACCOUNT` / `FLOODGATE_TRIP`.

## [0.2.0] – 2026-06-28

### Added
- Match runner: Elo rating, CI, LOS, illegal-move detection, repetition draw, SFEN openings.
- `SpeculativeSearcher` enabled in USI; king-capture panics fixed.
- NNUE training pipeline improvements.
- GitHub Actions CI + smoke job; all clippy warnings fixed.
- `setoption EvalFile` support in USI engine.
- CI pre-commit hook (`.githooks/pre-commit`).

### Fixed
- Mate score direction in `spec_alpha_beta`.
- NMP fail-soft + depth-scaled LMR formula.
- **CSA time tracking**: `parse_time_from_echo` now handles `+9796FU,T18` server echo format; `time_left_ms` was never decremented before.
- Read `Total_Time`/`Byoyomi`/`Increment` from `Game_Summary` header instead of parsing the game_id string.

## [0.1.0] – Initial

- NNUE-based shogi engine with alpha-beta search.
- CSA v2.2 TCP client for floodgate.
- USI protocol support.
