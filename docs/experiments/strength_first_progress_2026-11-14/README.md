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

### S01実行結果と停止

隔離source `220a3eee35de0e79301b8a59d6d1149fccaf6629`で、通常版・計測用版・profile採取32件・PGO版の全工程が完了。`CARGO_BUILD_JOBS=1`、訓練1T、材料評価500ms。工程全体は10分以内、出力は約129MiB。manifest SHA-256 `e64fe7b70ff9dcbba76edef8ca9883cc48aa522c92aea1c203db625d138c5375`、通常版binary `aca23749f974d6ab81a5e6dcd03937b1386322ef8bbf2de58b95c095b2fcb206`、PGO版 `f03b3da9fd2f9606e604b6dd637cbbb239967230bbcffb9869533cffbae1e2f0`。両者はr6を`FV_SCALE=24`で16/16回明示ロードした。

別の16局面・深さ10は、両側16/16で返却手、score、PV、nodes、完了深さが一致し、全32プロセスが`quit`後exit0・reaped。固定深さrawのSHA-256は通常版`b1add6e571fc7846365c0afc1a65d25476d98243d4269f62871326d11a0891ac`、PGO版`7fd375c8ed0d816fbc042ad20a36afa182311d75989edff1af8a61feae5121f2`。このrunnerはPVの合法手再生とscore boundの照合を行わないため、その2点は未確認。固定深さの短い経過時間を速度判定へ使用しない。

300msの16局面A/B/B/Aは64/64検索を完了し、r6ロード・正常終了を各行で確認。探索的な局面単位wall NPS比の幾何平均はPGO/通常`0.98858`、位置bootstrap 95%範囲`0.94642–1.02870`（seed`20261114`）。完了深さは改善1・悪化5・同等10局面。raw SHA-256 `72c04a054b5c29fb8bf09ad2757a674df5e8ad89b507f8bab674ffc29c624377`、集計SHA-256 `14cd0e9bdc334dad92c4a61a6ee80977a8775e7823cb74aff5a765c8db6747e5`。計測スクリプトSHA-256 `b11a349a5cbcea894c6af7f105210862d55cc2ebddb290828c426dae3ce89e9e`。結果を見た後に入力や閾値は変えていない。

**判定 `HOST_UNQUALIFIED_RESOURCE_STOP_S01`。** 連続したホスト負荷監視がなく、事後には別の100% CPU処理を確認した。したがって上の数値は資格ある速度優位・劣位のどちらも示さず、進行条件は成立しない。終了時のディスク空きは約543MiBで、登録した開始時6GiB条件を大きく下回った。その後約5.6GiBへ回復したが、まだ6GiB未満。試行出力約129MiBと隔離worktree約19MiBだけでは一時的な空きの約12GiB減少を説明できず、他作業を削除・停止しない。追加ビルド、タイミング試験、対局は停止。再開条件は、空き6GiB以上・他の高負荷処理なし・連続ホスト監視が可能な状態。今回のraw・通常版と候補binary・compiler profileは、GitHub対象外のローカル台帳`tasks/strength-progress-20261114`にSHA照合後保存した（同台帳commit `ffb566e`、`14f40fa`）。旧試行判定も保持し、同じ不適格結果を棋力・速度の採用証拠へ転用しない。

保存済みbinaryとprofile・rawを参照しないことを確認した後、この試行だけの再生成可能な`target-native`、`target-generate`、`target-use`（計約113MiB）は削除した。保存した成果物や他作業のファイルは削除していない。

## S02: 時間配分経路の適用可否（読取のみ）

次の低費用候補を選ぶため、実コードと旧対局条件を照合した。`crates/sekirei-usi/src/go.rs`は持ち時間・加算時間を受けるとsoft limitを設定する。`SpecTopN=0`の1T Autoは通常Searcherへ進み、その完了反復後の停止判定で`TM_MODE`を読む。一方、秒読みだけ・`movetime`ではsoft limitを設定しないため、そこで`TM_MODE`を比べても機構が働かない。

保存済みr6のFloodgate試験は持ち時間300秒・加算10秒で、機構の適用範囲に入る。旧材料評価・別ソースの`TM_MODE=1`対0は10秒＋0.1秒、410局で212勝181敗17分、推定+26±34 Elo、事前のSPRT境界に届かず未確定だった。現在のr6へその数値を移さない。S02は実装・検索・対局0回。次の試行は、ホスト資源が回復した後、実際にsoft limitが発動する短い持ち時間条件と新規局面を先に固定してから選ぶ。

## S03: 時間配分の小さな作用確認（事前登録）

S01の不適格なPGO時間測定は再判定しない。S02で適用経路を確認でき、空き容量が約13GiBへ回復したため、独立した`TM_MODE`の技術診断を行う。

**入力・条件:** S01と同じ清潔な隔離source `220a3eee35de0e79301b8a59d6d1149fccaf6629`を`--features tune`で1ジョブのreleaseビルドにする。局面は追跡済み`compiler_pgo_4t_v0.4.1/holdout32.sfen`（SHA-256 `a7edd63629414ce85f994167262994d8b3b4acf8bfbe400c938049b51eeb454a`）の先頭8件。材料評価、1T、`Hash=64`、`SearchMode=Auto`、`SpecTopN=0`、bookなし、`MoveOverhead=50`、`go btime 10000 wtime 10000 binc 100 winc 100`を固定する。同一バイナリでA=`T_TM_MODE=0`、B=`1`のみ変更。1局面A/B/B/Aの32探索、各回新規process・`usinewgame`・`isready`後に局面指定する。r6やSuisho5の評価は候補選択に使わない。

**資源・判定:** 実行前に他engine/compiler jobとメモリ・空きを確認。ビルドは1ジョブ、壁時計5分、生成物1GiB、開始時空き6GiB以上。診断は逐次1T、全体60秒、1呼出し最大2秒、出力32MiB。全32検索の合法な返却、正常終了、go送信からbestmoveまで1,000ms以下を先に確認。A/Bで停止時間か完了深さの差が1局面以上あれば`MECHANISM_ACTIVE`として別の品質評価を検討し、差がなければ`NO_EFFECT_IN_SAMPLE`で止める。手の変化や深さだけで棋力向上・採用とはしない。ホスト負荷の連続監視がなければ時間値は探索的な作用確認に限る。旧410局へ合算しない。結果を見て局面や閾値を変更しない。
