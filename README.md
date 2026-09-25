# Sekirei — Rust Shogi Engine

[![CI](https://github.com/kent-tokyo/sekirei/actions/workflows/ci.yml/badge.svg)](https://github.com/kent-tokyo/sekirei/actions/workflows/ci.yml)
[![Release](https://img.shields.io/badge/release-v0.3.48-blue)](https://github.com/kent-tokyo/sekirei/releases/tag/v0.3.48)
[![crates.io](https://img.shields.io/crates/v/sekirei.svg)](https://crates.io/crates/sekirei)
[![License](https://img.shields.io/crates/l/sekirei.svg)](https://github.com/kent-tokyo/sekirei/blob/main/LICENSE)

[日本語](README_ja.md)

Sekirei is an experimental shogi engine written in pure Rust. Release `0.3.48`
provides a USI engine, CSA client, match runner, NNUE trainer, and reusable core
library. It adds a bounded check-only df-pn mate-solver API and an example
probe; normal engine search does not call it yet. No network is bundled or
adopted; local diagnostics are not a formal playing-strength claim.

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

Register the resulting `sekirei` executable in a USI-compatible shogi GUI.
Without a checkpoint the engine uses material evaluation. `EvalFile` accepts
Sekirei weights and supported external HalfKP 256x2-32-32 `nn.bin` files; an
external file remains subject to its own license. The 0.3.38
checkpoint remains available as a separately versioned optional artifact at
[`weights/sekirei-nnue-v0.3.38.bin`](weights/sekirei-nnue-v0.3.38.bin):

```bash
sekirei /path/to/sekirei-nnue-v0.3.38.bin
```

For a GUI, set `EvalFile` and `NnueOutput=absolute` before `isready`.
For an external HalfKP file, use `FV_SCALE` (default 16; consult the network's
documentation) instead of `NnueOutput`.
Verify the SHA-256 and license in [the weight artifact card](weights/README.md).
The historical gate is recorded there; a current local B-vs-material diagnostic
(1/32 at 1 second and 2/32 at 5 seconds) does not support a blanket strength
recommendation. NNUE weights remain separate CC BY 4.0 artifacts.

## What is included

- Shogi rules, SFEN/USI notation, alpha-beta/PVS, quiescence, ordering, and a
  lock-free TT.
- Optional speculative search, Lazy SMP, NNUE evaluation/training, MCTS, and
  df-pn. Experimental modes are capabilities, not strength claims.
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

The `usi` command lists all options. For deterministic diagnostics use
`Threads=1` and `SpecTopN=0`; speculative parallel modes may vary by schedule.
Weight validation, format, and the external-SFNN boundary are in
[NNUE weights](docs/nnue_weights.md).

`EvalFile` also accepts external evaluation files in the common
`HalfKP 256x2-32-32` `nn.bin` format. The format is detected from the file
header, and `FV_SCALE` (default 16) sets the output divisor. Sekirei does not
bundle any such file: obtain it yourself and follow that file's license. The
reader is an independent implementation, not derived from another engine's
source.

## Build and verify

```bash
cargo build --release
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
python3 scripts/check_release_metadata.py --allow-planned-release-manifest
```

Benchmark and cross-library commands, their pinned inputs, and historical
scope are indexed in [scripts/README.md](scripts/README.md). A component timing
is not an overall speed or playing-strength ranking.

## Matches and CSA/Floodgate

Use `sekirei-match` for local USI matches. For unattended offline self-play
with durable records:

```bash
python3 scripts/run_local_selfplay.py --games 1000 \
  --weights /path/to/weights.bin \
  --positions data/gate/openings_standard.sfen
```

Runs save a manifest, kifu, CSA, search data, and deduplication metadata under
ignored `data/runs/`. Normal collection requires `--weights` and `--positions`.
Same-engine self-play is training/regression data, not an Elo claim.

`sekirei-csa` provides CSA/Floodgate play. Inject credentials at runtime and
never commit credentials, game records, generated weights, or training data.

Self-play Elo is relative to the selected baseline and is not a Floodgate or
human rating. Statistical gates distinguish `PASS`, `FAIL`, and
`INCONCLUSIVE`.

## NNUE training

Run `cargo run --release -p sekirei-train -- --help` for the training CLI.
This checkout pins `lineprior 0.12.0`; its external data scripts are verified
with `shogiesa 0.10.0`. Recipes and resume tooling are indexed in
[scripts/README.md](scripts/README.md); generated artifacts stay outside Git.

## Documentation

- [Changelog](CHANGELOG.md)
- [NNUE weights and licensing](docs/nnue_weights.md)
- [Analysis benchmark contract](docs/amateur_analysis_benchmark.md)
- [Mobile integration status](docs/mobile_integration.md)
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
