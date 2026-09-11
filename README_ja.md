# Sekirei — Rust製将棋エンジン

[![CI](https://github.com/kent-tokyo/sekirei/actions/workflows/ci.yml/badge.svg)](https://github.com/kent-tokyo/sekirei/actions/workflows/ci.yml)
[![crates.io](https://img.shields.io/crates/v/sekirei.svg)](https://crates.io/crates/sekirei)
[![License](https://img.shields.io/crates/l/sekirei.svg)](https://github.com/kent-tokyo/sekirei/blob/main/LICENSE)

[English](README.md)

Sekirei は Rust で実装した実験的な将棋エンジンです（現在のリリース: `0.3.34`）。USI、CSA/floodgate クライアント、
USI 対 USI の棋力テスト、NNUE スタイル評価に対応しています。棋力と評価品質は開発中で、
ここでは絶対レーティングや他エンジンを上回るという主張はしていません。

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
cargo run --release -p sekirei
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
- ロックフリー置換表、任意で有効化できる投機的並列探索、opt-inのLazy SMP探索。
- 実験的なroot-level MCTS pilotと、深さ・ノード上限付きのopt-in bounded df-pn API。
- ファイルから読み込むNNUEスタイルの差分評価。
- 外部YaneuraOu系SFNNのヘッダー検査と来歴・互換性manifest検証。外部重みの推論対応そのものはまだ別課題です。
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
手順序付け・枝刈りの各種ヒューリスティック、任意の投機的探索、opt-inのLazy SMP探索を実装しています。
`SpecTopN=0` で投機的探索を無効にできます。特異延長の検証探索は無制限の置換表書き込みから
除外しており、部分的な検証結果が親ノードの再利用可能なエントリを上書きしないようにしています。
root-level MCTSとbounded df-pnは研究用のopt-in APIで、標準USIモードには接続されておらず、棋力の証明ではありません。
限定的な詰み探索を試す場合は、USIオプション`SearchMode=Dfpn`を指定できます。`depth`を
深さ上限として使う実験用モードであり、通常の対局モードや棋力比較には使用しません。

## ビルドとテスト

```bash
cargo build --release
cargo test --release
cargo bench --bench movegen -p sekirei-bench
```

### ローカル性能スナップショット

v0.3.27のホットパス最適化により、開発用Macのstartpos中央値は、合法手生成が
8.2711 usから2.2151 us、Perft(3)が9.2530 msから2.1082 ms、深さ4探索が
22.544 msから7.659 msになりました。探索の長い方の確認では20サンプルを使用しています。
これは異種Apple CPUコア上のローカルな機構診断であり、他環境での性能、棋力、Eloの主張ではありません。

固定した`rsshogi`コミットおよび`shogi_core`の基礎処理と比較する軽量診断は次で実行できます。

```bash
cargo run --release -p sekirei-bench --bin cross_library
```

合法手生成とPerft(3)は`rsshogi`と同じ局面で比較します。`shogi_core`は合法手生成器を
持たないため、局面更新の行は合法手生成やPerftの比較ではありません。結果と比較範囲は
`scripts/benchmark_reports/cross_library_v0.3.33.md`に記録しています。

レポート記載のv0.3.33最適化スナップショットを10,000反復x7標本で測定した結果、開始局面の
Perft(2)は6.455 us対8.008 us、Perft(3)は192.399 us対250.672 us、持駒あり中盤局面の
Perft(2)は31.900 us対51.440 usで、固定した`rsshogi`よりローカルでは高速でした。
単発の合法手生成と全状態do/undo診断は引き続き`rsshogi`が高速なため、これは限定した
Perft結果であり、速度全般の首位主張ではありません。
同じ候補をcleanなrevision `4c568b1`と比較したCriterion深さ4探索では、中央値が
3.348 msから2.265 msへ短縮され、約1.48倍高速でした。

プロセス全体の重みを変更せずに NNUE チェックポイントを確認する場合:

```bash
cargo run --release -p sekirei-bench --bin nnue_probe -- /path/to/weights.bin
# 自動処理では --json、任意局面では --sfen "<SFEN>" を繰り返し指定
```

評価値、スコアレンジ、平均、分散、基準局面との差分を表示します。標準プローブには駒得と王位置の感度検査も含まれます。`--json` で機械可読形式にでき、`--strict` ではスコアレンジが 8 cp 未満の定数・準定数出力、駒得または手番への感度不足、または再読込非決定を異常終了にできます。
出力分散の確認にも使えます。このプローブは診断用であり、棋力テストではありません。
チェックポイントは `nnue_probe` や `EvalFile` で読み込める推論互換形式です。推論用`.bin`は
オプティマイザ状態を持たず、訓練用にはAdam sidecarと完全resume sidecarを別に保存します。
JSON出力には判定閾値 `strict_min_range_cp` と判定結果 `strict_pass` も含まれます。
さらに `l2_distinct_values`、`l2_bias_distinct_values`、
`out_distinct_values` で後段層の定数化を直接確認できます。

外部SFNN成果物は2段階で扱います。Rustの`sekirei_core::external_eval::read_sfnn_header`と
`scripts/inspect_external_sfnn.py`は安全上限付きのヘッダー検査だけを行います。出典、ライセンス、
再配布条件、adapter状態は次で検証できます。

```bash
python3 scripts/validate_external_eval_manifest.py \
  scripts/fixtures/external_eval_manifest_v1.json
```

manifestが有効でも、外部ファイルの推論互換性は保証しません。特徴量対応、パラメータ配置、
数値一致は別途adapter検証が必要です。

マテリアル評価で起動:

```bash
cargo run --release -p sekirei
```

NNUE 重みを指定して起動:

```bash
cargo run --release -p sekirei -- /path/to/weights.bin
```

USIループを開始せずにバージョンを表示:

```bash
cargo run --release -p sekirei -- --version
```

簡単な使い方は `--help` で表示できます。

## USI オプション

`usi` コマンド後に全オプションを表示します。主なものは次の通りです。

- `Hash`, `Threads`, `MoveOverhead`
- `SearchMode`（デフォルトは`Speculative`、任意で`LazySMP`）
- `Ponder`, `MultiPV`
- `EvalFile`（`isready` 時に読み込み）
- `SpecTopN`（デフォルト `3`、`0` で無効）
- `UseBook`, `BookFile`, `BookMaxPly`, `BookMinConfidence`

`SpecTopN > 0` では投機タスクのスケジューリングにより、同一条件でも探索結果が変わる
場合があります。再現性を優先する比較では、可能な限り `SpecTopN=0` を使用してください。正しさの診断では
`Threads=1`、`Parallel=1`、`SpecTopN=0` を固定し、速度測定や対局結果とは分けて記録します。

`SearchMode=LazySMP`では、`Threads`が独立worker数を指定します。各workerは局面とheuristic
tableを専有し、ロックフリー置換表と停止flagだけを共有します。このモードはopt-inで、
デフォルトは`SearchMode=Speculative`です。

## CSA / floodgate

```bash
cargo run --release -p sekirei-csa -- \
  --user <name> --trip <secret> --game floodgate-300-10F \
  --record-dir data/floodgate --loop
```

`FLOODGATE_ACCOUNT` と `FLOODGATE_TRIP` も利用できます。認証情報、棋譜、重み、訓練データを
コミットしないでください。対局終了後の棋譜と途中切断時の部分棋譜は、既定で
`data/floodgate/`にCSA形式で保存します。`--record-dir`で保存先を変更できます。各着手後に
flushするため、保存に失敗しても対局自体は中断しません。

事後分析用に `--analysis-dir <dir>` を指定すると、CSA棋譜に対応する
`*.analysis.jsonl` サイドカーを保存します。ヘッダにはエンジンバージョンと評価値の視点を記録し、各探索について、探索前SFEN、選択したCSA手、
評価値、完了深さ、ノード数、経過時間、hashfullを `sekirei.analysis-record.v1` として記録します。
終了局では最後に `game_end` と結果を記録し、通信異常などで途中終了した場合も
`aborted` を付けて未完了のまま残さないようにします。終了時はバッファをflushし、ファイルを同期します。
replay分析では、この結果とCSA末尾の結果マーカーも突合します。
デフォルトは無効で、これは診断記録であり棋力の証明ではありません。CSA本文は変更しません。
生成したサイドカーは `python3 scripts/validate_analysis_record.py <file>.analysis.jsonl` で検証できます。
完全なサイドカーには `game_end` が必須で、途中で切れた記録は有効な対局分析として扱いません。
`--analysis-dir` 有効化前に作成したCSA棋譜には探索評価値が含まれないため、後から評価ログとして扱うことはできません。
対応するCSA棋譜と突合して評価値の急落候補を出すには、
`python3 scripts/analyze_analysis_record.py <game>.csa <game>.analysis.jsonl --output swing.json` を実行します。
これは指し手列のreplay突合による診断であり、再探索や棋力の証明ではありません。
複数局を結果別に集計し、評価値の急落を順位付けするには
`python3 scripts/summarize_analysis_swings.py data/reports/*.json --output swing-summary.json` を使います。
Floodgateの保存ディレクトリ全体を処理するには
`python3 scripts/analyze_floodgate_analysis.py --csa-dir data/floodgate --analysis-dir data/floodgate-analysis --output floodgate-analysis.json` を使います。
v1以前のサイドカーは `legacy_analysis` として明示的に除外され、壊れたv1サイドカーも集計されません。
mate相当スコア（絶対値800,000cp以上）や終局記録は `terminal_records` に分離し、通常局面の
評価反転件数（`normal_swings`）には含めません。
通常反転は負方向（`negative_swings`）と正方向（`positive_swings`）にも分けます。
各候補には手数比による `opening` / `middlegame` / `endgame` のフェーズも付きます。
さらに探索深さ（`shallow` / `medium` / `deep`）と経過時間（`fast` / `normal` / `slow`）を付け、
探索不足による揺らぎと評価器由来の揺らぎを切り分けます。
これらの分析fixtureは診断契約の回帰チェックとしてCIでも実行されます。
負方向の候補をSFENから駒得差・手数フェーズ・探索条件付きで抽出するには
`python3 scripts/classify_swing_positions.py floodgate-analysis.json --output negative-swings.json` を使います。
候補には直前の評価値とplyも含まれるため、崩れ始めた手の直前局面へ遡れます。
棋譜から直前2手（`previous_move_csa` / `two_moves_back_csa`）も付与します。
候補SFENのNNUE/material比較は `cargo run --release -p sekirei-core --bin compare_eval -- <weights> "<sfen>"` で実行できます。
CIでは合法なCSA着手の適用と、着手前SFENの一致もfixtureで確認します。
盤面状態まで検証するには `sekirei-analysis-replay` をビルドし、CSA棋譜とサイドカーを渡します。
違法手やSFEN不一致があれば失敗します。

クライアントは`startpos`を仮定せず、サーバーが送るCSA局面を読み込みます。標準的な`PI`省略表記と
持ち駒宣言に対応し、受信した指し手の色と内部手番も検証します。サーバーが中間通知として
`#RESIGN`を送る場合も、後続の`#WIN`・`#LOSE`・`#DRAW`まで待って最終結果を記録します。
これにより、プロトコル・盤面同期の失敗と評価値による敗勢を区別できます。

boundedまたは中断された補助探索が詰み相当のスコアを返しても、それだけでは自動投了しません。
合法手が残っていれば合法手へフォールバックするため、時間制限や未確定探索が誤投了として記録されることを防ぎます。

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

教師探索の葉評価はデフォルトで駒得評価です。固定教師による自己蒸留実験では、変更しない
checkpointを1つ指定します。

```bash
cargo run --release -p sekirei-train -- \
  --games /path/to/csa_dir --output student.bin --epochs 3 \
  --teacher-eval nnue --teacher-weights teacher.bin
```

教師重みのhashはteacher cache、完全resume fingerprint、checkpoint metadataに記録されます。
別の教師で作成したcacheやresume checkpointは、ラベル生成元を黙って混在させず拒否します。
このオプションは実験条件を固定するものであり、それ自体は棋力向上の証拠ではありません。

固定深さのラベル生成に極端な外れ値がある場合、`--label-time-ms N`でcache missごとの教師探索に
wall-clock上限を設定できます。この上限もcache identity、resume fingerprint、checkpoint metadataへ
含まれるため、上限付きラベルと無制限ラベルが混在することはありません。
単一threadの再現可能なラベル生成では`--label-nodes N`を優先してください。host負荷に依存しない
決定論的node上限を設定し、同じcache／resume／metadata契約へ含めます。

訓練データ、チェックポイント、重み、対局結果、実験ログはローカル生成物として公開リポジトリ
から除外しています。プロジェクトで生成したNNUE重みはCC BY 4.0で別途ライセンスします。
詳細は[NNUE-LICENSE.md](NNUE-LICENSE.md)を参照してください。

epochごとのチェックポイントには、raw f32重み、Adamのモーメント、optimizer stepを含む
訓練専用の`.adam.json` sidecarも保存されます。`--resume-adam`でこの状態を復元できます。
推論用`.bin`は分離され、エンジン互換形式のままです。診断分類は元のmanifestを変更せず、
release manifest形式のコピーへ追加できます。

```bash
python3 scripts/classify_evaluator_failure.py diagnostic.json \
  --manifest release-manifest-v0.3.33.json \
  --output release-manifest-v0.3.33-diagnostic.json
```

実運用fixtureと生成物は `python3 scripts/validate_release_manifest.py
scripts/fixtures/release_manifest_diagnostic_v1.json` でschema検証できます。完全resumeは
`--resume-checkpoint`を使うと、raw重み、Adam状態、完了epoch、データカーソル、学習設定fingerprintを
epoch境界で復元し、設定不一致を拒否します。`--resume-checkpoint-every-games N`を指定すると、
CSAではゲーム境界、positions modeでは位置chunk境界でもatomicに保存します。teacher cacheも含めるため、
再開時にラベル生成条件が黙って変わりません。
小規模なCLI統合回帰は `bash scripts/test_resume_cli_fixture.sh` で実行できます。
resume検証の系譜は `python3 scripts/record_resume_run.py --checkpoint run.resume.json --log run.log --dataset data.jsonl --output resume-manifest.json`
で記録できます。生成物は `sekirei.resume-manifest.v1` schemaで、checkpointとログのhashを分けて保持します。
検証済みresume証跡をrelease manifestのコピーへ接続するには、`python3 scripts/attach_resume_manifest.py --release-manifest release-manifest-v0.3.33.json --resume-manifest resume-manifest.json --output release-manifest-with-resume.json`を使います。元のrelease manifestは変更しません。

保存したSharedMcts transcriptと診断manifestの整合性を確認するには、`python3 scripts/verify_mcts_diagnostic.py --manifest candidate-manifest.json --transcript shared-mcts-transcript.txt`を使います。schemaと3つの診断カウントを確認しますが、強さの主張は行いません。
CIでは保存後にartifactを別jobで取得し、同じschema・整合性検証を再実行します。

3つの合法局面（開始局面、自然なcommuting move局面、進行局面）で2つのMCTS pilotを固定予算で比較するには、`cargo run --release -p sekirei-core --example mcts_fixed_budget_diagnostic`を実行します。既定は64 simulations・depth 4です。`--simulations 8 --max-depth 2`で小さい予算も実行でき、各予算を別の検証済み比較manifestへ記録します。再現性の診断であり、強さの主張ではありません。
この固定予算ログはSharedMctsのCI診断artifactにも保存し、artifact-audit jobで再確認します。
CIでは局面別比較も検証済みmanifestコピーへ記録します。このコピーは診断専用で、release artifactではありません。
比較manifestは`python3 scripts/summarize_mcts_comparison.py candidate-comparison-manifest.json`で要約できます。node削減率と一致分類（exact／best_move_only／divergent）だけを出力し、`strength_claim`は常に`false`です。
CIでは大・小2つの予算の要約JSONを比較artifactに保存し、JSON形式と強さ主張なしの分類を再確認します。
複数予算のmanifestは`python3 scripts/aggregate_mcts_summaries.py small-manifest.json full-manifest.json --output budget-summary.json`で1つに集約できます。
CI artifactには個別要約とともに、この集約JSONも保存します。
接続後の `resume_verification.artifacts` にはcheckpointと実行ログを別artifactとして記録します。

atomicなcheckpoint境界で意図的に停止して再開する例:

```bash
cargo run -p sekirei-train -- --positions positions.jsonl --epochs 20 \
  --checkpoint-dir checkpoints --output weights.bin \
  --resume-checkpoint-every-games 1000 --stop-after-resume-checkpoint
cargo run -p sekirei-train -- --positions positions.jsonl --epochs 20 \
  --output weights.bin --resume-checkpoint weights.resume.json
```

resumeは、未対応schema、optimizer状態の欠落・形式不正・非有限値、学習設定fingerprint不一致、
現在のepochを超えるカーソル、`--resume-adam`との同時指定、完了済みepoch以下の目標epochを拒否します。

## ライセンスと帰属表示

Sekireiのソースコードは、利用者の選択によりMIT LicenseまたはApache License, Version 2.0で
ライセンスします。[LICENSE-MIT](LICENSE-MIT)または[LICENSE-APACHE](LICENSE-APACHE)を参照してください。
[NOTICE](NOTICE)に記載された著作権表示と帰属表示を保持してください。

Sekireiをベースにした製品での推奨表示：

```text
This product is based on Sekirei,
an open-source shogi engine developed by Kentaro Tanabe.

https://github.com/kent-tokyo/sekirei
```

製品にLegal Notices画面がある場合は、上記を表示例として利用できます。これは強く推奨する
表示ですが、標準ライセンスによる広告上の必須条件ではありません。許可なくSekireiの名前や
ロゴを使い、公式承認済みであるかのように示してはいけません。NNUE重みは別個の成果物として
CC BY 4.0でライセンスします。詳細は[NNUE-LICENSE.md](NNUE-LICENSE.md)を参照してください。

現在のrelease manifestは
[`release-manifest-v0.3.33.json`](release-manifest-v0.3.33.json)に保存しています。現行のLazy SMP
USI smoke transcriptは[`scripts/fixtures/usi_smoke_v0.3.33.txt`](scripts/fixtures/usi_smoke_v0.3.33.txt)です。
いずれもリリース監査用の証跡であり、棋力の主張ではありません。

リリース前には、コンパイルやエンジン実行を行わずに公開メタデータを確認できます。

```bash
python3 scripts/check_release_metadata.py
```

全crateのmanifest、`Cargo.lock`、CHANGELOG、英日README、ライセンス・帰属表示ファイルが
現行バージョンと一致することを検査します。
