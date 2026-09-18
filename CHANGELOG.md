# Changelog

This file summarizes recent releases. The
[archived detailed history](CHANGELOG_ARCHIVE.md)
preserves older per-change notes.

## [Unreleased]

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
