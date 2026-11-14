# 写真⑧の詰み探索改善

公開範囲: 修正コードと本計画・検証報告。本文の `tasks/...` と回帰入力ファイルはローカル保管の証跡で、このブランチには含めない。

2026-11-14。実装は隔離ブランチ `codex/photo08-mate-depth-20261114`、基準はPR #279 head `cda4b98279696395ca1aa661e0d104af7adb6736`。共有checkoutの他作業は変更していない。計画は[こちら](../plans/photo-tsume-08-search-2026-11-14.md)、試行の契約と保持した失敗は台帳（ローカル記録: `tasks/photo-tsume-20261114/search-improvement/LEDGER.md`）。

## 得られた結果

⑧の初手は **▲６四銀打（S*6d）**。変更前は25ply・400万節点で `SearchNodes`、最終候補T8-C4では同じ上限内の **1,711,883節点** で証明できた。玉方の全合法応手を含む木は4,576節点・919個の詰み終端で、変更していない基準版の検証器に別processで再生して成功した。

得た戦略の最長枝は25ply。最短25手、初手唯一性、雑誌の作意手数を証明した結果ではない。合法手生成は探索と検証で共通なので、別の将棋ルール実装による独立証明とも呼ばない。

| 入力・条件 | 基準版 | T8-C4 |
|---|---:|---:|
| ⑤、17ply/100万節点 | 証明、167,388節点 | 証明、157,335節点 |
| ⑥、同上 | 証明、340,892節点 | 証明、397,636節点 |
| ⑦、同上 | 証明、114,861節点 | 証明、88,271節点 |
| ⑧、25ply/400万節点 | 未解決、SearchNodes | 証明、1,711,883節点 |
| 別由来14題、各25ply/100万節点 | 13題を証明 | 14題を証明 |
| 別由来の既存解13題の合計節点 | 135,498 | 124,369（−8.21%） |
| ⑤〜⑦を加えた既存解16題の合計節点 | 758,639 | 767,611（＋1.18%） |

解を失った局面は0。単一局面の最大節点増加は36.4%で、事前に要調査とした2倍増は0件。⑥など悪化した局面も表に残した。10%以内という合計費用基準を満たしたが、この既使用集合の結果から一般の棋力・同時間性能を主張しない。

## 原因の切り分けと変更

元の証明表は局面hashだけをキーにしていた。内部の対照試験では、既存の3手詰めを残り1手として先に調べると、その結果が残り3手の検索でも再利用され、rootを1節点で棄却した。freshな表では28節点で証明し、完全解析でも55節点で3手詰めだった。具体的な再現例は[Issue #287](https://github.com/kent-tokyo/sekirei/issues/287)に登録した。

⑧でも異なる残り手数での参照を観測したが、取り出した6例の小解析では結論の誤りを確認しなかった。その6例が未解決の直接原因とは断定しない。41plyで深さ打切りが0でも未解決だったため、単純な深さ上限不足とも扱わなかった。

最終変更は `mate.rs` に限定する。

1. 未完了の証明数・反証数を残り手数ごとに保持する。
2. 手がない終端だけを深さに依存しない結果として共用する。
3. 完了した証明は証明済みの手数以上へ、有界反証は反証済みの手数以下へ共用する。反復打切りに依存する根拠を伝播し、そのような結論を範囲共用しない。矛盾した範囲を観測したhashは範囲共用を停止する。
4. 子を再探索する閾値を `second + 1` から `second + second/4 + 1` へ広げる。親閾値の制約は維持する。元の25ply診断で約75%の訪問が再訪だったことに基づく1候補で、比率の探索や結果後の調整は行っていない。

深さ分離だけのC1、終端共用のC2、範囲共用のC3は⑧を証明したが、費用基準を超えた。各判定をNO_PASSとして保持し、新機構C4だけが今回の全基準を満たした。反復経路に依存する既存のexact表の扱いを全面的に修復したわけではなく、最終木の反復検査も引き続き必要。

## 検証と資源

全て単一worker・逐次実行、16MiB stack、各探索deadline25秒・外側30秒、抽出/検証は各10万節点。RSS監視上限512MiB。別作業のwasm-packを観測した区間があり、時間は探索的観測として扱った。source・binary・入力hash、停止理由、終了状態を保存している。

最終候補の計測source SHA-256: `309c1579a92cc823b292c649d413a5280c60242d5118dc220ce78ed76536c98b`。
テストを追加して整形したsourceと、計測sourceのproduction部分が整形後に同一であることも確認した。識別子（ローカル記録: `tasks/photo-tsume-20261114/search-improvement/final-source-identity.json`）。

最終版の詰み関連テストは36件成功（失敗0）。深さの事前探索による反例、1/2/3plyの境界、完了結果を共有できる方向、反復依存の伝播、矛盾する範囲の共有停止、⑧の400万節点以内での証明を含む。Rustfmtと `cargo clippy --offline --release -p sekirei-core --all-targets ... -- -D warnings` も成功。テストログ（ローカル記録: `tasks/photo-tsume-20261114/search-improvement/final-core-mate-tests.log`）、Clippyログ（ローカル記録: `tasks/photo-tsume-20261114/search-improvement/final-clippy.log`）。

修正はcommit `24fca55526563252e139dd2ab47a423f708fd28f` に保存し、共有リポジトリの `codex/photo08-mate-depth-20261114` から参照できる。変更ファイルは `crates/sekirei-core/src/mate.rs` のみ。mainへの切替・統合は行っていない。レビュー用差分（ローカル記録: `tasks/photo-tsume-20261114/search-improvement/mate-depth.patch`）。

## 証跡

- ⑧の初手・最長枝・証明木の識別子（ローカル記録: `tasks/photo-tsume-20261114/search-improvement/final-solution.json`）
- ⑧の全証明木を含む出力（ローカル記録: `tasks/photo-tsume-20261114/search-improvement/threshold-candidate/8/stdout.txt`）
- 基準版検証器による別process再生（ローカル記録: `tasks/photo-tsume-20261114/search-improvement/threshold-candidate/8-replay/stdout.txt`）
- 固定14題の全比較（ローカル記録: `tasks/photo-tsume-20261114/search-improvement/T8-C4-comparison.json`）
- 再利用反例の対照（ローカル記録: `tasks/photo-tsume-20261114/search-improvement/T8-02-summary.json`）
- 探索内訳（ローカル記録: `tasks/photo-tsume-20261114/search-improvement/T8-01-summary.json`）
- 1〜4とコミット内容の完了監査（ローカル記録: `tasks/photo-tsume-20261114/search-improvement/completion-audit.json`）

本結果は詰み探索APIの修正と固定集合での確認。mainへの統合、公開リリース、通常対局の棋力確認は別の状態として扱う。
