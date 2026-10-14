# Script index

`scripts/` contains reproducibility and diagnostic tools, not a second public
CLI. Use the Cargo binaries for normal engine operation. Before running a
script, inspect `--help`, choose a new ignored output directory, and retain its
manifest with the result.

The workspace pins `lineprior 0.12.6`. The default USI runtime excludes it;
only `sekirei-train` and an explicitly enabled `sekirei/opening-book` feature
use it. The data-pipeline wrappers have been checked with the external
`shogiesa 0.11.6` CLI. `sekirei-train` alone depends on `shogiesa-core 0.11.6`
for the typed JSONL contract; the default USI engine, search core, CSA client,
and match runner do not.

The shogiesa/Quietset wrappers are diagnostic only; Sekirei's internal
`--label-*` search remains the teacher. Before Quietset runs, the
`verify-shogiesa-label-run` helper applies shogiesa's typed manifest and
artifact verifier to the exact positions and observations streams. Complete
digest provenance is the default; compatibility mode must be explicit.
Sekirei then checks its own depth, `UseBook=false`, and diagnostic-only policy.

## Release and public boundary

- `.github/workflows/api-compatibility.yml` runs `cargo-semver-checks` for
  the only published library crate, `sekirei-core`. It compares against
  v0.3.73 with `release-type: patch`, so a 0.4 version bump cannot hide a
  breaking public Rust API change. The crates.io publish workflow repeats the
  same check on the exact tag before uploading any crate. The unpublished
  WASM crate remains covered by its generated TypeScript/package contract.
- `test_api_compatibility_workflow.py` pins the baseline, exact action commit,
  full-history checkout, package scope, and pre-publish ordering. The publish
  workflow also verifies the exact tag, all seven workspace versions, the
  planned release manifest, and the GitHub Release WASM metadata before the
  first irreversible registry upload.

