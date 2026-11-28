# Sekirei — Rust製将棋エンジン

[![CI](https://github.com/kent-tokyo/sekirei/actions/workflows/ci.yml/badge.svg)](https://github.com/kent-tokyo/sekirei/actions/workflows/ci.yml)
[![codecov](https://codecov.io/gh/kent-tokyo/sekirei/branch/main/graph/badge.svg?flag=rust-engine)](https://codecov.io/gh/kent-tokyo/sekirei)
[![Ask DeepWiki](docs/assets/ask-deepwiki.svg)](https://deepwiki.com/kent-tokyo/sekirei)
[![Release](https://img.shields.io/badge/release-v0.5.5-blue)](https://github.com/kent-tokyo/sekirei/releases/tag/v0.5.5)
[![crates.io](https://img.shields.io/crates/v/sekirei.svg)](https://crates.io/crates/sekirei)
[![License](https://img.shields.io/crates/l/sekirei.svg)](LICENSE)

[English](README.md)

SekireiはPure Rustで実装した実験的な将棋エンジンです。`0.5.5`はUSIエンジン、
CSAクライアント、対局ランナー、ブラウザAPIを提供します。
既定は駒得評価で、NNUEを使うには重みファイルの指定が必要です。

## エンジンを動かす

```bash
cargo install sekirei
sekirei
```

実行ファイルをUSI対応GUIへ登録します。このcheckoutを動かす場合：

```bash
cargo run --release -p sekirei
```

USIの`usi`で全オプションを確認できます。1ワーカーの診断では
`Threads=1`、`SpecTopN=0`を指定します。`MultiPV=1`のとき、
`SearchMode=Auto`は1ワーカーで逐次探索、複数ワーカーでLazy SMPを使います。

任意の`nn_r6`重みを使う場合は、`EvalFile`にパスを指定し、
`NnueOutput=absolute`、`FV_SCALE=24`を設定します。
取得先、ハッシュ、対応形式、確認済みの範囲は[NNUE重み](docs/nnue_weights.md)を参照してください。

## 利用できる機能

| インターフェース | 用途 |
|---|---|
| `sekirei` | USIエンジン。alpha-beta/PVS、静止探索、lock-free TT |
| `sekirei-csa` | CSA/Floodgateクライアント |
| `sekirei-match` | USIエンジン同士のローカル対局 |
| `sekirei-train`パッケージ | NNUE学習。`cargo run --release -p sekirei-train -- --help`で引数を確認 |
| [sekirei-wasm](crates/sekirei-wasm/README.md) | SFEN、合法手、上限付き1ワーカー解析、MultiPV、詰み解析 |

ブラウザ用パッケージは`npm install sekirei-wasm@0.5.5`で導入します。
評価器のバイト列を呼出側から渡せますが、重みは同梱しません。
配布物の照合は[配布ガイド](docs/releases/npm_wasm.md)にまとめています。

定跡はCargoの`opening-book`機能と、`BookFile`・`UseBook=true`の指定が必要です。
既定の定跡は同梱しません。任意設定の27点法入玉宣言はrootで扱います。
規則付き参照探索は通常のUSI・並列探索へ未接続です。
実験的な探索機能や局所診断による棋力向上は確認されていません。

## ビルドと検証

```bash
cargo build --release
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

対象を絞ったテスト、カバレッジ、公開メタデータの確認は[開発手順](docs/development.md)、
対局・学習・運用ツールは[スクリプト索引](scripts/README.md)を参照してください。

## 現在の状態と文書

現在の優先事項（2026-11-27）は、棋力測定を安く日常的にすること（並列対局と固定YaneuraOuゲージ）、
その後により大きなNNUEを学習することです。棋力に影響しうる変更は
[標準の棋力テスト](docs/strength_testing.md)で採否を決めます。作業ルールは[AGENTS.md](AGENTS.md)、
それ以前の計画の到達点は[計画の棚卸し](docs/plans/README.md)にあります。

- [文書索引](DOCUMENTATION.md)
- [ベンチマークと検証範囲](docs/benchmarks.md)
- [変更履歴](CHANGELOG.md)

## ライセンス

ソースは[MIT](LICENSE-MIT) OR [Apache-2.0](LICENSE-APACHE)です。[NOTICE](NOTICE)を保持してください。
本プロジェクトが配布するNNUE成果物には[別の条件](NNUE-LICENSE.md)が適用されます。
外部重みは各ファイルのライセンスに従います。
