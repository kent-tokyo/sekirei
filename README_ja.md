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

## NNUE 訓練

### CSA ファイルから（スタンドアロン）

```bash
# 基本: floodgate CSA から訓練（http://wdoor.c.u-tokyo.ac.jp/shogi/ からダウンロード）
cargo run --release -p sekirei-train -- \
  --games /path/to/csa_dir --output weights.bin \
  --epochs 3 --quiet --min-ply 20 --min-rate 1800 --label-depth 4
```

### Quietset を使った安定性フィルタリング

[quietset](https://github.com/kent-tokyo/quietset) は複数の探索深度でのラベル安定性を評価し、ノイズの多い教師ラベルを除外します。

```bash
# 1. 複数深度で観測データを出力
cargo run --release -p sekirei-train -- \
  --games /path/to/csa_dir --export observations.jsonl \
  --depths 2,4,6,8 --quiet --min-ply 20 --min-rate 1800

# 2. 安定度スコアリング
quietset score observations.jsonl > scored.jsonl

# 3a. 安定局面のみで学習（stability >= 0.85）
cargo run --release -p sekirei-train -- \
  --games /path/to/csa_dir --output weights_keep.bin \
  --scored scored.jsonl --min-stability 0.85 --epochs 3

# 3b. または stability_score でロスを重み付け（不安定局面の寄与を小さくする）
cargo run --release -p sekirei-train -- \
  --games /path/to/csa_dir --output weights_weighted.bin \
  --scored scored.jsonl --stability-weighted --epochs 3
```

### shogiesa + quietset 公式パイプライン

[shogiesa](https://github.com/kent-tokyo/shogiesa) が局面の抽出・ラベリングを担当し、
[quietset](https://github.com/kent-tokyo/quietset) が安定性スコアリングを担当します。
sekirei-train は `--positions` で positions.jsonl を直接受け取ります（CSA パース不要）。

ワンショットのパイプラインスクリプトで全ステージを実行し、最後に Elo gate を通します：

```bash
# Tier 1 — クイック（depths 2,4、数時間）
bash scripts/train_with_shogiesa_quietset.sh data/csa weights_new.bin data/weights_v007.bin

# Tier 2 — 標準（depths 2,4,6）
DEPTHS=2,4,6 bash scripts/train_with_shogiesa_quietset.sh data/csa weights_new.bin data/weights_v007.bin

# Tier 3 — ディープ: 境界局面のみ depth 4,6,8 で再ラベルして再訓練
# Step 1: 境界局面を高 depth でスコアして別ファイルに保存
quietset select data/stage3/scored.jsonl --class borderline \
  | shogiesa label --engine ./target/release/sekirei --depths 4,6,8 \
  | quietset score --profile game-ai \
  > data/stage3/deep_scored.jsonl
# Step 2: EXTRA_SCORED でマージしながら再訓練
EXTRA_SCORED=data/stage3/deep_scored.jsonl \
DEPTHS=2,4,6 \
bash scripts/train_with_shogiesa_quietset.sh data/csa weights_deep.bin data/weights_v007.bin
```

中間ファイルは `data/runs/<タイムスタンプ>/` 以下に保存されます（`RUN_DIR=...` で変更可）。
各実行後に `manifest.json` が生成され、重みとパラメータが紐付けられます。
環境変数オーバーライド: `DEPTHS`, `GAMES`, `MIN_PLY`, `MAX_PLY`, `RUN_DIR`, `EXTRA_SCORED`。手動で各ステージを実行する場合：

```bash
# Stage 1: 局面抽出
shogiesa extract --input ./data/csa --out data/stage1/positions.jsonl \
  --min-ply 20 --max-ply 160 --every-n-plies 4 --dedup

# Stage 2: ラベル付け
shogiesa label --input data/stage1/positions.jsonl \
  --engine ./target/release/sekirei --depths 2,4 --timeout-ms 10000 \
  --out data/stage2/observations.jsonl

# Stage 3: 安定性スコアリング
quietset score data/stage2/observations.jsonl --profile game-ai > data/stage3/scored.jsonl

# 訓練
cargo run --release -p sekirei-train -- \
  --positions data/stage1/positions.jsonl \
  --scored data/stage3/scored.jsonl \
  --stability-weighted --validation-ratio 0.1 \
  --checkpoint-dir data/checkpoints \
  --output data/weights_new.bin
```

## 棋力回帰テスト

変更が本当に棋力向上につながったかを確認するには、既知のベースラインと対局して Elo gate を通してください。重みの変更は必ず gate を通過してから採用します。

```bash
# ワンショット回帰（ビルド → 400局 → PASS/FAIL/INCONCLUSIVE を出力）
bash scripts/strength_regression.sh weights_new.bin weights_base.bin

# または既存の result JSON に対して gate を手動実行
cargo run --release -p sekirei-match-runner -- gate result.json \
  --pass-elo 20 --pass-los 0.95 --fail-elo -10
```

match runner は対局ごとの結果を `--json` の `<name>.json` と並べて `<name>.jsonl` にも
保存します。`gate` はこの JSONL を読み込み、[veridict](https://github.com/kent-tokyo/veridict)
（`--metric elo`）で判定をやり直します。veridict は点推定ではなく**信頼区間**で判定します：

| 判定 | 条件 |
|------|------|
| **PASS** | 信頼区間の下限 ≥ pass 閾値（デフォルト +20 elo） |
| **FAIL** | 信頼区間の上限 ≤ fail 閾値（デフォルト −10 elo） |
| **INCONCLUSIVE** | 両閾値をまたぐ — 局数を増やして再試験 |

これは単純な点推定判定より厳格です：信頼区間が 0 をまたいでいる限り、たまたま良い点推定が
出ても PASS にはならず INCONCLUSIVE のままです。Elo/LOS の点推定（同じ対局データから算出）は
引き続き人間向けレポート行として表示され、result JSON の `elo_diff`、`elo_ci_low`、
`elo_ci_high`、`los` も従来通り含まれます。`.jsonl` が存在しない古い result JSON
（この変更より前に生成されたもの）は、従来の点推定 + LOS 判定にフォールバックし、
その旨が gate の出力に明示されます。

自己対局の Elo は、その対局における `engine2` に対する相対値でしかなく、floodgate のような
外部レーティングプールとは本来無関係です。ベースラインの絶対レーティングについて何らかの見込みが
あるなら、`--anchor <rating>` で判定に使われた Elo 効果量を大まかな推定値に変換できます:
`est_rating ≈ anchor + effect`。あくまで方向性の目安であり実測ではありません（自己対局の Elo と
レーティングプールの Elo は同じスケールではないため）。デフォルト値は無く、省略時の出力は従来通り
変わりません。

```bash
cargo run --release -p sekirei-match-runner -- gate result.json --anchor 1850
# report: elo_diff=+82.6  los=96.9%  games=60
# veridict: metric=elo  effect=+82.6 elo  95% CI=[+41.0, +124.2]  CI lower bound ... meets the pass threshold ...
# PASS  est_rating≈1933 (anchor=1850)
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
- Pondering 対応済み（`go ponder` / `ponderhit`）。`setoption name Ponder value true` で有効化
- MultiPV 対応済み（`setoption name MultiPV value N`）

## 名前の由来

**SEKIREI** — *Shogi Engine for Kifu-Informed Reasoning and Efficient Inference*

セキレイ（鶺鴒）は、ハクセキレイなどに代表される小型の鳥で、
尾をリズミカルに上下に振りながら素早く動き回ることで知られています。

小さく俊敏で、常に動き続ける——
投機的先読みで早めに手を絞り込み、探索しながら修正していく
このエンジンのスタイルと重なります。

## ライセンス

以下のいずれかのライセンスの下に提供されます（お好みで選択可）。

- Apache License, Version 2.0
  ([LICENSE-APACHE](LICENSE-APACHE) または https://www.apache.org/licenses/LICENSE-2.0)
- MIT license
  ([LICENSE-MIT](LICENSE-MIT) または https://opensource.org/licenses/MIT)

## コントリビューション

特段の申告がない限り、本プロジェクトに意図的に提出されたコントリビューションは
上記のデュアルライセンス条件の下でライセンスされます。

Sekirei はピュア Rust のオリジナル将棋エンジンです。GPL ライセンスのコードは
含みません。アルゴリズムは先行研究から学びますが、実装はクリーンルームで行い、
プロジェクトのパーミッシブライセンスと互換性を維持します。