| Tool | Purpose |
|---|---|
| `check_release_metadata.py` | Checks all six published crates plus the WASM crate, lockfile, changelog, README, license files, tag, and release manifest. Use `--allow-planned-release-manifest` only before publication. |
| `check_wasm_package_metadata.py` | Checks that a generated WASM package or `.tgz` has matching README, package, install URL, and asset versions. |
| `check_public_surface.py` | Prevents internal roadmap material from entering Git and checks license references. |
| `check_documentation_references.py` | Verifies local links in both READMEs. |
| `validate_release_manifest.py` | Validates release-manifest schema and referenced artifacts. |
| `verify_release_publication.py` | After publication, checks every crate on crates.io (present, not yanked) and the WebAssembly asset's SHA-256 and size; `--write` records `publish.status=verified` and the workflow run in the manifest. |
| `validate_nnue_release_artifact.py` | Validates a versioned NNUE file, checksum, model card, license boundary, and declared gate scope. The v2 HalfKP contract additionally requires hashed training-shard and whole-game hold-out manifests, raw 1/4-thread current-engine gate evidence, and an exact engine commit. |
| `validate_nnue_candidate_card.py` | Validates an unpublished self-NNUE candidate card and, with `--artifact`, its size, SHA-256, HalfKP header, descriptor, and FV scale. It cannot authorize distribution or a default evaluator. |
| `check_halfkp_oracle.py` | Compares `halfkp_oracle` HalfKP scores with a separately executed reference USI engine; see [NNUE weights](../docs/nnue_weights.md#external-halfkp-networks). |
| `run_ab_match.py` | Fixed-protocol A/B self-play (material-only by default, optional shared HalfKP) or node-limited YaneuraOu ladder. One thread and `SpecTopN=0` remain the defaults; repeatable per-side options support isolated search experiments. SPRT uses the same BayesElo trinomial likelihood as veridict; `--paired-sprt --games-per-position N` evaluates only complete same-position, opposite-colour pairs, while `--games` remains a hard cap. Every run fails closed unless stdout agrees with a sequential replay of the durable JSONL and with the raw JSON plus an exact, non-empty kifu set containing no artifact-write or invalid-game records. Result evidence hashes the JSONL and an ordered per-kifu manifest. Normal completion requires a `complete` snapshot; an early SPRT stop may use the last durable `running` snapshot because the match runner publishes a game result only after that snapshot. `--result-json` atomically writes the shared gate-result schema and must be distinct from the raw JSON and text log. `--gate-observation-declaration` validates and carries a prospective, hashed feature/group declaration into that terminal result. Local diagnostic only. |
| `launch_floodgate_once.py` | Starts one rendered bounded Floodgate LaunchAgent without combining `RunAtLoad=true` and `kickstart`; refuses registered labels and records the single-run acceptance receipt. |
| `nps_threads.py` | Measures NPS, depth, and repeated-position best-move stability across explicit thread counts for Sekirei or a separately supplied USI engine. It supports wall-time and fixed-depth budgets. For Lazy SMP, it also records selected-worker share, worker agreement, stop-lag, and discarded root-result nodes. `--lazy-diagnostics` adds worker TT hit/store counts, shared-table collision topology, sampled within-/cross-worker position revisits, and local/foreign/unknown TT-hit attribution for every worker, including the selected worker separately; run that counter lane separately because it adds overhead. `--require-lazy-diagnostics` rejects any search missing one of those complete records. `--require-root-hint-diagnostics` fail-closes an opt-in helper-root-hint lane and records accepted slot updates, depth-qualified reads, actual root-order applications, first-move beta cuts, aspiration retry nodes, stale-hint retry nodes, and depth/generation gaps. NMP depth histograms and dominated-helper retirement records are retained when emitted. JSONL rows include binary, effective-corpus, and source-corpus SHA-256 provenance. Speed diagnostic only. |
| `paired_search_diagnostics.py` | Freezes both executables and input bytes, then performs fresh-process A/B/B/A per position for a single search factor. Separates fixed-depth bestmove/score/PV/nodes identity at one worker, shared-TT nondeterminism, counter-free timing, and selected-depth/helper-iteration/foreign-TT/nonselected-work diagnostics at 1/2/4/8 workers. Incomplete runs fail closed. Does not estimate Elo or open a strength gate. |
| `generate_search_diagnostic.py` | Creates new seeded, score-blind quiet/tactical diagnostic positions from Sekirei legal playouts and rule probes, with complete replay and executable-hash provenance. Reads no training corpus or external evaluator. Diagnostic-only; do not reuse as an independent gate. |
| `validate_paired_pv.py` | Freezes the Rust legal-move tool and replays every recorded PV, checking its bestmove prefix and each legal transition. Fails on incomplete runs or illegal PVs. This is a same-rules contract check, not an independent oracle or a proof of mate scores. |
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
- For the experimental four-worker depth ladder, add 256 while retaining the
  normal depth-skew bit. This changes offsets from `(0, 1, 0, 1)` to
  `(0, 1, 2, 3)`. Its initial screen was neutral and it is not a playing
  default. See the
  [sampled-overlap report](../docs/experiments/lazy_smp_work_overlap_v0.3.72/README.md).

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

## JSONL ingestion and audit

`train --positions` keeps in-memory storage by default. Use
`--positions-storage disk` to snapshot accepted JSONL rows in an anonymous
system-temporary file and decode one position at a time. Source caps, the legacy
SFEN split, exact seeded shuffle, teacher-cache update timing and resume recipe
stay the same across both modes. Disk storage uses temporary space proportional
to accepted input bytes and an O(N) row-offset index; shuffle indices and teacher
caches also remain O(N). It does not change the HalfKP packed-data trainer.

Every successful ingestion writes `<output stem>.positions-manifest.json`
(or `--positions-manifest PATH`) atomically after all inputs reach EOF and
requested SHA-256 checks pass. Counts precede source caps, splits and training
filters. The manifest includes exact input hashes, accepted/skipped counts,
rejection categories and up to eight examples with physical line numbers,
zero-based byte offsets, raw-row hashes and 256-character previews. Raw hashes
include original line endings. Strict, I/O, callback and hash failures do not
publish a new complete manifest. Pre-existing audit evidence is left intact.

```bash
cargo run --release -p sekirei-train -- --positions /path/to/positions.jsonl \
  --positions-storage disk --positions-audit-only \
  --positions-manifest data/runs/ingest/audit.json
python3 scripts/benchmark_positions_ingest.py --binary target/release/train \
  --output data/runs/ingest-benchmark --records 10000 --repeats 2
TRAIN_BIN="$PWD/target/release/train" python3 -B scripts/test_positions_ingest.py
```

Audit-only exits before evaluator initialization or training. The benchmark
creates synthetic data, alternates the storage modes, checks matching reports,
and records binary/input hashes, host/toolchain, elapsed time and maximum RSS.
Its results measure ingestion only, not epoch throughput or playing strength.

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
For launchd, render the one-shot template with `KeepAlive=false` and start it
through the idempotent helper:

```bash
python3 scripts/launch_floodgate_once.py \
  --plist "$HOME/Library/LaunchAgents/com.kent-tokyo.sekirei-floodgate.plist" \
  --runtime-dir "$HOME/.local/state/sekirei/floodgate/RUN_ID"
```

The checked-in template has `RunAtLoad=true`, so the helper runs `bootstrap`
only. Do not follow it with `kickstart -k`. For a custom plist with
`RunAtLoad=false`, the helper registers it and performs one non-forcing
`kickstart`. It refuses an already registered label, prints the runtime
directory, and writes `launchd-start.json`. A bounded run is accepted only
when `launchctl print` reports `runs = 1`; a duplicate launch remains recorded
with `accepted=false`. Unload an intentionally stopped old label before using
a new runtime. The client-side `--max-games` remains the sole cumulative game
cap; do not wrap launchd or the client in an external restart loop.

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

### Useful Lazy SMP depth experiments

`generate_search_diagnostic.py` makes score-blind legal-play diagnostics using
Sekirei's rule tools. `paired_search_diagnostics.py` freezes binaries/corpus
and compares one option in per-position ABBA order, saving complete raw USI
logs and paired depth/work metrics. Counter-free NPS and instrumented TT/NMP
lanes stay separate. See [experiment specification](../docs/experiments/lazy_smp_useful_depth_v0.4.0/README.md).

### Pinned YaneuraOu throughput comparison

`compare_yaneura_speed.py` runs the existing NPS driver in sequential ABBA
order for each requested thread count. Supply an explicit shared HalfKP file
named `nn.bin`, since YaneuraOu loads `EvalDir/nn.bin`. The runner fixes hash
size to 64 MiB, FV_SCALE to 24, books off, and Sekirei's MoveOverhead to zero.
It disables diagnostic counters, verifies complete position coverage and
artifact hashes before and after each arm, preserves logs, and kills only its
own subprocess session on timeout. Ordinary Lazy SMP completion summaries
with zero TT counters are allowed.
The NPS driver waits for `readyok` after each new-game and position command
before timing `go` through `bestmove`. Rows identify this boundary in
`provenance.elapsed_scope`; reported engine NPS remains a separate metric.

```bash
python3 scripts/compare_yaneura_speed.py \
  --engine /path/to/sekirei --reference /path/to/YaneuraOu \
  --evalfile /path/to/nn.bin --positions /path/to/positions.sfen \
  --threads 1,2,4,8 --count 4 --movetime 1000 \
  --output-directory results/new-throughput-capture
cargo run --release -p sekirei-core --example halfkp_synthetic_fixture -- \
  results/synthetic-halfkp.bin 20261014
```

The output directory must be new. Pin the reference revision, compiler and
ISA flags separately and retain startup logs confirming evaluation loading.
For short runs on a shared machine, supply one position per capture so each
ABBA quartet stays close in time. Cross-engine NPS counts different trees;
completed depths are descriptive and do not establish equivalent search work
or playing strength. Use synthetic or material evaluation to qualify local
optimizations. The external shared-evaluator lane measures Sekirei versus
YaneuraOu and must not select candidates, tune defaults or generate training
labels. The synthetic fixture carries no playing strength.

### Independent material-opening gate input

The core gate_openings example generates fresh default-material trajectories
with full move prefixes. freeze_material_gate_openings.py requires identity
exclusions from diagnostics, prior strength gates and the NNUE hold-out owner,
then freezes tools and validates complete histories.
run_ab_match.py --openings-manifest replays this contract before/after material
selfplay and revokes H1 on changed input. See the [gate preparation contract](../docs/experiments/aspiration_independent_gate_v0.4.1/README.md).
