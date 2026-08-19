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

The engine binary's own dependency chain is intentionally small:
`sekirei-usi` depends only on `sekirei-core`, `rayon`, and `lineprior`
(this project's own opening-book/related crate); `sekirei-core` depends
only on `rayon`. Neither pulls in a GUI toolkit, network stack, or
anything license-heavy. This repo does not independently audit or assert
the license of `rayon`/`lineprior` themselves here — verify directly
(`cargo tree`, or each crate's own published license) before relying on a
"no GPL anywhere in the dependency graph" claim; what's asserted here is
only that the *direct* dependency list is short and inspectable, not a
transitive-license audit result.

## USI options relevant to resource-constrained deployment

| Option | Type | Default | Notes |
|---|---|---|---|
| `Hash` | spin | 64 (MB) | TT size |
| `Threads` | spin | 0 (unset → rayon's own default, `num_cpus`) | Sizes the global search thread pool. **If never explicitly set, silently uses `num_cpus` threads** — set this explicitly on a mobile device rather than relying on the default. |
| `SpecTopN` | spin | 3 | Sizes a *separate*, dedicated speculative-search thread pool. **Real concurrent compute-thread demand is `Threads + SpecTopN`, not `Threads` alone** (`docs/design/pr5_pool_isolation_static_audit.md`) — account for this in capacity planning, especially on a phone/tablet's more limited core count. Setting `SpecTopN` to `0` disables speculative search entirely if you need the lowest possible thread footprint. |
| `MultiPV` | spin | 1 | For "best/second-best move" display (per issue #44's own stated use case) |
| `EvalFile` | string | (empty) | Path to a trained NNUE weight file — see `docs/nnue_weights.md`. **Without one set, evaluation is a real material-count fallback, not NNUE** (`crates/sekirei-core/src/eval.rs`) — this is functionally correct shogi, but not what an "evaluation graph and blunder detection" feature needs. |
| `MoveOverhead` | spin | 50 (ms) | Standard time-management safety margin |
| `UseBook` / `BookFile` / `BookMaxPly` / `BookMinConfidence` | — | book on, `data/opening_book.jsonl`, 30, 0.20 | Opening book — the default `BookFile` path won't exist in a mobile app bundle; set an app-bundled path or `UseBook=false` |

## Memory footprint for fully-on-device analysis

A weight file is ≈1.24 MB (flat "A" architecture) or ≈10.0 MB
(king-relative "B-small", currently experimental — see
`docs/nnue_weights.md`) on disk, loaded once into a process-global
`OnceLock`. `Hash` (TT size, MB) is the other controllable memory knob.
Neither scales with position complexity or game length. No measurement of
actual peak RSS under a real mobile OS/sandbox has been done as part of
this repo's own testing — the smoke-test figures that exist
(`docs/experiments/king_relative_b_small_phase3_diagnostic.md`) are from a
desktop training run, not an on-device inference measurement, and
shouldn't be extrapolated to a phone without your own verification.

## What this does NOT have, as of this writing

- No official mobile FFI/bindings (see above).
- No production-recommended trained NNUE weight file (`docs/nnue_weights.md`)
  — material-fallback-only until you supply your own `EvalFile`.
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
match those engines' playing strength (see `docs/nnue_weights.md`'s
material-fallback-until-validated status). If your priority is integration
simplicity and license clarity over maximum engine-vs-engine Elo, that's
the actual, honest tradeoff this project currently offers.
