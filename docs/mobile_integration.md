# Mobile / on-device integration — current state of affairs

This document states facts about what exists today. It does not claim
mobile-readiness Sekirei doesn't yet have — see the "Not yet true" section
before building on any assumption here.

## The stable integration surface today: the USI binary

The only currently-supported integration point is the `sekirei` binary
(`crates/sekirei-usi`) speaking [USI](https://en.wikipedia.org/wiki/USI_(protocol))
(Universal Shogi Interface) over stdin/stdout — the same protocol shogi
GUIs use to drive engines. There is **no official iOS/Android FFI layer,
no C ABI, no JNI/Swift bindings** anywhere in this repository (confirmed:
no `ffi`/`bindings`/`jni`/`swift` directory exists in the tree). Embedding
`sekirei` in a mobile app today means either:

- Cross-compiling the `sekirei` binary for the target platform and driving
  it as a subprocess over stdin/stdout (works, but "subprocess" is an
  unusual shape for a mobile app sandbox — verify this fits your platform's
  constraints before committing to it), or
- Writing your own FFI layer around `sekirei-core`'s Rust API directly
  (`Board`, `Searcher`/`SpeculativeSearcher`, `nnue::load_weights`) —
  possible in principle since it's a normal Rust library crate, but you
  would be the first to do this; there's no existing example, wrapper, or
  tested integration pattern for it in this repo.

## Dependency footprint (the part relevant to embedding)

The default engine binary's direct dependency chain is small: `sekirei-usi`
uses `sekirei-core` and `rayon`; `sekirei-core` uses `rayon`. `lineprior
0.12.1` is included only when building `sekirei` with the optional
`opening-book` feature.
Neither is a GUI or network stack. This is not a transitive-license audit:
verify the resolved graph and each license before making a license claim.

## USI options relevant to resource-constrained deployment

| Option | Type | Default | Notes |
|---|---|---|---|
| `Hash` | spin | 64 (MB) | TT size |
| `Threads` | spin | 0 (one effective worker until explicitly set) | With `SearchMode=Auto`, one effective worker uses sequential search and values above one use Lazy SMP. Set this explicitly on a mobile device. |
| `SearchMode` | combo | `Auto` | Selects sequential search for one worker, Lazy SMP for multiple workers, and the existing speculative backend when `MultiPV > 1`. Explicit `Speculative`, `LazySMP`, `Dfpn`, and `SharedMcts` modes remain available. |
| `SpecTopN` | spin | 0 | Sizes the separate speculative-search pool only in explicit `SearchMode=Speculative`. Non-zero values add concurrent compute demand and may vary by schedule. |
| `MultiPV` | spin | 1 | For "best/second-best move" display (per issue #44's own stated use case) |
| `EvalFile` | string | (empty) | Path to a trained NNUE weight file — see `docs/nnue_weights.md`. **Without one set, evaluation is a real material-count fallback, not NNUE** (`crates/sekirei-core/src/eval.rs`) — this is functionally correct shogi, but not what an "evaluation graph and blunder detection" feature needs. |
| `MoveOverhead` | spin | 50 (ms) | Standard time-management safety margin |
| `UseBook` / `BookFile` / `BookMaxPly` / `BookMinConfidence` | — | absent in the default build; with `opening-book`: off, `data/opening_book.jsonl`, 30, 0.20 | Opening book is compile-time and runtime opt-in. Package the artifact yourself, record its SHA-256, set its app-bundled path, then enable `UseBook`. |

## Memory footprint for fully-on-device analysis

A weight file is ≈1.24 MB (flat "A" architecture) or ≈10.0 MB
(king-relative "B-small", currently experimental — see
`docs/nnue_weights.md`) on disk, loaded once into a process-global
`OnceLock`. `Hash` (TT size, MB) is the other controllable memory knob.
Neither scales with position complexity or game length. No measurement of
actual peak RSS under a real mobile OS/sandbox has been done as part of this
repo's own testing. The B-small outcome summarized in
[`design/nnue_architecture_next_candidate.md`](design/nnue_architecture_next_candidate.md)
comes from desktop validation, not on-device inference, and must not be
extrapolated to a phone without direct measurement.

## What this does NOT have, as of this writing

- No official mobile FFI/bindings (see above).
- No on-device production validation for the available optional NNUE weight
  (`docs/nnue_weights.md`). The versioned checkpoint can be supplied through
  `EvalFile`; material evaluation remains the fallback when it is omitted or
  cannot be loaded.
- No verified on-device (iOS/Android) memory or battery profiling.
- No published benchmark against amateur-game analysis quality specifically
  (this project's own strength-gating work has focused on engine-vs-engine
  Elo, not analysis-mode metrics like blunder-detection recall).

## Where this differs from established alternatives

Engines in the YaneuraOu/Suisho lineage are strong and well-established,
but typically come with a heavier native-toolchain/licensing footprint for
embedded use than a small, permissively-licensed (MIT/Apache-2.0), pure-Rust
crate offers by construction. That's a structural, license-and-toolchain
difference, not a strength claim — Sekirei does not currently claim to
match those engines' playing strength (see `docs/nnue_weights.md` for the
local-gate-only checkpoint claim boundary). If your priority is integration
simplicity and license clarity over maximum engine-vs-engine Elo, that's
the actual, honest tradeoff this project currently offers.
