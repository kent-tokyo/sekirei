# 公開棋譜から復元した難詰めの追加監査（PT44–PT52）

2026-11-14。一次資料は[Generated Tsume-Shogi v4 のデータカード](https://huggingface.co/datasets/tokoroten-jp/generated-tsume-shogi/blob/main/README.md)と[公開Floodgate棋譜](https://wdoor.c.u-tokyo.ac.jp/shogi/)。前段の`sound`系列19局面は[PT43一覧](RESULTS-PT43.md)を参照。

この追加系列は公開図の`yozume`・`defect`ラベルを詰みの正解には使わず、元棋譜の場所を探す索引に限定した。`floodgate/train`全7 Parquet区画をSHA照合して行順に監査すると、13手以上・代表の適格行は7件。すべて異なる元棋譜の二玉局面をCSA合法再生で復元し、同じ固定mate driverで13手詰みを完了証明した。出典索引と元局面は[PT46候補](corpus-pt46-candidates.json)、[PT50の7番目](corpus-pt50-candidate.json)、[PT47の6局面](corpus-pt47-original.json)、[PT50の元局面](corpus-pt50-original.json)に保存。proofは[PT48](receipt-pt48.json)と[PT51](receipt-pt51.json)。

同じr6通常探索設定（1T、Hash64、NNUE r6/FV_SCALE24、100万節点または1秒、各冷process）で7件を調べた。証明初手と一致4件、別手でmate13表示2件、別手で非mate表示1件。[PT49の6件](receipt-pt49.json)と[PT52の1件](receipt-pt52.json)が生ログを含む。別手mate表示のうち2015年局面はPVが合法に詰み終局、2013年局面はPVが12手で途切れて終局を確認できず、全応手の強制詰みも未証明。これら2件を失着とは数えない。

明確な有界探索の取りこぼしは[2017-12-07の元棋譜](https://wdoor.c.u-tokyo.ac.jp/shogi/x/2017/12/07/wdoor+floodgate-300-10F+gps_l+Sunfish4-AWS09+20171207150005.csa)の局面。元二玉局面の13手詰み初手`3a2a`を1,878節点で証明したが、r6通常探索は100万節点で`4c4a+`、評価cp -456を返した。再現条件を[既存issue #280](https://github.com/kent-tokyo/sekirei/issues/280#issuecomment-6873538580)に追記し、issueがOPENであることを確認した。

PT50で最後に固定した2021年局面は証明初手`9f9e`をr6も選び、mate13・496,652節点、13手PVは合法に詰み終局へ達した。資料図は余詰のある作品であり、元二玉局面に関する独立証明と通常探索の結果だけを述べる。

この7件は`sound`系列と選定条件が異なる。26局面を棋力率に換算せず、検索パラメータの既定変更・棋力向上の主張もしない。両系列の登録済み適格行はすべて処理し、追加探索は`PUBLIC_FLOODGATE_ADVANCED_CORPUS_EXHAUSTED`および`PUBLIC_FLOODGATE_FLAWED_SOURCE_INDEX_EXHAUSTED`で停止する。次は#280の除外範囲網羅性・fresh開幕・資源契約を整えた独立H1、または別出典を結果前に固定した診断。現在、対局や監視jobは起動していない。
