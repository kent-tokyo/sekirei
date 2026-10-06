# Script index

`scripts/` contains reproducibility and diagnostic tools, not a second public
CLI. Normal engine use should go through the Cargo binaries. Most tools write
only under ignored `data/` or `results/`; inspect `--help`, use a new run
directory, and retain the produced manifest with the result.

The workspace pins `lineprior 0.12.1`. The default USI runtime excludes it;
only `sekirei-train` and an explicitly enabled `sekirei/opening-book` feature
use it. The data-pipeline wrappers have been checked with the external
`shogiesa 0.11.0` CLI. `sekirei-train` alone depends on `shogiesa-core 0.11.0`
for the typed JSONL contract; the default USI engine, search core, CSA client,
and match runner do not.

`train_with_shogiesa_quietset.sh`, `redo_quietset_bc.sh`, and
`train_with_loss_mining.sh` implement one explicit teacher contract:
shogiesa observations are diagnostic inputs to Quietset only, while Sekirei's
internal `--label-*` search is the sole teacher target. Before training,
`validate_shogiesa_diagnostic_contract.py` checks the supported CLI/schema,
requested versus achieved depth, score provenance, cache accounting, and the
label manifest. The trainer metadata links that manifest by SHA-256, and the
run manifest records shogiesa labeling and internal training wall time
separately. A diagnostic/teacher depth mismatch is rejected unless the run
records a reason.

## Release and public boundary

| Tool | Purpose |
|---|---|
| `check_release_metadata.py` | Checks crate versions, lockfile, changelog, README, license files, tag, and release manifest. Use `--allow-planned-release-manifest` only before publication. |
| `check_public_surface.py` | Prevents internal roadmap material from entering Git and checks license references. |
| `check_documentation_references.py` | Verifies local links in both READMEs. |
| `validate_release_manifest.py` | Validates release-manifest schema and referenced artifacts. |
| `verify_release_publication.py` | After publication, checks every crate on crates.io (present, not yanked) and the WebAssembly asset's SHA-256 and size; `--write` records `publish.status=verified` and the workflow run in the manifest. |
| `validate_nnue_release_artifact.py` | Validates a versioned NNUE file, checksum, model card, license boundary, and declared gate scope. |
| `check_halfkp_oracle.py` | Compares `halfkp_oracle` HalfKP scores with a separately executed reference USI engine; see [NNUE weights](../docs/nnue_weights.md#external-halfkp-networks). |
| `run_ab_match.py` | Fixed-protocol A/B self-play or node-limited YaneuraOu ladder with one external HalfKP network. One thread and `SpecTopN=0` remain the diagnostic defaults; explicit per-side thread and search-mode options support parallel studies. Local diagnostic only. |
| `nps_threads.py` | Measures NPS and depth scaling across explicit thread counts for Sekirei or a separately supplied USI engine. Speed diagnostic only. |
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

`split_gensfen_by_game.py` prepares in-house HalfKP data without allowing
adjacent positions from one self-play game to leak across train and validation.
It requires the six-column output from the current `gensfen`, joins games that
share an exact SFEN, optionally limits positions contributed by one long game,
and records input/output hashes plus phase and result counts in a manifest.
Legacy five-column rows fail closed because they cannot prove game separation.

```bash
python3 scripts/split_gensfen_by_game.py data/selfplay/part*.txt \
  --train-out data/selfplay/train.txt \
  --validation-out data/selfplay/validation.txt \
  --manifest data/selfplay/split-manifest.json \
  --validation-ratio 0.10 --seed 42 --max-positions-per-game 64
cargo run --release -p sekirei-core --example halfkp_pack -- \
  pack data/selfplay/train.bin data/selfplay/train.txt
cargo run --release -p sekirei-core --example halfkp_pack -- \
  pack data/selfplay/validation.bin data/selfplay/validation.txt
```

## Local self-play and CSA

- `spsa.py` tunes search parameters with paired self-play. It requires the
  explicit `--baseline material` contract, starts both Sekirei arms without
  evaluator files, and fixes `Threads=1`, `SpecTopN=0`, and `UseBook=false`.
  External evaluators are deliberately unsupported by this entry point.
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

For an unattended bounded Floodgate run, let the client own the game limit;
do not use an external launcher that restarts a successful batch:

```bash
sekirei-csa --loop --max-games 5 --eval nnue --weights /path/to/weights.bin \
  --record-dir data/floodgate/run --analysis-dir data/floodgate/run/analysis \
  --run-manifest data/floodgate/run/manifest.json \
  --status-file data/floodgate/run/status.json
```

`--max-games` is process-wide across reconnects. Protocol errors stop the
client instead of requesting another game; the status and manifest retain the
completed-attempt count and terminal reason without credentials.

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
