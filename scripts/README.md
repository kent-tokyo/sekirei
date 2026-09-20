# Script index

The `scripts/` directory contains reproducibility and validation tools, not a
second public CLI. Prefer Cargo binaries for normal engine use. Scripts may
write under ignored `data/` or `results/`; inspect `--help` and use a fresh run
directory before starting a long job.

## Release and public-contract checks

- `check_release_metadata.py`: crate versions, lockfile, changelog, README,
  license files, optional tag, and release manifest. A pre-publish manifest
  must be checked with `--allow-planned-release-manifest`; the default
  `--require-release-manifest` accepts only a post-publish, workflow-verified
  manifest.
- `check_public_surface.py`: keeps internal `ROADMAP.md` out of Git and checks
  required license references.
- `check_documentation_references.py`: verifies local paths in both READMEs.
- `validate_release_manifest.py`: schema and artifact validation.
- `validate_nnue_release_artifact.py`: verifies a versioned NNUE binary,
  checksum, model card, license boundary, and the declared local gate scope.
- `test_public_contracts.sh`: lightweight aggregate for the public boundary.

## Performance diagnostics

- `run_component_benchmark.py`: capture component timings with provenance.
- `nnue_forward_bench`: Cargo benchmark binary for a named weight file's
  forward pass on fixed fixtures. `q21_same_time_profile.py` captures the
  complementary fixed-time T-versus-material search-cost profile; both are
  diagnostics, not strength gates.
- `component_benchmark_preflight.py`: load, thermal, and measurement-contract
  checks before a capture.
- `run_component_aa_batch.py`, `aggregate_component_aa.py`: A/A noise-floor
  collection and aggregation.
- `compare_component_benchmarks.py`,
  `aggregate_cross_library_components.py`: bounded cross-run comparison.
- `preflight_speed_corpus.py`: validates the fixed speed corpus before timing.

Reports under `benchmark_reports/` are historical measurements on their named
host and revision. They are not general speed or Elo rankings.

## Strength gates and search diagnostics

- `run_fixed_depth_ab.py`: guarded fixed-depth A/B or repeatability run.
- `gate_resource_preflight.py`: resource admission check.
- `gate_orchestrator.py`: resumable shard orchestration.
- `create_strength_gate_manifest.py`, `record_strength_gate_execution.py`,
  `finalize_strength_gate_execution.py`: freeze and audit a gate.
- `sprint_gate.sh`, `run_frozen_strength_gate.sh`: bounded wrappers for an
  already-defined gate; they do not choose a candidate.

Use `SpecTopN=0`, one engine thread, and one parallel shard for deterministic
correctness diagnostics unless a preregistered experiment explicitly tests
parallel behavior. A completed harness run is not automatically a strength
result.

## Local self-play collection

- `run_local_selfplay.py`: one-command, offline self-play collector. It builds
  missing release binaries, runs the same engine on both sides, and stores
  `run-manifest.json`, a combined log, USI kifu, replay-validated CSA games,
  a per-move transcript with USI score/depth/nodes/PV, incremental JSONL and
  summary snapshots, and `dedup-index.json` under a fresh
  `data/runs/local_selfplay_<UTC>/` directory. A collection run must name both
  `--weights <file>` and `--positions <file>`; the manifest records the weight
  SHA-256, strict probe result, evaluator kind, and fixed USI options. The only
  exception is an explicit `--material-only --startpos-smoke` run of at most
  two games, which is marked as non-NNUE, non-strength evidence. Use
  `--games-per-position <n>` for a fixed opening corpus. Openings and engine colors are exhausted before a
  condition is reused; raw duplicates are retained while the dedup index names
  one representative for downstream training. `--dry-run` checks the planned
  command and writes a manifest without playing. A weighted run fails closed
  before launching an engine unless `nnue_probe --strict` passes; it records
  the engine, runner, probe, weights, opening file hashes and explicit
  `NnueOutput`. It also records source identity, toolchain, and resource
  preflight. A signal interruption writes an SHA-256-backed
  `interruption-snapshot.json`; this is distinct from a completed aggregate.
  Empty or invalid opening files are rejected. The transcript
  stores only the last completed primary-PV iteration, avoiding mixed-depth or
  secondary-MultiPV scores. `build_selfplay_ledger.py` merges replay-verified
  representative runs into a development-only split/overlap ledger and freezes
  a bounded diagnostic position set; it is not a training exporter.
- `profile_nnue_transcript.py`: measures static output diversity from an
  explicitly named NNUE weight file on replayable transcript positions. It
  records the source evaluator separately, so a material or legacy run cannot
  be relabelled as NNUE evidence.
