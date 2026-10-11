# Sekirei WebAssembly API

This crate packages Sekirei's rules and a bounded computer-move search for a
static browser client. It is separate from the native USI binary.

## Install

The prebuilt vX.Y.Z ES-module package is published with the
GitHub Release:

```sh
npm install https://github.com/kent-tokyo/sekirei/releases/download/vX.Y.Z/sekirei-wasm-X.Y.Z.tgz
```

`scripts/build_wasm_package.py` resolves this example from package metadata in
the README bundled into the release archive.

## Build

```sh
rustup target add wasm32-unknown-unknown
cargo install wasm-pack --version 0.15.0 --locked
python3 scripts/build_wasm_package.py
```

The generated `pkg/` directory is a self-contained ES module package. A static
client can import it without maintaining game state on a server. The helper
runs `wasm-pack` with the locked Cargo graph and adds `LICENSE-MIT`,
`LICENSE-APACHE`, and `NOTICE` to the package.

```js
import init, {
  startSfen,
  legalMoves,
  analyzeMate,
  analyzeMateInOne,
  applyMove,
  computerMove,
  analyzePosition,
  loadEvaluator,
  searchCapabilities,
} from "./pkg/sekirei_wasm.js";

await init();
const capabilities = searchCapabilities();
console.log(capabilities.effectiveWorkers); // 1
// Optional, one-shot evaluator loading. Run this before creating any board.
// The caller is responsible for the weight file's source and license.
const weightBytes = new Uint8Array(await (await fetch("./weights.bin")).arrayBuffer());
const loaded = loadEvaluator(weightBytes);
console.log(loaded.evaluatorId, loaded.weightSha256);
loaded.free();
let sfen = startSfen();
console.log(legalMoves(sfen));
sfen = applyMove(sfen, "7g7f");
const reply = computerMove(sfen, 4, 50_000);
console.log(reply.effectiveWorkers); // 1
sfen = applyMove(sfen, reply.bestMove);
reply.free();

const evaluation = analyzePosition(sfen, 4, 50_000);
console.log(evaluation.kind, evaluation.scoreCp, evaluation.bound);
evaluation.free();

const problem = analyzeMateInOne(
  "4k4/2S3S2/2SGpGS2/9/4R4/9/9/9/4K4 b - 1",
);
console.log(problem.validPosition, problem.solutions, problem.uniqueSolution);

const longerProblem = analyzeMate(
  "4k4/9/2G3S2/5R3/2GG5/9/9/9/4K4 b - 1",
  5,
  1_000_000,
);
console.log(longerProblem.outcome, longerProblem.shortestMatePly); // mate, 3
```

Rejected input throws an object with stable `code` and human-readable
`message` fields.

## Position evaluation contract

`analyzePosition(sfen, maxDepth, maxNodes)` is additive; `computerMove` is
unchanged.
It returns a `PositionAnalysis` wasm-bindgen object; call `free()` when done.

| Field | Contract |
| --- | --- |
| `kind` | `cp`, `mate`, `terminal`, or `unknown` |
| `scoreCp` | Present only for normal completed scores; positive favors the side to move |
| `matePlies`, `winner` | Mate distance (absolute plies) and winning side (`b` = Sente, `w` = Gote); not a shortest-mate proof |
| `sideToMove`, `scorePerspective`, `scoreUnit` | SFEN side (`b`/`w`), `sideToMove`, and `cp` (normal scores only) |
| `depth`, `bound`, `bestMove`, `principalVariation` | All come from the same completed iteration; the PV is a legal primary-line prefix and `bound` is `exact`, `lower`, `upper`, or `unknown` |
| `pvSource`, `candidateLineCount` | `completed_iteration` with one primary line, or `none` with zero lines; fallback moves are never promoted to a completed PV |
| `aborted`, `abortReason`, `nodes` | Node exhaustion, optional `node_limit`, and total nodes including a partial deeper pass |
| `usedFallback` | No iteration completed, but a legal fallback move exists |
| `terminalReason`, `inCheck` | Optional `checkmate`/`no_moves`, and whether the moving king is in check |
| `evaluatorId`, `evaluatorVersion`, `evaluatorWeightSha256` | Actual evaluator identity and exact input-byte hash; material has no hash |
| `engineVersion`, `apiVersion` | Cargo version and contract schema `3` |

An interruption **after** a shallow pass can still return its completed score
and bound with `aborted = true`. `bound` describes that score, not the later
interrupted pass. Before any pass completes, `kind = "unknown"`, `depth = 0`,
`bound = "unknown"`, `principalVariation = []`, and both numeric score fields are absent. The core's
fallback/static score is never presented as a measured zero or completed
evaluation. Mate scores are decoded here, never passed as huge centipawns.
Mate distances and bounds describe the search result, not exhaustive
shortest-mate validation; use `analyzeMate` for that different contract.

