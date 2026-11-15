# NM05 棋士実戦の次の一手診断結果

[探索前契約](NM05.md)に固定した[原記事](https://kasukabekidokan.com/2026-0212/)の独立対局2局面を使用。問題図の原画像は[三枚堂・西尾の70手後](https://kasukabekidokan.com/wp-content/uploads/2026/02/%E4%B8%89%E6%9E%9A%E5%A0%82-%E9%81%94%E4%B9%9F%E4%B8%83%E6%AE%B5x%E8%A5%BF%E5%B0%BE-%E6%98%8E%E4%B8%83%E6%AE%B5_70_.jpg) SHA-256 `ed072ae1c1d4a0323b8b94529613c4cd51e8a88d9bb2a52e6c5eb117b267ff3c`、[中村・藤森の61手後](https://kasukabekidokan.com/wp-content/uploads/2026/02/%E4%B8%AD%E6%9D%91%E5%A4%AA%E5%9C%B0-%E5%85%AB%E6%AE%B5x%E8%97%A4%E6%A3%AE%E5%93%B2%E4%B9%9F-%E4%BA%94%E6%AE%B5_61_.jpg) SHA-256 `91e7276e18bdb2a5a5ec405bee518ab9ec270ebefb02b1053db14e094f9cc007`。独立転記SFEN一致・双方の玉・標準40枚を確認した。記事の掲載手 `7d4d`/`S*7e` は現coreで合法、各一手後の[解答図71](https://kasukabekidokan.com/wp-content/uploads/2026/02/%E4%B8%89%E6%9E%9A%E5%A0%82-%E9%81%94%E4%B9%9F%E4%B8%83%E6%AE%B5x%E8%A5%BF%E5%B0%BE-%E6%98%8E%E4%B8%83%E6%AE%B5_71_.jpg)/[解答図62](https://kasukabekidokan.com/wp-content/uploads/2026/02/%E4%B8%AD%E6%9D%91%E5%A4%AA%E5%9C%B0-%E5%85%AB%E6%AE%B5x%E8%97%A4%E6%A3%AE%E5%93%B2%E4%B9%9F-%E4%BA%94%E6%AE%B5_62_.jpg)と盤面・持駒が一致。画像中の図外の歩アイコンは駒数からUI装飾と判断した。元対局全手は入手・照合していない。入力JSON SHA-256 `9249e5a0fe691dd3ac1060f579d20fe0aa7521c81eb14893c9b8d1cc921a5aea`、原画像とrawはGit対象外。

r6重み SHA-256 `634bdf527e7306045c8278df1d5311777561dfb09c1ebe6829ff3f4d883cc469`、診断binary SHA-256 `27f7f2328a9fb4239710035ef0177cce467df4ce213e6445442bf92f3157ca7a`、`FV_SCALE=24`、absolute、1T、冷プロセス。共有ホストでは別プロジェクトのCargo/Rustコンパイル中かつ空き容量約1.2GiBだったため、固定節点の探索的診断に限定し、同時間速度の資格には使わない。4回の通常探索は外側20秒上限内の3.6秒で終了し、全PVが合法。raw SHA-256 `0b61ed56c7e273ed94309088b4275d556d22c1549cc8587a23aacc08ea624877`。

| 問題 | 10万節点 | 100万節点 | 判定 |
| --- | --- | --- | --- |
| ▲4四龍 `7d4d` | `7d4d`、+248cp、完了深さ11 | `7d4d`、+237cp、完了深さ17 | 両予算で記事手と一致 |
| △7五銀打 `S*7e` | `8e8f`（△8六歩）、+62cp、完了深さ12 | `6f6g+`（△6七歩成）、+36cp、完了深さ18 | 短予算は記事が失敗例に挙げる手。100万節点の別手は記事に反証がない |

全通常探索は節点上限で中断したが、記録した完了反復はexact、PV合法。得点は各手番側視点であり、対局勝率ではない。2問目の記事には△8六歩▲同角の後、△7五銀打では▲同角から飛車も取られるという説明がある。この部分手順 `8e8f 5i8f S*7e 8f7e` は合法。一方、Sekireiの10万節点PVは△7五銀打ではなく△6七歩成を続けるので、記事の説明だけで返却手全体を敗着と証明できない。

登録規則に沿って2問目の△8六歩と掲載△7五銀打だけを各100万節点で強制root診断。前者 -107cp/完了深さ18、後者 +98cp/完了深さ17で、両方節点上限中断・深さ不一致・bound unknown。差を確定優劣とせず`NM05_SHORT_BUDGET_DISAGREEMENT_UNRESOLVED`で停止。強制raw SHA-256は△8六歩 `4fae0a46e830fcad68469179036c246a6f593516516bdcdb7d62096ae451045a`、△7五銀打 `de3e507604e4dfb0ff5e168e40d114ca9e9ec479fb2d7e34acbce8163c3db47f`。

独立対局2件で同種の救済可能な失着という登録条件は満たさない。探索/NNUE候補を作らず、この記事の追加予算や順送りは行わない。NM01/NM02の未使用確認15題を保持。棋力改善、既定変更、独立対局の判定なし。次の入力は、Sekireiの実際の返却手に対する具体的で合法な反証と救済手を持つ別対局。
