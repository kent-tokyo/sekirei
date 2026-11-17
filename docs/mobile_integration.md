# Mobile and on-device integration

Sekirei has a browser API and a native USI process interface. It does not yet
ship an iOS/Android FFI layer or claim production mobile readiness.

## Supported integration surfaces

Native integrations use the `sekirei` binary (`crates/sekirei-usi`) over USI.
There is no official C ABI, JNI, Swift binding, or mobile package. A native app
must either:

- cross-compile `sekirei` and drive it as a subprocess, if the target sandbox
  permits that model; or
- build an application-specific wrapper around `sekirei-core`.

Browser or WebView clients can use [`sekirei-wasm`](../crates/sekirei-wasm/README.md)
for stateless SFEN, legal-move, bounded-search, MultiPV, and mate-analysis APIs.
The package is sequential and starts with material evaluation, but a caller can
load one validated SEKIRW or HalfKP evaluator byte image before analysis. It
does not bundle a weight or expose the native engine's multi-threaded search.

## Dependencies

The native engine depends on `sekirei-core` and `rayon`; core also uses
`web-time`. The optional `opening-book` feature adds `lineprior`, `serde`,
`serde_json`, and `sha2`. Inspect the resolved Cargo graph for the target app;
this list is not a transitive-license audit.

## USI options relevant to resource-constrained deployment

| Option | Type | Default | Notes |
|---|---|---|---|
| `Hash` | spin | 64 (MB) | TT size |
| `Threads` | spin | 0 (one effective worker until explicitly set) | With `SearchMode=Auto`, one effective worker uses sequential search and values above one use Lazy SMP. Set this explicitly on a mobile device. |
| `SearchMode` | combo | `Auto` | Selects sequential search for one worker, Lazy SMP for multiple workers, and the existing speculative backend when `MultiPV > 1`. Explicit `Speculative`, `LazySMP`, `Dfpn`, and `SharedMcts` modes remain available. |
| `SpecTopN` | spin | 0 | Sizes the separate speculative-search pool only in explicit `SearchMode=Speculative`. Non-zero values add concurrent compute demand and may vary by schedule. |
| `MultiPV` | spin | 1 | Number of root candidates |
| `EvalFile` | string | (empty) | NNUE path; without one, Sekirei uses material evaluation |
| `MoveOverhead` | spin | 50 (ms) | Standard time-management safety margin |
| `UseBook` / `BookFile` / `BookMaxPly` / `BookMinConfidence` | — | absent in the default build; with `opening-book`: off, `data/opening_book.jsonl`, 30, 0.20 | Opening book is compile-time and runtime opt-in. Package the artifact yourself, record its SHA-256, set its app-bundled path, then enable `UseBook`. |

## Memory

The optional r6 HalfKP file is 64,216,957 bytes (about 61.2 MiB), as recorded
in its [release card](../weights/sekirei-halfkp-r6-v0.5.0.json). The historical
flat file is about 1.24 MB. File size is not peak process memory: include the TT,
search state, per-worker accumulators, and loading buffers in a device budget.
Native weights are installed once per process; changing them needs a restart.
`Hash` controls TT size. Peak RSS, battery use, and thermal behavior have not
been measured on iOS or Android.

## Current limits

- No official native mobile FFI/bindings.
- No on-device production validation for the available optional NNUE weight
  (`docs/nnue_weights.md`). Native USI loads a versioned checkpoint through
  `EvalFile`; WebAssembly accepts caller-supplied bytes through
  `loadEvaluator`. Both surfaces retain material evaluation when no evaluator
  is loaded, and neither bundles a recommended production checkpoint.
- No verified on-device (iOS/Android) memory or battery profiling.
- No published amateur-game analysis benchmark. The format exists in
  [`amateur_analysis_benchmark.md`](amateur_analysis_benchmark.md), but no
  result has been published.

The former mobile benchmark proposal is covered by the
[2026-11-16 plan reset](plans/README.md); no mobile qualification is scheduled.
