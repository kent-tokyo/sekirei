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
| `validate_nnue_candidate_card.py` | Validates an unpublished self-NNUE candidate card and, with `--artifact`, its size, SHA-256, HalfKP header, descriptor, and FV scale. It cannot authorize distribution or a default evaluator. |
| `validate_nnue_cpu_pilot.py` | Validates a frozen, short AWS CPU-pilot plan only after a colour-reversed material gate, complete candidate/data/resume evidence, and an observed CPU-dominant bottleneck. It does not provision cloud resources. |
| `check_halfkp_oracle.py` | Compares `halfkp_oracle` HalfKP scores with a separately executed reference USI engine; see [NNUE weights](../docs/nnue_weights.md#external-halfkp-networks). |
| `run_ab_match.py` | Fixed-protocol local A/B, paired SPRT, or node-limited YaneuraOu measurement. It hashes inputs and fails closed on incomplete or inconsistent JSONL, kifu, snapshots, or result sidecars. `--supervised --result-json PATH` adds a one-shot parent that records a terminal error if the wrapper or match child exits unexpectedly. Material-only, one thread and `SpecTopN=0` are the defaults. |
| `launch_floodgate_once.py` | Starts one rendered bounded Floodgate LaunchAgent without combining `RunAtLoad=true` and `kickstart`; refuses registered labels and records the single-run acceptance receipt. |
| `nps_threads.py` | Thread-scaling diagnostic for NPS, completed depth, best-move stability and optional Lazy SMP/TT/root-hint counters. `--summary-json` binds TT reuse, sampled overlap, discarded root work and worker-node balance for 4T/8T diagnosis. Counter-enabled runs are separate from speed runs; no output qualifies a speed or Elo claim. It completes new-game readiness before setting the position because some engines reset their board on `isready`. |
| `paired_search_diagnostics.py` | Fresh-process A/B/B/A comparison for one search factor. Separates deterministic signatures, counter-free timing and opt-in parallel diagnostics; it does not estimate Elo. |
| `build_pgo_lane.py` | Creates native, instrumented, and profile-use binaries from one clean source state using a fresh material-only compiler-profile corpus by default. Optional paired `--training-evalfile` / `--training-eval-card` arguments accept a public-card-verified, own-trained HalfKP file read-only; card FV_SCALE, model/card hashes and runtime load acknowledgement are required. This profiles runtime code without updating weights or selecting a model. It checks source and input hashes between phases, separates build-script profiles from normal-exit training profiles, and preserves command diagnostics with the toolchain, corpus, profile and binary hashes. It does not qualify speed, strength, or default adoption; freeze the emitted binaries into a separate host-qualified benchmark or match gate. |
| `generate_search_diagnostic.py` | Creates new seeded, score-blind quiet/tactical diagnostic positions from Sekirei legal playouts and rule probes, with complete replay and executable-hash provenance. Reads no training corpus or external evaluator. Diagnostic-only; do not reuse as an independent gate. |
| `validate_paired_pv.py` | Freezes the Rust legal-move tool and replays every recorded PV, checking its bestmove prefix and each legal transition. Fails on incomplete runs or illegal PVs. This is a same-rules contract check, not an independent oracle or a proof of mate scores. |
| `test_public_contracts.sh` | Lightweight aggregate for the public contract. |
| `check_rust_coverage.sh` | Runs the default workspace, opening-book tests, isolated tunable-search contracts, a bounded trainer smoke, and native WASM API contracts under `cargo-llvm-cov`; emits LCOV/JSON and fails below the documented 98% engine/library line-coverage contract. |

The supervised gate requires a fresh output namespace. Its attempt directory
remains after completion and prevents accidental replay. On interruption, the
terminal result is `error` with null Elo/SPRT verdict and separate observed and
verified game counts; raw match artifacts are retained. SIGTERM to the outer
supervisor is handled, while its own SIGKILL requires an OS-level observer.

### Before an EC2 CPU pilot

Keep a pilot plan outside the repository with every referenced artifact next to
it, then validate it before provisioning anything:

```bash
python3 scripts/validate_nnue_cpu_pilot.py /path/to/frozen-pilot.json
```

The plan is intentionally rejected unless the candidate has passed a
colour-reversed material gate, the data/whole-game hold-out/checkpoint-resume
evidence hashes match, and a local measurement resolves correctness, teacher,
and model-design concerns before identifying one CPU-dominant phase. It also
requires a bounded AWS CPU workload, duration, required throughput/RSS/resume
metrics, and hard stop conditions. Passing this check defines a pilot; it does
not provision infrastructure or approve the candidate for release.

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

Future counter-free `paired_search_diagnostics.py` movetime runs can opt into
bounded admission with `--idle-wait-seconds 300`, `--idle-poll-seconds 10`
and `--idle-stable-samples 3`. The default wait is zero and retains immediate
preflight behavior. This option changes measurement admission, not search
policy, the 25% per-process CPU threshold, an evaluator or a default.

The enabled lane records every initial/busy/unknown/idle snapshot before any
engine launch. Three consecutive idle snapshots are needed by default; a
busy or unavailable snapshot resets the count. A deadline closes the new
output as `blocked_host_busy` with zero searches. Interruption and frozen-input
drift also refuse dispatch. No external process is stopped, and `--allow-busy`,
fixed-depth/node budgets or counter-enabled lanes cannot use this admission.
The wait deadline uses a monotonic clock; an OS inspection can add its command
timeout, so it is not a hard real-time bound.

Add `--idle-min-free-mib 1024 --idle-stable-samples 5` to require both an
idle CPU snapshot and at least 1 GiB free on the output filesystem for five
consecutive samples. The capacity option defaults to zero and requires a
positive idle wait. Low or unavailable capacity resets the joint count while
the original CPU state/count remain separate. A timeout closes the output as
`blocked_host_resources` with zero searches. Capacity is checked again after
input verification, before each ABBA search and at postflight. A refusal before
the first dispatch returns `blocked_capacity_before_dispatch`; loss before a
later search leaves an incomplete run with its completed-search count. A low
postflight sample disqualifies timing even when all searches completed. These
are sampled checks, not disk reservation or monitoring inside a search.

The manifest retains the request, sample history and final admission decision.
Enabled idle runs atomically replace the manifest using the fingerprinted
artifact helper; a failure before replacement preserves the earlier document.
Frozen engines/corpus and measurement-source hashes are rechecked before and
after searching. The usual postflight load guard remains required. Samples do
not prove continuous host isolation. Use a new output directory under a
prospective protocol; this feature does not reopen or qualify previously closed studies,
and does not replace independent match-input/exclusion requirements.

Historical [benchmark reports](../docs/experiments/benchmark_reports/README.md)
apply only to their recorded host, revision, corpus, and operation. They are
neither general speed rankings nor Elo claims.

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
rejection categories and up to eight examples with decoded line numbers,
zero-based decoded byte offsets, row hashes and 256-character previews. Row hashes
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
The [initial storage comparison](../docs/experiments/positions_ingestion_v0.4.1/README.md)
retains synthetic-input RSS/time measurements and compatibility checks.

### Optional compressed JSONL

Build `sekirei-train` with `positions-gzip`, `positions-zstd`, or both. They are
independent and disabled by default. Gzip uses
[flate2 1.1.10](https://docs.rs/flate2/1.1.10/flate2/) with the explicit
`miniz_oxide` backend; zstd uses [ruzstd 0.9.1](https://docs.rs/ruzstd/0.9.1/ruzstd/).
Both decoders use Rust implementations without C/C++ bindings. The adapter uses
safe Rust; this does not assert that all transitive dependencies contain no unsafe.

```bash
cargo build --release -p sekirei-train --bin train \
  --features positions-gzip,positions-zstd
target/release/train --positions /path/to/positions.jsonl.zst \
  --positions-storage disk --positions-audit-only \
  --positions-manifest data/runs/compressed-ingest/audit.json
TRAIN_BIN="$PWD/target/release/train" POSITIONS_CODECS=gzip,zstd \
  python3 -B scripts/test_positions_compression.py
```

Content magic takes precedence over the filename. A `.gz`/`.gzip` or
`.zst`/`.zstd` suffix also selects the corresponding decoder when magic is missing,
so truncated headers do not silently become skipped JSONL. Disabled codecs report
the required Cargo feature. Concatenated gzip members and zstd frames are decoded
as one stream, including zstd skippable frames. Checksum errors, truncated streams,
invalid trailing data and unsupported frames fail the entire ingest, even without
`--strict-positions`, before publishing a complete audit. Zstd dictionaries are
unsupported, and the decoder window is limited to 8 MiB.

`report.sha256`, `report.input_bytes`, `--positions-sha256` and
`--validation-positions-sha256` always identify the physical file bytes before
decoding. `report.decoded_sha256` and `report.decoded_bytes` identify the exact JSONL
stream, including blank lines and original line endings; `report.compression`
records the selected codec. Rejection offsets, line numbers and row hashes refer
to that decoded stream. Plain input has identical physical and decoded identities.
The existing resume identity uses the physical hash: recompressing or switching
codecs requires a new run even if the decoded JSONL is unchanged. Changing only
memory/disk storage still permits resume.
The [compression validation](../docs/experiments/positions_compression_v0.4.1/README.md)
records the six-way audit/training comparisons and local coverage result.

## NNUE candidates and gates

`run_observed_process.py --state-dir /path/to/run/observer -- COMMAND ...`
launches exactly one child with no automatic retry. It fsyncs attempt-specific
stdout/stderr files, append-only start/exit events, and an atomic status file
with periodic free-space/CPU/RSS observations. A separate observer can record
`SIGKILL` even when the child cannot log; that signal alone does not prove an
OOM cause. An observer killed together with the child cannot record its own
exit, so launch it under the OS supervisor and retain the OS job's last exit
status as a second layer. A later observer records a previous unclosed attempt
as unknown and refuses to duplicate a child PID that may still be alive.
An interrupted `starting` state is also blocked because the spawn may have
succeeded before its PID was persisted. The tool observes the direct child
leader; it does not certify that descendants which detach into other process
groups have exited. The structured runner must own and inspect those workers.
It does not resume a qualification, select outcomes, or modify gate evidence.

`nnue_gate_host.py` provides shared-host **strength** trial admission. On the
preregistered 10-core policy, three consecutive observations at no more than
300% foreign CPU (three cores) and at least 512 MiB free admit the next
colour-reversed pair. CPU activity during a pair is telemetry only: finish
and retain both games, regardless of outcome or transient load, then wait
before the next pair if needed. Missing observations defer the next dispatch.
Do not use these rules to qualify idle-host speed/NPS, discard loaded games,
reopen historical invalid trials, or reinterpret a failed strength verdict.
Pin the policy and its source before a fresh trial. Engine/weight identities,
load acknowledgements, complete paired records and artifact validation remain
required; actual protocol or I/O failures still stop the trial.


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
During an active game, the client emits receive-idle diagnostics with the
expected response, elapsed wait and buffered partial-line byte count. Its
absolute response deadline uses the opponent's advertised time control and
remaining time inferred from received move echoes. Our move echo has a
2-minute deadline. `--active-receive-deadline-seconds` can
override it for a controlled run. Between games the 70-minute read timeout
still applies. A waiting-state `WouldBlock` or `TimedOut` read records
`between_game_receive_timeout` with the raw error kind/errno, time since the
last server line, and buffered partial-line size. It ends the bounded run
without replay; the status and one-shot receipt retain the completed count.
The server's silence and any link to a storage failure still need independent
evidence. An active-game deadline records
`active_receive_deadline` and ends the one-shot run with the active game
incomplete; it does not declare a CSA result or retry that game.
The supervisor records event and log write failures with their errno in the
one-shot receipt and on stderr. It emits `logging_failed` while the child is
running. If a child log fails, it still drains the
pipe and marks the receipt `log_persistence_failed`; the child is not replayed.
If the completion receipt itself cannot be saved, the supervisor exits with
an error instead of claiming a durable completion. A pre-spawn marker prevents
automatic or manual replay when the receipt is missing; such a run needs
operator inspection. Existing terminal client status or run manifests also
prevent replay when a run predates the marker.
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
python3 scripts/check_release_manifest_layout.py
```

Prefer one parameterized tool over near-duplicate wrappers. Retire one-off
instrumentation only after its evidence is retained and no live workflow or
document references it. `cleanup_runs.sh` is dry-run by default and requires
`APPLY=1` for deletion. Its protection scan includes Python and shell scripts
under `scripts/`, including the historical single-run tools in [`archive/`](archive/README.md).

## Experiment playbooks

Keep experiment-specific commands and decision rules with their dated
evidence instead of extending this tool index:

- [Useful Lazy SMP depth](../docs/experiments/lazy_smp_useful_depth_v0.4.0/README.md):
  score-blind corpus generation, ABBA diagnostics, and separate counter-free
  and instrumented lanes.
- [`compare_yaneura_speed.py`](compare_yaneura_speed.py): shared-evaluator
  ABBA throughput comparison. Pin both builds, evaluator, corpus, hash, CPU,
  compiler and ISA; keep NPS, completed work, and playing strength separate.
- [Independent material-opening gate](../docs/experiments/aspiration_independent_gate_v0.4.1/README.md):
  unused-opening exclusions, frozen manifests, paired SPRT, and fail-closed
  evidence requirements.
- [Deferred quiet HalfKP](../docs/experiments/quiet_halfkp_qs_v0.4.1/README.md)
  and [refresh-column fusion](../docs/experiments/halfkp_refresh_tile_v0.4.1/README.md):
  completed independent 4T speed gates. Neither candidate established an
  engine-level gain; both remain disabled.
- [Quiet-TT staging](../docs/experiments/quiet_tt_stage_v0.4.1/README.md),
  [continuation rows](../docs/experiments/quiet_cont_rows_v0.4.1/README.md), and
  [cooperative prestart cap](../docs/experiments/cooperative_root_prestart_cap_v0.4.1/README.md):
  closed default-off screens with negative, incomplete, or timing-blocked
  evidence.

The corresponding entry points are `generate_search_diagnostic.py`,
`paired_search_diagnostics.py`, `compare_yaneura_speed.py`,
`freeze_material_gate_openings.py`, and `run_ab_match.py`. Always inspect
`--help` and use a new output directory; the dated playbook, not this index,
defines the experiment contract.
