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

## Dependency footprint

The default engine binary's direct dependency chain is small: `sekirei-usi`
uses `sekirei-core` and `rayon`; `sekirei-core` uses `rayon`. `lineprior
0.12.6` is included only with the optional `opening-book` feature. This is not
a transitive-license audit; inspect the resolved graph for a shipped app.

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

A weight file is ≈1.24 MB (flat "A" architecture) or ≈10.0 MB
(king-relative "B-small", currently experimental — see
`docs/nnue_weights.md`) on disk, loaded once into a process-global
`OnceLock`. `Hash` (TT size, MB) is the other controllable memory knob.
Neither scales with position complexity or game length. Peak RSS, battery use,
and thermal behavior have not been measured on iOS or Android. The B-small
outcome summarized in
[`design/nnue_architecture_next_candidate.md`](design/nnue_architecture_next_candidate.md)
comes from desktop validation, not on-device inference, and must not be
extrapolated to a phone without direct measurement.

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

## Positioning

Sekirei offers a Pure Rust, MIT OR Apache-2.0 codebase and a small browser
surface. That is an integration property, not a playing-strength claim. See
[`nnue_weights.md`](nnue_weights.md) for the evaluator evidence boundary.
