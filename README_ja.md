# Sekirei — Rust製将棋エンジン

[![CI](https://github.com/kent-tokyo/sekirei/actions/workflows/ci.yml/badge.svg)](https://github.com/kent-tokyo/sekirei/actions/workflows/ci.yml)
[![codecov](https://codecov.io/gh/kent-tokyo/sekirei/branch/main/graph/badge.svg?flag=rust-engine)](https://codecov.io/gh/kent-tokyo/sekirei)
[![Ask DeepWiki](https://deepwiki.com/badge.svg)](https://deepwiki.com/kent-tokyo/sekirei)
[![Release](https://img.shields.io/badge/release-v0.3.67-blue)](https://github.com/kent-tokyo/sekirei/releases/tag/v0.3.67)
[![crates.io](https://img.shields.io/crates/v/sekirei.svg)](https://crates.io/crates/sekirei)
[![License](https://img.shields.io/crates/l/sekirei.svg)](https://github.com/kent-tokyo/sekirei/blob/main/LICENSE)

[English](README.md)

SekireiはPure Rustで実装した実験的な将棋エンジンです。リリース`0.3.67`では、
学習データの同一性検査、定跡manifest、gate結果のexport、WASM package検証を
強化しました。棋力向上は主張していません。測定結果は、使用した評価関数、
ハードウェア、局面集合、探索条件の範囲で扱います。

## まず動かす

公開版をインストールする場合：

```bash
cargo install sekirei
sekirei
```

最新checkoutを実行する場合：

```bash
git clone https://github.com/kent-tokyo/sekirei.git
cd sekirei
cargo run --release -p sekirei
```

生成された`sekirei`をUSI対応GUIへ登録します。`EvalFile`を指定しない場合は駒得評価を
使います。0.3.38用NNUEは互換性確認と実験用の任意artifactとして残しています。

```bash
sekirei /path/to/sekirei-nnue-v0.3.38.bin
```

GUIでは`isready`より前に`EvalFile`と`NnueOutput=absolute`を設定します。外部HalfKP
256x2-32-32では`FV_SCALE`を使い、そのファイルのライセンスに従ってください。形式、
SHA-256、測定範囲は[NNUE重み](docs/nnue_weights.md)にまとめています。

## 主な機能

- 将棋ルール、SFEN/USI表記、alpha-beta/PVS、静止探索、手順序、lock-free TT。
- 任意の投機探索、Lazy SMP、NNUE学習、MCTS、df-pn。実験機能は棋力主張ではありません。
- CSA/Floodgate・USI対局ツール。coreの探索・評価に`unsafe`を使いません。

| コマンド | package | 用途 |
|---|---|---|
| `sekirei` | `sekirei` | USIエンジン |
| `sekirei-csa` | `sekirei-csa` | CSA/Floodgate client |
| `sekirei-match` | `sekirei-match-runner` | USI対局runner |
| `train` | `sekirei-train` | NNUE学習 |

## ブラウザ / WebAssembly

[`sekirei-wasm`](crates/sekirei-wasm/README.md)は、SFEN解析、合法手、検証付き着手、
合法なPVを返す上限付き逐次探索、最短詰み解析を提供します。駒得評価だけを使い、
ネイティブUSIバイナリの依存関係は変えません。

ビルド済みのv0.3.67 ES module packageは
[GitHub Release](https://github.com/kent-tokyo/sekirei/releases/download/v0.3.67/sekirei-wasm-0.3.67.tgz)
から取得できます。

## エンジン設定

USIの`usi`で全optionを表示します。`SearchMode=Auto`は1 workerで逐次探索、複数workerで
Lazy SMP、`MultiPV>1`ではroot候補backendを使います。決定論的な診断は`Threads=1`、
`SpecTopN=0`で実行します。

## Buildと検証

```bash
cargo build --release
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
bash scripts/check_rust_coverage.sh
python3 scripts/check_release_metadata.py --allow-planned-release-manifest
```

カバレッジ確認には`cargo-llvm-cov 0.8.7`、`llvm-tools-preview`、`jq`が必要です。
再利用可能なRustコードと配布するUSI実行系の行カバレッジを90%以上に固定します。
任意の定跡・探索調整機能も対象です。ベンチマーク・診断用バイナリと、対局や学習を
編成するCLI入口は通常CIで検証しますが、このバッジの集計対象からは分離します。

測定ツールと過去結果の範囲は[script索引](scripts/README.md)に集約しています。
component単位の時間は総合速度や棋力順位を示しません。

## 対局・CSA/Floodgate

ローカルUSI対局には`sekirei-match`を使います。外部サービスなしで対局記録を
継続収集する場合：

```bash
python3 scripts/run_local_selfplay.py --games 1000 \
  --weights /path/to/weights.bin \
  --positions data/gate/openings_standard.sfen
```

`data/runs/`以下へmanifest、棋譜、CSA、探索値、重複情報を保存します。通常収集では重みと
開始局面を明示します。`sekirei-csa`はCSA/Floodgate対局に対応します。認証情報は実行時だけ
渡してください。同一engine自己対局は学習・回帰用で、Floodgateや人間のレートではありません。

## NNUE学習

学習CLIは`cargo run --release -p sekirei-train -- --help`で確認できます。学習側は
`lineprior 0.12.2`と`shogiesa-core 0.11.1`を固定し、外部wrapperは`shogiesa 0.11.1`で
確認しています。生成dataと重みはGit外に保存します。recipe、resume、hash、対局単位の
分割条件は[scripts索引](scripts/README.md)を参照してください。

opening bookは既定では無効で、artifactも同梱しません。book生成時は入力・設定・出力hashを
記録したsidecar manifestを作ります。position学習では入力dataのSHA-256を記録し、開始前に
期待値との一致を必須にできます。

## 文書

- [文書索引](DOCUMENTATION.md)
- [変更履歴](CHANGELOG.md)
- [NNUE重みとlicense](docs/nnue_weights.md)
- [script・検証tool索引](scripts/README.md)

実装、test、測定、公開artifactは別の主張です。過去の実験文書は当時のversionと範囲を
保存するもので、明示的に再検証しない限り現行engineの結果ではありません。

## ライセンスと帰属表示

source codeは[MIT](LICENSE-MIT) OR [Apache-2.0](LICENSE-APACHE)です。
[NOTICE](NOTICE)の帰属表示を保持してください。NNUE重みはCC BY 4.0の別成果物です。
詳細は[NNUE-LICENSE.md](NNUE-LICENSE.md)を参照してください。

推奨表示：

```text
This product is based on Sekirei,
an open-source shogi engine developed by Kentaro Tanabe.

https://github.com/kent-tokyo/sekirei
```

この表示は推奨であり、追加の広告条項ではありません。許可なくSekireiの名称やlogoを使い、
公式・承認済みであるかのように示してはいけません。
