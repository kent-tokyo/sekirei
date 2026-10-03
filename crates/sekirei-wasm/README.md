# Sekirei WebAssembly API

This crate packages Sekirei's rules and a bounded computer-move search for a
static browser client. It is separate from the native USI binary.

## Install

The prebuilt v0.3.57 ES-module package is published with the GitHub Release:

```sh
npm install https://github.com/kent-tokyo/sekirei/releases/download/v0.3.57/sekirei-wasm-0.3.57.tgz
```

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
  searchCapabilities,
} from "./pkg/sekirei_wasm.js";

await init();
const capabilities = searchCapabilities();
console.log(capabilities.effectiveWorkers); // 1
let sfen = startSfen();
console.log(legalMoves(sfen));
sfen = applyMove(sfen, "7g7f");
const reply = computerMove(sfen, 4, 50_000);
console.log(reply.effectiveWorkers); // 1
sfen = applyMove(sfen, reply.bestMove);

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

- Built-in material evaluation only; no external weights are loaded.
- Sequential search only. `searchCapabilities()` reports `maxWorkers = 1`,
  `effectiveWorkers = 1`, `workerThreadsSupported = false`, and
  `sharedArrayBufferRequired = false`.
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