- `summarize_q12_screen.py`: rebuilds a Q12 engineering-screen manifest from
  every completed `pairN/result.json`. It rejects incomplete pairs, mismatched
  engine contracts, and invalid two-game colour pairs rather than allowing a
  hand-maintained aggregate to decide candidate promotion.
- `test_usi_state_transitions.py`: launches the release USI and checks
  stop/ponder/ponderhit state transitions, one-bestmove-per-search behavior,
  current-root mate-score signs, and the limited scope of root-safety metrics.
- `compare_cshogi_oracle.py`: compares exact legal USI move sets for a fixed
  SFEN corpus with a diagnostic-only `cshogi==1.0.5` Linux container. Build it
  with `docker build -t sekirei-cshogi-oracle:1.0.5 -f
  scripts/cshogi-oracle.Dockerfile .`, then run
  `python3 scripts/compare_cshogi_oracle.py`. The container is an oracle, not
  a runtime dependency or a correctness verdict when the two implementations
  disagree.

Same-engine self-play is useful training and regression data, but it is not
an Elo measurement or evidence that either version is stronger.

## Floodgate and post-game analysis

- `floodgate_supervisor.py`: persistent process supervision and bounded retry.
- `record_floodgate_manifest.py`, `finalize_csa_run_manifest.py`: preserve
  binary, options, records, and completion status.
- `analyze_floodgate_analysis.py`, `classify_swing_positions.py`: convert
  recorded sidecars into review candidates without inventing missing values.
- `run_core_floodgate_diagnostic.py`: local re-search of frozen positions.
- `run_selfplay_swing_diagnostic.py`: replay selected self-play positions with
  their exact game history, then compare free, forced-root, TT, NMP, LMR, and
  fixed-depth cells. It isolates causes; it is not move-regret evidence.
- `run_root_ordering_diagnostic.py`: records legal-generator root order,
  completed iterative-deepening passes, and opt-in root mate-safety cost for
  replayed self-play positions. Disabling root mate safety is diagnostic-only.

Credentials must be supplied at runtime. Never put `FLOODGATE_TRIP` in Git,
manifests, plist files, command transcripts, or saved logs.

## NNUE training and checkpoint diagnostics

- `run_self_distill_multiseed.sh`: reproducible multi-seed self-distillation.
- `record_resume_run.py`, `attach_resume_manifest.py`: record verified resume
  lineage without modifying the source release manifest.
- `compare_teacher_evals.py`, `analyze_nnue_calibration.py`,
  `analyze_nnue_outliers.py`: evaluator diagnostics.
- `build_teacher_strata_corpus.py`, `summarize_nnue_profile_strata.py`:
  deterministic phase/material/mate strata and root-error summaries. Material
  includes board and hands from the side-to-move perspective; mate and
  incomplete searches are excluded from ordinary-CP error metrics. They are
  calibration diagnostics, not checkpoint selection or strength gates.
- `select_longrun_checkpoint.py`, `select_king_relative_checkpoint.py`: apply
  experiment-specific, validation-only selection rules.
- `build_selfplay_color_pairs.py`, `freeze_selfplay_calibration_split.py`,
  `freeze_selfplay_diagnostic_corpus.py`: freeze C5 diagnostic inputs without
  reusing positions across their declared split.
- `run_fixed_selfplay_diagnostic.py`,
  `run_c5e_teacher_search_distribution_diagnostic.py`,
  `run_selfplay_calibration_holdout.py`, `run_selfplay_search_factorial.py`:
  run bounded evaluator, calibration, and search-factorial diagnostics.
- `audit_historical_nnue_teachers.py`, `diagnose_c5h_teacher_transfer.py`,
  `classify_fixed_selfplay_diagnostic.py`, and the two `summarize_*` scripts:
  audit or summarize saved diagnostics. They do not train, select, or deploy a
  weight.
- `cleanup_runs.sh`: dry-run by default; removes old completed intermediate
  stages only when `APPLY=1` is explicitly set.

The longer shell launchers preserve historical experiment recipes. Keep them
only while their result or selection document still references them.

## Tests and maintenance

Most Python tools have a matching `test_<name>.py` file and use only the
standard library. Run the focused test when editing a script, then run:

```bash
python3 scripts/check_documentation_references.py
python3 scripts/check_public_surface.py
python3 scripts/check_release_metadata.py
```

Do not add paired scripts that differ only by an environment-variable name or
hard-coded output path. Use one parameterized entry point instead. Remove
one-off instrumentation after its evidence has been captured and no live
workflow or document references it.
