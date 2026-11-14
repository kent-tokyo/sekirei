# Issue #275: 防御候補後の相手詰み診断

## 事前の問いと上限

候補を合法性確認後に私有の盤面へ適用し、既存の完全詰み解析を呼べば、探索評価値と詰み証明を混ぜずに防御手を比較できるか。まず issue の2候補 `8a8b+` と `S*2g` を同じ最大9手、合計50万節点、合計5秒で確認する。`Mate / NoMate / Unknown` と不正候補・資源打切りを区別できなければ停止する。通常探索や重みは変更しない。

## 実装と呼出し

`sekirei_core::defense::analyze_defense_threats` は入力順に候補を返す。`analyze_all_defense_threats` は合法手を全件対象にし、両関数とも候補間で節点と締切を共有する。候補は `sfen::move_from_usi` で元局面から解析できる。各 `DefenseThreatOutcome::Analyzed` の `MateAnalysis` に相手の状態、最短証明手数、全最短初手、使用節点、打切り理由を残す。`IllegalMove`、`Skipped`、探索途中の `TimedOut` に詰みの判定はない。時間切れは診断専用の状態で示し、既存の公開 `MateAnalysisAbortReason` に変更を加えない。

```rust
use std::time::Duration;
use sekirei_core::{board::Board, defense::analyze_defense_threats, sfen::move_from_usi};

let board = Board::from_sfen("1R1+R2gnk/4+P2sl/l2p+Bp1pp/p1p3g2/3np4/PPP3PPP/L4Pg1S/+b2+s4L/1N4g1K b Sn3p 1")?;
let moves = [move_from_usi("8a8b+", &board)?, move_from_usi("S*2g", &board)?];
let report = analyze_defense_threats(&board, &moves, 9, 500_000, Duration::from_secs(5));
```

`NoMate` は指定手数以内の連続王手による強制詰みがないという結果であり、無制限の不詰や防御手の安全を意味しない。締切は訪問節点ごとに確認するため、1回の合法手生成の実行中はわずかに超過しうる。

## 検証と判断

2026-11-14 の公開 `main` (`03dc17db`) を基点とする隔離checkoutで、上記2候補は `8a8b+` 後に相手 `Mate`・最短1手・`N*2g`、`S*2g` 後に `NoMate`。先後、打ち駒、成り・不成、王手を放置する違法手、合計節点0/1、期限切れ、全合法手、元盤面の復元を対象テストで確認した。`cargo test -p sekirei-core --lib` は470件通過・10件無視、`cargo clippy -p sekirei-core --lib -- -D warnings` と `cargo fmt --all -- --check` も通過。これは診断機能の確認であり、通常探索の棋力改善や issue #277 の解決ではない。

最初のPRのCIで、既存の公開 enum に `Deadline` を追加すると `cargo-semver-checks` が破壊的変更と判定した。公開 enum を元に戻し、診断専用の `DefenseLimit` / `TimedOut` に分離した。同じCIで公開 `main` に元からある `sekirei-train/src/trainer.rs` の重複Clippy属性がworkspaceの2 jobを止めていたため、隔離PRで重複行を1つ削除した。修正後の `cargo clippy --workspace --all-targets -- -D warnings` は通過した。

[PR #278](https://github.com/kent-tokyo/sekirei/pull/278) は `57f85130` にマージされ、issue #275 は完了として閉鎖された。GitHub の必須チェック設定がないため、`--auto` の指定は保留にならず直ちにマージされた。PR 側の全テスト、公開 API 互換性、成果物監査は成功。マージ後 `main` のCI・公開 API 互換性と coverage は成功。coverage run `43538764140` は51,666/52,704行（98.0305%）で98%ゲートを通過した。PR 側の coverage run `43537616108` も成功した。
