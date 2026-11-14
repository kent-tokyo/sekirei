# Issue #281: search-diagnostic の剪定 ablation

## 問いと範囲

公開 `main` の `57f85130` では、`--disable-nmp` と `--disable-lmr` は `PruningConfig` を変えるが、既定の `SEARCH_V2=1` の null move、主 LMR、後段の reduced probe はその設定を読まない。root と cooperative root helper の LMR も設定を読まない。既定値は変えず、診断スイッチが実際の探索深さを制御するかを確認する。対象は局所的な固定局面・最大10万節点、逐次1Tの機能テストとし、棋力・速度の判定には使わない。

## 実装と検証

V2 の null move と LMR、後段 reduced probe、root と cooperative root helper の LMR に `PruningConfig` の条件を追加した。V2 の浅い枝刈りに使う reduction-derived depth は既存のまま残す。既定の `PruningConfig` は両スイッチ有効であり、通常探索の経路は変更しない。

V2 本体を実行するテストでは、null move 無効時に試行回数0、有効時に1を確認し、LMR 無効時には reduced probe と fallback の深さ減少が0、有効時には reduced probe が発生することを確認した。root helper のテストでも無効時の reduced probe 0を確認した。3件の対象テスト、`cargo clippy -p sekirei-core --all-targets -- -D warnings`、`cargo fmt --all -- --check` は通過した。全workspace CIと coverage はPRで別途確認する。

これは診断の有効性修正であり、公開 Floodgate 局面の詰み初手選択や棋力改善を示さない。#280 の次の比較は、この修正を含む同一ソース・固定条件で行う。

## PR CI の受信期限テスト

PR 初回の coverage では、別領域の CSA テスト `active_receive_deadline_preserves_bounded_progress_and_partial_line` が `completed_attempts=0` で失敗した。150 ms の期限では、計測付き・混雑した runner 上で第1局の結果処理を終える前に期限切れになる。テストの期限を1秒にし、局間の待ち時間をそれより長く保った。局所の通常テストと `cargo llvm-cov` の同一テストは各1件通過した。実運用の既定期限は変更していない。全 CI の再結果は別途確認する。
