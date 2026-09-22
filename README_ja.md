# Sekirei — Rust製将棋エンジン

[![CI](https://github.com/kent-tokyo/sekirei/actions/workflows/ci.yml/badge.svg)](https://github.com/kent-tokyo/sekirei/actions/workflows/ci.yml)
[![Release](https://img.shields.io/badge/release-v0.3.42-blue)](https://github.com/kent-tokyo/sekirei/releases/tag/v0.3.42)
[![crates.io](https://img.shields.io/crates/v/sekirei.svg)](https://crates.io/crates/sekirei)
[![License](https://img.shields.io/crates/l/sekirei.svg)](https://github.com/kent-tokyo/sekirei/blob/main/LICENSE)

[English](README.md)

SekireiはPure Rustで実装した実験的な将棋エンジンです。リリース`0.3.42`は、
USIエンジン、CSA client、対局runner、NNUE trainer、再利用可能なcore libraryを
含みます。NNUE推論・cost診断、固定validation入力、監査付きpairwise/listwise
順位学習を追加しました。新しいcheckpointは採用しておらず、棋力向上の主張は
行いません。

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
material評価を使います。0.3.38用NNUEは、別管理された任意の公開artifactとして
0.3.42でも引き続き利用でき、
[`weights/sekirei-nnue-v0.3.38.bin`](weights/sekirei-nnue-v0.3.38.bin)です。

```bash
sekirei /path/to/sekirei-nnue-v0.3.38.bin
```

GUIでは`isready`より前に、`EvalFile`へcheckpointの絶対pathを、
`NnueOutput`へ`absolute`を設定します。model cardには0.3.38当時のpaired gateを
履歴として残しています。現行engineでのlocal診断では、material評価に対し
1秒/手で1/32、5秒/手で2/32の得点でした。この少数比較だけで既定評価器は
切り替えませんが、棋力面での一律推奨は保留します。

使用前に[重みartifact card](weights/README.md)でSHA-256とライセンスを検証してください。
NNUE重みはcrateに同梱せず、MIT/Apache-2.0のソースとCC BY 4.0の重みを分離します。

## 主な機能

- 合法手生成、成り、駒打ち、SFEN、USI表記を含む将棋ルール。
- 反復深化alpha-beta/PVS、静止探索、枝刈り、手順序、lock-free TT。
- 任意の投機探索とLazy SMP。決定的診断では`SpecTopN=0`、`Threads=1`を使用。
- 差分更新NNUE評価と再現可能な学習pipeline。
- 上限付きroot MCTSとdf-pn詰み探索API。いずれも実験機能で、棋力主張ではありません。
- CSA v2.2/Floodgate対局とUSI同士のmatch runner。
- coreの探索・評価に`unsafe`を使わないPure Rust実装。

| コマンド | package | 用途 |
|---|---|---|
| `sekirei` | `sekirei` | USIエンジン |
| `sekirei-csa` | `sekirei-csa` | CSA/Floodgate client |
| `sekirei-match` | `sekirei-match-runner` | USI対局runner |
| `train` | `sekirei-train` | NNUE学習 |

## エンジン設定

USIの`usi`コマンドで全optionを表示します。主なoptionは`Hash`、`Threads`、
`MoveOverhead`、`Ponder`、`MultiPV`、`EvalFile`、`NnueResidualScalePermille`、
`SearchMode`、`SpecTopN`、定跡関連です。

既定は`SearchMode=Speculative`で、並列modeは結果が揺れる場合があります。決定論的な
確認では`Threads=1`、`SpecTopN=0`を使います。重みの検証、format、外部SFNNの
限定的な対応範囲は[NNUE重み](docs/nnue_weights.md)に集約しています。

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

Ignoredの`data/runs/`以下へmanifest、棋譜、CSA、各手の探索値、重複情報を保存します。
同一engine自己対局は学習・回帰用であり、Elo測定ではありません。

`sekirei-csa`はCSA/Floodgate対局に対応します。認証情報は実行時に注入し、認証情報、
棋譜、生成重み、学習dataをcommitしないでください。

自己対局Eloは選択した基準に対する相対値で、Floodgateや人間のレートではありません。
統計gateは`PASS`、`FAIL`、`INCONCLUSIVE`を区別します。

## NNUE学習

学習CLIは`cargo run --release -p sekirei-train -- --help`で確認できます。再現可能なrecipe、
診断、resume toolingは[scripts索引](scripts/README.md)に集約し、生成artifactはGit外に
保持します。

## 文書

- [変更履歴](CHANGELOG.md)
- [NNUE重みとlicense](docs/nnue_weights.md)
- [解析benchmark契約](docs/amateur_analysis_benchmark.md)
- [mobile組み込みの現状](docs/mobile_integration.md)
- [script・検証tool索引](scripts/README.md)

実装、test、測定、公開artifactは別の主張です。過去の実験文書は当時のversionと範囲を
保存するもので、明示的に再検証しない限り現行engineの結果ではありません。

## ライセンスと帰属表示

source codeは利用者の選択により[MIT](LICENSE-MIT)または
[Apache-2.0](LICENSE-APACHE)で利用できます。[NOTICE](NOTICE)にあるSekireiと
Kentaro Tanabeの著作権・帰属表示を保持してください。NNUE重みは別成果物として
CC BY 4.0でlicenseします。詳細は[NNUE-LICENSE.md](NNUE-LICENSE.md)を参照してください。

推奨表示：

```text
This product is based on Sekirei,
an open-source shogi engine developed by Kentaro Tanabe.

https://github.com/kent-tokyo/sekirei
```

この表示は推奨であり、追加の広告条項ではありません。許可なくSekireiの名称やlogoを使い、
公式・承認済みであるかのように示してはいけません。
