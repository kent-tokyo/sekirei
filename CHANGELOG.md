# Changelog

This file summarizes recent releases. The
[archived detailed history](CHANGELOG_ARCHIVE.md)
preserves older per-change notes.

## [Unreleased]

No changes yet.

## [0.5.0] – 2026-10-20

Version 0.5.0 releases the optional in-house `nn_r6` HalfKP artifact with a
strictly validated provenance and material-gate bundle. The default material
evaluator and ordinary search policy are unchanged; this is not a general
playing-strength claim.

### Added

- Publish `nn_r6` as a CC BY 4.0 GitHub Release asset, with a model card,
  redacted training and whole-game hold-out manifests, and independent 1T/4T
  colour-reversed material-gate evidence tied to the pinned source commit.
- Retain fail-closed gate supervision, frozen opening-manifest binding, and
  bounded CSA receive diagnostics in the release line.

## [0.4.5] – 2026-10-19

Version 0.4.5 records fail-closed release evidence for the in-house `nn_r6`
HalfKP candidate. The public artifact bundle is not released because its card
does not yet satisfy the repository's strict release-artifact validator. The
default material evaluator and ordinary search policy are unchanged. This
release makes no general playing-strength claim.

### Added

- Preserve the recorded fixed-source 1T/4T `nn_r6` material-gate results and
  current-source compatibility receipt. Distribution remains blocked until a
  validator-accepted model card binds the owner attestation, evidence, license
  and artifact; `nn_r6` is neither default nor publicly released.

## [0.4.4] – 2026-10-17

Version 0.4.4 fixes an aspiration-window boundary and strengthens the
reproducibility of PGO and CPU-pilot evidence. The default evaluator and
ordinary search policy remain unchanged, so this release makes no general
playing-strength claim.

### Fixed

- Re-search the full aspiration window before returning an exact score after a
  widened window, including mate-band retries.
- Preserve bounded compiler/profile command diagnostics and absolute profile
  paths in incomplete PGO-lane receipts.

### Changed

- Require a hashed, candidate-bound CPU measurement record before an EC2 NNUE
  pilot can be considered ready.

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

## Older releases

See [CHANGELOG_ARCHIVE.md](CHANGELOG_ARCHIVE.md) for v0.3.73 and earlier.
