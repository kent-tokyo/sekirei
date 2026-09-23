# Changelog

This file summarizes recent releases. The
[archived detailed history](CHANGELOG_ARCHIVE.md)
preserves older per-change notes.

## [Unreleased]

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
