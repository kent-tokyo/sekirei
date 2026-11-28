# ベンチマークの使い分け

測定の入口をこの文書にまとめます。過去の未完了試験は実行待ちの予定ではありません。
以下は測定方法と保存済みの結果への案内です。

## 測りたいものから選ぶ

| 対象 | ツール・手順 | 得られるもの |
|---|---|---|
| 手生成、盤面更新、評価器などの部品 | [sekirei-bench](../crates/sekirei-bench/README.md)、[component capture](../scripts/run_component_benchmark.py) | 同じ処理範囲の時間、ばらつき、実行ファイルとソースの識別情報 |
| 探索の手・得点・ノード数 | [固定深さA/B](../scripts/run_fixed_depth_ab.py)、[対の探索診断](../scripts/paired_search_diagnostics.py) | 局面ごとの変化。ノード減少だけでは棋力向上を示さない |
| 同時間の探索 | [対の探索診断](../scripts/paired_search_diagnostics.py) | 固定条件での時間と探索結果。混雑時の診断は速度の比較から分ける |
| 棋力 | [標準の棋力テスト](strength_testing.md)（[A/B対局](../scripts/run_ab_match.py)） | 変更の採否（SPRT）と版ごとのYaneuraOuゲージ |
| 棋譜解析の品質 | [解析記録の仕様](amateur_analysis_benchmark.md) | 候補手の一致、評価の安定性、未完了率、遅延。公開済みの集計結果はない |
| PGXとの環境操作比較 | [PGXアダプター](../crates/sekirei-bench/README.md#pgx-environment-adapter) | 合法手マスクと1手更新のCPU処理時間。探索や棋力は測らない |

各Pythonツールの引数は `python3 scripts/<tool>.py --help` で確認できます。
実行前に比較する入力、判定、時間・スレッド・メモリ・標本数の上限を決め、
[資源確認](../scripts/gate_resource_preflight.py)と使用中のジョブを確認します。

## 比較に残す情報

ソースと実行ファイルの版・ハッシュ、ビルド設定、評価方式と重みのハッシュ、
局面集合、CPU、Threads、Hash、探索上限を記録します。計時範囲を揃え、
失敗・未完了・中断を結果から消さずに残します。逐次探索の診断は原則
`Threads=1`、`SpecTopN=0`。並列探索を調べる場合は同一バイナリの反復も確認します。

部品測定の保存先には `provenance.json`、`samples.csv`、
`validated_summary.json` と凍結した実行ファイルが入ります。既存の出力先を
上書きしません。新規captureはclean checkoutが既定で、`--allow-dirty` は
作業中の差分を含む診断用です。測定条件の詳細は各ツールと試験記録を参照してください。

速度、候補手の品質、対局成績は別に判断します。NPS比からEloを換算したり、
4Tの結果を8Tへ適用したりしません。棋力に影響する変更の採否は
[標準の棋力テスト](strength_testing.md)だけで決めます。

## 保存済みの結果

以下は記録された版と条件での結果です。現行版の総合性能表には読み替えません。

| 記録 | 読む際の範囲 |
|---|---|
| [部品・他ライブラリ・MCTS](experiments/benchmark_reports/README.md) | 2026-09の測定と訂正。無効行、混雑時の値、A/Aのばらつきを区別 |
| [固定深さA/Bの履歴](experiments/fixed_depth_gate_run_index.md) | 投機探索の非決定性と測定手順の修正 |
| [PGO 4T](experiments/compiler_pgo_4t_v0.4.1/README.md)、[PGO 8T](experiments/compiler_pgo_8t_v0.4.2/README.md) | 特定ビルドの比較。通常のCargoビルド全般には適用しない |
| [自前HalfKP PGO v0.4.4](experiments/own_halfkp_pgo_8t_v0.4.4/README.md) | 固定重み・8Tの結果 |
| [自前HalfKP PGO v0.5.1](experiments/own_halfkp_pgo_8t_v0.5.1/README.md) | 速度改善は未確定、候補は不採用 |
| [11月14日のPGO診断](experiments/strength_first_progress_2026-11-14/README.md) | 固定した旧ソースの1T速度確認。4Tは不進行、棋力は未確認 |
| [v0.5.4のPGO診断](plans/evaluation-disagreement-2026-11-16.md) | ホスト条件不通過。探索的な速度観測のみ |
| [r6重みの対局資格](../weights/releases/README.md) | 固定した1T・4Tで駒得評価との独立対局。外部レートや現行版全般の棋力は示さない |

未実施の続きは自動再開しません。過去の入力・結果・判定はそのまま保存します。

## YaneuraOuゲージの推移

[標準の棋力テスト](strength_testing.md#2-版ごとの進捗yaneuraouゲージ)の結果を版ごとに追記します。

| 版 | コミット | 対局数 | 勝率 | Elo差（95%区間） | 備考 |
|---|---|---|---|---|---|
| v0.5.5 + r6 | `e1037a7`（v0.5.5のエンジンと同一） | 200（100組） | 61.25%（122勝1分77敗） | +79.5（±49.7） | 2026-11-28、Apple M4・3並列、`NodesLimit=160000`、gauge.sfen、1手1秒。組単位は30勝62分8敗（+77.7±41.4）。同条件の1回目は99組で中断したため除外 |
