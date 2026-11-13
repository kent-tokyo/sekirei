# 棋力向上経路の実行記録（2026-11-14）

[実行計画](../../plans/strength-first-2026-11-14.md)の優先順位を使う。過去のr6超え新重み試行は[別記録](../nnue_r6_progress_2026-11-13/README.md)に保持し、その判定は変更しない。私有重み・生成棋譜・未資格の実測rawはGitへ入れない。

## S00: r6のローカル起動確認

公開カードのr6 SHA-256は`634bdf527e7306045c8278df1d5311777561dfb09c1ebe6829ff3f4d883cc469`。このMacのローカル重みを再ハッシュし、完全一致した。現在の`target/release/sekirei`はSHA-256`272b88abd9726572ecc43587f15e67f731b1605cd697be1ea93b7b3a23ef12f6`で、Git HEADとのソース同一性は未証明。`Threads=1`、`SpecTopN=0`、`EvalFile`に上記ファイル、`FV_SCALE=24`を設定して`isready`を送ると、`info string FV_SCALE 24`、`NNUE weights loaded from ...`、`evaluator format HalfKP 256x2-32-32`、`readyok`を確認した。対局と探索は0回。

過去のFloodgate試験の終了receiptはr6使用と10試行完了を記録するが、現在の常用GUI設定や実行中サービスを証明しない。対象の常用設定はまだ特定していない。ローカルの明示起動ならr6は使用できる。既定の材料評価や配布設定は変更していない。

## S01: ソース同一PGOの事前登録

**問い:** 現行の清潔なソースを1TでPGOビルドすると、r6を明示ロードした同時間探索の有効仕事が増えるか。旧ソースのPGO結果は事前の根拠であり、今回の判定へ合算しない。

**入力と隔離:** この計画を記録したGit commitを清潔な隔離worktreeにcheckoutし、`scripts/build_pgo_lane.py`をそのまま使用する。訓練局面は追跡済み`pgo_native_v0.4.1/compiler-profile-training.sfen`の先頭32件。PGO訓練は材料評価、1T、各500ms、`Hash=64`。現行runnerはr6のrelease cardを訓練用candidate cardとして受けないため、ツール改変やカードの偽装を行わない。通常版とPGO版は同一ソース・同一最適化設定、PGOフラグだけ異なる。

**資源上限:** 開始前に他のengine/compiler/training job、メモリ圧迫、空き容量を確認する。`CARGO_BUILD_JOBS=1`、訓練も1Tで逐次実行。全工程の壁時計上限10分、生成物上限3GiB、開始時空き6GiB以上。上限やtoolchain不備、source汚染があれば`RESOURCE_OR_INPUT_STOP`として終了し、同じ結果を求める再実行はしない。通常の60秒診断上限を超えるコンパイルは、この登録された一工程に限る。

**最小の検証と進行:** ビルドが完了したら、訓練とは別の追跡済み`own_halfkp_pgo_8t_v0.4.4/own-speed32.sfen`先頭16件をr6・`FV_SCALE=24`・1Tで使用する。まず通常版とPGO版の固定深さ10で返却手、score/bound、PV、nodes、完了深さの同一性と合法性を照合する。差異・ロード失敗があれば停止。次に同じ16局面で300msのA/B/B/Aを1組ずつ、1T・`Hash=64`・book無効で測る。全位置の完了depth、返却手の変化、wall NPS比、外れ値、ホスト負荷を保存する。PGOの速度進行条件は、適格な16位置の幾何平均wall NPS比が1.03以上、位置単位bootstrap 95%下限が1.00超、かつ完了depthの悪化が4位置以下。ホストの厳密なタイミング隔離ができない場合は探索的結果とし、速度進行判定を出さない。初回の結果を見て閾値や局面を変えない。

この試行はビルド・正しさ・速度の開発判定まで。速くても棋力向上の実証には新しい開始局面での別登録対局が必要で、既定採用や公開は行わない。外部エンジン・Suisho5の評価やラベルは候補選択へ使わない。
