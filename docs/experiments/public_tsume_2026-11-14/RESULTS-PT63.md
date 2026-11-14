# 次の公開双玉問題と#280の探索切り分け（PT56–PT63）

2026-11-14。[詰将棋メーカーの双玉タグ一覧](https://tsumeshogi.com/tags/%E5%8F%8C%E7%8E%89?page=1)の掲載順から、13～21手の先頭4題を結果前に固定した。[入力と出典SHA](corpus-pt56-positions.json)を保存。二玉の投稿創作図であり、Floodgateの元実戦局面とは別系列。

Sekireiの固定詰みAPI（掲載手数以内、100万節点）で[15手と13手の2題を完了証明](receipt-pt57.json)した。残り19手・13手の2題は上限で`Unknown`。証明済み2題のr6通常探索は、診断adapterの入力ID欠落による[PT58技術停止](PLAN.md)を記録してから修正して実行し、[両題で証明初手を選んだ](receipt-pt59.json)。返却手は合法王手。PVは途中で途切れており、詰みAPIによる完了証明とは区別する。投稿図から新しいSekirei欠陥は確認していない。

#280の既知見落とし2局面は、同じ隔離ソース・100万節点で[材料評価だけに替えて対照](receipt-pt60.json)した。2012年の証明初手`4a3b`は材料評価でも選ばれず、2017年の`3a2a`は材料評価では114,138節点・mate13で選ばれた。後者はr6評価と探索の相互作用が関わる可能性、前者は評価を外しても残る探索課題を示すが、因果・棋力は未判定。[issue #280へ記録](https://github.com/kent-tokyo/sekirei/issues/280#issuecomment-6874737206)した。

2012年のr6診断で、隔離sourceのCLIへ尺度24指定だけを追加した[差分](pt61-scale-flag.patch)を使った。通常探索は既知結果`2e2d`/cp985/100万節点を再現。正解初手`4a3b`にrootを固定しても[LMRありではcp784/100万節点で未到達](receipt-pt61.json)だった。V2 LMRを無効にすると[固定rootは120,365節点でmate scoreに到達](receipt-pt62.json)したが、[全root探索は依然`2e2d`/cp929/100万節点](receipt-pt63.json)だった。PVの合法性、盤面復元、プロセス終了を確認。LMRがこの枝の到達に関係し、全rootの選択には別の予算配分等が残る観測として[issue #280へ追記](https://github.com/kent-tokyo/sekirei/issues/280#issuecomment-6874811389)した。

これらは選択済み開発局面の機構診断であり、正式な棋力率・速度優位・既定採用の証拠ではない。登録したLMR全off診断は終了。次の正式候補判断には、既存H1契約の除外範囲網羅性、fresh開幕、資源枠が必要。現時点の停止理由は`QUALIFICATION_INPUT_AND_RESOURCE_LIMIT`。

追跡用の[PT64反復trace](receipt-pt64.json)は、LMR全offの全root100万節点で前回と同じ`2e2d`を返した。正解初手`4a3b`は初期合法143手中49番目、完了した深さ6～12の最善手は`2e2d`だった。49番目という初期順位から実際の枝ごとの節点数までは分からない。3範囲の除外候補はいずれも`complete_for_scope=false`で、現在の入力検証器は独立H1を開始させない。この既知局面での反復を終え、未使用二玉実戦局面または除外範囲の網羅性確認を再開条件とする。
