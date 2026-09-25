# Sekirei — Rust製将棋エンジン

[![CI](https://github.com/kent-tokyo/sekirei/actions/workflows/ci.yml/badge.svg)](https://github.com/kent-tokyo/sekirei/actions/workflows/ci.yml)
[![Release](https://img.shields.io/badge/release-v0.3.48-blue)](https://github.com/kent-tokyo/sekirei/releases/tag/v0.3.48)
[![crates.io](https://img.shields.io/crates/v/sekirei.svg)](https://crates.io/crates/sekirei)
[![License](https://img.shields.io/crates/l/sekirei.svg)](https://github.com/kent-tokyo/sekirei/blob/main/LICENSE)

[English](README.md)

SekireiはPure Rustで実装した実験的な将棋エンジンです。リリース`0.3.48`は、
USIエンジン、CSA client、対局runner、NNUE trainer、再利用可能なcore libraryを
含みます。node・ply・期限を制限できる王手のみのdf-pn詰み探索APIとprobe例を追加します。
通常のengine探索はまだこのAPIを呼びません。重みの同梱・採用は行わず、local診断を正式な
棋力向上の主張としては扱いません。

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

生成された`sekirei`をUSI対応GUIのエンジンとして登録します。checkpointなしでは
material評価を使います。`EvalFile`はSekirei重みのほか、対応する外部HalfKP
256x2-32-32の`nn.bin`を読み込めます。外部fileのlicenseは各fileに従います。
0.3.38用NNUEは、別管理された任意の公開artifactとして引き続き利用でき、
[`weights/sekirei-nnue-v0.3.38.bin`](weights/sekirei-nnue-v0.3.38.bin)です。

```bash
sekirei /path/to/sekirei-nnue-v0.3.38.bin
```

GUIでは`isready`より前に`EvalFile`と`NnueOutput=absolute`を設定します。
外部HalfKP fileでは`NnueOutput`ではなく`FV_SCALE`（既定16、重みの説明を優先）を
設定します。
使用前に[重みartifact card](weights/README.md)でSHA-256とライセンスを検証してください。
0.3.38のgateは同cardに履歴として残しますが、現行local診断のB対materialは
1秒/手で1/32、5秒/手で2/32であり、一律の棋力推奨はしません。NNUE重みは
CC BY 4.0の別artifactです。

## 主な機能

- 将棋ルール、SFEN/USI表記、alpha-beta/PVS、静止探索、手順序、lock-free TT。
- 任意の投機探索、Lazy SMP、NNUE学習、MCTS、df-pn。実験modeは棋力主張ではありません。
- CSA/Floodgate・USI対局tool。coreの探索・評価に`unsafe`を使わないPure Rust実装。

| コマンド | package | 用途 |
|---|---|---|
| `sekirei` | `sekirei` | USIエンジン |
| `sekirei-csa` | `sekirei-csa` | CSA/Floodgate client |
| `sekirei-match` | `sekirei-match-runner` | USI対局runner |
| `train` | `sekirei-train` | NNUE学習 |

## エンジン設定

USIの`usi`で全optionを表示します。決定論的な診断は`Threads=1`、`SpecTopN=0`で
実行します。投機並列modeはscheduleにより揺れる場合があります。重みの検証、format、
外部SFNNの範囲は[NNUE重み](docs/nnue_weights.md)を参照してください。

`EvalFile`は、一般的な`HalfKP 256x2-32-32`形式（`nn.bin`）の外部評価関数も読み込めます。
形式はfile headerから自動判別し、出力の除数は`FV_SCALE`（既定16）で設定します。
Sekireiはこの種のfileを同梱しません。各自で入手し、そのfileのlicenseに従ってください。
読み込み部は独自実装で、他engineのsource codeに由来しません。

## Buildと検証

```bash
cargo build --release
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
python3 scripts/check_release_metadata.py --allow-planned-release-manifest
```

Benchmark・競合比較のcommand、固定入力、過去結果の範囲は
[script索引](scripts/README.md)に集約しています。Component単位の時間は、総合速度や
棋力順位を示しません。

## 対局・CSA/Floodgate

ローカルUSI対局には`sekirei-match`を使います。外部サービスなしで対局記録を
継続収集する場合：

```bash
python3 scripts/run_local_selfplay.py --games 1000 \
  --weights /path/to/weights.bin \
  --positions data/gate/openings_standard.sfen
```

Ignoredの`data/runs/`以下へmanifest、棋譜、CSA、探索値、重複情報を保存します。
通常収集には`--weights`と`--positions`が必要です。同一engine自己対局は学習・回帰用で、
Elo測定ではありません。

`sekirei-csa`はCSA/Floodgate対局に対応します。認証情報は実行時に注入し、認証情報、
棋譜、生成重み、学習dataをcommitしないでください。

自己対局Eloは選択した基準に対する相対値で、Floodgateや人間のレートではありません。
統計gateは`PASS`、`FAIL`、`INCONCLUSIVE`を区別します。

## NNUE学習

学習CLIは`cargo run --release -p sekirei-train -- --help`で確認できます。このcheckoutは
`lineprior 0.12.0`を固定し、外部data scriptは`shogiesa 0.10.0`で確認しています。
recipeとresume toolingは[scripts索引](scripts/README.md)を参照し、生成artifactはGit外に保持します。

## 文書

- [変更履歴](CHANGELOG.md)
- [NNUE重みとlicense](docs/nnue_weights.md)
- [解析benchmark契約](docs/amateur_analysis_benchmark.md)
- [mobile組み込みの現状](docs/mobile_integration.md)
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