Legal-move exhaustion returns `terminal` without a finite score. Its `winner`
is the other side, including non-check `no_moves` (shogi has no chess
stalemate draw); its `bound` is `exact`, `depth` and `nodes` are zero, and there
is no best move. This stateless SFEN API cannot adjudicate repetition,
perpetual check, resignation, or other game-history outcomes.

Malformed SFEN and limits still throw structured `invalid_sfen`,
`input_too_large`, or `invalid_search_limit` errors. Analysis additionally
requires exactly one king per side and the non-moving king not in check;
otherwise it throws `invalid_position`. Before board parsing it also rejects
inventories beyond the physical supply (both colors' board and hand pieces
combined: 18 pawns, 4 each lance/knight/silver/gold, 2 each bishop/rook).
Promoted pieces count as their base kind; omitted pieces are allowed. This
guards packed hand storage and later captures from oversized-input traps.
The moving king may be in check.
The existing rule and move-selection APIs retain their original contracts.

The same depth 1–8, nodes 1–100,000, sequential one-worker,
4 MiB TT limits apply. Run synchronous analysis in an application Web Worker
for responsive UI; worker orchestration is outside this API. There are no
weight files bundled in this build. Cache clients should include
the schema, evaluator and engine identities, search limits, and package/source
hash: a candidate can share a Cargo version with a prior release.

## Browser evaluator loading

`loadEvaluator(Uint8Array)` accepts complete `SEKIRW01` or
`HalfKP 256x2-32-32` images. It validates the format, exact size, numeric
values, and the advertised maximum before changing evaluator state. Success
returns the format, byte count, evaluator ID/version, and SHA-256 of the exact
input bytes. Rejected input throws `invalid_evaluator` or
`evaluator_too_large` and leaves material evaluation active.

Loading is deliberately one-shot for each WebAssembly instance. A second
load throws `evaluator_already_loaded`; terminate and recreate the application
Web Worker to select another checkpoint. `searchCapabilities()` publishes the
accepted formats, maximum byte count, and `evaluatorReloadSupported = false`.
The package does not fetch or redistribute weights, and this API does not make
a playing-strength claim for any checkpoint.

## Mate-in-one validation

`analyzeMateInOne(sfen)` treats the SFEN side to move as the attacker and uses
the same complete legal-move rules as `sekirei-core`. It checks that each side
has exactly one king and that the defender is not already in check, then tests
every legal attack and every legal reply. This includes distant and discovered
checks, promotion choices, drops, nifu, dead-rank restrictions, and
uchifuzume.

The result exposes `validPosition`, `defenderAlreadyInCheck`, `invalidReason`,
the sorted USI `solutions`, and `uniqueSolution`. A parseable but invalid
problem returns `validPosition = false`; malformed SFEN throws the structured
`invalid_sfen` error.

## Bounded shortest-mate validation

`analyzeMate(sfen, maxPly, nodeLimit)` searches odd depths in increasing order.
The attacker plays checking moves only and the defender may choose every legal
reply. A completed `mate` result contains the shortest mate length and all
first moves that force mate at that length. `no_mate` means the full requested
bound was searched. If the node budget expires, the result is `unknown`,
`aborted` is true, `reason` is `node_limit`, and partial solutions are omitted.

The result exposes `validPosition`, `outcome`, `shortestMatePly`, `solutions`,
`uniqueSolution`, `nodes`, `aborted`, and `reason`. `maxPly` is limited to 1
through 15 and `nodeLimit` to 1 through 1,000,000. Callers can pass an
intermediate SFEN to inspect later attacking turns with the same contract.

## Browser search contract

- Built-in material evaluation until the caller explicitly loads one validated
  evaluator byte image. No filesystem API or weight file is bundled.
- Sequential search only. `searchCapabilities()` reports `maxWorkers = 1`,
  `effectiveWorkers = 1`, `workerThreadsSupported = false`, and
  `sharedArrayBufferRequired = false`. It also reports
  `maxCandidateLines = 1` and `multiPvSupported = false`; browser analysis
  currently exposes only the primary PV.
- Every `computerMove` result repeats the actual `effectiveWorkers` value. Do
  not derive a core selector from `navigator.hardwareConcurrency`; this build
  accepts no worker-count setting and always falls back to one worker.
- `maxDepth`: 1 through 8.
- `maxNodes`: 1 through 100,000.
- A 4 MiB transposition table is created for each `computerMove` call.
- If the node budget expires before completing a root move, the API returns a
  deterministic legal fallback and sets `usedFallback` to `true`.

These limits bound one synchronous call. Applications should still avoid
calling the search repeatedly from latency-sensitive UI handlers.
