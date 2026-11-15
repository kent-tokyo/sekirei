# 公開上級詰将棋・現行 Sekirei 診断（2026-11-15）

## PT65: 未使用の双玉投稿を固定（結果を見る前の契約）

- 目的: 現行 `main` の詰み API と実戦用通常探索で、前回と異なる公開双玉問題から再現可能な探索課題を探す。問題ごとの診断であり、棋力率ではない。
- ソース: `https://tsumeshogi.com/tags/%E5%8F%8C%E7%8E%89?page=2`。取得済み HTML は 90,205 byte、SHA-256 `98cbb5fcbe1e001e2b2661df14e041f578c8bdcf5591d4f0559a368e8680f5a0`。JSON-LD の掲載順24件から、表題に半角数字で `13/15/17/19/21手詰` と明記された先頭最大4件を選ぶ。前回 PT56 の URL は除外し、その場合は後続を繰り上げない。
- 固定入力: 掲載7番 `https://tsumeshogi.com/problems/y_lmsyrb84`（13手）、9番 `https://tsumeshogi.com/problems/2frbor958v`（17手）、24番 `https://tsumeshogi.com/problems/pts7ssn-tp`（15手）の3件。各ページを1回取得し、HTML の `data-sfen` と掲載手数、URL、HTML SHA-256を保存する。`K` と `k` 各1枚を確認。取得上限は各20秒・1MiB、合計80秒。構造変更・パース失敗は技術停止し、差し替えない。
- 詰み API: 現行クリーン source `f84aa55d197d4b4cd4b377f80a2757b9e8654bfc` から release core をビルドし、既存 `scripts/public_tsume_mate_probe.rs` をリンクする。1T逐次、各掲載手数を最大深さ、100万節点、外側4秒、出力各1MiB。`Mate valid=true aborted=false` のみ全合法応手に対する証明、`Unknown` は予算内未解決。掲載手数の相違だけで欠陥と断定しない。異常終了は停止。上限変更・題の入替なし。
- 通常探索: 完了証明かつ初手1手の全件のみ、同じ source の release USI で default material と公開 r6 NNUE を別々に1冷 processずつ実行する。1T、Hash64、book無効、Speculative/SpecTopN0、`go nodes 1000000 movetime 1000`、外側4秒、出力各1MiB。r6 は重み SHA-256、FV_SCALE24、absolute のロード ACK を記録。返却手の合法性・王手・mate score と PV の合法終局を区別する。別手が非王手なら詰将棋と通常探索の目的関数差として記録。王手の別手かつ非mate表示なら既存 #280 と重複・再現性を調べ、該当時に issue へ追記する。正常終了を確認し、二条件を混ぜた棋力主張はしない。
- 資源: ビルドは1 job、探索も逐次。ビルド外側180秒、探索は最大12冷 process・総外側60秒、出力合計12MiB以内。開始時の空き28GiB、VM free約1.4GiB。OSのプロセス一覧は許可されず取得不可のため、同時実行を作らず異常な負荷を観測したら停止する。

## 実行結果

