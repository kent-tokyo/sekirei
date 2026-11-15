# NM04 ▲6四角の実戦問題の結果

[原作者の記事](https://note.com/lefthorse/n/nce0e4ff414d7)の問題図01-Aを独立に2回読み、SFEN `k1g3Rnl/ls2+N4/1p5pp/p1p3p2/1n2p2PP/PBPPS1P2/1P1SP4/1KG1s+p3/LN6+b b G2Prgl 1` で一致した。手数欄1は元棋譜不在の仮値。画像SHA-256 `64a38edcabdde6bfda3acc74b76ee524d3450b96461b7b469ed67d1f1edd89c2`、入力集合SHA-256 `4e6daeb9c7de5edede77cf782f99b1ddb01812efc41e77d23f118ccc4ac06d5a`。双方の玉、駒種別の標準40枚、記事の▲6四角`8f6d`と△8一金打`G*8a`の合法性を確認。画像と入力はGit対象外の `data/runs/public-next-move-20261115/` に保存した。

r6/尺度24/1Tの固定10万・100万節点通常探索は、ともに`G*7b`（▲7二金打）を返した。各完了深さ11/14、評価+519/+585cp、PV合法、上限で中断。raw SHA-256 `de0560a9bde5ac325d4db803dc23deb923f2f2c59df52acdbd5f183530af213c`。登録規則に従う100万節点の強制rootは、記事手`8f6d`が+371cp/深さ15、返却手`G*7b`が+569cp/深さ14、双方中断。raw SHA-256 `aeee933a160e19b659a89c9fac5585c59e1c396d93b8c1be20ee4dc4932b5243`。参考手順の▲6四角△8一金打も合法。

記事は▲6四角の働きと後手の受けを説明するが、▲7二金打への反証は無い。強制rootの深さも異なるため、Sekireiの手を敗着と判定できない。`AUTHOR_MOVE_DISAGREEMENT_NO_RESCUE_PROOF_NM04`として候補選択を止める。記事の同一対局の後続図を新しい独立問題として数えない。NM01/NM02の確認15題は未使用のまま。棋力改善・既定変更・独立対局の判定なし。
