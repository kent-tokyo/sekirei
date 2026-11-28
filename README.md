# Sekirei — Rust Shogi Engine

[![CI](https://github.com/kent-tokyo/sekirei/actions/workflows/ci.yml/badge.svg)](https://github.com/kent-tokyo/sekirei/actions/workflows/ci.yml)
[![codecov](https://codecov.io/gh/kent-tokyo/sekirei/branch/main/graph/badge.svg?flag=rust-engine)](https://codecov.io/gh/kent-tokyo/sekirei)
[![Ask DeepWiki](docs/assets/ask-deepwiki.svg)](https://deepwiki.com/kent-tokyo/sekirei)
[![Release](https://img.shields.io/badge/release-v0.5.5-blue)](https://github.com/kent-tokyo/sekirei/releases/tag/v0.5.5)
[![crates.io](https://img.shields.io/crates/v/sekirei.svg)](https://crates.io/crates/sekirei)
[![License](https://img.shields.io/crates/l/sekirei.svg)](LICENSE)

[日本語](README_ja.md)

Sekirei is an experimental Pure Rust shogi engine. Version `0.5.5`
provides a native USI engine, CSA client, match runner, and browser API.
It uses material evaluation by default; NNUE requires an explicit weight file.

## Run the engine

```bash
cargo install sekirei
sekirei
```

Register the executable in a USI-compatible GUI. To run this checkout:

```bash
cargo run --release -p sekirei
```

Send `usi` to list options. For one-worker diagnostics, set `Threads=1`
and `SpecTopN=0`. With `MultiPV=1`, `SearchMode=Auto` uses sequential
search for one worker and Lazy SMP for multiple workers.

To use the optional `nn_r6` weight, set `EvalFile` to its path,
`NnueOutput=absolute`, and `FV_SCALE=24`.
See [weight setup and evidence](docs/nnue_weights.md) for downloads,
checksums, supported formats, and the scope of its qualification.

## What is available

| Interface | Purpose |
|---|---|
| `sekirei` | USI engine: alpha-beta/PVS, quiescence search, lock-free TT |
| `sekirei-csa` | CSA/Floodgate client |
| `sekirei-match` | Local matches between USI engines |
| `sekirei-train` package | NNUE training; run with `cargo run --release -p sekirei-train -- --help` |
| [sekirei-wasm](crates/sekirei-wasm/README.md) | SFEN, legal moves, bounded single-worker analysis, MultiPV, mate analysis |

Install the browser package with `npm install sekirei-wasm@0.5.5`.
It accepts caller-supplied evaluator bytes and bundles no weights.
Package identity is recorded in the [distribution guide](docs/releases/npm_wasm.md).

Opening books require the `opening-book` Cargo feature, `BookFile`, and
`UseBook=true`; no default book is shipped. The optional 27-point entering-king
declaration works at the root. Its rules-aware reference search is not
connected to ordinary USI or parallel search. Experimental search modes and
local diagnostics do not establish playing-strength gains.

## Build and check

```bash
cargo build --release
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

See [development checks](docs/development.md) for focused tests, coverage,
and release metadata checks. Use the [script index](scripts/README.md) for
matches, training, and operational tools.

## Project status and documentation

Current priority (2026-11-27): make strength measurement cheap and routine
(parallel match games and a fixed YaneuraOu gauge), then train a larger NNUE.
Every change that may affect strength is judged by the
[standard strength test](docs/strength_testing.md). The working rules are in
[AGENTS.md](AGENTS.md); earlier plans are summarized in the
[plan review](docs/plans/README.md).

- [Documentation index](DOCUMENTATION.md)
- [Benchmarks and evidence](docs/benchmarks.md)
- [Changelog](CHANGELOG.md)

## License

Source: [MIT](LICENSE-MIT) OR [Apache-2.0](LICENSE-APACHE); retain [NOTICE](NOTICE).
Project-distributed NNUE artifacts have separate terms in
[NNUE-LICENSE.md](NNUE-LICENSE.md). External weights retain their own licenses.
