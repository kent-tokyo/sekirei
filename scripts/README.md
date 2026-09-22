# Script index

`scripts/` contains reproducibility and diagnostic tools, not a second public
CLI. Normal engine use should go through the Cargo binaries. Most tools write
only under ignored `data/` or `results/`; inspect `--help`, use a new run
directory, and retain the produced manifest with the result.

## Release and public boundary

| Tool | Purpose |
|---|---|
| `check_release_metadata.py` | Checks crate versions, lockfile, changelog, README, license files, tag, and release manifest. Use `--allow-planned-release-manifest` only before publication. |
| `check_public_surface.py` | Prevents internal roadmap material from entering Git and checks license references. |
| `check_documentation_references.py` | Verifies local links in both READMEs. |
| `validate_release_manifest.py` | Validates release-manifest schema and referenced artifacts. |
| `validate_nnue_release_artifact.py` | Validates a versioned NNUE file, checksum, model card, license boundary, and declared gate scope. |
| `test_public_contracts.sh` | Lightweight aggregate for the public contract. |

## Measurement and rules diagnostics

- `run_component_benchmark.py`, `component_benchmark_preflight.py`, and
  `run_component_aa_batch.py` capture component timing with provenance and an
  A/A noise floor. `compare_component_benchmarks.py` compares compatible
  captures only.
- `nnue_forward_bench`, `q21_same_time_profile.py`, and
  `q21_residual_ablation.py` separate evaluator cost from evaluator content.
  They are not strength gates.
- `preflight_speed_corpus.py` and `compare_cshogi_oracle.py` validate a
  bounded rule/move-set contract. `cshogi` is an optional diagnostic oracle,
  never a runtime dependency.
- `run_fixed_depth_ab.py` and `gate_resource_preflight.py` provide guarded
  deterministic A/B and resource admission checks. Use `Threads=1` and
  `SpecTopN=0` unless the experiment explicitly studies parallelism.

Historical reports in `benchmark_reports/` apply only to their recorded host,
revision, corpus, and operation. They are neither general speed rankings nor
Elo claims.

## NNUE candidates and gates

Candidate scripts are grouped by a frozen experiment identifier. An identifier
is a contract, not a model name:

1. prepare/freeze the input, source identity, evaluator, and decision rule;
2. run one declared factor change;
3. finalize from complete artifacts only; and
4. keep `PASS`, `FAIL`, `INCONCLUSIVE`, and resource-censored runs separate.

Unreleased candidate tools may exist only in a developer worktree. Their
authoritative status and acceptance conditions belong in internal `ROADMAP.md`
and run manifests, not this public index. They must not be represented as a
shipped checkpoint or strength result.

Useful shared entry points include `gate_orchestrator.py`,
`create_strength_gate_manifest.py`, `record_strength_gate_execution.py`,
`finalize_strength_gate_execution.py`, `run_self_distill_multiseed.sh`,
`record_resume_run.py`, and `attach_resume_manifest.py`. Run the paired
`test_<tool>.py` test whenever changing one of these tools.

## Local self-play and CSA

- `run_local_selfplay.py` collects offline same-engine games. A normal run
  requires both `--weights` and `--positions`; it records weight SHA-256,
  evaluator mode, fixed USI options, kifu/CSA, primary-PV per-move data, and
  durable summaries. `--material-only --startpos-smoke` is limited to two
  smoke games and is never NNUE or strength evidence.
- `build_selfplay_ledger.py` and `profile_nnue_transcript.py` audit replayable
  records and evaluator provenance. They are not training exporters or Elo
  estimators.
- `floodgate_supervisor.py`, `record_floodgate_manifest.py`, and
  `finalize_csa_run_manifest.py` preserve external-game execution evidence.
  Supply credentials only at runtime; never store `FLOODGATE_TRIP` in Git,
  manifests, plist files, commands, or logs.

Same-engine self-play supports training and regression work. It does not show
that either revision is stronger.

## Editing and cleanup

Most Python tools use only the standard library and have a focused
`test_<tool>.py`. After an edit, run that focused test, then:

```bash
python3 scripts/check_documentation_references.py
python3 scripts/check_public_surface.py
python3 scripts/check_release_metadata.py
```

Prefer one parameterized tool over near-duplicate wrappers. Retire one-off
instrumentation only after its evidence is retained and no live workflow or
document references it. `cleanup_runs.sh` is dry-run by default and requires
`APPLY=1` for deletion.
