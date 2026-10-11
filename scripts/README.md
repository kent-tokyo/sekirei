# Script index

`scripts/` contains reproducibility and diagnostic tools, not a second public
CLI. Use the Cargo binaries for normal engine operation. Before running a
script, inspect `--help`, choose a new ignored output directory, and retain its
manifest with the result.

The workspace pins `lineprior 0.12.4`. The default USI runtime excludes it;
only `sekirei-train` and an explicitly enabled `sekirei/opening-book` feature
use it. The data-pipeline wrappers have been checked with the external
`shogiesa 0.11.4` CLI. `sekirei-train` alone depends on `shogiesa-core 0.11.4`
for the typed JSONL contract; the default USI engine, search core, CSA client,
and match runner do not.

The shogiesa/Quietset wrappers are diagnostic only; Sekirei's internal
`--label-*` search remains the teacher. Before Quietset runs, the
`verify-shogiesa-label-run` helper applies shogiesa's typed manifest and
artifact verifier to the exact positions and observations streams. Complete
digest provenance is the default; compatibility mode must be explicit.
Sekirei then checks its own depth, `UseBook=false`, and diagnostic-only policy.

## Release and public boundary

| Tool | Purpose |
|---|---|
| `check_release_metadata.py` | Checks crate versions, lockfile, changelog, README, license files, tag, and release manifest. Use `--allow-planned-release-manifest` only before publication. |
| `check_wasm_package_metadata.py` | Checks that a generated WASM package or `.tgz` has matching README, package, install URL, and asset versions. |
| `check_public_surface.py` | Prevents internal roadmap material from entering Git and checks license references. |
| `check_documentation_references.py` | Verifies local links in both READMEs. |
| `validate_release_manifest.py` | Validates release-manifest schema and referenced artifacts. |
| `verify_release_publication.py` | After publication, checks every crate on crates.io (present, not yanked) and the WebAssembly asset's SHA-256 and size; `--write` records `publish.status=verified` and the workflow run in the manifest. |
| `validate_nnue_release_artifact.py` | Validates a versioned NNUE file, checksum, model card, license boundary, and declared gate scope. |
| `validate_nnue_candidate_card.py` | Validates an unpublished self-NNUE candidate card and, with `--artifact`, its size, SHA-256, HalfKP header, descriptor, and FV scale. It cannot authorize distribution or a default evaluator. |
| `check_halfkp_oracle.py` | Compares `halfkp_oracle` HalfKP scores with a separately executed reference USI engine; see [NNUE weights](../docs/nnue_weights.md#external-halfkp-networks). |
| `run_ab_match.py` | Fixed-protocol A/B self-play (material-only by default, optional shared HalfKP) or node-limited YaneuraOu ladder. One thread and `SpecTopN=0` remain the defaults; repeatable per-side options support isolated search experiments. `--result-json` atomically writes the shared, fail-closed gate-result schema and must be distinct from the raw JSON and text log. `--gate-observation-declaration` validates and carries a prospective, hashed feature/group declaration into that terminal result. Local diagnostic only. |
| `nps_threads.py` | Measures NPS, depth, and repeated-position best-move stability across explicit thread counts for Sekirei or a separately supplied USI engine. It supports wall-time and fixed-depth budgets. For Lazy SMP, it also records selected-worker share, worker agreement, and stop-lag. `--lazy-diagnostics` adds worker TT hit/store counts and shared-table collision topology; run that counter lane separately because it adds overhead. Speed diagnostic only. |
| `test_public_contracts.sh` | Lightweight aggregate for the public contract. |
| `check_rust_coverage.sh` | Runs the default workspace, opening-book tests, isolated tunable-search contracts, a bounded trainer smoke, and native WASM API contracts under `cargo-llvm-cov`; emits LCOV/JSON and fails below the documented 98% engine/library line-coverage contract. |

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
- `run_q26_search_profile.py` profiles a replayable fixed-node or fixed-time
  corpus. Its report includes pooled qsearch calls, top-level calls, exits,
  and searched-move rates; inclusive qsearch time overlaps its leaf timers and
  must not be added to them.
- `run_tactical_line_diagnostic.py` keeps the initial choice, opponent reply,
  and subsequent response as separate fixed-node observations. Every normal
  and fixed-root candidate search starts a fresh process; the node budget is
  per invocation. Reference moves in the case file are labels for comparison,
  never assertions that an engine alternative is wrong. The shipped fixture
  contains only the fully specified Question 2 position from Issue #149.
- For a Lazy SMP shared-TT causal control, add 128 to the usual `LazyFlags`
  value. This gives every worker a private table while retaining the other
  selected policy bits. `Hash` remains the total memory budget and is divided
  among the private tables; this is a diagnostic setting, not a playing
  default.

Historical reports in `benchmark_reports/` apply only to their recorded host,
revision, corpus, and operation. They are neither general speed rankings nor
Elo claims.

Example material-only tactical-line capture (repeat `--nodes` for a budget
ladder):

```bash
cargo build --release -p sekirei-core --bins
python3 scripts/run_tactical_line_diagnostic.py \
  --cases scripts/fixtures/tactical_line_cases_v1.json \
  --engine target/release/sekirei-search-diagnostic \
  --history-sfen target/release/history_sfen \
  --nodes 100000 --nodes 2000000 \
  --output data/runs/tactical-lines/report.json
```

Add `--weights FILE --nnue-output absolute` for a distinct NNUE lane. The
report hashes both executables, the case file, and the optional weights.

## NNUE candidates and gates

Freeze the input, evaluator, hashes, factor change, and decision rule before a
candidate run. Finalize only complete artifacts, and keep `PASS`, `FAIL`,
`INCONCLUSIVE`, and resource-censored outcomes distinct. Useful entry points:

- `gate_orchestrator.py` and the `*_strength_gate_*` scripts manage gate
  declarations and terminal evidence.
- `record_resume_run.py` and `attach_resume_manifest.py` preserve resumable
  training lineage.
- `split_gensfen_by_game.py` creates whole-game train/validation splits from
  current six-column gensfen data. Legacy five-column rows fail closed.
- `train_halfkp.py` trains the in-house HalfKP network. Use `--val-data` with a
  game-level split; validation loss alone does not select a candidate.
- `export_gate_observations.py` converts independent terminal gate manifests
  into lineprior `GateObservation` JSONL. It rejects outcome-derived features,
  missing or changed declaration hashes, missing groups, incompatible feature
  sets, and conflicting retries. A non-empty export remains contract evidence
  until at least 20 independent groups are available; automated acquisition is
  never enabled. Use `--allow-empty` to archive a deterministic zero-row
  readiness report when no historical run is eligible.
- `validate_book_ab_bundle.py` verifies hashes, arm symmetry, decision-to-
  terminal joins, book coverage, fallbacks, uncertainty, and cost for an
  archived held-out `UseBook=false`/`true` diagnostic.
- `prepare_book_ab_split.py` freezes disjoint CSA subsets for opening-book
  training and held-out evaluation. `summarize_book_coverage_preflight.py`
  validates a preregistered, one-ply decision-log probe and stops before the
  full A/B gate when actual book selection coverage is too low.

Validate exported observations with:

```bash
python3 scripts/export_gate_observations.py run*/final.json \
  --output data/gates/history.jsonl --report data/gates/export-report.json
cargo run -p sekirei-train --bin validate_gate_observations -- \
  data/gates/history.jsonl
```

`candidate_id` identifies one exact artifact/configuration; `group_id` keeps a
shared recipe, lineage, and dataset in one cross-validation group. GateModel
output is advisory and never authorizes deployment or a strength claim.

Historical book and GateObservation pilots are under `docs/experiments/`.
They preserve their original scope and are not current strength evidence.

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

`train_halfkp.py` consumes packed shards. `--init` resumes a float checkpoint;
`--fact` folds in a king-independent factor. Inputs remain separate read-only
memory maps and only one batch is copied at a time. Outputs are written by
atomic replacement, and direct, symlink, or hardlink aliases to an input are
rejected. New checkpoints record the shuffle contract. Resume a legacy
full-memory checkpoint only at an epoch boundary.

```bash
python3 scripts/train_halfkp.py --data data/selfplay/train.bin \
  --val-data data/selfplay/validation.bin --init data/selfplay/prev.pt \
  --out data/selfplay/next.bin --save data/selfplay/next.pt
```

## Local self-play and CSA

- `spsa.py` tunes search parameters with paired self-play. It requires the
  explicit `--baseline material` contract, starts both Sekirei arms without
  evaluator files, and fixes `Threads=1`, `SpecTopN=0`, and `UseBook=false`.
  External evaluators are deliberately unsupported by this entry point.
- `run_local_selfplay.py` collects offline games with explicit weights and
  openings, recording evaluator identity, options, kifu/CSA, PV data, and a
  durable summary. Material/startpos mode is smoke-only.
- `build_selfplay_ledger.py` and `profile_nnue_transcript.py` audit replayable
  records and evaluator provenance. They are not training exporters or Elo
  estimators.
- `floodgate_supervisor.py`, `record_floodgate_manifest.py`, and
  `finalize_csa_run_manifest.py` preserve external-game execution evidence.
  Supply credentials only at runtime; never store `FLOODGATE_TRIP` in Git,
  manifests, plist files, commands, or logs.

For a bounded Floodgate run, let the client own the game limit:

```bash
sekirei-csa --loop --max-games 5 --eval nnue --weights /path/to/weights.bin \
  --record-dir data/floodgate/run --analysis-dir data/floodgate/run/analysis \
  --run-manifest data/floodgate/run/manifest.json \
  --status-file data/floodgate/run/status.json
```

`--max-games` persists across reconnects. After a completed game, the client
may recover an authenticated transport close up to three times in-process;
protocol, recording, and mid-game failures remain terminal. Exhausted recovery
is recorded as `post_game_reconnect_exhausted` without discarding completed
records. A non-zero one-shot child exit is also persisted to status and the
run manifest; an active game is retained as an uncounted `incomplete_game`.
For launchd, use the one-shot template with `KeepAlive` disabled.

Before tournament deployment, run the loopback-only two-process rehearsal:

```bash
python3 scripts/run_denryu_rehearsal.py \
  --binary target/release/sekirei-csa \
  --output data/rehearsals/denryu-$(date +%Y%m%d-%H%M%S)
```

It runs 7 games, restarts the real client, then runs 7 more while preserving
the cumulative ceiling. See
[`DENRYU_REHEARSAL.md`](DENRYU_REHEARSAL.md) for the recovery
checklist. `--completed-attempts` is an explicit resume input and must come
from a retained terminal status or manifest, never from an estimate.

Same-engine self-play supports training and regression work. It does not show
that either revision is stronger.

## Editing and cleanup

Most Python tools use only the standard library and have a focused
`test_<tool>.py`. After an edit, run that focused test, then:

Shared byte hashing, canonical JSON hashing, and atomic JSON replacement live
in `sekirei_tools.artifacts`. Maintained release and gate entry points should
reuse these primitives; historical one-off experiment scripts remain frozen
until their archived evidence no longer depends on their exact behavior.

```bash
python3 scripts/check_documentation_references.py
python3 scripts/check_public_surface.py
python3 scripts/check_release_metadata.py
```

Prefer one parameterized tool over near-duplicate wrappers. Retire one-off
instrumentation only after its evidence is retained and no live workflow or
document references it. `cleanup_runs.sh` is dry-run by default and requires
`APPLY=1` for deletion.
