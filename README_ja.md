# Sekirei — Rust製将棋エンジン

[![CI](https://github.com/kent-tokyo/sekirei/actions/workflows/ci.yml/badge.svg)](https://github.com/kent-tokyo/sekirei/actions/workflows/ci.yml)
[![crates.io](https://img.shields.io/crates/v/sekirei.svg)](https://crates.io/crates/sekirei)
[![License](https://img.shields.io/crates/l/sekirei.svg)](https://github.com/kent-tokyo/sekirei/blob/main/LICENSE)

[English](README.md)

Sekirei は Rust で実装した実験的な将棋エンジンです。USI、CSA/floodgate クライアント、
USI 対 USI の棋力テスト、NNUE スタイル評価に対応しています。棋力と評価品質は開発中で、
ここでは絶対レーティングや他エンジンを上回るという主張はしていません。

Rust製の将棋（日本のチェス）エンジン、USI対応エンジン、NNUE評価、並列探索、CSA/Floodgate
対局、自己対局による棋力テストに関心がある開発者向けの実験プロジェクトです。

## まず動かす

crates.ioからUSIエンジンをインストールできます。

```bash
cargo install sekirei
sekirei
```

ソースからビルドする場合:

```bash
git clone https://github.com/kent-tokyo/sekirei.git
cd sekirei
cargo run --release -p sekirei-usi
```

USIコマンドを標準入力から読み取るため、対応する将棋GUIでインストール済みの
`sekirei`実行ファイルをエンジンとして指定できます。重みファイルなしではマテリアル評価を
使い、先頭引数にNNUE重みファイルを渡すと学習済み評価を有効化します。

```bash
sekirei /path/to/weights.bin
```

## 主な機能

- 9×9盤、合法手生成、成り、駒打ち、SFEN、USI指し手に対応。
- 将棋GUI接続用のUSIエンジン。
- 反復深化、negamax/alpha-beta、PVS/YBW並列探索、静止探索、手順序付け、枝刈り。
- ロックフリー置換表と、任意で有効化できる投機的並列探索。
- ファイルから読み込むNNUEスタイルの差分評価。
- CSA v2.2 / Floodgateクライアント。
- 自己対局、回帰テスト、相対Elo推定用のUSI対USIマッチランナー。
- CSA棋譜または抽出済み局面からのNNUE学習パイプライン。

## 現在の状態

- ピュア Rust。コアの探索・評価コードに `unsafe` はありません。
- USI エンジン: `sekirei`
- CSA クライアント: `sekirei-csa`
- 棋力テスト: `sekirei-match`
- NNUE 訓練: `train`（パッケージ名は `sekirei-train`）
- NNUE 重みはファイルから読み込み、リポジトリには同梱していません。

crates.ioの`sekirei`パッケージはUSIエンジンのバイナリです。リポジトリはCargo workspace
として構成され、再利用可能な`sekirei-core`ライブラリと補助CLIツールも公開しています。

## 構成

```text
crates/sekirei-core/         局面、合法手生成、探索、置換表、評価
crates/sekirei-usi/          USI エンジン（sekirei）
crates/sekirei-csa/          CSA/floodgate クライアント（sekirei-csa）
crates/sekirei-match-runner/ USI 対 USI 棋力テスト（sekirei-match）
crates/sekirei-train/        NNUE 訓練（train）
crates/sekirei-bench/        ベンチマーク
scripts/                     訓練・棋力テスト用スクリプト
```

コアには alpha-beta/negamax、PVS/YBW 並列探索、反復深化、静止探索、ロックフリー置換表、
手順序付け・枝刈りの各種ヒューリスティック、任意の投機的探索を実装しています。
`SpecTopN=0` で投機的探索を無効にできます。

## ビルドとテスト

```bash
cargo build --release
cargo test --release
cargo bench --bench movegen -p sekirei-bench
```

プロセス全体の重みを変更せずに NNUE チェックポイントを確認する場合:

```bash
cargo run --release -p sekirei-bench --bin nnue_probe -- /path/to/weights.bin
# 自動処理では --json、任意局面では --sfen "<SFEN>" を繰り返し指定
```

評価値、スコアレンジ、平均、分散、基準局面との差分を表示します。`--json` で機械可読形式にできます。
出力分散の確認にも使えます。このプローブは診断用であり、棋力テストではありません。

マテリアル評価で起動:

```bash
cargo run --release -p sekirei-usi
```

NNUE 重みを指定して起動:

```bash
cargo run --release -p sekirei-usi -- /path/to/weights.bin
```

## USI オプション

`usi` コマンド後に全オプションを表示します。主なものは次の通りです。

- `Hash`, `Threads`, `MoveOverhead`
- `Ponder`, `MultiPV`
- `EvalFile`（`isready` 時に読み込み）
- `SpecTopN`（デフォルト `3`、`0` で無効）
- `UseBook`, `BookFile`, `BookMaxPly`, `BookMinConfidence`

`SpecTopN > 0` では投機タスクのスケジューリングにより、同一条件でも探索結果が変わる
場合があります。再現性を優先する比較では、可能な限り `SpecTopN=0` を使用してください。

## CSA / floodgate

```bash
cargo run --release -p sekirei-csa -- \
  --user <name> --trip <secret> --game floodgate-300-10F --loop
```

`FLOODGATE_ACCOUNT` と `FLOODGATE_TRIP` も利用できます。認証情報、棋譜、重み、訓練データを
コミットしないでください。

## 棋力テスト

```bash
cargo run --release -p sekirei-match-runner -- \
  --engine1 ./target/release/sekirei \
  --engine2 /path/to/other-engine \
  --games 100 --byoyomi 10000 \
  --positions data/gate/openings_standard.sfen \
  --games-per-position 4 --json results/run.json
```

既存の結果 JSON は `gate` で判定できます。自己対局の Elo は指定したベースラインに対する
相対値であり、floodgate レーティングではありません。

## NNUE 訓練

CSA 対局または抽出済み局面を入力できます。全オプションは次で確認してください。

```bash
cargo run --release -p sekirei-train -- --help
```

例:

```bash
cargo run --release -p sekirei-train -- \
  --games /path/to/csa_dir --output weights.bin --epochs 3
```

訓練データ、チェックポイント、重み、対局結果、実験ログはローカル生成物として公開リポジトリ
から除外しています。

## ライセンス

Apache License, Version 2.0 または MIT license のいずれかを選択できます。
