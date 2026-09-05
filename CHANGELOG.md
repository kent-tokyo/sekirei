# Changelog

## Unreleased

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

- NNUE weight loading now rejects files with trailing bytes instead of
  silently accepting a malformed or incompatible artifact.
- NNUE weight loading now rejects `NaN` and infinite floating-point values
  before they can enter evaluation.
- NNUE weight saving now writes through a same-directory temporary file and
  atomic rename, preventing interrupted saves from leaving truncated artifacts.
- NNUE weight saves now call `sync_all()` before the atomic rename so a
  completed save is flushed before it becomes visible at the final path.
- NNUE weight saving now rejects non-finite floating-point values before
  creating a checkpoint file.
- Teacher-cache writes now flush the completed temporary file before the
  atomic rename and clean up the temporary file on write or rename failure.
- Teacher-cache entries are now written in sorted SFEN order for deterministic
  artifacts and reproducible cache hashes.
- Teacher-cache loading now ignores score values outside the representable
  `i32` range instead of allowing a lossy integer cast.
- Teacher labeling accepts an optional per-search hard time limit whose value
  is bound into cache identity, resume fingerprints, and checkpoint metadata.
- Checkpoint metadata sidecars now use the same flushed temporary-file and
  atomic-rename path as weight checkpoints.
- Diagnostic trace sidecars now use atomic writes as well, preventing partial
  JSON/JSONL artifacts after an interrupted training run.
- Checkpoint metadata serialization errors are now returned cleanly instead of
  panicking the trainer.
- Opening-book and dataset-export outputs now use flushed atomic writes,
  preventing interrupted runs from leaving partial artifacts.
- Book and dataset-export write errors are now propagated instead of being
  silently ignored.
- Diagnostic percentile calculations now ignore non-finite samples instead
  of panicking during sorting.
- Diagnostic mean/std and accumulated cosine metrics now return neutral zero
  values for non-finite inputs instead of propagating `NaN`/`Inf`.
- Pearson and vector-cosine diagnostics now apply the same finite-value guard.
- Diagnostic weight-difference and weight-norm calculations now return zero
  for non-finite results instead of poisoning checkpoint metadata.
- Diagnostic vector and layer norm calculations now apply the same finite
  result guard.
- `mean_std` now validates its computed mean and standard deviation before
  exposing them to checkpoint metadata.
- Diagnostic vector comparisons now return zero for length-mismatched inputs
  instead of silently truncating to the shorter vector.
- Added a deterministic NNUE weight save/load roundtrip guard covering every
  layer and bias array.
- Atomic NNUE, teacher-cache, and training-sidecar writes now use per-process
  unique temporary names, avoiding collisions between concurrent saves.
- L2 diagnostic norm helpers now reject malformed matrix shapes without
  panicking or indexing beyond the provided data.
- CP/WDL gradient trace construction now rejects mismatched accumulator
  shapes and non-finite means without panicking.
- Added lightweight material-evaluation probes for start-position neutrality,
  hand-piece value, and side-to-move sign handling.
- Extended the material probes to cover on-board piece value and side-to-move
  sign handling.

## [0.3.6] – 2026-08-30

- Aligned all workspace crate versions and `Cargo.lock` with the `v0.3.6` release.
- Updated the public documentation and release metadata for the current distribution.

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