2026-11-15に[GitHubのmain commit API](https://api.github.com/repos/kent-tokyo/sekirei/commits/main)をHTTP 200で読み、`sha=f84aa55d197d4b4cd4b377f80a2757b9e8654bfc`がローカルHEADと一致した。応答SHA-256 `2309d56549317775f50a699a1188b8059f08b1447fbe83286b09679c7806d2b8`。この時点の公開mainであることを確認。Gitのリモート照会はDNS解決エラーだったが、API応答で同一性を別途確認した。

PT65の[固定局面](corpus-pt65.json) SHA-256 `2b8ff90a0e54f3ff745818c88c3316536eff440a8bea28858cba34e8c742c148`は3題とも双玉。現行source `f84aa55`からの core rlib SHA-256 `50852f595b99e33ff4629a623c09c1a0dba111ad28ccd60d66d223d3e2c8a1ee`、詰みdriver `d5975a731877662f226bbebf5eb26aa51b50c9c9af0007c087c35c952011bb57`、USI binary `5559011835e204942e9584821b75f190bbf7d43fb29084b5f4ec7782025792e2`を使用。driver の初回リンクはApple ldがLLVM bitcodeを読めず失敗し、前回PT11と同じ `-C panic=abort -C lto=thin` で成功した。

[詰みreceipt](receipt-pt65-mate.json) SHA-256 `1a49db056e46588cf33e29382fb687a632a93ffc76728f80db6d78f38dba0bb9`: 7番13手と24番15手は各100万節点で `Unknown`。9番17手は297,761節点で `Mate valid=true aborted=false`、唯一の証明初手`P*1b`。未解決2題は不詰・欠陥とみなさない。

[通常探索receipt](receipt-pt65-search.json) SHA-256 `158c08b8cca470b964fe52ac5190cb0451ca07501ed06324e492c834b02cc426`: 証明済み9番だけを材料評価とr6重みSHA-256 `634bdf...`で実行。両方100万節点打切りで`2d2c+`、非mate score（材料cp -408、r6 cp 869）。[同一coreの合法再生](receipt-pt65-replay.json) SHA-256 `0c5cf11438b2a4cae0870076be6be2349251fc0f7606538a63d1540702cbc7e5`: 証明初手は合法王手、両通常探索初手は合法非王手、PV各13手合法で詰み終局ではない。創作詰将棋と実戦用探索の目的差であり、新規issueなし。公開ソースの掲載手数を独立正解扱いしない。なお現行ビルドには `UseBook` USI option がなく、opening-book feature が無効であることをコードで確認した。既定値変更・棋力主張なし。

## PT66: 同サイト次ページの未使用双玉3題（結果を見る前の契約）

- 入力: `https://tsumeshogi.com/tags/%E5%8F%8C%E7%8E%89?page=3` を1回取得した91,960 byteのHTML SHA-256 `51d11b7404facd67fd369ef4d553544c7bd3d5d7904dd6a53b9380e15c73a25e`。JSON-LD掲載順24件からPT65と同じ半角 `13/15/17/19/21手詰` の先頭最大4件を選び、前回URLと一致したら除外・繰上げなし。該当は6番 `https://tsumeshogi.com/problems/pgt1mj6ikq`（17手）、13番 `https://tsumeshogi.com/problems/asipdynfmk`（19手）、14番 `https://tsumeshogi.com/problems/migumgysm0`（17手）の3件。
- ページ取得・局面抽出・双玉確認、詰みAPI、完了証明題のみ通常探索の手順と判定はPT65と同じ。sourceは現行 `f84aa55`、同じbinary・rlib・driver・r6重みハッシュを再使用。各取得20秒・1MiB、証明各100万節点・4秒、通常探索は材料/r6の順に各100万節点・movetime1秒・外側4秒。全て逐次、最大9冷process・総外側45秒、出力各1MiB。`Unknown`で上限を動かさず、同じ出典での反復が実戦棋力への情報を持たなければ次の系列を選び直す。

## PT66結果

[固定局面](corpus-pt66.json) SHA-256 `023d138a564804c80afcdb8b6896b296f864fcc5c1e6f544e0bf3c288be0f7d2`は3題とも双玉。[詰みreceipt](receipt-pt66-mate.json) SHA-256 `4f9976a17e5e40a60e28f36ffa75649120d41b6c410f0edff785582364aa10e4`: 掲載6・14番の17手は各100万節点で `Unknown`。13番の19手は8,372節点で `Mate valid=true aborted=false`、唯一の証明初手`N*2c`。未解決2題は不詰・欠陥とみなさない。

[通常探索receipt](receipt-pt66-search.json) SHA-256 `351a0a460047420d3333e9c5d48eb7ca5c8ca7e70903c6e2ea10a92b8be25989`: 証明済み13番のみ。材料評価は100万節点で`N*2d`・cp -10677、r6は765,952節点で証明初手`N*2c`・cp -926。両方非mate表示。[同一coreの合法再生](receipt-pt66-replay.json) SHA-256 `3f02ae0d348f3478df8d84c773e97ec77020c0f3587edabccf4e2b2e66b442c4`: 証明初手とr6返却手は合法王手、材料評価の別手は合法非王手。両PVは合法だが13/12手で途切れ、詰み終局ではない。r6の初手一致はこの人工局面1件の観測であり、棋力向上の確認ではない。通常探索の非mate表示も詰みAPIの証明を覆さない。新規issue・既定変更なし。

## この反復の判定

現行source `f84aa55`で未使用の双玉投稿6題を2ページから固定し、詰みAPIは17手と19手を各1題完了証明、残り4題は100万節点内 `Unknown`。証明済み2題の通常探索4条件では、r6が1題の証明初手と一致したが非mate表示、残り3条件は別手だった。別手3条件はすべて非王手で、連続王手の創作詰将棋と実戦探索の目的差に属する。人工題のページをさらに順送りしても実戦棋力の課題を切り分ける期待値が低く、`CREATIVE_TWO_KING_OBJECTIVE_LIMIT`でこの系列を停止。別の実戦二玉13手以上の公開出典が必要。前回のGenerated Tsume-Shogi v4 `floodgate`適格範囲は処理済み。別の「21手以上」実戦詰将棋掲示板は閲覧ページが403で取得できず、出典と入力を固定できなかった。再開条件は新しい公開元棋譜または信頼できる局面台帳の特定、もしくは#280の独立H1入力・資源契約が整うこと。実行中の対局・監視jobなし。棋力・既定採用は未判定。

## PT67: 21手以上の双玉投稿を現行Sekireiで解く（実行前登録）

ユーザーが21手以上へ範囲を絞ったため、前段の13～19手創作図の棋力診断とは別に、長手数詰みAPIの到達能力を調べる。前回取得した双玉タグ[page 2](https://tsumeshogi.com/tags/%E5%8F%8C%E7%8E%89?page=2)のHTML SHA-256 `98cbb5fcbe1e001e2b2661df14e041f578c8bdcf5591d4f0559a368e8680f5a0`、[page 3](https://tsumeshogi.com/tags/%E5%8F%8C%E7%8E%89?page=3)のHTML SHA-256 `51d11b7404facd67fd369ef4d553544c7bd3d5d7904dd6a53b9380e15c73a25e`を入力索引とする。両ページのJSON-LD掲載順から、表題が半角数字の21手以上で前回PT65/66未使用の先頭3件を固定する。結果はpage 2掲載15番 `https://tsumeshogi.com/problems/jlyg0bwfn2`（25手）、page 3掲載9番 `https://tsumeshogi.com/problems/lgoldo4sxn`（29手）、page 3掲載18番 `https://tsumeshogi.com/problems/49smnkg_p3`（25手）。これ以外へ結果後に差し替えない。

各問題ページは1回取得、20秒・1MiBまで。`data-sfen`、タイトル、掲載手数、URL、HTML SHA-256を保存し、双玉と合法パースを確認。構造が変われば技術停止する。主判定は最新公開main `f84aa55d197d4b4cd4b377f80a2757b9e8654bfc` の詰みAPI（core rlib SHA-256 `50852f595b99e33ff4629a623c09c1a0dba111ad28ccd60d66d223d3e2c8a1ee`、driver SHA-256 `d5975a731877662f226bbebf5eb26aa51b50c9c9af0007c087c35c952011bb57`）で1T逐次実行。各掲載手数を最大深さ、500万節点、外側10秒、出力1MiB。`Mate valid=true aborted=false`だけを全合法応手の完了証明、`NoMate`はその上限内の不詰、`Unknown`/時間切れは未解決とする。掲載手数は独立正解とみなさず、上限延長しない。

詰みAPIが`Unknown`だった題のみ、同じsourceのUSI `SearchMode Dfpn`を1T・Hash64・500万節点・深さ掲載手数・movetime 5000ms・外側10秒で1冷process実行する。`info string dfpn outcome mate`かつ`completed true`だけを別APIの完了証明とし、fallback bestmoveや`unknown`を証明と呼ばない。詰みAPIで完了した題はDfpnを重ねない。両APIとも完了証明できた題があれば、その結果を見て別IDで通常探索の契約を記録してから実行する。創作図と実戦棋力は区別し、明確な再現不具合だけissue化する。

容量約1.8GiB・VM free約1.4GiBのため新規ビルドなし。既存release USI SHA-256 `5559011835e204942e9584821b75f190bbf7d43fb29084b5f4ec7782025792e2`を再利用。最大6冷process、総外側60秒・各出力1MiB、逐次実行。OSのプロセス一覧は取得不可だが、この作業から同時エンジンjobは起動しない。2026-11-15にGitHub APIでremote mainが指定SHAと一致した。資源異常・入力不一致なら停止する。

## PT67結果

[固定局面](corpus-pt67.json) SHA-256 `0ed0434a9c49e3a63f850a9ecdd872ca8fdbc7f1322c78acc1e8491156e2a620`は25・29・25手の3題とも双玉で、各HTML SHAとSFENを保存した。[詰みAPI receipt](receipt-pt67-mate.json) SHA-256 `116a34ac739bfd09b27aa7fd6df2cd1b265d5026a336b24fc0c1e63c3d13b598`: 3題とも500万節点で`Unknown`、`aborted=true`。不詰判定ではない。

[USI Dfpn receipt](receipt-pt67-dfpn.json) SHA-256 `14b94e4e328d382195de65d4060237e6ee62b8f926a2f100e0bbd1e92ccea4c5`: 掲載15番は`unknown`、9番は`disproven`、18番は`proven`を表示したが、3件とも`aborted=true`・`completed=false`。よってこの表示だけでは強制詰み/不詰を確定しない。18番のUSI提示手`7b8a`は合法だが非王手で、連続王手の詰将棋の初手にはならない。同じソースの`dfpn.rs`は攻方の全合法手を探索するため、一般の強制詰みと詰将棋を区別する。結果後の節点拡張なし。

## PT68: 長手数の独立証明木を確認（実行前登録）

PT67のUSI `Dfpn` は25手題の一つで`outcome proven`を返したが、`aborted true`・`completed false`だった。29手題も`disproven`と表示されたが同じく未完了で、いずれも強制詰み/不詰の判定に使わない。現行core `mate::prove_mate_existence` は高速探索の候補を証明木として抽出し、全合法応手を独立再生できた場合だけ`tree=Some`を返す。そこでPT67の同じ3題を掲載順に、同じsource/rlib・1Tで**別アルゴリズム**の存在証明を一度ずつ行う。各題の最大深さは掲載手数、search上限500万節点、抽出100万節点、再生100万節点、内部deadline8秒、外側10秒、出力各1MiB、総外側30秒。異常または資源逼迫は停止。`tree=Some`かつ`stop_reason=None`、代表手順・最長枝手数・各節点数を保存できた題のみ存在証明として採用する。`SearchNodes`・`SearchDeadline`・`ExtractionIncomplete`等は未解決で、既存試行の統計結果や最短手数とは混ぜない。証明木検証が失敗した場合は再現を保存してissueを検討する。上限を結果後に伸ばさない。

## PT68結果

[独立証明木receipt](receipt-pt68-proof.json) SHA-256 `e5724f3f55aeb6414ad4d1af34b0807bd88a43340db4cf77628b894d78fdcbe4`、driver SHA-256 `3d56764ba7ed6195635bcfb3d2118edd2ba2281e1005b2ea0e6a7ab19ab112f5`。掲載15番[25手題](https://tsumeshogi.com/problems/jlyg0bwfn2)は高速探索570,766節点、証明木抽出52,191節点、全合法応手再生21,864節点で`tree=Some`・停止理由なし。証明初手`R*4b`、最長枝25手。掲載18番[25手題](https://tsumeshogi.com/problems/49smnkg_p3)は高速探索42,442節点、抽出651節点、再生260節点で同じく`tree=Some`・停止理由なし。証明初手`3b2b`、最長枝25手。双方の初手は同一coreの合法手・王手と確認した。これは**25手以内の強制詰み存在証明**であり、25手が最短である証明ではない。掲載9番[29手題](https://tsumeshogi.com/problems/lgoldo4sxn)は高速探索500万節点で`SearchNodes`、証明木なし。未解決であり、不詰ではない。USI Dfpnの未完了表示を不具合とは断定しない。

## PT69: 証明木検証済み25手2題の通常探索（実行前登録）

PT68で完了した掲載15・18番だけをこの順に、現行source `f84aa55`のUSI binary SHA-256 `5559011835e204942e9584821b75f190bbf7d43fb29084b5f4ec7782025792e2`で材料評価と公開r6 NNUE（SHA-256 `634bdf527e7306045c8278df1d5311777561dfb09c1ebe6829ff3f4d883cc469`）の順に各1冷processずつ調べる。1T、Hash64、book feature無効、Speculative/SpecTopN0、r6はFV_SCALE24/absoluteのロードACK必須。各`go nodes 1000000 movetime 1000`、外側4秒、出力1MiB、総外側16秒、逐次実行。返却手・score/PV、終了、初手の合法性・王手・証明初手との一致を保存。違う合法非王手は詰将棋との目的差、違う王手は別解可能性を残す。違法手・異常終了・矛盾する完了詰み表示ならissueを検討する。25手2題は創作図であり、これを棋力率や既定採用へ使わない。

## PT69結果

[通常探索receipt](receipt-pt69-search.json) SHA-256 `a49ebd4f0444d31f30174b165ff04f32055f7e67805b02e2072f179348e24755`、[同一coreでの合法再生receipt](receipt-pt69-replay.json) SHA-256 `c2787e804fc3e0ac25799dd3a4ce1f1863655d877ba36566af897d7bedb97d26`。4条件とも正常終了し、初手は合法王手、PVは合法だが詰み終局に達していない。全件100万節点で打切り、mate scoreは表示されなかった。

掲載15番25手題は、材料評価が証明初手`R*4b`（depth 13、cp -7185）、r6が別の王手`R*3b`（depth 15、cp -858）。別手が強制詰みかどうかは未検証であり、失着と断定しない。掲載18番25手題は材料評価・r6とも証明初手`3b2b`（depth 18/13、cp -3860/-808）。4条件中3条件で検証済み証明初手と一致したが、25手以内の詰みを通常探索が証明したわけではない。創作局面2題から棋力差・r6の優劣を推定しない。

## 21手以上の反復判定

PT67～PT69では、未使用の公開双玉25・29・25手題3件を固定した。最短全応手の詰みAPIは500万節点では全件未解決だったが、別の存在証明APIは25手題2件で全合法応手の証明木を抽出・独立再生した。29手題は同上限で未解決。USI Dfpnの途中表示は3件とも`completed=false`で、強制詰み・不詰の証明には使わない。通常探索の別手1件は合法王手で代替解の可能性が残り、違法手・終了異常・検証済み証明との矛盾は確認できなかったため新規issueなし。

この創作双玉系列は長手数の存在証明APIと通常探索の挙動を確認する目的を満たした。追加の同系題を順送りしても#280の実戦探索課題や独立棋力判定への情報量が低いため、`CREATIVE_21_PLUS_STRENGTH_SCOPE_LIMIT`で停止する。次の有意な入力は、未使用の公開実戦棋譜から復元した二玉21手以上の局面、または#280の独立H1除外範囲と資源契約。新たな出典と事前契約が整えば別IDで再開する。既定変更・棋力向上の確認なし。実行中のローカル探索・監視jobなし。

## PT70: Floodgate由来と説明された公開局面集の21手以上候補を識別（実行前登録）

2020年の[公開投稿](https://mevius.5ch.io/test/read.cgi/bgame/1606100344/285-n)に、Floodgateの詰み終局棋譜から採ったと説明される[配布ファイル](https://drive.google.com/file/d/1KrGoyaJdse9Cq30rXnKTZw0L_fvZg2Pv/view)がリンクされ、21手以上も含むとされる。Drive metadataは`floodgate詰将棋.txt`、6,917 byte。直接ダウンロードで得た86行・CRLFのテキストSHA-256は`e51376c1c528c737001f8f62a40f1ddb57d922e4bb987555326950a6cba9404c`。各行に` sfen `を前置したSFENがあり、掲載手数・元CSAのID/手数は**含まれない**。したがって配布者の説明だけで元実戦棋譜への独立紐付けや21手以上を確定しない。rawは`/private/tmp/sekirei-pt70-drive-probe`に置き、公開リポジトリへ全件転記しない。

入力識別だけを目的に、このファイルの行順1～86を固定し、現行main `f84aa55d197d4b4cd4b377f80a2757b9e8654bfc`の既存詰みdriver SHA-256 `d5975a731877662f226bbebf5eb26aa51b50c9c9af0007c087c35c952011bb57`、core rlib SHA-256 `50852f595b99e33ff4629a623c09c1a0dba111ad28ccd60d66d223d3e2c8a1ee`を使用する。全行のSFENを検査し、各行を1T・最大25手・20万節点・外側2秒で一度だけ逐次解析。全体の外側45秒、ログ1MiB以内。`Mate valid=true aborted=false shortest_mate_ply>=21`の**最初の最大3行**を候補に固定し、3件に達したら以降を見ない。上限停止・`Unknown`は長手数の根拠にしない。候補ゼロならこの入力監査で停止し、結果後の上限延長・行の差替えなし。候補が出ても探索結果で選別したため統計的な解決率・棋力率には使わず、次IDで証明木を別検証する。エラーまたは資源逼迫時は停止し、済んだ行を残す。

**PT70技術停止:** [receipt-pt70-index.json](receipt-pt70-index.json)の1行目で`SFEN: move number must be at least 1`となり、解析は0件。元ファイルの全86行の最終欄は`0`で、現行パーサは受け付けない。探索結果を観測した後の選別ではなく、入力形式の不一致で停止した。

## PT70b: SFEN手数欄のみ正規化して同じ索引を監査（実行前登録）

PT70と同じ86行・行順・source/driver/rlib・各25手/20万節点/2秒・全体45秒・最初の完了21手以上最大3件という選定条件を保つ。`sfen <盤面> <手番> <持駒> 0`の最後の手数欄だけ`1`に変えてパーサへ渡し、元行・元SHAと正規化後のSFENを候補receiptへ残す。ほかの欄や行順は変えない。手数欄が0以外、構造不正、異常終了なら停止。前回は1行目のパース時に停止し探索結果ゼロなので、完了した解析の繰り返し・予算延長ではない。

## PT70b結果と次の順位

[入力監査receipt](receipt-pt70b-index.json) SHA-256 `743b151f5400b8abca050f4bdd677b5a06b0d1023cff589fe195cec787e3b142`。86行すべてを5.838秒で逐次調査し、異常終了・無効局面なし。完了した22局面の最短距離は1手1件、3手2件、5手5件、7手3件、9手5件、11手5件、13手1件。残り64局面は各20万節点で`Unknown aborted=true`。21手以上の完了確認は0件で、**配布集に21手以上がないことは意味しない**。配布者の説明する元Floodgate棋譜IDはテキストに含まれず、実戦起源の独立確認もできない。技術停止receipt SHA-256 `5c63e8c5716ffde3e01268d815ad77a8758a4dbe0e68b3546454741f72109841`を保持。外部ファイルの手数欄`0`はSekireiのSFEN不具合としてissue化しない。

現時点の次の情報価値は、(1) 元公開CSAと21手以上の局面を同時に特定できる資料、(2) その後の証明木・通常探索対照、(3) #280の独立H1入力・資源契約の順。このファイルの64件で節点だけを増やすと「未解決」の再試行になり、手数ラベルや元棋譜対応も得られないため実施しない。NIIの[実戦詰将棋生成研究](https://www.nii.ac.jp/tatsujin/2025/03/b8bdc691db6fa2c2d083fe0cede486f4305c094e.pdf)には21手以上20問の記述があるが、局面と棋譜の配布リンクは確認できなかった。[Generated Tsume-Shogi v4](https://huggingface.co/datasets/tokoroten-jp/generated-tsume-shogi/blob/main/README.md)は公開表の対象が最大15手。[21手以上掲示板](https://z.wikiwiki.jp/tsuminoarushougi/all-comments?page=9)は検索結果に存在するが、ページ直接取得はHTTP 403で入力を固定できない。新しい元CSA付き公開局面が見つかるまで`SOURCE_PROVENANCE_21_PLUS_LIMIT`でこの系列を停止。新規issue・既定変更・棋力向上の確認なし。実行中のローカル探索・監視jobなし。

## PT71: 元棋譜IDが示された最長21手局面の出典固定（実行前登録）

新たに[suimonの実戦詰将棋第1回・問題2](https://suimon-shogi.com/tumeshogi-01/)を発見。記事HTML 342,946 byte、SHA-256 `dab9e4df7198f902e038a34d7e8676ca0ceb78949dec1fc049b1de0a5a48d550`。2013-05-31の`wdoor+floodgate-900-0+ponanza-990XEE+NineDayFever_XeonE5-2690_16c+20130531163001`、103手まで指した後手番を図示し、実戦主線19手・最長21手と記す。以前の固定corpusに同じ対局IDは見つからなかった。記事の手数は独立正解とせず、元棋譜の局面と別にSekireiで検証する。

固定した[公式FloodgateのCSA候補](https://wdoor.c.u-tokyo.ac.jp/shogi/view/2013/05/31/wdoor+floodgate-900-0+ponanza-990XEE+NineDayFever_XeonE5-2690_16c+20130531163001.csa)を1回、20秒・最大1MiBで取得。HTTP200・ファイル名一致ならSHAを保存し、既存`public_tsume_csa_source.rs`と同じcore rlibによる合法CSA再生で103手後の二玉SFENと次の実戦手を固定する。articleの盤面との照合が可能なら行う。404や再生失敗なら原因を記録して技術停止し、別対局へ差し替えない。資源は逐次、追加ビルドは既存小helperのリンク1本のみ・外側120秒・追加ディスク100MiB以内、空き700MiB未満なら開始しない。局面固定後にPT72の証明・探索上限を結果を見る前に登録する。これはsource選定であり棋力率ではない。

**PT71技術停止:** 固定した公式CSA直リンクはHTTP 404（応答146 byte）。元棋譜の合法再生は未実施。記事の対局IDと図面を元CSAで独立照合したとは主張しない。ディスク空き965MiBで2013年アーカイブの一括取得・抽出は行わない。

## PT71b: 同じ記事内の103手後図を固定（実行前登録）

PT71の元CSAリンク404後、同じ固定記事の問題2には`shogi-time`の棋譜属性に「手数＝103」後手番の図面、両持駒、実戦継続が埋め込まれている。HTML SHA `dab9e4df7198f902e038a34d7e8676ca0ceb78949dec1fc049b1de0a5a48d550`を再確認し、その問題2の図面だけを盤面行9×9・双方の持駒・後手番・SFEN手数104へ変換する。駒種/枚数、双方の玉、各段9マス、記事の実戦次手`△７九飛`の合法性を同一coreで確認し、1局面の出典固定receiptを作る。記事の図を出典と呼び、公式CSAの合法再生とは区別する。構造不一致・実戦次手違法なら停止し、局面を補正しない。新規エンジンビルドなし。成功した場合のみ、別IDの証明木と通常探索の有界契約を結果前に記録する。

**PT71b結果:** [記事図面corpus](corpus-pt71b.json) SHA-256 `b31dba003d2ac9c9e60dc6e57f72b0654f3070eb47db22a40e214b84b19638b1`。SFENは`l1B5l/1r7/pk4g1p/1pPps3G/4N4/1P1P1p3/PnGNP3P/2K6/L7L w BG4Pr3sn3p 104`。盤面81マス・両玉各1枚を確認し、同一coreで記事の次手`R*7i`が合法王手と判定された。これは記事図面の変換と初手チェックであり、元CSA全手の合法再生や最長21手の独立証明ではない。

## PT72: 記事図面の21手以内存在証明と最短距離診断（実行前登録）

PT71bの固定SFENのみ、source `f84aa55d197d4b4cd4b377f80a2757b9e8654bfc`・rlib SHA-256 `50852f595b99e33ff4629a623c09c1a0dba111ad28ccd60d66d223d3e2c8a1ee`で、既存証明木driver SHA-256 `3d56764ba7ed6195635bcfb3d2118edd2ba2281e1005b2ea0e6a7ab19ab112f5`を1T・最大21手・search500万節点、抽出/検証各100万節点、内部8秒/外側10秒、ログ1MiBで一度実行する。`tree=Some`・停止理由なしで全合法応手の21手以内存在証明とする。`SearchNodes`等は未解決。成功した場合だけ既存最短全応手driver SHA-256 `d5975a731877662f226bbebf5eb26aa51b50c9c9af0007c087c35c952011bb57`を最大21手・500万節点・外側10秒で一度実行し、`Mate valid=true aborted=false distance=21`なら最短21手とする。`Unknown`は最短未確定、`Mate`の短い距離は記事ラベルとの違いとして記録。双方1T逐次、最大2 process/総外側20秒。停止・異常・空き700MiB未満なら次へ進まない。結果後の上限延長なし。証明成功なら通常探索の契約を別IDで登録し、創作図PT67との棋力率には混ぜない。

**PT72結果:** [証明と最短診断receipt](receipt-pt72-article-mate.json) SHA-256 `faa86d51056e753ff16698348687aef33aa4e9af250c2bd0d87cf815c4b037fd`。存在証明は探索369,724節点・証明木抽出2,494節点・全合法応手再生834節点で`verified=true stop=None`。証明初手は記事の実戦初手と同じ`R*7i`、最長枝21手。代表手順は9手だが最長防御枝21手の証明木を検証した。最短全応手APIは500万節点で`Unknown aborted=true`、最短21手かどうかは未確定。記事の記載を独立の最短正解扱いしない。元CSA直リンクの404も依然残る。

## PT73: 実戦記事21手以内証明局面の通常探索（実行前登録）

PT72の証明済み初手`R*7i`を、同じ固定SFENで現行release USI SHA-256 `5559011835e204942e9584821b75f190bbf7d43fb29084b5f4ec7782025792e2`の材料評価と公開r6 NNUE SHA-256 `634bdf527e7306045c8278df1d5311777561dfb09c1ebe6829ff3f4d883cc469`で比較する。各1冷process・1T・Hash64・Speculative/SpecTopN0、r6はEvalFile/FV_SCALE24/absolute ACK必須、`go nodes 1000000 movetime 1000`、外側4秒、逐次・総8秒・出力各1MiB。bestmoveとscore/PV、正常終了、初手の合法性・王手、PVの合法終局、証明初手との一致を記録。別手でも実戦用探索の選択を即失着とせず、代替詰みや時間差の可能性を残す。違法手・異常終了・完了証明と明確な矛盾があれば既存#280と重複を確認してissue/追記を検討。局面は記事図面からの復元で元CSA合法再生未了、単一局面の棋力率や既定採用には使わない。

**PT73結果:** [通常探索receipt](receipt-pt73-article-search.json) SHA-256 `1dd53f83c57601953c556403ad8bdac1929f6af0cc4946a4b068a33bb746d2ca`、[同一coreの合法再生](receipt-pt73-article-replay.json) SHA-256 `e9329db3768655b323b6881f615dc87699be8c75c066943896d94b9bb53d96df`。両process正常終了、各100万節点で非mate表示。材料評価は`8b1b`・depth13・cp211で合法非王手、PV9手合法だが終局ではない。r6は`R*3h`・depth17・cp -129で合法王手、PV17手合法だが終局ではない。いずれも21手以内を証明した`R*7i`を選ばなかった。r6の別王手が代替詰みか、この固定時間でどちらが実戦的に優れるかは未判定。探索性能の有界な観測であり、失着・実戦棋力低下・エンジンの正しさの破綻とは断定しない。既存#280は同種の公開実戦由来二玉局面の詰み初手見逃しを扱うため、新規issueを重複作成せず、出典限界を明記した追記を検討する。

## PT71c: Floodgateの別の公式CSA経路で元棋譜を照合（実行前登録）

既存#280の公式CSAリンクは`/shogi/x/...`経路を使う。PT71の`/view/...`404は棋譜不存在を示さないため、同じ対局IDの[固定`/x/` URL](https://wdoor.c.u-tokyo.ac.jp/shogi/x/2013/05/31/wdoor+floodgate-900-0+ponanza-990XEE+NineDayFever_XeonE5-2690_16c+20130531163001.csa)を一度だけ20秒・1MiBで取得する。HTTP200ならSHAを保存し、既存`public_tsume_csa_source.rs`を同一core release rlibに1 job・外側120秒・追加ディスク100MiB以内でリンクして103手後を合法再生、PT71bのSFENと次の手が一致するか確認する。空き700MiB未満または取得・再生失敗なら停止し、別の対局やURLへ差し替えない。2013年公式アーカイブは222MBで、現在の空きが約1～3GiBに揺れるためこの契約ではダウンロードしない。記事由来のPT72/PT73判定は変更しない。

**PT71c結果:** 公式`/x/` CSAがHTTP200・13,197 byteで取得でき、SHA-256 `43f6059752e1d135c86f69a1822d9d90a70f7feb37c61e88d846118cfd4674f4`。イベントID・対局者名が記事と一致。既存CSA parserを同一coreへリンクしたhelper SHA-256 `7739984f33a1dfe9522ba8df88bd1b65fecc29830faf151692395c0d656f2f66`で全122手を合法パースし、103手後SFENと次手`R*7i`が記事図面の固定値と完全一致した。[照合receipt](receipt-pt71c-csa.json) SHA-256 `b637f3eda96f834e1f3fcbd11398c91edd353939a93bda604d84d44110b946fe`。PT71の404は誤ったURL経路による技術停止であり、元実戦の出典限界は解消。証明・通常探索の数値と判定はPT72/PT73から変更しない。

**issue記録:** 既存の[探索課題 #280 にこの局面を追記](https://github.com/kent-tokyo/sekirei/issues/280#issuecomment-6887345383)。最初は記事図面だけの証拠として投稿し、PT71cの公式CSA再生後に同コメントを修正。更新された本文がissueの読み戻しで表示された。新規issueを重複作成しない。証明初手を選ばないことは100万節点での有界な探索課題であり、別手の敗着・棋力低下や検索原因の確定ではない。

## 次の判定

この反復は新規の公開元実戦二玉局面を1件追加し、21手以内の全合法応手の存在証明と、同一source通常探索2条件の初手不一致を固定した。記事の「最長21手」と証明木最長枝21は一致するが、最短21手の独立確認は500万節点で`Unknown`。r6の別王手`R*3h`が代替詰みかも不明。今後この既知局面を結果後に予算拡張して合格扱いせず、#280の独立H1入力・除外範囲・資源契約が整えばそこで候補を評価する。追加の実戦長手数局面は元CSA付きの新出典を事前固定して別IDで調べる。既定変更・棋力向上の確認なし。実行中のローカル探索・監視jobなし。
