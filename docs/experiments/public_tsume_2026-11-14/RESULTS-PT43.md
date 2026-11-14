# 公開Floodgate由来の上級詰み全候補: 元実戦局面の有界診断

[Generated Tsume-Shogi v4](https://huggingface.co/datasets/tokoroten-jp/generated-tsume-shogi/blob/main/README.md)の`floodgate/train` 7区画で、公開図の`label=sound`・13手以上・代表・防御確認済みに当たる19行を検査した。各図から元Floodgate棋譜へ遡り、二玉の元局面を合法再生した。19局面は18対局に由来し、2019-10-01の同一棋譜から2局面ある。2008年と2009年の個別CSA URLは404のため、[公式年別アーカイブ](https://wdoor.c.u-tokyo.ac.jp/shogi/)の掲載SHAを照合して該当1ファイルずつを抽出した。

Sekireiの詰みAPIは元局面15件で全合法応手の13手以上の詰みを完了証明した。元局面で7手・9手となった2件、13手以内の連続王手詰みなしとなった2件はr6通常探索の比較から除いた。公開図の手数は駒を減らした図の手数であり、元局面の正解ラベルではない。

| 元局面でのr6確認 | 件数 |
| --- | ---: |
| 証明済み最短詰み初手と一致 | 12 |
| 初手は異なるがmate表示、合法PVが詰み終局へ到達 | 1 |
| 初手が異なり、100万節点以内にmate表示なし | 2 |
| 元局面は13手以上の完了証明対象外 | 4 |

r6の確認は1T・Hash64・最大100万節点/1000msの開発診断である。PT19の3件のうち2020年と2012年5月はS21の別source binaryを使い、残りは隔離source `1e963743`のtune binaryを使う。初手一致件数を対局棋力や同一条件の勝率とは扱わない。mate表示の別手は合法PVの詰み終局だけを確認し、全応手の強制詰みは未証明。初手不一致かつ非mate表示の2件は[#280](https://github.com/kent-tokyo/sekirei/issues/280)に条件を追記済み。候補`T_V2_CHECK_R=1`は2023年の既知例を救ったが、別の2012年例には転移しなかったため既定値を変えない。

| 元棋譜 | 元の手数 | 元局面の詰み証明 | r6返却手 / 表示 | 判定 |
| --- | ---: | --- | --- | --- |
| [2023年・fg-20230118-p96](https://wdoor.c.u-tokyo.ac.jp/shogi/x/2023/01/18/wdoor+floodgate-300-10F+Krist_483_473stb_1000k+AI_Ari_Shogi_Super_torch_test2+20230118230001.csa) | 96手後 | 13手 `2b2a+` | `3e2d` / cp 507 (1,000,000節点) | 別手・非mate表示 |
| [2017年・fg-20171030-p156](https://wdoor.c.u-tokyo.ac.jp/shogi/x/2017/10/30/wdoor+floodgate-300-10F+gps_l+Sunfish4-AWS09+20171030130000.csa) | 156手後 | 13手 `G*8c` | `G*8c` / mate 13 (58,103節点) | 初手一致 |
| [2020年・fgv4-3289cadb780b](https://wdoor.c.u-tokyo.ac.jp/shogi/x/2020/01/28/wdoor+floodgate-300-10F+Krist_483_473stb_16t_100m+elmo_WCSC27_479_1000k+20200128023006.csa) | 102手後 | 13手 `6c7b` | `6c7b` / mate 13 (105,525節点) | 初手一致 |
| [2014年・fgv4-398d95eb7dce](https://wdoor.c.u-tokyo.ac.jp/shogi/x/2014/02/28/wdoor+floodgate-900-0+Poseidon+sakurapyon_4G+20140228160003.csa) | 90手後 | 13手 `G*8b` | `G*8b` / mate 13 (33,752節点) | 初手一致 |
| [2012年・fgv4-3beeeb068548](https://wdoor.c.u-tokyo.ac.jp/shogi/x/2012/05/27/wdoor+floodgate-900-0+gps500+Gasyou_Atom-D510_1c2t+20120527233003.csa) | 78手後 | 13手 `2d2c+` | `2d2c+` / mate 13 (130,643節点) | 初手一致 |
| [2012年・fgv4-4bd360a34f3e](https://wdoor.c.u-tokyo.ac.jp/shogi/x/2012/06/24/wdoor+floodgate-900-0+kakusan_hadouhou+sakurapyon-2012-0.015kpp+20120624030005.csa) | 122手後 | 7手 `3h3g` | 対象外 | 元局面は対象外 |
| [2012年・fgv4-664b2c354ebe](https://wdoor.c.u-tokyo.ac.jp/shogi/x/2012/03/03/wdoor+floodgate-900-0+gps_l+NanohaWCSC21USI+20120303233003.csa) | 88手後 | 13手 `4a3b` | `2e2d` / cp 985 (1,000,000節点) | 別手・非mate表示 |
| [2019年・fgv4-6e83e3a0b021](https://wdoor.c.u-tokyo.ac.jp/shogi/x/2019/10/01/wdoor+floodgate-300-10F+Krist_483_473stb_16t_100m+elmo_WCSC27_479_1000k+20191001193001.csa) | 90手後 | 13手 `9f9b+` | `9f9b+` / mate 13 (392,529節点) | 初手一致 |
| [2021年・fgv4-88f7cf8b3b8e](https://wdoor.c.u-tokyo.ac.jp/shogi/x/2021/10/30/wdoor+floodgate-300-10F+CR8+Yss1000k+20211030133003.csa) | 132手後 | 13手 `5a8a` | `G*7a` / mate 15 (362,689節点) | 別手・mate表示/PV終局 |
| [2019年・fgv4-89f5024b081d](https://wdoor.c.u-tokyo.ac.jp/shogi/x/2019/10/01/wdoor+floodgate-300-10F+Krist_483_473stb_16t_100m+elmo_WCSC27_479_1000k+20191001193001.csa) | 88手後 | 15手 `9d8b+` | `9d8b+` / cp 768 (1,000,000節点) | 初手一致 |
| [2021年・fgv4-96e7bb41abff](https://wdoor.c.u-tokyo.ac.jp/shogi/x/2021/11/20/wdoor+floodgate-300-10F+tanuki2+coduck_oci_1c+20211120190002.csa) | 86手後 | 13手 `S*7b` | `S*7b` / mate 13 (36,914節点) | 初手一致 |
| [2022年・fgv4-a6792e9c7119](https://wdoor.c.u-tokyo.ac.jp/shogi/x/2022/04/05/wdoor+floodgate-300-10F+elmo_WCSC27_479_1000k+nene_20220403_ipad9+20220405060008.csa) | 142手後 | 13手 `8c8b+` | `8c8b+` / mate 13 (18,100節点) | 初手一致 |
| [2022年・fgv4-bcc4e63ffdeb](https://wdoor.c.u-tokyo.ac.jp/shogi/x/2022/02/19/wdoor+floodgate-300-10F+Krist_483_473stb_1000k+dlshogi_pre17_po10000+20220219050010.csa) | 80手後 | 13手 `G*2b` | `G*2b` / mate 13 (33,562節点) | 初手一致 |
| [2020年・fgv4-c447ae92bc1f](https://wdoor.c.u-tokyo.ac.jp/shogi/x/2020/09/21/wdoor+floodgate-300-10F+Ayame_4t+elmo_WCSC27_479_1000k+20200921113006.csa) | 70手後 | 13手 `9f8e` | `9f8e` / mate 15 (14,200節点) | 初手一致 |
| [2010年・fgv4-df42bb7bd6bf](https://wdoor.c.u-tokyo.ac.jp/shogi/x/2010/01/10/wdoor+floodgate-900-0+gps99+gps_l+20100110183004.csa) | 134手後 | — | 対象外 | 元局面は対象外 |
| [2013年・fgv4-e431511566b5](https://wdoor.c.u-tokyo.ac.jp/shogi/x/2013/06/02/wdoor+floodgate-900-0+gps_normal+sakurapyon_2013+20130602073006.csa) | 168手後 | 13手 `S*8d` | `S*8d` / mate 17 (159,282節点) | 初手一致 |
| [2008年・fgv4-f66b24492c26（年別archive）](https://wdoor.c.u-tokyo.ac.jp/shogi/archive/wdoor2008.7z) | 62手後 | 13手 `G*2e` | `G*2e` / mate 13 (271,927節点) | 初手一致 |
| [2012年・fgv4-f9f2dff5b629](https://wdoor.c.u-tokyo.ac.jp/shogi/x/2012/03/05/wdoor+floodgate-900-0+gps_normal+disney+20120305043007.csa) | 102手後 | 9手 `S*2c` | 対象外 | 元局面は対象外 |
| [2009年・fgv4-4f8f519d92bd（年別archive）](https://wdoor.c.u-tokyo.ac.jp/shogi/archive/wdoor2009.7z) | 110手後 | — | 対象外 | 元局面は対象外 |

入力、証明、探索の各SHAと局面別のsource identityは[機械可読の集約台帳](summary-pt43.json)に保存した。PT32・PT33・PT40の入力停止、PT27の診断CLI不一致は[実験台帳](PLAN.md)に残した。正式な独立対局H1は除外範囲の網羅性確認、fresh開幕、資源契約待ちであり、今回の戦術診断から棋力向上や既定採用を主張しない。
