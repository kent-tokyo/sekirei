# Sekirei — Rust製将棋エンジン

[![CI](https://github.com/kent-tokyo/sekirei/actions/workflows/ci.yml/badge.svg)](https://github.com/kent-tokyo/sekirei/actions/workflows/ci.yml)
[![Release](https://img.shields.io/badge/release-v0.3.37-blue)](https://github.com/kent-tokyo/sekirei/releases/tag/v0.3.37)
[![crates.io](https://img.shields.io/crates/v/sekirei.svg)](https://crates.io/crates/sekirei)
[![License](https://img.shields.io/crates/l/sekirei.svg)](https://github.com/kent-tokyo/sekirei/blob/main/LICENSE)

[English](README.md)

SekireiはPure Rustで実装した実験的な将棋エンジンです。リリース`0.3.37`は、
USIエンジン、CSA/Floodgateクライアント、対局runner、NNUE学習ツール、再利用可能な
core libraryを含みます。棋力は開発中であり、ローカル診断や自己対局の結果を絶対レート
としては扱いません。

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
material評価を使います。NNUEを使う場合は重みを指定します。

```bash
sekirei /path/to/weights.bin
```

NNUE重みはcrateに同梱していません。

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
`MoveOverhead`、`Ponder`、`MultiPV`、`EvalFile`、`SearchMode`、`SpecTopN`、
定跡関連です。

既定は`SearchMode=Speculative`です。`SearchMode=LazySMP`では各workerが盤面と
heuristicを個別に持ち、TTと停止flagを共有します。並列探索はscheduleにより同条件でも
結果が揺れる場合があります。

重みをprocess全体へ組み込まずに検査できます。

```bash
cargo run --release -p sekirei-bench --bin nnue_probe -- \
  /path/to/weights.bin --strict --json
```

strict probeは定数・準定数出力、material／手番感度不足、layer縮退、再読込の非決定性を
検出します。modelとlicense境界は[NNUE重み](docs/nnue_weights.md)を参照してください。
外部SFNNは現在、headerと由来の限定検査までで、推論互換性は未実装です。

## build・test・benchmark

```bash
cargo build --release
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo bench --bench movegen -p sekirei-bench
```

固定した競合比較診断：

```bash
cargo run --release -p sekirei-bench --bin cross_library -- --check
cargo run --release -p sekirei-bench --bin cross_library -- --components
```

v0.3.35の10 session比較では、共通する合法手生成3 caseのrsshogi/Sekirei比が
1.1596倍（95% CI 1.1364〜1.1833倍）でした。全状態更新、NNUE、探索、棋力は対象外で、
総合速度順位を示しません。hostと測定境界は
[10 session report](scripts/benchmark_reports/cross_library_component_10session_2026-09-12.md)に記録しています。

## 対局・CSA/Floodgate

ローカル対局：

```bash
cargo run --release -p sekirei-match-runner -- \
  --engine1 ./target/release/sekirei \
  --engine2 /path/to/other-engine \
  --games 100 --byoyomi 10000 \
  --positions data/gate/openings_standard.sfen \
  --games-per-position 4 --json results/run.json
```

CSA client：

```bash
cargo run --release -p sekirei-csa -- \
  --user <name> --trip <secret> --game floodgate-300-10F \
  --record-dir data/floodgate --loop
```

command lineの代わりに`FLOODGATE_ACCOUNT`と`FLOODGATE_TRIP`を環境から注入できます。
認証情報、棋譜、生成重み、学習dataはcommitしないでください。`--analysis-dir <dir>`は
各手のschema付き診断sidecarを保存します。過去に存在しない評価値を観測値として補完しません。

自己対局Eloは選択した基準に対する相対値で、Floodgateや人間のレートではありません。
統計gateは`PASS`、`FAIL`、`INCONCLUSIVE`を区別します。

## NNUE学習

```bash
cargo run --release -p sekirei-train -- --help
cargo run --release -p sekirei-train -- \
  --games /path/to/csa_dir --output weights.bin --epochs 3
```

固定NNUE教師、決定的なnode上限label、validation split、Adam状態、epoch途中の完全resumeに
対応します。学習artifactと推論用`.bin`は分離します。運用scriptは
[scripts索引](scripts/README.md)を参照してください。

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
