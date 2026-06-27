# Sekirei

[![CI](https://github.com/kent-tokyo/sekirei/actions/workflows/ci.yml/badge.svg)](https://github.com/kent-tokyo/sekirei/actions/workflows/ci.yml)

[English](README.md)

Sekireiは、投機的並列探索とNNUEスタイル評価を探求するRust製の実験的将棋エンジンです。
USI/CSA経由で対局可能ですが、棋力・時間制御・評価品質はまだ改善中です。

Rustの所有権モデルを活用することで、アトミクスのみによる安全な並行探索（`unsafe`なし）を探求しています。

## 現在の状態

- USI対応（ShogiGUI 等で利用可能）
- CSAクライアントでfloodgate接続中（アカウントは `.env` の `FLOODGATE_ACCOUNT` で設定）
- NNUEスタイル評価対応（重みファイルは同梱なし・CSAデータからの訓練またはマテリアル評価にフォールバック）
- floodgateレートは計測中（実戦テスト中）

## 設計原則

- **コアロジックにおける `unsafe` ゼロ** — 並行処理はすべてRustの型システム・アトミクス・安全なプリミティブで実装
- **100% Pure Rust** — 探索・評価パスにC++ラッパーやFFIを一切使用しない

## アーキテクチャ

```
crates/
  sekirei-core/   — エンジン本体ライブラリ
    board.rs      — 局面表現 + do_move/undo_move/do_null_move
    movegen.rs    — 合法手生成（generate_legal_moves, generate_legal_captures）
    search.rs     — YBW並列アルファ・ベータ + 一般的な探索最適化
    eval.rs       — NNUE評価（重み未ロード時はマテリアルにフォールバック）
    nnue.rs       — NNUEアキュムレータ（差分更新・SIMD対応・実行時重みロード）
    tt.rs         — ロックフリー置換表（XOR-trick・深さ優先置換）
    speculative.rs — 投機的先読み + RAIIキャンセル
    policy.rs     — 先読み用軽量手スコアリング
  sekirei-usi/          — USIサーバー → バイナリ: sekirei
  sekirei-csa/          — floodgate CSAクライアント → バイナリ: sekirei-csa
  sekirei-match-runner/ — USI対USI棋力テスト管理 → バイナリ: sekirei-match
  sekirei-train/        — NNUE訓練パイプライン（CSAパーサー・Adam SGD・重みI/O）
  sekirei-bench/        — マイクロベンチマーク（手生成・perft・探索・評価）
```

## 探索機能（現在実装済み）

| 技術 | 状態 |
|------|------|
| アルファ・ベータ（Negamax） | yes |
| PVS + YBW並列探索（rayon） | yes |
| 反復深化 | yes |
| ロックフリー置換表（深さ優先） | yes |
| 静止探索 + Delta Pruning | yes |
| キラームーブ（ply毎に2手） | yes |
| ヒストリーヒューリスティック | yes |
| アスピレーションウィンドウ | yes |
| Late Move Reduction（LMR） | yes |
| Null Move Pruning（R=3） | yes |
| Reverse Futility Pruning（depth ≤ 3） | yes |
| Futility Pruning（depth 1） | yes |
| Late Move Pruning（depth ≤ 2） | yes |
| 投機的先読み探索 | yes |
| NNUE評価 | `cargo run -p sekirei -- weights.bin` で有効化 |

## ロードマップ

| フェーズ | 目標 | 状態 |
|---------|------|------|
| 1 | 基盤構築：Bitboard MoveGen・do/undoムーブ・Perft | 完了 |
| 2 | ロックフリー置換表 & YBW並列探索 | 完了 |
| 2.5 | 探索最適化（killer・history・LMR・NMP・RFP・futility・LMP） | 完了 |
| 3 | 投機的エンジン（先読みスポーン・RAIIキャンセル） | 完了 |
| 4 | NNUE統合（重みI/O・eval配線・訓練パイプライン） | 完了 |
| 5 | プロトコル & 実戦（CSA/floodgate・マッチ管理） | 進行中 |

## ビルドと実行

```bash
# 開発ビルド
cargo build

# 最適化ビルド（.cargo/config.toml 経由で target-cpu=native 適用）
cargo build --release

# テスト
cargo test

# ベンチマーク
cargo bench --bench movegen

# USIエンジン起動（マテリアル評価フォールバック）
cargo run --release -p sekirei

# USIエンジン起動（NNUE有効）
cargo run --release -p sekirei -- weights.bin

# floodgate 接続（CSAクライアント）
cargo run --release -p sekirei-csa -- --user <名前> --password <パスワード> --loop

# 棋力テスト: sekirei vs sekirei（10局・1秒秒読み）
cargo run --release -p sekirei-match-runner -- \
  --engine1 ./target/release/sekirei \
  --engine2 ./target/release/sekirei \
  --games 10 --byoyomi 1000

# 棋力テスト: sekirei vs 外部エンジン
cargo run --release -p sekirei-match-runner -- \
  --engine1 ./target/release/sekirei \
  --engine2 /path/to/suisho5 \
  --games 100 --byoyomi 10000

# NNUE訓練（floodgate CSAファイルを別途ダウンロード）
# データ: http://wdoor.c.u-tokyo.ac.jp/shogi/
cargo run --release -p sekirei-train -- --games /path/to/csa_dir --output weights.bin --epochs 3
```

## ベンチマーク

Apple M4 Pro での実測値（`cargo build --release`、`target-cpu=native`）。

| 指標 | 値 |
|------|---|
| 合法手生成（初期局面） | ~5.5 µs / 呼び出し |
| NNUE 評価（初期局面） | ~18.7 ns / 呼び出し |
| 探索 depth 4（初期局面） | ~3.6 ms |
| 探索 NPS（NNUE、10 秒秒読み） | ~1.1M nps、depth 13 |
| テストスイート | 15 テスト全通過 |

floodgate: 実戦テスト中（レートは計測中）。

## 現在の制限事項

- NNUE 重みファイルは同梱なし。floodgate CSA データから訓練するかマテリアル評価にフォールバック
- `setoption EvalFile` 対応済み。ゲーム中の重み再ロードにはエンジン再起動が必要
- Pondering 未対応

## 名前の由来

**Sekirei**（セキレイ）は、ハクセキレイなどに代表される小型の鳥で、
尾をリズミカルに上下に振りながら素早く動き回ることで知られています。

小さく俊敏で、常に動き続ける——
投機的先読みで早めに手を絞り込み、探索しながら修正していく
このエンジンのスタイルと重なります。
