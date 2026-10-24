# Script index

`scripts/` contains reproducibility and diagnostic tools. Use the Cargo
binaries for normal engine operation. Before running a script, inspect
`--help`, choose a new ignored output directory, and retain its manifest.

The default USI engine does not depend on `lineprior` or `shogiesa-core`.
The trainer uses both; the optional opening-book feature uses `lineprior`.
The pinned versions are in the workspace manifests. External
shogiesa/Quietset wrappers are diagnostic only. Their verifier requires
complete digests and typed manifests before Sekirei checks depth,
`UseBook=false`, and the diagnostic-only policy.

## Release and public boundary

`.github/workflows/api-compatibility.yml` checks the published `sekirei-core`
API against v0.3.73 before merge; the publish workflow repeats it on the exact
tag before upload. `test_api_compatibility_workflow.py` checks the baseline,
action pin, history depth, and ordering. Release checks also bind all seven
workspace versions, the manifest, and WASM metadata.

| Tool | Purpose |
|---|---|
| `check_release_metadata.py` | Checks all six published crates plus the WASM crate, lockfile, changelog, README, license files, tag, and release manifest. Use `--allow-planned-release-manifest` only before publication. |
| `check_wasm_package_metadata.py` | Checks WASM package/version agreement and embedded build provenance; `--release-manifest` also compares the frozen source baseline with the release record. Archives through v0.5.0 retain their legacy metadata contract. |
| `check_wasm_npm_distribution.py` | Verifies the tagged Cargo version and Release asset hash, required package files, and per-file SHA-256 parity with the npm download. See [npm distribution](../docs/releases/npm_wasm.md). |
| `finalize_wasm_release_manifest.py` | After a v0.5.1+ tag build, verifies the asset checksum and embedded build identity before recording its final hash and size in a post-Release manifest copy. |
| `record_wasm_npm_status.py` | Produces a validated release-manifest copy for a verified npm publish or an unverified publish attempt. |
| `check_public_surface.py` | Prevents internal roadmap material from entering Git and checks license references. |
| `check_documentation_references.py` | Verifies local links in both READMEs. |
| `validate_release_manifest.py` | Validates release-manifest schema and referenced artifacts. |
| `check_release_manifest_layout.py` | Requires only the current manifest in the repository root and verified historical manifests under `docs/releases/manifests/`. |
| `verify_release_publication.py` | After publication, checks every crate on crates.io (present, not yanked) and the WebAssembly asset's SHA-256 and size; `--write` records `publish.status=verified` and the workflow run in the manifest. |
| `validate_nnue_release_artifact.py` | Validates a versioned NNUE file, checksum, model card, license boundary, and declared gate scope. The v2 HalfKP contract additionally requires hashed training-shard and whole-game hold-out manifests, raw 1/4-thread current-engine gate evidence, and an exact engine commit. |
| `export_nnue_gate_evidence.py` | Builds a gate-evidence summary from a retained terminal match receipt and raw match JSON; it rejects incomplete or mismatched material gates. |
| `validate_nnue_candidate_card.py` | Validates an unpublished self-NNUE candidate card and, with `--artifact`, its size, SHA-256, HalfKP header, descriptor, and FV scale. It cannot authorize distribution or a default evaluator. |
| `validate_nnue_cpu_pilot.py` | Validates a frozen, short AWS CPU-pilot plan only after a colour-reversed material gate, complete candidate/data/resume evidence, and an observed CPU-dominant bottleneck. It does not provision cloud resources. |
| `check_halfkp_oracle.py` | Compares `halfkp_oracle` HalfKP scores with a separately executed reference USI engine; see [NNUE weights](../docs/nnue_weights.md#external-halfkp-networks). |
| `run_ab_match.py` | Fixed-protocol local A/B, paired SPRT, or node-limited YaneuraOu measurement. It hashes inputs and fails closed on incomplete or inconsistent JSONL, kifu, snapshots, or result sidecars. `--supervised --result-json PATH` adds a one-shot parent that records a terminal error if the wrapper or match child exits unexpectedly. Material-only, one thread and `SpecTopN=0` are the defaults. |
| `launch_floodgate_once.py` | Starts one rendered bounded Floodgate LaunchAgent without combining `RunAtLoad=true` and `kickstart`; refuses registered labels and records the single-run acceptance receipt. |
| `nps_threads.py` | Thread-scaling diagnostic for NPS, completed depth, move stability, and opt-in parallel counters. Counter-enabled runs are separate from speed runs; neither establishes Elo. |
| `paired_search_diagnostics.py` | Fresh-process A/B/B/A comparison for one search factor. Separates deterministic signatures, counter-free timing and opt-in parallel diagnostics; it does not estimate Elo. |
| `build_pgo_lane.py` | Builds native, instrumented, and profile-use binaries from one clean source and frozen corpus. Optional own-HalfKP profiling needs a verified public card and exact load identity. Qualify speed, strength, and default adoption in separate gates. |
| `generate_search_diagnostic.py` | Creates new seeded, score-blind quiet/tactical diagnostic positions from Sekirei legal playouts and rule probes, with complete replay and executable-hash provenance. Reads no training corpus or external evaluator. Diagnostic-only; do not reuse as an independent gate. |
| `validate_paired_pv.py` | Freezes the Rust legal-move tool and replays every recorded PV, checking its bestmove prefix and each legal transition. Fails on incomplete runs or illegal PVs. This is a same-rules contract check, not an independent oracle or a proof of mate scores. |
| `test_public_contracts.sh` | Lightweight aggregate for the public contract. |
| `check_rust_coverage.sh` | Runs the default workspace, opening-book tests, isolated tunable-search contracts, a bounded trainer smoke, and native WASM API contracts under `cargo-llvm-cov`; emits LCOV/JSON and fails below the documented 98% engine/library line-coverage contract. |

For a frozen HalfKP candidate versus its incumbent, use
`--candidate-evalfile NEW --baseline-evalfile BEST --evalfile none`,
`--openings-manifest`, and `--result-json` with `run_ab_match.py`. The binary
hashes must match, the weight hashes must differ, and both loaders must
acknowledge the exact weights before a paired-SPRT verdict is accepted.

The supervised gate requires a fresh output namespace. Its attempt directory
remains after completion and prevents accidental replay. On interruption, the
terminal result is `error` with null Elo/SPRT verdict and separate observed and
verified game counts; raw match artifacts are retained. SIGTERM to the outer
supervisor is handled, while its own SIGKILL requires an OS-level observer.

`validate_nnue_cpu_pilot.py /path/to/frozen-pilot.json` checks a bounded CPU
pilot plan against prior material-gate, data, hold-out, resume, and bottleneck
evidence. A valid plan neither provisions resources nor approves a candidate.

## Measurement and rules diagnostics

YaneuraOu match comparisons request `MinimumThinkingTime=1`, the valid
lower bound in the pinned reference. Zero is silently rejected there. Equal
requested byoyomi is distinct from a hard per-move wall-time limit; the runner
retains its separate hang timeout.

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

### Waiting for a quiet host before timing

For a new counter-free, movetime `paired_search_diagnostics.py` run, use
`--idle-wait-seconds 300 --idle-poll-seconds 10 --idle-stable-samples 3`.
Three consecutive idle samples admit dispatch; add
`--idle-min-free-mib 1024 --idle-stable-samples 5` for a joint disk and CPU
floor. The default wait is zero. These options cannot be combined with
`--allow-busy`, fixed-depth/node budgets, or counters.

Timeout records `blocked_host_busy` or `blocked_host_resources` with zero
searches. Capacity is checked before, between, and after ABBA searches;
loss invalidates timing. The manifest retains samples and frozen-input
checks. Admission does not reserve resources or reopen an older study.

Historical [benchmark reports](../docs/experiments/benchmark_reports/README.md)
apply only to their recorded host, revision, corpus, and operation. For
tactical-line captures, use `run_tactical_line_diagnostic.py --help` and
keep material and explicit-weight lanes separate.

## JSONL ingestion and audit

`train --positions` stores rows in memory by default.
`--positions-storage disk` snapshots accepted rows to a temporary file,
using disk space proportional to the input and an O(N) offset index. Both
modes retain source caps, seeded shuffle, teacher-cache timing, and resume
identity. The packed-data HalfKP trainer is unchanged.

After EOF and requested hash checks, ingestion atomically writes
`<output stem>.positions-manifest.json` or `--positions-manifest PATH`.
It records input hashes, accepted/skipped counts, rejection reasons, and
bounded row examples. A failed ingest leaves prior audit evidence intact.

```bash
cargo run --release -p sekirei-train -- --positions /path/to/positions.jsonl \
  --positions-storage disk --positions-audit-only \
  --positions-manifest data/runs/ingest/audit.json
```

`--positions-audit-only` exits before training. The
[ingestion comparison](../docs/experiments/positions_ingestion_v0.4.1/README.md)
measures synthetic-input time and RSS, not training throughput or strength.

### Compressed JSONL

Build with `positions-gzip`, `positions-zstd`, or both; neither is enabled
by default. The safe-Rust adapter uses Rust gzip/zstd decoders without C/C++
bindings; this does not characterize all transitive dependencies.

```bash
cargo build --release -p sekirei-train --bin train \
  --features positions-gzip,positions-zstd
```

Content magic takes precedence over suffix. Truncation, bad checksums,
unsupported frames, and invalid trailing data fail the whole ingest.
Zstd dictionaries are unsupported and its window is limited to 8 MiB.
Physical-file hashes govern resume, so recompression needs a new run;
decoded hashes and row offsets refer to the JSONL stream. See the
[compression validation](../docs/experiments/positions_compression_v0.4.1/README.md).

## NNUE candidates and gates

Freeze input, evaluator, hashes, factor change, and decision rule before
each candidate run. Finalize only complete artifacts; keep `PASS`, `FAIL`,
`INCONCLUSIVE`, and resource-censored outcomes distinct. The
[weight contract](../docs/nnue_weights.md) covers release eligibility.

| Tool | Role |
|---|---|
| `run_observed_process.py` | Launch one child and record fsynced start/exit events, output, status, and resource samples. No automatic retry. |
| `nnue_gate_host.py` | Admit the next colour-reversed strength pair after the preregistered shared-host resource checks. |
| `gate_orchestrator.py`, `*_strength_gate_*` | Freeze declarations and terminal gate evidence. |
| `record_resume_run.py`, `attach_resume_manifest.py` | Preserve checkpoint and resume lineage. |
| `split_gensfen_by_game.py` | Split current six-column gensfen by whole game; reject legacy five-column rows. |
| `train_halfkp.py` | Train from separate read-only packed shards with a game-level `--val-data` split. Validation loss alone does not select a candidate. |
| `export_gate_observations.py` | Export eligible independent gate manifests as lineprior `GateObservation` JSONL. |
| `validate_book_ab_bundle.py`, `prepare_book_ab_split.py`, `summarize_book_coverage_preflight.py` | Verify held-out opening-book comparison and selection coverage. |

The observer distinguishes an observed signal from its unknown cause.
An unclosed attempt remains unknown; it must not be replayed while a child
may still be alive. Its 8 MiB journal reserve permits one ENOSPC retry;
a second persistence failure stops the owned child, never a statistical trial.
Keep the OS job's exit status as a second observation layer.

For the preregistered 10-core shared-host policy, three samples with at most
300% foreign CPU and at least 512 MiB free admit the next colour-reversed
pair. Load during a pair is telemetry: retain both games and wait before
dispatching another pair. Missing samples defer dispatch. This is a strength
trial admission rule, not an idle-host speed qualification or a reason to
discard played games. Keep exact evaluator acknowledgements and paired records.

`export_gate_observations.py` rejects changed declarations, outcome-derived
features, incomplete groups, and conflicting retries. Its output remains
contract evidence until 20 independent groups are available; it does not
enable automated acquisition or deployment. `candidate_id` identifies one
artifact/configuration, while `group_id` keeps shared lineage in one
cross-validation group. GateModel output is advisory.

```bash
python3 scripts/export_gate_observations.py run*/final.json \
  --output data/gates/history.jsonl --report data/gates/export-report.json
cargo run -p sekirei-train --bin validate_gate_observations -- \
  data/gates/history.jsonl
```

Historical book and gate pilots under [`docs/experiments/`](../docs/experiments/)
retain their recorded scope. `train_halfkp.py --help` documents packed-shard
training and checkpoint-resume options; new checkpoints record the shuffle
contract and reject output aliases to any input.

## Local self-play and CSA

- `spsa.py` tunes search parameters with paired, material-only self-play;
  it fixes one thread, `SpecTopN=0`, and `UseBook=false`.
- `run_local_selfplay.py` records offline games, evaluator identity,
  options, kifu/CSA, PV data, and a durable summary. Material/startpos is
  smoke-only; same-engine self-play alone does not show a strength gain.
- `build_selfplay_ledger.py` and `profile_nnue_transcript.py` audit
  replayable records and evaluator provenance.
- `floodgate_supervisor.py`, `record_floodgate_manifest.py`, and
  `finalize_csa_run_manifest.py` preserve one-shot external-game evidence.
  Supply credentials only at runtime; do not store `FLOODGATE_TRIP` in
  Git, manifests, plist files, commands, or logs.

For a bounded Floodgate run, let the client own the cumulative game limit:

```bash
sekirei-csa --loop --max-games 5 --eval nnue --weights /path/to/weights.bin \
  --record-dir data/floodgate/run --analysis-dir data/floodgate/run/analysis \
  --run-manifest data/floodgate/run/manifest.json \
  --status-file data/floodgate/run/status.json
```

Set `--fv-scale` from the model card (the optional `nn_r6` release asset
requires 24; the CSA default is 16). The run manifest records the selected
value before connection. The client permits up to three authenticated
post-game transport reconnects without resetting `--max-games`.
Protocol, recording, and mid-game failures remain terminal. An active game
at a terminal failure is retained as uncounted `incomplete_game`.

While waiting, status records receive-idle diagnostics and periodic
heartbeats. An active-game deadline records `active_receive_deadline`;
a between-game timeout records `between_game_receive_timeout` with OS
error details, elapsed silence, and partial-line size. Neither declares
a game result or proves why the server stopped responding.

The one-shot supervisor records log/receipt failures and never replays a
child after an uncertain spawn or terminal status. A missing completion
receipt requires inspection of the pre-spawn marker and child state.
To launch the checked-in `RunAtLoad=true`, `KeepAlive=false` template:

```bash
python3 scripts/launch_floodgate_once.py \
  --plist "$HOME/Library/LaunchAgents/com.kent-tokyo.sekirei-floodgate.plist" \
  --runtime-dir "$HOME/.local/state/sekirei/floodgate/RUN_ID"
```

The helper uses `bootstrap` without an extra `kickstart` and refuses
registered labels. For `RunAtLoad=false`, it performs one non-forcing
`kickstart`. Its receipt accepts a bounded run only when launchd reports
one launch. Do not add an external restart loop.

Before tournament deployment, run the loopback two-process rehearsal:

```bash
python3 scripts/run_denryu_rehearsal.py \
  --binary target/release/sekirei-csa \
  --output data/rehearsals/denryu-$(date +%Y%m%d-%H%M%S)
```

[`DENRYU_REHEARSAL.md`](DENRYU_REHEARSAL.md) contains the recovery
checklist. Set `--completed-attempts` only from retained terminal evidence.

## Editing and cleanup

Shared byte hashing, canonical JSON hashing, and atomic JSON replacement live
in `sekirei_tools.artifacts`. Maintained release and gate entry points should
reuse these primitives; historical one-off experiment scripts remain frozen
until their archived evidence no longer depends on their exact behavior.

Most Python tools use only the standard library and have a focused
`test_<tool>.py`. After an edit, run that focused test, then:

```bash
python3 scripts/check_documentation_references.py
python3 scripts/check_public_surface.py
python3 scripts/check_release_metadata.py
python3 scripts/check_release_manifest_layout.py
```

Prefer one parameterized tool over near-duplicate wrappers. Retire one-off
instrumentation only after its evidence is retained and no live workflow or
document references it. `cleanup_runs.sh` is dry-run by default and requires
`APPLY=1` for deletion. Its protection scan includes Python and shell scripts
under `scripts/`, including the historical single-run tools in [`archive/`](archive/README.md).

## Experiment playbooks

Keep experiment decisions with their dated evidence:

- [Useful Lazy SMP depth](../docs/experiments/lazy_smp_useful_depth_v0.4.0/README.md)
  and [material-opening gate](../docs/experiments/aspiration_independent_gate_v0.4.1/README.md)
  define separate diagnostics and strength protocols.
- [`compare_yaneura_speed.py`](compare_yaneura_speed.py) compares pinned
  builds and evaluators under an ABBA throughput protocol, not Elo.
- [Deferred quiet HalfKP](../docs/experiments/quiet_halfkp_qs_v0.4.1/README.md),
  [refresh-column fusion](../docs/experiments/halfkp_refresh_tile_v0.4.1/README.md),
  [quiet-TT staging](../docs/experiments/quiet_tt_stage_v0.4.1/README.md),
  [continuation rows](../docs/experiments/quiet_cont_rows_v0.4.1/README.md), and
  [cooperative prestart cap](../docs/experiments/cooperative_root_prestart_cap_v0.4.1/README.md)
  retain their default-off verdicts and exact conditions.
