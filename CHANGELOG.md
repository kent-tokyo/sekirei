# Changelog

This file summarizes recent releases. The
[archived detailed history](CHANGELOG_ARCHIVE.md)
preserves older per-change notes.

## [Unreleased]

### Changed

- Move v0.4.0–v0.4.3 release notes to the archive, separate search unit tests
  from the production module, and share the DFPN/SharedMcts abort timer setup.
  Search behavior and public APIs are unchanged.
- Archive historical benchmark reports and unused release smoke transcripts,
  extract root-split unit tests, and share USI diagnostic-context construction.
- Share searcher field initialization across constructors and centralize CSA run
  manifest edits without changing the search policy or manifest format.
- Archive a run-specific historical gate script and retain cleanup protection
  for shell and Python references under `scripts/`.
- Move CSA protocol tests and match-runner gate/summarize logic into dedicated
  modules, and share empty USI search-result diagnostics across backends.

## [0.5.1] – 2026-10-20

Version 0.5.1 adds a WASM package identity available before search and a
validated path for publishing the same package through npm. The GitHub Release
archive remains the source for the npm publication check. The default search
and material evaluator are unchanged; this release makes no strength claim.

### Added

- Expose `packageInfo()` with the API version, engine version, source and build
  commits when known, release/candidate/local provenance, and the initial
  material evaluator identity.
- Verify tagged WASM build identity and archive checksums with a two-stage
  release manifest, then check per-file parity, lockfile integrity, and a fresh
  install before recording an npm publication as verified.

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

## Older releases

See [CHANGELOG_ARCHIVE.md](CHANGELOG_ARCHIVE.md) for v0.4.3 and earlier.
