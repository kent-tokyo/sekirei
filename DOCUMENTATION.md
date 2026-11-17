# 文書索引

初めて使う場合は[日本語README](README_ja.md)または[English README](README.md)から始めます。
進行中・保留中の強化計画は2026-11-16にすべて終了し、方針を見直す状態に戻しました。
現在、新しい強化実験は選定していません。過去の記録に残る「次の作業」は実行予定ではありません。

## 使う・開発する

| 目的 | 文書 |
|---|---|
| 重みを選び、読み込む | [NNUE重み](docs/nnue_weights.md)、[ファイル配置](weights/README.md) |
| ビルド・テストを行う | [開発手順](docs/development.md) |
| 速度や棋力の測定方法・結果を調べる | [ベンチマーク](docs/benchmarks.md) |
| 対局・学習・運用ツールを探す | [スクリプト索引](scripts/README.md) |
| ブラウザへ組み込む | [WASM API](crates/sekirei-wasm/README.md)、[配布記録](docs/releases/npm_wasm.md) |
| モバイルへ組み込む | [対応範囲と制限](docs/mobile_integration.md) |
| 保存済みの解析を閲覧する | [Explorer](explorer/README.md)、[解析レコード仕様](docs/amateur_analysis_benchmark.md) |
| CSA接続・復旧を確かめる | [ローカルリハーサル](scripts/DENRYU_REHEARSAL.md) |

## 現状と履歴を調べる

| 目的 | 文書 |
|---|---|
| 終了した計画と未確認事項を確認する | [計画の棚卸し](docs/plans/README.md) |
| 作業上のルールを確認する | [AGENTS.md](AGENTS.md)、[指示の履歴](docs/agent-instructions/README.md) |
| バージョンごとの変更を調べる | [変更履歴](CHANGELOG.md)、[古い変更履歴](CHANGELOG_ARCHIVE.md) |
| 実験の条件・判定を調べる | [実験記録](docs/experiments/README.md) |
| 設計時の判断を調べる | [設計記録](docs/design/README.md) |
| 公開済み成果物を照合する | [リリース記録](docs/releases/README.md) |

現在の状態は上記の入口にまとめ、細かな条件・ハッシュ・結果は元の記録を参照します。
過去資料の「次の作業」は現行の実行指示ではありません。原判定を変更したり、
古い測定値を現行版の速度・棋力として扱ったりしません。

ローカルの`ROADMAP.md`と`tasks/`はGit管理外です。未公開の重み・学習データ・
生ログも公開文書へ移しません。
