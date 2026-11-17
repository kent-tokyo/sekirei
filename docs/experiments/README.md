# 実験記録

ここには事前登録、条件、ハッシュ、測定値、判定を残す。
2026-11-16の[計画リセット](../plans/README.md)で対象になった
未完了作業・継続予定は終了した。旧「次の作業」を自動実行しない。

## 主な記録への入口

| 調べたい内容 | 記録 |
|---|---|
| 最近の強化経路と不進行判定 | [r6以後の候補](nnue_r6_progress_2026-11-13/README.md)、[探索・PGO・時間配分](strength_first_progress_2026-11-14/README.md) |
| Floodgateの5敗と手選択 | [初回](floodgate_v054_r6_rescue_2026-11-16/RESULTS.md)、[再分析](floodgate_v054_r6_rescue_2026-11-16/REANALYSIS.md)、[CSA](floodgate_v054_r6_rescue_2026-11-16/CSA_CHOICE.md) |
| 公開詰み問題の証明・通常探索 | [11月14日](public_tsume_2026-11-14/PLAN.md)、[11月15日](public_tsume_2026-11-15/PLAN.md) |
| 公開「次の一手」 | [索引と最終状態](issue289-published-next-move-2026-11-15.md) |
| 古いコンポーネント・他ライブラリ比較 | [ベンチマーク報告一覧](benchmark_reports/README.md) |
| 固定深さゲートの履歴 | [実行索引](fixed_depth_gate_run_index.md) |
| 公開NNUEの初期検討 | [事前登録](official_nnue_v1_preregistration.md)、[検証損失と対局の監査](validation_metrics_vs_gate_elo_audit.md) |

その他のディレクトリは候補名・当時のバージョンで探す。
現行の測定手順は[ベンチマーク](../benchmarks.md)、重みの利用条件は
[NNUE重み](../nnue_weights.md)を参照する。

原本は当時の条件と判定を検証できるよう保持する。
未使用の確認局面、入力の除外範囲、独立採用の条件も維持し、
終了した試験を増量・合算して合格へ変更しない。
