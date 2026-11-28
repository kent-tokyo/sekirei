# Script index

Use the Cargo binaries for normal play. This directory contains development,
measurement, and release tools. Run commands from the repository root; inspect
`--help` and choose a fresh ignored output directory before starting a run.

This index describes available tools, not work to resume. Strength changes
are judged with the [standard strength test](../docs/strength_testing.md);
working rules are in [AGENTS.md](../AGENTS.md).

## Documentation and release checks

For documentation edits:

```bash
python3 scripts/check_documentation_references.py
python3 scripts/check_public_surface.py
```

The reference checker covers the two public READMEs; check links in other
edited files separately. For version or release-record edits, also run:

```bash
python3 scripts/check_release_metadata.py
python3 scripts/check_release_manifest_layout.py
```

| Purpose | Tools |
|---|---|
| Rust coverage and public contracts | `check_rust_coverage.sh`, `test_public_contracts.sh` |
| Release manifest validation | `validate_release_manifest.py` |
| Published crates and release assets | `verify_release_publication.py` |
| WASM package and npm parity | `check_wasm_package_metadata.py`, `check_wasm_npm_distribution.py` |
| Record verified WASM distribution | `finalize_wasm_release_manifest.py`, `record_wasm_npm_status.py` |
| NNUE release/candidate identity | `validate_nnue_release_artifact.py`, `validate_nnue_candidate_card.py` |

See [release records](../docs/releases/README.md),
[npm distribution](../docs/releases/npm_wasm.md), and
[NNUE weights](../docs/nnue_weights.md) for the applicable contracts.

## Measurements and diagnostics

Start with the [benchmark guide](../docs/benchmarks.md). NPS, elapsed time,
move quality, and playing strength require different evidence.

| Question | Tools |
|---|---|
| Does a search change alter deterministic results or timing? | `paired_search_diagnostics.py`, `run_fixed_depth_ab.py` |
| How does search scale across threads? | `nps_threads.py` |
| How does a pinned YaneuraOu build compare? | `compare_yaneura_speed.py`, `run_ab_match.py` |
| Which component costs time? | `run_component_benchmark.py`, `component_benchmark_preflight.py`, `run_component_aa_batch.py`, `compare_component_benchmarks.py` |
| Is a move or PV legal? | `preflight_speed_corpus.py`, `compare_cshogi_oracle.py`, `validate_paired_pv.py` |
| How do selected NNUE moves compare? | `compare_selected_nnue_moves.py`, `classify_selected_move_outcomes.py`, `nnue_screen_robustness.py` |
| What happens along a tactical line? | `run_tactical_line_diagnostic.py` |
| Where does search spend work? | `run_q26_search_profile.py`, `q21_same_time_profile.py`, `q21_residual_ablation.py` |
| Can a source-identical PGO build help? | `build_pgo_lane.py` |

The [former detailed reference](archive/script-index-before-2026-11-16.md)
retains plan schemas, sampling limits, and diagnostic interpretation rules.
It is a historical reference; verify the selected tool's current interface
before defining a new experiment. Frozen runs continue to use their pinned code.

### Waiting for a quiet host before timing

`paired_search_diagnostics.py` supports bounded admission for counter-free
movetime measurements: `--idle-wait-seconds 300 --idle-poll-seconds 10
--idle-stable-samples 3`. Add `--idle-min-free-mib 1024` for a disk-space floor.
These options cannot be combined with `--allow-busy`, fixed depth/nodes, or
counters. A timeout records a blocked result with zero searches; it does not
authorize a retry or reopen an old trial.

## Training data and NNUE

`train --help` describes the native trainer; `train_halfkp.py --help` describes
packed HalfKP training. Keep data and unpublished weights outside Git. Split
training and validation by whole game, and preserve hold-outs and input hashes.
Lower validation loss alone does not qualify a weight.

| Purpose | Tools |
|---|---|
| New diagnostic positions | `generate_search_diagnostic.py` |
| Whole-game dataset split | `split_gensfen_by_game.py` |
| Frozen opening corpora | `freeze_material_gate_openings.py`, `freeze_balanced_gate_openings.py` |
| Reserved-root exclusions | `export_opening_identity_exclusions.py`, `stream_opening_exclusions.py` |
| Training and bounded pilot | `train_halfkp.py`, `run_halfkp_pilot.py` |
| Checkpoint lineage | `record_resume_run.py`, `attach_resume_manifest.py` |
| Integer parameters and reference scores | `compare_halfkp_parameters.py`, `check_halfkp_oracle.py` |
| Raw gate evidence | `export_nnue_gate_evidence.py`, `export_gate_observations.py` |
| Completed paired-match analysis | `analyze_paired_openings.py` |

For JSONL ingestion without training:

```bash
cargo run --release -p sekirei-train -- --positions /path/to/positions.jsonl \
  --positions-storage disk --positions-audit-only \
  --positions-manifest data/runs/ingest/audit.json
```

`disk` storage retains an O(N) row-offset index and a temporary row file.
Optional `positions-gzip,positions-zstd` features enable compressed input.
Ingest errors fail the run; physical-file hashes govern resume, so recompression
requires a new run. See the retained [ingestion](../docs/experiments/positions_ingestion_v0.4.1/README.md)
and [compression](../docs/experiments/positions_compression_v0.4.1/README.md) records.

## Matches and process supervision

| Purpose | Tools |
|---|---|
| Paired local A/B or SPRT ([standard test](../docs/strength_testing.md)) | `run_ab_match.py` |
| Parallel colour-reversed pairs, SPRT or fixed-count Elo | `run_parallel_sprt.py` |
| Offline self-play and replay audit | `run_local_selfplay.py`, `build_selfplay_ledger.py`, `profile_nnue_transcript.py` |
| Search-parameter tuning (SPSA; material-only for now, r6 support not yet added) | `spsa.py` |
| One-shot process observation | `run_observed_process.py` |
| Host/capacity admission | `nnue_gate_host.py`, `gate_resource_preflight.py`, `guard_halfkp_capacity.py` |
| Bounded Floodgate operation | `launch_floodgate_once.py`, `floodgate_supervisor.py` |
| CSA identity and terminal records | `record_floodgate_manifest.py`, `finalize_csa_run_manifest.py` |
| Offline CSA recovery rehearsal | `run_denryu_rehearsal.py`; [instructions](DENRYU_REHEARSAL.md) |

Set the CSA client's cumulative limit with `--loop --max-games N` and retain
its manifest, status journal, CSA files, and terminal receipt. Pass the weight's
declared `--fv-scale` (`nn_r6`: 24; client default: 16). Supply credentials only
at runtime; never put `FLOODGATE_TRIP` in Git, commands, manifests, or logs.

Observers record what happened and do not retry terminal failures. Verify the
owned child has stopped before considering any new attempt; preserve incomplete
records and distinguish unknown causes from observed signals. Recovered capacity
does not reopen a rejected statistical trial. Historical recovery details are in
the [former reference](archive/script-index-before-2026-11-16.md).

## Maintenance

Most Python tools use the standard library and have `test_<tool>.py` tests.
Run focused tests for changed behavior. Shared hashing and atomic JSON writes
live in `sekirei_tools.artifacts`.

`cleanup_runs.sh` is dry-run by default; deletion requires `APPLY=1`. Confirm
active users and retained references first. The protection scan includes
[archived scripts](archive/README.md). Preserve scripts pinned by historical
evidence even when they are no longer normal entry points.
