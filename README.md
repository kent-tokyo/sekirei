# Sekirei — Rust Shogi Engine

[![CI](https://github.com/kent-tokyo/sekirei/actions/workflows/ci.yml/badge.svg)](https://github.com/kent-tokyo/sekirei/actions/workflows/ci.yml)
[![crates.io](https://img.shields.io/crates/v/sekirei.svg)](https://crates.io/crates/sekirei)
[![License](https://img.shields.io/crates/l/sekirei.svg)](https://github.com/kent-tokyo/sekirei/blob/main/LICENSE)

[日本語](README_ja.md)

Sekirei is an experimental **shogi (Japanese chess) engine written in Rust** (current release:
`0.3.35`). It speaks the
Universal Shogi Interface (USI) protocol used by shogi GUIs, supports CSA/Floodgate games, and
includes NNUE-style evaluation, parallel alpha-beta search, and tools for self-play strength
testing.

This project is for developers interested in Rust game engines, shogi search, safe parallelism,
NNUE evaluation, and reproducible engine experiments. Playing strength and evaluation quality
are still under development; Sekirei makes no absolute rating or superiority claim.

## Quick start

Install the USI engine from crates.io:

```bash
cargo install sekirei
sekirei
```

Or build the latest source checkout:

```bash
git clone https://github.com/kent-tokyo/sekirei.git
cd sekirei
cargo run --release -p sekirei
```

The binary reads USI commands from standard input. To connect Sekirei to a compatible shogi GUI,
select the installed `sekirei` executable as the engine command. Sekirei can run without a
checkpoint using its material-evaluation fallback; pass an NNUE weight file as the first argument
to enable a trained evaluator:

```bash
sekirei /path/to/weights.bin
```

## Features

- Rust shogi engine with a 9×9 board, legal move generation, promotion, drops, SFEN, and USI moves.
- USI engine binary for shogi GUI integration and command-line analysis.
- Iterative deepening, negamax/alpha-beta search, PVS/YBW parallel search, quiescence search,
  move ordering, and pruning heuristics.
- Lock-free transposition table, optional speculative parallel search, and an
  opt-in Lazy SMP search backend.
- An opt-in deterministic root MCTS pilot with injectable policy/value providers.
- An opt-in bounded df-pn mate-search API with node/depth limits and safe
  `Unknown` results when the configured boundary is reached.
- NNUE-style efficiently updatable evaluation with file-based checkpoints.
- Bounded inspection of external YaneuraOu-style SFNN headers with provenance
  and compatibility-manifest validation; this does not yet enable external-weight inference.
- CSA v2.2 / Floodgate client for automated games.
- USI-vs-USI match runner for self-play, regression testing, and relative Elo estimation.
- NNUE training pipeline from CSA games or extracted positions.
- Pure Rust core logic with no `unsafe` blocks in the core search and evaluation code.

## Status

- Pure Rust; the core search and evaluation code contains no `unsafe`.
- USI engine binary: `sekirei`.
- CSA client binary: `sekirei-csa`.
- Match runner binary: `sekirei-match`.
- NNUE training binary: `train` (package: `sekirei-train`).
- NNUE weights are loaded from a file and are not bundled.

The `sekirei` package on crates.io is the USI engine binary. The repository is a Cargo workspace
that also publishes the reusable `sekirei-core` library and supporting command-line tools.

## Repository layout

```text
crates/sekirei-core/         board, move generation, search, TT, evaluation
crates/sekirei-usi/          USI engine (`sekirei`)
crates/sekirei-csa/          CSA/floodgate client (`sekirei-csa`)
crates/sekirei-match-runner/ USI-vs-USI match runner (`sekirei-match`)
crates/sekirei-train/        NNUE training pipeline (`train`)
crates/sekirei-bench/        benchmarks
scripts/                     training and strength-test helpers
```

The core currently includes alpha-beta/negamax, PVS/YBW parallel search, iterative deepening,
quiescence search, a lock-free transposition table, common move-ordering and pruning heuristics,
optional speculative search, and an opt-in Lazy SMP backend. `SpecTopN=0` disables speculative
search and is useful when a repeatable run is required. A verification search used by singular extensions is deliberately
excluded from unrestricted TT writes, so a partial verification result cannot overwrite the
parent node's reusable entry.

The core also contains experimental root-level MCTS and bounded df-pn APIs.
They are opt-in research components, are not wired into the default USI mode,
and do not establish a playing-strength result.

For a bounded mate probe, the USI option `SearchMode=Dfpn` selects the df-pn
backend. It is intentionally opt-in, uses the requested `depth` as its ply
boundary, and should not be treated as the default playing mode or as a
strength comparison.

## Build and test

```bash
cargo build --release
cargo test --release
cargo bench --bench movegen -p sekirei-bench
```

### Local performance snapshot

The v0.3.27 hot-path pass reduced the median start-position timings on the development Mac from
8.2711 us to 2.2151 us for legal move generation, from 9.2530 ms to 2.1082 ms for Perft(3), and
from 22.544 ms to 7.659 ms for depth-4 search. The longer search sample used 20 measurements.
These are local mechanism diagnostics on heterogeneous Apple CPU cores, not portable performance,
playing-strength, or Elo claims.

For a low-cost cross-library diagnostic against the pinned `rsshogi` commit and
`shogi_core` primitive, run:

```bash
cargo run --release -p sekirei-bench --bin cross_library
cargo run --release -p sekirei-bench --bin cross_library -- --check
cargo run --release -p sekirei-bench --bin cross_library -- --components
```

The v7 harness validates every ply and undo of the six-move fixture before
timing. The old v6 six-move roundtrip timings are invalid (moves were undone
too early). v7 move generation observes output slices without the old
representation-dependent encoding checksum, so those timings are not directly
comparable either. `--components` isolates warm initialization, buffers,
board updates with/without NNUE, NNUE inference, and output conversion, retaining
66 fixed cases with 21 raw samples plus p50/p95. NNUE uses deterministic synthetic LCG weights;
this is not a trained-engine search benchmark. Capture a frozen executable and
source hashes with `scripts/run_component_benchmark.py --help`.
The fixed timing parameters are recorded in
`scripts/fixtures/speed_contract_v1.json`; captures with mismatched headers are
rejected by the validator.
The [component measurement report](scripts/benchmark_reports/components_2026-09-12.md)
records the repaired protocol, the SFEN initialization pilot and its load limitations.
The isolated `sekirei_nnue_refresh` cases separate accumulator-refresh cost from
NNUE forward cost; their pilot results are recorded in the internal report.
Library users that only need rules state can use `Board::from_sfen_rules_only` to
skip the NNUE refresh; the returned board has a valid hash but must be refreshed
with `Board::refresh_acc` before NNUE evaluation or incremental NNUE updates.
The SP0 smoke corpus contains 128 positions: eight categories, four original
and four mirrored positions per category, and both sides to move. It is
structurally checked with `python3 scripts/validate_speed_corpus.py`; it is
still a smoke corpus rather than a full legal-move parity proof over generated
game histories. `python3
scripts/preflight_speed_corpus.py --binary PATH` additionally checks the
recorded legal-move and Perft(2) counts against Sekirei and rsshogi. Compare two
completed component captures with
`python3 scripts/compare_component_benchmarks.py --baseline DIR --candidate DIR`.
The corpus also stores the sorted expected USI move set for each position, so
the preflight rejects an equal-count but different move set.
It also stores and verifies the Perft(2) divide for every legal root move,
including the sum against the aggregate count.
The canonical cases array is covered by a SHA-256 recorded in the fixture.
New captures require a clean worktree; use `--allow-dirty` only for explicitly
diagnostic runs, whose dirty status is retained in `provenance.json`. The
`--build` mode performs the pinned offline release build before capture and
records its command and profile.
The resulting single capture is summarized in
`scripts/benchmark_reports/component_current_release_2026-09-12.md`; it is
diagnostic evidence, not the formal ten-session gate.
The current ten-capture same-binary A/A noise-floor result is recorded in
`scripts/benchmark_reports/component_aa_current_2026-09-12.md`.
The corresponding three-case, ten-session rsshogi comparison is recorded in
`scripts/benchmark_reports/cross_library_component_10session_2026-09-12.md`.
The corpus preflight also applies and reverses each case's verified one-move
sequence; the six-ply nested roundtrip remains covered by the component
diagnostic preflight. It now additionally generates twelve deterministic legal
moves from every corpus SFEN using the seed in `speed_contract_v1.json` and
verifies the nested roundtrip in both engines.

The latest maintenance pass also split root-search safety stages, shared the
alpha-beta beta-cutoff bookkeeping, and simplified quiet-move derived-bitboard
updates. These are correctness/readability refactors included in release
`0.3.35`; they are not presented as a measured speed increase.

The SP0 smoke corpus is checked structurally with
`python3 scripts/validate_speed_corpus.py`. It contains 128 verified smoke
positions; the separate `python3 scripts/validate_speed_corpus_split.py`
validator confirms the fixed 64/64 tuning/hold-out partition. These checks do
not cover full generated game histories.

The report compares legal move generation and Perft(3) with `rsshogi` on the
same fixture. `shogi_core` has no legality checker or move generator, so its
state-update row is not a move-generation or Perft comparison. Results and
scope limitations are recorded in
`scripts/benchmark_reports/cross_library_v0.3.33.md`.

The v0.3.33 optimization snapshot recorded in that report (10,000 iterations x
seven samples) is locally faster than pinned `rsshogi` on start-position
Perft(2) (6.455 us vs 8.008 us), Perft(3) (192.399 us vs 250.672 us), and
midgame-with-hands Perft(2) (31.900 us vs 51.440 us). `rsshogi` is still faster
for one-shot legal move generation in that historical protocol. The full-state
roundtrip also includes setup and Sekirei NNUE work absent from the reference;
it cannot establish a pure board-update ranking. This is a bounded Perft result
rather than a general speed claim.
Against clean revision `4c568b1`, the same candidate reduced the Criterion
depth-4 search median from 3.348 ms to 2.265 ms (about 1.48x faster).

Probe an NNUE checkpoint without enabling process-global engine weights:

```bash
cargo run --release -p sekirei-bench --bin nnue_probe -- /path/to/weights.bin
# For automation, add --json; custom positions use repeated --sfen "<SFEN>".
```

The probe reports evaluator scores, score range, mean, variance, and reference
deltas, plus `constant_output` and `reload_deterministic` flags. Add `--json`
for machine-readable output; `--strict` exits non-zero for constant or
near-constant output (range below 8 cp), missing material/side-to-move sensitivity, or
non-deterministic reload. It is a
diagnostic, not a strength test.
JSON output also includes `strict_min_range_cp` and `strict_pass` so automated
candidate selection can record the exact health rule used.
It also records `l2_distinct_values`, `l2_bias_distinct_values`, and
`out_distinct_values` to expose collapsed later layers directly.

Checkpoint files are inference-compatible when loaded by `nnue_probe` or `EvalFile`. The inference
`.bin` remains optimizer-free; training emits separate Adam and full-resume sidecars.

External SFNN artifacts are intentionally handled in two stages. The Rust
`sekirei_core::external_eval::read_sfnn_header` API and
`scripts/inspect_external_sfnn.py` perform bounded header inspection only.
Validate source, license, redistribution, and adapter status with
`python3 scripts/validate_external_eval_manifest.py scripts/fixtures/external_eval_manifest_v1.json`.
A valid manifest does not make an external file inference-compatible; feature mapping,
parameter layout, and numerical agreement require a separate adapter review.

Run the USI engine without weights (material evaluation fallback):

```bash
cargo run --release -p sekirei
```

Run it with NNUE weights:

```bash
cargo run --release -p sekirei -- /path/to/weights.bin
```

Print the version without starting the USI loop:

```bash
cargo run --release -p sekirei -- --version
```

Use `--help` for a concise usage summary.

## USI options

The engine reports the complete option list after `usi`. The main options are:

- `Hash`, `Threads`, `MoveOverhead`
- `SearchMode` (`Speculative` by default; optional `LazySMP`)
- `Ponder`, `MultiPV`
- `EvalFile` (loaded on `isready`)
- `SpecTopN` (default `3`; `0` disables speculation)
- `UseBook`, `BookFile`, `BookMaxPly`, `BookMinConfidence`

With `SpecTopN > 0`, scheduling of speculative tasks can make otherwise identical searches
nondeterministic. Use `SpecTopN=0` for deterministic comparisons where practical. For correctness
diagnostics, keep `Threads=1`, `Parallel=1`, and `SpecTopN=0`; this control is separate from
timing and match-strength measurements.

In `SearchMode=LazySMP`, `Threads` selects the number of independent workers. Workers use private
boards and heuristic tables while sharing the lock-free transposition table and stop flag. This
mode remains opt-in; the default is `SearchMode=Speculative`.

## CSA / floodgate

```bash
cargo run --release -p sekirei-csa -- \
  --user <name> --trip <secret> --game floodgate-300-10F \
  --record-dir data/floodgate --loop
```

`FLOODGATE_ACCOUNT` and `FLOODGATE_TRIP` may be used instead of command-line credentials.
Completed games and partial games are saved locally as CSA files under `data/floodgate/` by
default. Use `--record-dir` to choose another directory. Records are flushed after every move;
save failures are reported but do not abort the live game. Do not commit credentials, game
records, weights, or generated training data.

For opt-in post-game analysis, pass `--analysis-dir <dir>`. The client writes a sidecar
`*.analysis.jsonl` file with one schema-versioned summary for each Sekirei search. Its header records
the engine version and score perspective; each search record contains pre-move SFEN,
selected CSA move, score, completed depth, nodes, elapsed time, and hashfull. The default is off;
completed games receive a final `game_end` result event, and interrupted games are closed with
`aborted` so the sidecar is not left as an apparently complete trace. Final buffers are flushed
and synced. The sidecar is diagnostic evidence, not a strength claim.
Replay analysis also checks that this result matches the CSA result marker.
The CSA record remains unchanged.
CSA files created before `--analysis-dir` was enabled do not contain per-move search values and
cannot be retroactively treated as evaluation traces.
Validate a generated sidecar with `python3 scripts/validate_analysis_record.py <file>.analysis.jsonl`.
The validator requires a final `game_end`; an interrupted trace is not accepted as a complete game analysis.
Align it with the corresponding CSA game and flag score swings with
`python3 scripts/analyze_analysis_record.py <game>.csa <game>.analysis.jsonl --output swing.json`.
This is a replay alignment diagnostic, not an engine re-search or strength claim.
Aggregate reports by game result and rank negative swings with
`python3 scripts/summarize_analysis_swings.py data/reports/*.json --output swing-summary.json`.
For a whole Floodgate record directory, use
`python3 scripts/analyze_floodgate_analysis.py --csa-dir data/floodgate --analysis-dir data/floodgate-analysis --output floodgate-analysis.json`.
Pre-v1 sidecars are explicitly excluded under `legacy_analysis`; malformed v1 sidecars remain
excluded under `invalid_analysis`. Mate-like scores (absolute score at least 800,000cp) and final
game records are separated as `terminal_records`, while `normal_swings` contains only ordinary
position score reversals.
Normal reversals are also split into `negative_swings` and `positive_swings` for failure-oriented diagnosis.
Each candidate also carries a coarse `opening`/`middlegame`/`endgame` phase based on its ply ratio.
Depth (`shallow`/`medium`/`deep`) and elapsed-time (`fast`/`normal`/`slow`) bands are also included
to distinguish search-budget instability from evaluator instability.
These analysis fixtures run in CI as diagnostic contract checks.
To extract negative candidates with SFEN-derived material balance, game phase, and search conditions,
run `python3 scripts/classify_swing_positions.py floodgate-analysis.json --output negative-swings.json`.
Each candidate includes the preceding score and ply, allowing review of the position immediately before the reversal.
The preceding two CSA moves are also attached as `previous_move_csa` and `two_moves_back_csa`.
Compare NNUE and material scores for a candidate SFEN with
`cargo run --release -p sekirei-core --bin compare_eval -- <weights> "<sfen>"`.
The CI replay fixture also exercises legal CSA application and pre-move SFEN equality.
For full board-semantic verification, build `sekirei-analysis-replay` and run it with the CSA
file and sidecar; it rejects illegal CSA moves and SFEN mismatches before reporting alignment.

The client consumes the server-provided CSA position instead of assuming `startpos`. It supports
the standard `PI` shorthand and hand declarations, validates the color of every received move,
and preserves the final `#WIN`/`#LOSE`/`#DRAW` result when a server sends an intermediate
`#RESIGN` marker. These checks are important for distinguishing a protocol/board-sync failure
from an engine evaluation result.

The client does not automatically resign on a mate-like score produced by a bounded or interrupted
auxiliary search. It continues with a legal fallback move when one exists; this prevents a time
limit or an inconclusive diagnostic search from being recorded as a false resignation.

## Match testing

```bash
cargo run --release -p sekirei-match-runner -- \
  --engine1 ./target/release/sekirei \
  --engine2 /path/to/other-engine \
  --games 100 --byoyomi 10000 \
  --positions data/gate/openings_standard.sfen \
  --games-per-position 4 --json results/run.json
```

Use `gate` on a result JSON to evaluate an existing match. Self-play Elo is relative to the
selected baseline and is not a floodgate rating.

## NNUE training

The training command accepts CSA games or pre-extracted positions. For all options, run:

```bash
cargo run --release -p sekirei-train -- --help
```

Example:

```bash
cargo run --release -p sekirei-train -- \
  --games /path/to/csa_dir --output weights.bin --epochs 3
```

Teacher-search leaves use material evaluation by default. To run a
fixed-teacher/self-distillation experiment, select one immutable checkpoint:

```bash
cargo run --release -p sekirei-train -- \
  --games /path/to/csa_dir --output student.bin --epochs 3 \
  --teacher-eval nnue --teacher-weights teacher.bin
```

The teacher weight hash is included in teacher-cache entries, complete-resume fingerprints, and
checkpoint metadata. A cache or resume checkpoint from another teacher is rejected instead of
silently mixing label sources. This option defines an experiment contract; it is not by itself a
strength claim.

Use `--label-time-ms N` to place a hard wall-clock limit on each cache-miss teacher search when
fixed-depth labeling has pathological outliers. The limit is part of the cache identity, resume
fingerprint, and checkpoint metadata, so bounded labels cannot be mixed with unlimited labels.
For reproducible single-thread labeling, prefer `--label-nodes N`: it applies a deterministic node
budget and is bound into the same cache/resume/metadata contract without depending on host load.

Training data, checkpoints, weights, match output, and experiment logs are local artifacts and
are intentionally excluded from the public repository. NNUE weight files produced for this
project are licensed separately under CC BY 4.0; see [NNUE-LICENSE.md](NNUE-LICENSE.md).

Epoch checkpoints also write a training-only `.adam.json` sidecar containing raw f32 parameters,
Adam moments, and the optimizer step. Resume that state with `--resume-adam`; the inference `.bin`
file remains separate and compatible with the engine. A diagnostic classification can be attached
to a release-manifest-shaped copy without modifying the original:

```bash
python3 scripts/classify_evaluator_failure.py diagnostic.json \
  --manifest release-manifest-v0.3.34.json \
  --output release-manifest-v0.3.34-diagnostic.json
```

Validate the operational fixture or a generated copy with
`python3 scripts/validate_release_manifest.py scripts/fixtures/release_manifest_diagnostic_v1.json`.
For an epoch-boundary full training resume, use `--resume-checkpoint`; it restores the raw weights,
Adam state, completed epoch, data cursor, and recipe fingerprint and rejects recipe mismatches. Set
`--resume-checkpoint-every-games N` to persist an atomic mid-epoch cursor (for positions mode, N is
the position chunk size); the teacher cache is included so a resumed run does not silently change labels.
The small end-to-end regression is available as `bash scripts/test_resume_cli_fixture.sh`.
Resume verification lineage can be recorded with
`python3 scripts/record_resume_run.py --checkpoint run.resume.json --log run.log --dataset data.jsonl --output resume-manifest.json`.
The generated artifact uses the `sekirei.resume-manifest.v1` schema and keeps checkpoint/log hashes separate.
Attach verified resume evidence to a release-manifest copy with
`python3 scripts/attach_resume_manifest.py --release-manifest release-manifest-v0.3.34.json --resume-manifest resume-manifest.json --output release-manifest-with-resume.json`.
The source release manifest is not modified.
The attached `resume_verification.artifacts` list identifies the checkpoint and execution log separately.

To verify a saved SharedMcts transcript against its diagnostic manifest copy,
run `python3 scripts/verify_mcts_diagnostic.py --manifest candidate-manifest.json --transcript shared-mcts-transcript.txt`.
This checks the manifest schema and all three diagnostic counts; it does not make a strength claim.
CI also downloads the named diagnostic artifact in a separate audit job and repeats both checks.

For a small deterministic comparison of the two MCTS pilots on three legal
positions (including a natural commuting-move and developed position), run
`cargo run --release -p sekirei-core --example mcts_fixed_budget_diagnostic`.
It defaults to 64 simulations at depth four and reports nodes, score, selected move,
and sharing hits; repeatability is diagnostic evidence, not a strength claim.
Pass `--simulations 8 --max-depth 2` for a smaller budget; each budget is
recorded in its own validated comparison manifest.
The same bounded log is retained with the SharedMcts CI diagnostic artifact and
checked again by the artifact-audit job.
CI also records the per-position comparison in a validated manifest copy;
the copy remains diagnostic-only and is not a release artifact.
Summarize such a copy with `python3 scripts/summarize_mcts_comparison.py
candidate-comparison-manifest.json`; the report contains node reduction and
agreement classification fields only, with `strength_claim` fixed to `false`.
CI retains both full- and small-budget summaries with the comparison artifact
and checks their JSON shape and non-strength classification.
Aggregate multiple budget manifests with `python3 scripts/aggregate_mcts_summaries.py
small-manifest.json full-manifest.json --output budget-summary.json`.
The CI artifact includes this aggregate alongside the individual summaries.

For a controlled interruption at an atomic checkpoint boundary:

```bash
cargo run -p sekirei-train -- --positions positions.jsonl --epochs 20 \
  --checkpoint-dir checkpoints --output weights.bin \
  --resume-checkpoint-every-games 1000 --stop-after-resume-checkpoint
cargo run -p sekirei-train -- --positions positions.jsonl --epochs 20 \
  --output weights.bin --resume-checkpoint weights.resume.json
```

Resume rejects an unsupported schema, missing or malformed optimizer state,
non-finite values, a recipe fingerprint mismatch, a cursor beyond the current
epoch, simultaneous `--resume-adam` and `--resume-checkpoint`, or a target
epoch that has already been completed.

## License and attribution

The Sekirei source code is licensed under the MIT License or the Apache License, Version 2.0,
at your option: [LICENSE-MIT](LICENSE-MIT) or [LICENSE-APACHE](LICENSE-APACHE). Please retain
[NOTICE](NOTICE), including the copyright and attribution notice.

Recommended attribution for products based on Sekirei:

```text
This product is based on Sekirei,
an open-source shogi engine developed by Kentaro Tanabe.

https://github.com/kent-tokyo/sekirei
```

If a product has a Legal Notices screen, the attribution above is a suitable display. The
attribution is strongly recommended, but it is not an advertising requirement of the standard
licenses. Do not use the Sekirei name or logo to imply official endorsement or approval without
permission. NNUE weight files are separate artifacts and are licensed under CC BY 4.0 as
described in [NNUE-LICENSE.md](NNUE-LICENSE.md).

The current release record is kept in
[`release-manifest-v0.3.34.json`](release-manifest-v0.3.34.json). The current Lazy SMP USI smoke
transcript is [`scripts/fixtures/usi_smoke_v0.3.34.txt`](scripts/fixtures/usi_smoke_v0.3.34.txt).
These are release-audit evidence, not strength claims.

For an opt-in MCTS candidate diagnostic, create a validated manifest copy without modifying the
release record:

```bash
python3 scripts/record_mcts_manifest.py \
  --release-manifest release-manifest-v0.3.34.json \
  --output candidate-manifest.json --mode SharedMcts \
  --simulations 4 --arena-nodes 31 --transposition-hits 0
```

When the counts are present in a captured USI transcript, they can be extracted without manual
copying:

```bash
python3 scripts/record_mcts_transcript.py \
  --release-manifest release-manifest-v0.3.34.json \
  --transcript shared-mcts-transcript.txt --output candidate-manifest.json
```

Before a release, check the public metadata without compiling or running the engine:

```bash
python3 scripts/check_release_metadata.py
```

This verifies that all crate manifests, `Cargo.lock`, the changelog, the English and Japanese
README, and the license/attribution files agree on the current version.
