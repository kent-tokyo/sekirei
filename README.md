# Sekirei — Rust Shogi Engine

[![CI](https://github.com/kent-tokyo/sekirei/actions/workflows/ci.yml/badge.svg)](https://github.com/kent-tokyo/sekirei/actions/workflows/ci.yml)
[![Release](https://img.shields.io/badge/release-v0.3.67-blue)](https://github.com/kent-tokyo/sekirei/releases/tag/v0.3.67)
[![crates.io](https://img.shields.io/crates/v/sekirei.svg)](https://crates.io/crates/sekirei)
[![License](https://img.shields.io/crates/l/sekirei.svg)](https://github.com/kent-tokyo/sekirei/blob/main/LICENSE)

[日本語](README_ja.md)

Sekirei is an experimental Pure Rust shogi engine. Release `0.3.67` strengthens
training-data identity checks, opening-book provenance, gate export, and WASM
package validation. It does not claim a playing-strength improvement; measured
results remain scoped to the exact evaluator, hardware, corpus, and settings.

## Quick start

Install the released USI engine:

```bash
cargo install sekirei
sekirei
```

Or run the current checkout:

```bash
git clone https://github.com/kent-tokyo/sekirei.git
cd sekirei
cargo run --release -p sekirei
```

Register `sekirei` in a USI-compatible GUI. Without `EvalFile`, it uses
material evaluation. The optional 0.3.38 checkpoint remains available for
compatibility and experimentation:

```bash
sekirei /path/to/sekirei-nnue-v0.3.38.bin
```

Set `EvalFile` and `NnueOutput=absolute` before `isready`. External HalfKP
256x2-32-32 files use `FV_SCALE` instead and remain subject to their own
licenses. See [NNUE weights](docs/nnue_weights.md) for formats, checksums, and
the evidence boundary.

### Browser / WebAssembly

[`sekirei-wasm`](crates/sekirei-wasm/README.md) provides SFEN parsing, legal
moves, validated move application, bounded sequential search with a legal PV,
and bounded shortest-mate analysis. It uses material evaluation and does not
change the native USI dependency graph.

The prebuilt v0.3.67 ES-module package is available from the
[GitHub Release](https://github.com/kent-tokyo/sekirei/releases/download/v0.3.67/sekirei-wasm-0.3.67.tgz).

## What is included

- Shogi rules, SFEN/USI notation, alpha-beta/PVS, quiescence, ordering, and a
  lock-free TT.
- Optional speculative search, Lazy SMP, NNUE evaluation/training, MCTS, and
  df-pn. Experimental modes are not strength claims.
- CSA/Floodgate and USI match tooling; core search/evaluation is Pure Rust
  with no `unsafe`.

Workspace binaries:

| Command | Package | Purpose |
|---|---|---|
| `sekirei` | `sekirei` | USI engine |
| `sekirei-csa` | `sekirei-csa` | CSA/Floodgate client |
| `sekirei-match` | `sekirei-match-runner` | USI match runner |
| `train` | `sekirei-train` | NNUE training |

## Engine configuration

The `usi` command lists all options. `SearchMode=Auto` selects sequential
search for one worker and Lazy SMP for multiple workers; `MultiPV>1` uses the
root-candidate backend. For deterministic diagnostics use `Threads=1` and
`SpecTopN=0`.

## Build and verify

```bash
cargo build --release
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
python3 scripts/check_release_metadata.py --allow-planned-release-manifest
```

Tooling and historical benchmark scope are indexed in
[scripts/README.md](scripts/README.md). Component timings are not overall speed
or playing-strength rankings.

## Matches and CSA/Floodgate

Use `sekirei-match` for local USI matches. For unattended offline self-play
with durable records:

```bash
python3 scripts/run_local_selfplay.py --games 1000 \
  --weights /path/to/weights.bin \
  --positions data/gate/openings_standard.sfen
```

Runs save manifests, kifu, CSA, search data, and deduplication metadata under
ignored `data/runs/`. Normal collection requires explicit weights and opening
positions. `sekirei-csa` supports CSA/Floodgate; inject credentials only at
runtime. Same-engine results are relative regression evidence, not Floodgate
or human ratings.

## NNUE training

Run `cargo run --release -p sekirei-train -- --help` for the training CLI.
Training pins `lineprior 0.12.2` and `shogiesa-core 0.11.1`; external wrappers
are checked against `shogiesa 0.11.1`. Generated data and weights stay outside
Git. Recipes, resume rules, hashes, and game-level split requirements are in
[scripts/README.md](scripts/README.md).

Opening-book support is an explicit build and runtime choice:

```bash
cargo build --release -p sekirei --features opening-book
# Then set BookFile to the shipped artifact and UseBook=true.
```

No default opening book is shipped. Book builds write a sidecar manifest with
input, configuration, and output hashes; optional decision logs support held-
out paired experiments. Position training records content SHA-256 identities
and can require expected hashes before starting.

## Documentation

- [Documentation index](DOCUMENTATION.md)
- [Changelog](CHANGELOG.md)
- [NNUE weights and licensing](docs/nnue_weights.md)
- [Script and validation-tool index](scripts/README.md)

Implementation, tests, measurements, and released artifacts are separate
claims. Historical experiment reports preserve their original version and
scope; they are not statements about the current engine unless explicitly
revalidated.

## License and attribution

Source code is [MIT](LICENSE-MIT) OR [Apache-2.0](LICENSE-APACHE); retain
[NOTICE](NOTICE). NNUE files are separate CC BY 4.0 artifacts; see
[NNUE-LICENSE.md](NNUE-LICENSE.md).

Recommended attribution:

```text
This product is based on Sekirei,
an open-source shogi engine developed by Kentaro Tanabe.

https://github.com/kent-tokyo/sekirei
```

This display is recommended, not an additional advertising clause. Do not use
the Sekirei name or logo to imply official endorsement without permission.
