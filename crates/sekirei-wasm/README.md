# Sekirei WebAssembly API

This crate packages Sekirei's rules and a bounded computer-move search for a
static browser client. It is separate from the native USI binary.

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
```

Rejected input throws an object with stable `code` and human-readable
`message` fields.

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
