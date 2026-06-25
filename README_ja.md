# Janos（コードネーム：Paradigm）

[English](README.md)

**JANOS** = *Jet-speed Ancestry Node Optimizer of Shogi*

世界最高峰の将棋AI（水匠・翡翠クラス）を超えることを目標とした、Rust製の投機的並列将棋AIエンジンです。

Rustの所有権システムと型システムを活用することで、**超攻撃的な投機的並列探索と安全な即時キャンセル機構**を実現します。これはC++では安全に実装することがほぼ不可能な手法です。

## 名前の由来

「Janos」という名前は、いずれも「János」という名を持つ3人のハンガリー人への敬意から生まれました。彼らはそれぞれ、このプロジェクトのコンセプトを体現しています。

| 人物 | 体現するもの | Janosとの対応 |
|------|-------------|--------------|
| **ジョン・フォン・ノイマン**（Margittai Neumann János）— ゲーム理論の創始者 | 緻密で厳密な論理 | 数学的に正しい探索木の設計 |
| **バルトーク・ベーラ**（Bartók Béla Viktor János）— 伝統を解体し、不協和音すら武器にして全く新しい音楽体系を築き上げた作曲家 | 既存パラダイムの破壊と創造 | C++が支配するエンジン開発の世界をRustで塗り替える |
| **ハーリ・ヤーノシュ**（コダーイの歌劇の主人公）— 常識を超える大風呂敷と冒険譚を持つホラ吹き男爵 | 常識を超えた大胆な大局観 | 投機的先読み：正しいと確認する前に指し手に賭ける |

> 「緻密なロジック」「既存パラダイムの破壊と創造」「常識を超える大胆な大局観（投機的先読み）」
>
> これらすべての要素が、このプロジェクトのコンセプトと完璧にシンクロしています。C++の伝統的な最適化の壁を壊しに行くプロジェクトの旗印として、これ以上の名前はありません。

## 設計原則

- **コアロジックにおける `unsafe` ゼロ** — 並行処理はすべてRustの型システム・アトミクス・安全なプリミティブで実装
- **100% Pure Rust** — 探索・評価パスにC++ラッパーやFFIを一切使用しない

## アーキテクチャ

```
crates/
  shogi-core/   — エンジン本体ライブラリ
    board.rs    — 局面表現 + do_move/undo_move/do_null_move
    movegen.rs  — 合法手生成（generate_legal_moves, generate_legal_captures）
    search.rs   — YBW並列アルファ・ベータ + 全探索最適化
    eval.rs     — NNUE評価（重み未ロード時はマテリアルにフォールバック）
    nnue.rs     — NNUEアキュムレータ（差分更新・SIMD対応・実行時重みロード）
    tt.rs       — ロックフリー置換表（XOR-trick・深さ優先置換）
    speculative.rs — 投機的先読み + RAIIキャンセル
    policy.rs   — 先読み用軽量手スコアリング
  usi/          — USIサーバー → バイナリ: janos
  csa/          — floodgate CSAクライアント → バイナリ: janos-csa
  match-runner/ — USI対USI棋力テスト管理 → バイナリ: janos-match
  train/        — NNUE訓練パイプライン（CSAパーサー・Adam SGD・重みI/O）
  bench/        — マイクロベンチマーク（手生成・perft・探索・評価）
```

## 探索機能一覧

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
| NNUE評価 | `cargo run -p usi -- weights.bin` で有効化 |

## ロードマップ

| フェーズ | 目標 | 状態 |
|---------|------|------|
| 1 | 基盤構築：Bitboard MoveGen・do/undoムーブ・Perft | 完了 |
| 2 | ロックフリー置換表 & YBW並列探索 | 完了 |
| 2.5 | 探索最適化（killer・history・LMR・NMP・RFP・futility・LMP） | 完了 |
| 3 | 投機的エンジン（先読みスポーン・RAIIキャンセル） | 完了 |
| 4 | NNUE統合（重みI/O・eval配線・訓練パイプライン） | 完了 |
| 5 | プロトコル & 実戦（CSA/floodgate・マッチ管理・リリース） | 完了 |

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
cargo run --release -p usi

# USIエンジン起動（NNUE有効）
cargo run --release -p usi -- weights.bin

# floodgate 接続（CSAクライアント）
cargo run --release -p csa -- --user <名前> --password <パスワード> --loop

# 棋力テスト: janos vs janos（10局・1秒秒読み）
cargo run --release -p match-runner -- \
  --engine1 ./target/release/janos \
  --engine2 ./target/release/janos \
  --games 10 --byoyomi 1000

# 棋力テスト: janos vs 外部エンジン
cargo run --release -p match-runner -- \
  --engine1 ./target/release/janos \
  --engine2 /path/to/suisho5 \
  --games 100 --byoyomi 10000

# NNUE訓練（floodgate CSAファイルを別途ダウンロード）
# データ: http://wdoor.c.u-tokyo.ac.jp/shogi/
cargo run --release -p train -- --games /path/to/csa_dir --output weights.bin --epochs 3
```

## エージェントの役割

マルチエージェント協調開発モデルの詳細は [AGENTS.md](AGENTS.md) を参照してください。

## タスク管理

詳細なマイルストーンチェックリストは [tasks/todo.md](tasks/todo.md) を参照してください。  
設計上の教訓は [tasks/lessons.md](tasks/lessons.md) を参照してください。
