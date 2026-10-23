# Sekirei — Rust Shogi Engine

[![CI](https://github.com/kent-tokyo/sekirei/actions/workflows/ci.yml/badge.svg)](https://github.com/kent-tokyo/sekirei/actions/workflows/ci.yml)
[![codecov](https://codecov.io/gh/kent-tokyo/sekirei/branch/main/graph/badge.svg?flag=rust-engine)](https://codecov.io/gh/kent-tokyo/sekirei)
[![Ask DeepWiki](docs/assets/ask-deepwiki.svg)](https://deepwiki.com/kent-tokyo/sekirei)
[![Release](https://img.shields.io/badge/release-v0.5.1-blue)](https://github.com/kent-tokyo/sekirei/releases/tag/v0.5.1)
[![crates.io](https://img.shields.io/crates/v/sekirei.svg)](https://crates.io/crates/sekirei)
[![License](https://img.shields.io/crates/l/sekirei.svg)](https://github.com/kent-tokyo/sekirei/blob/main/LICENSE)

[日本語](README_ja.md)

Sekirei is an experimental Pure Rust shogi engine. Release `0.5.1` exposes
WASM package provenance before search and adds a checked npm publishing path.
The default search policy and material evaluator are unchanged. The optional
`nn_r6` HalfKP artifact remains available in the v0.5.0 GitHub Release; it is
not selected by default or a general playing-strength claim.

## Quick start

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

Register `sekirei` in a USI-compatible GUI. Without `EvalFile`, the engine
uses material evaluation. External HalfKP 256x2-32-32 files use `FV_SCALE`
and remain subject to their own licenses.
The CSA client accepts `--fv-scale <n>` from 1 to 128 (default 16). Pass
`--fv-scale 24` alongside `--eval nnue --weights` for the optional `nn_r6`
artifact. Its run manifest records the active divisor.
[NNUE weights](docs/nnue_weights.md) records supported formats, checksums, and
the evidence boundary. `nn_r6` passed independent colour-reversed material
gates at 1T and 4T on a pinned source commit, and its released evidence bundle
passes the strict artifact validator. It is not selected by default; those
gates are neither an external rating nor a general current-release strength
claim.

## Components

- Shogi rules, SFEN/USI notation, alpha-beta/PVS, quiescence search, move
  ordering, and a lock-free transposition table.
- Optional speculative search, Lazy SMP, NNUE training/evaluation, MCTS, and
  df-pn. Experimental modes are not strength claims.
- CSA/Floodgate and local USI match tooling. Core search and evaluation use no
  `unsafe` code.

| Command | Package | Purpose |
|---|---|---|
| `sekirei` | `sekirei` | USI engine |
| `sekirei-csa` | `sekirei-csa` | CSA/Floodgate client |
| `sekirei-match` | `sekirei-match-runner` | USI match runner |
| `train` | `sekirei-train` | NNUE training |

The `usi` command lists every engine option. `SearchMode=Auto` selects
sequential search for one worker and Lazy SMP for multiple workers. For
deterministic diagnostics, use `Threads=1` and `SpecTopN=0`.

### Browser / WebAssembly

[`sekirei-wasm`](crates/sekirei-wasm/README.md) provides SFEN parsing, legal
moves, validated move application, bounded one-worker search with an optional
caller-supplied evaluator, up to four MultiPV lines, and bounded shortest-mate
analysis. The prebuilt v0.5.1 ES-module
package is attached to the
[GitHub Release](https://github.com/kent-tokyo/sekirei/releases/download/v0.5.1/sekirei-wasm-0.5.1.tgz).

## Build and verify

```bash
cargo build --release
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
bash scripts/check_rust_coverage.sh
python3 scripts/check_release_metadata.py --allow-planned-release-manifest
python3 scripts/check_release_manifest_layout.py
```

The current release manifest stays in the repository root; earlier verified
evidence is kept in [the manifest archive](docs/releases/manifests/README.md).

The coverage command requires `cargo-llvm-cov 0.8.7`,
`llvm-tools-preview`, and `jq`. It enforces at least 98% line coverage for
reusable Rust code and the shipped USI runtime. CI runs this gate for
Rust/Cargo changes, every main update, and release tags; documentation-only
and ordinary script-only pull requests skip it. See the
[script index](scripts/README.md) for benchmark, gate, release, and training
tools. Changes to the published `sekirei-core` API are checked against
v0.3.73 before merge and again before publication; the 0.4 version bump does
not by itself authorize a compatibility break. Component timings are not
overall speed or strength rankings.

Dedicated PGO binaries exceeded the pinned YaneuraOu build on Apple M4:
**8T +4.65%** (95% interval: +2.55% to +6.69%) and **4T +7.1%**
(+5.1% to +9.0%). The [8T study](docs/experiments/compiler_pgo_8t_v0.4.2/README.md)
and [4T study](docs/experiments/compiler_pgo_4t_v0.4.1/README.md) independently
qualify their pinned binaries and workloads. Ordinary Cargo builds do not use
these profiles.
A separate [v0.4.4 own-HalfKP PGO study](docs/experiments/own_halfkp_pgo_8t_v0.4.4/README.md)
measured **8T +3.09%** (+0.76% to +5.40%) for its pinned artifact.

## Matches and training

Use `sekirei-match` for local USI matches. Durable offline self-play requires
explicit weights and opening positions:

```bash
python3 scripts/run_local_selfplay.py --games 1000 \
  --weights /path/to/weights.bin \
  --positions data/gate/openings_standard.sfen
```

Runs store manifests, records, CSA, search data, and deduplication metadata
under ignored `data/runs/`. Same-engine results are regression evidence, not
Floodgate or human ratings. `sekirei-csa` accepts credentials only at runtime.
For a match using `--json result.json`, failed engine responses are recorded in
`result.failures.jsonl` with elapsed time, response budget, engine arm and error.

For NNUE training, run `cargo run --release -p sekirei-train -- --help`.
Training pins `lineprior 0.12.6` and `shogiesa-core 0.11.6`; external wrappers
are checked against `shogiesa 0.11.6`. Data and weights remain outside Git.
Recipes, hashes, resume rules, and whole-game split requirements are in the
[script index](scripts/README.md).

Opening-book support is build-time and runtime opt-in:

```bash
cargo build --release -p sekirei --features opening-book
# Then set BookFile and UseBook=true.
```

No default book is shipped. Book and training workflows retain hashed
manifests so inputs, settings, and outputs can be audited.

## Documentation

- [Documentation index](DOCUMENTATION.md)
- [Changelog](CHANGELOG.md) and [archived history](CHANGELOG_ARCHIVE.md)
- [NNUE weights and licensing](docs/nnue_weights.md)
- [Script and validation-tool index](scripts/README.md)

Implementation, tests, measurements, and released artifacts are separate
claims. Historical reports keep their original version and scope unless they
are explicitly revalidated.

## License and attribution

Source code is [MIT](LICENSE-MIT) OR [Apache-2.0](LICENSE-APACHE); retain
[NOTICE](NOTICE). NNUE files are separate CC BY 4.0 artifacts; see
[NNUE-LICENSE.md](NNUE-LICENSE.md). The attribution text in `NOTICE` is
recommended when redistributing Sekirei, but it is not an additional
advertising clause. Do not imply official endorsement without permission.
