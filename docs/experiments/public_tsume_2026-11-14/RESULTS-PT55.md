# 別出典の13手詰12題（PT53–PT55）

2026-11-14。[ShogiBoardQの公開13手詰1000題](https://github.com/hnakada123/ShogiBoardQ/blob/2ab5f5f0a37857c5e921b5b0a1bae27ef3d6c0ed/data/tsumeshogi/tsume_13ply_1000_20261001.txt)から、取得前に固定した先頭12題を調べた。資料の[検査説明](https://github.com/hnakada123/ShogiBoardQ/blob/2ab5f5f0a37857c5e921b5b0a1bae27ef3d6c0ed/data/tsumeshogi/README.md)は参考手順の合法性・最短手数等を主張する。Sekirei側では別途、[入力](corpus-pt53-sbq13.json)の参考手順が全件合法に13手で詰み終局へ達することを確認した。

1玉の創作詰将棋12題に13手・100万節点の詰みAPIを逐次実行した結果、[11題は13手詰と完了証明](receipt-pt54.json)し、12題目は上限で`Unknown`。未解決を不詰とは扱わない。登録規則で進めた11題の[r6通常探索](receipt-pt55.json)は証明初手と7題一致、4題別手だった。[合法性と王手の検査](receipt-pt55-checks.json)では別手4件すべてが合法な非王手だった。通常探索は詰将棋の連続王手を課されないため、この4件は実戦局面の探索ミスを示さない。mate scoreやPVを全応手詰みの証明にも換算しない。

この系列は[Floodgateから復元した元二玉局面](RESULTS-PT43.md)とは入力と目的が異なる。残り988題の通常探索反復は実戦棋力への学習効率が低く、`ONE_KING_NORMAL_SEARCH_SCOPE_LIMIT`で停止した。強さの向上や既定値変更は未確認。[実戦局面の既知問題 #280](https://github.com/kent-tokyo/sekirei/issues/280)はOPENで、この系列からの新issueはない。
