# Sekirei — Rust製将棋エンジン

[![CI](https://github.com/kent-tokyo/sekirei/actions/workflows/ci.yml/badge.svg)](https://github.com/kent-tokyo/sekirei/actions/workflows/ci.yml)
[![codecov](https://codecov.io/gh/kent-tokyo/sekirei/branch/main/graph/badge.svg?flag=rust-engine)](https://codecov.io/gh/kent-tokyo/sekirei)
[![Ask DeepWiki](docs/assets/ask-deepwiki.svg)](https://deepwiki.com/kent-tokyo/sekirei)
[![Release](https://img.shields.io/badge/release-v0.4.5-blue)](https://github.com/kent-tokyo/sekirei/releases/tag/v0.4.5)
[![crates.io](https://img.shields.io/crates/v/sekirei.svg)](https://crates.io/crates/sekirei)
[![License](https://img.shields.io/crates/l/sekirei.svg)](https://github.com/kent-tokyo/sekirei/blob/main/LICENSE)

[English](README.md)

SekireiはPure Rustで実装した実験的な将棋エンジンです。リリース`0.4.5`では、
任意利用のHalfKP重み`nn_r6`を来歴・適格性の範囲とともに配布します。既定の探索方針と
駒得評価器は変更していません。`nn_r6`は実験用の任意資産であり、既定採用や一般的な
棋力向上を意味しません。

## まず動かす

```bash
cargo install sekirei
sekirei
```

最新のソースを実行する場合：

```bash
git clone https://github.com/kent-tokyo/sekirei.git
cd sekirei
cargo run --release -p sekirei
```

生成された`sekirei`をUSI対応GUIへ登録します。`EvalFile`を指定しない場合は
駒得評価を使います。外部HalfKP 256x2-32-32では`FV_SCALE`を使い、そのファイルのライセンスに
従ってください。対応形式、SHA-256、測定範囲は
[NNUE重み](docs/nnue_weights.md)にまとめています。`nn_r6`は記録済みの固定ソース
におけるネイティブ1T・4Tのmaterial gateに合格していますが、公開artifactの証跡が
strict validatorを通るまで配布しません。既定評価器にはせず、外部レートや今回の
リリースでの棋力向上を示すものでもありません。

## 構成

- 将棋ルール、SFEN/USI表記、alpha-beta/PVS、静止探索、手順序、lock-free TT。
- 任意の投機探索、Lazy SMP、NNUE学習・評価、MCTS、df-pn。実験機能は
  棋力を保証しません。
- CSA/FloodgateとローカルUSI対局のツール。コアの探索・評価に`unsafe`を
  使っていません。

| コマンド | パッケージ | 用途 |
|---|---|---|
| `sekirei` | `sekirei` | USIエンジン |
| `sekirei-csa` | `sekirei-csa` | CSA/Floodgateクライアント |
| `sekirei-match` | `sekirei-match-runner` | USI対局ランナー |
| `train` | `sekirei-train` | NNUE学習 |

USIの`usi`で全オプションを確認できます。`SearchMode=Auto`は1ワーカーで
逐次探索、複数ワーカーでLazy SMPを選びます。決定論的な診断では
`Threads=1`、`SpecTopN=0`を使います。

### ブラウザ / WebAssembly

[`sekirei-wasm`](crates/sekirei-wasm/README.md)は、SFEN解析、合法手、
検証付き着手、呼出側が渡す任意評価器を使う上限付き1ワーカー探索、最大4本の
MultiPV、最短詰み解析を提供します。ビルド済みのv0.4.5 ESモジュールパッケージは
[GitHub Release](https://github.com/kent-tokyo/sekirei/releases/download/v0.4.5/sekirei-wasm-0.4.5.tgz)
から取得できます。

## ビルドと検証

```bash
cargo build --release
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
bash scripts/check_rust_coverage.sh
python3 scripts/check_release_metadata.py --allow-planned-release-manifest
python3 scripts/check_release_manifest_layout.py
```

現行リリースのmanifestだけをリポジトリ直下に置き、検証済みの過去証跡は
[manifestアーカイブ](docs/releases/manifests/README.md)に保存します。

カバレッジ確認には`cargo-llvm-cov 0.8.7`、`llvm-tools-preview`、`jq`が
必要です。再利用可能なRustコードと配布するUSI実行系の行カバレッジを98%以上に
固定します。CIではRust/Cargo関連の変更、main更新、リリースタグで実行し、文書だけ、
または通常のスクリプトだけを変更するPRでは省略します。ベンチマーク、ゲート、
リリース、学習の各ツールは
[スクリプト索引](scripts/README.md)を参照してください。公開ライブラリ
`sekirei-core`のAPIは、マージ前と公開直前にv0.3.73との互換性を確認します。
0.4へのバージョン更新だけを理由に互換性の破壊を許可しません。処理単位の時間は
総合速度や棋力順位を示しません。

専用のPGOバイナリは、Apple M4の実測NPSで、固定したやねうらおうビルドを
**8Tで4.65%**（95%信頼区間：+2.55〜+6.69%）、**4Tで7.1%**
（同：+5.1〜+9.0%）上回りました。[8Tの測定条件と証跡](docs/experiments/compiler_pgo_8t_v0.4.2/README.md)と
[4Tの測定条件と証跡](docs/experiments/compiler_pgo_4t_v0.4.1/README.md)は独立した測定です。
通常のCargoビルドには、これらのプロファイルを適用していません。
v0.4.4の[自前HalfKPでプロファイルした別候補](docs/experiments/own_halfkp_pgo_8t_v0.4.4/README.md)も、
固定条件の8T比較で**+3.09%**（95%信頼区間：+0.76〜+5.40%）でした。

## 対局と学習

ローカルUSI対局には`sekirei-match`を使います。記録を残す自己対局では、
重みと開始局面を明示します。

```bash
python3 scripts/run_local_selfplay.py --games 1000 \
  --weights /path/to/weights.bin \
  --positions data/gate/openings_standard.sfen
```

実行結果はGit管理外の`data/runs/`に保存します。同一エンジンの自己対局は
学習・回帰用であり、Floodgateや人間のレートではありません。`sekirei-csa`の
認証情報は実行時だけ渡してください。

NNUE学習の引数は`cargo run --release -p sekirei-train -- --help`で確認できます。
学習側は`lineprior 0.12.6`と`shogiesa-core 0.11.6`を固定し、外部ラッパーは
`shogiesa 0.11.6`で確認しています。データと重みはGit外に保存します。
レシピ、ハッシュ、再開条件、対局単位の分割条件は
[スクリプト索引](scripts/README.md)にまとめています。

定跡はビルド時・実行時とも明示的に有効化します。

```bash
cargo build --release -p sekirei --features opening-book
# 続いてBookFileを設定し、UseBook=trueにします。
```

既定の定跡は同梱しません。定跡生成と学習では、入力・設定・出力を確認できる
ハッシュ付きマニフェストを保存します。

## 文書

- [文書索引](DOCUMENTATION.md)
- [変更履歴](CHANGELOG.md)と[過去の詳細履歴](CHANGELOG_ARCHIVE.md)
- [NNUE重みとライセンス](docs/nnue_weights.md)
- [スクリプト・検証ツール索引](scripts/README.md)

実装、テスト、測定、公開成果物は別の根拠です。過去の実験文書は、明示的に
再検証しない限り、記録された版と条件にだけ適用します。

## ライセンスと帰属表示

ソースコードは[MIT](LICENSE-MIT) OR [Apache-2.0](LICENSE-APACHE)です。
[NOTICE](NOTICE)の帰属表示を保持してください。NNUE重みはCC BY 4.0の別成果物です。
詳細は[NNUE-LICENSE.md](NNUE-LICENSE.md)を参照してください。`NOTICE`の表示例は
推奨であり、追加の広告条項ではありません。許可なく公式・承認済みと誤認させる
表示をしてはいけません。
