# 第7回電竜戦から学ぶSekireiの実装計画

作成: 2026-11-15。状態: **D7-01の局所実装と回帰検証、D7-03の登録済み局所停止診断は完了。D7-02・04〜06と実戦での効果判定は未完**。

## 入玉対応と停止確認を先に行い、弱点学習へ進む

目的は、このMacで使うSekireiの実戦棋力を上げること。最初に入玉宣言を対局・棋譜保存まで通し、終盤の停止遅延を測る。その後、自前の失着に根拠が集まった場合に、r6へつながる順位学習を小さく実装する。LayerStackと定跡拡張は条件付きの後続候補とする。

本計画は[棋力向上計画](strength-first-2026-11-14.md)の新しい着手順を補足する。M03の確認不合格、#280の独立H1条件、既存PGO・時間配分・NNUE・定跡試験の契約と判定は変更しない。初回は計画作成のみで、後続の継続作業でcore判定器に着手した。対局、学習、公開は未実施。

## 現行コードと結果から再利用するもの

調査開始時HEADは`d90f4be355f08e5b5bc089edf11e3791f341e60e`。並行作業で`baa77c1db3b8a1b8f10fad15db7aae28e6d9ae2c`へ進んだため、実装時はその時点のsourceを再確認し、隔離checkoutで固定する。共有checkoutの公開問題診断は本計画の編集対象に含めない。

| 確認したもの | 計画への反映 |
| --- | --- |
| [USI](../../crates/sekirei-usi/src/main.rs)と[CSA](../../crates/sekirei-csa/src/protocol.rs)に宣言勝ちを生成する経路を確認できない | root判定から送信までを実装対象にする。現時点はコード監査で、実行による失敗再現は未実施 |
| [match runner](../../crates/sekirei-match-runner/src/main.rs)は`win`を無条件に勝ちとする一方、`game_to_csa`は`EndReason::Win`を拒否する | 対局判定・棋譜保存を同じ変更単位に含める |
| [CSA学習reader](../../crates/sekirei-train/src/csa.rs)の`%KACHI`は実Floodgate例に合わせて直前着手側の勝ちと解釈する | 手番側が宣言する新しい棋譜をそのまま流すと勝敗が逆転し得る。宣言方式・勝者の情報と再読込み検証を先に設計する |
| [Budget](../../crates/sekirei-core/src/budget.rs)と投機探索には共有停止・別スレッド監視がある | 停止機構を重複実装しない。実際の応答と探索task終了を測る |
| [Rust順位学習器](../../crates/sekirei-train/src/main.rs)は自前SEKIRW系、[r6](../nnue_weights.md)はHalfKP 256x2-32-32 | 既存順位入力をr6へ直接渡せるとは扱わない。[HalfKP学習器](../../scripts/train_halfkp.py)の`--init-bin`と整数exportを再利用し、順位損失へのadapterを別工程にする |
| [NM01](../experiments/public_next_move_2026-11-15/RESULTS.md)・[NM02](../experiments/public_next_move_2026-11-15/RESULTS-NM02.md)は候補未成立、確認15題は未使用 | 同じ問題源の順送りを再開しない。この15題は元契約に留保し、新学習の教師にも使わない |
| [定跡利用率](../experiments/book_coverage_v0.3.68/README.md)は0/20、Finnyは既存、r6元教師rawは新実験に利用不可 | 定跡の全面拡大、Finnyの再実装、旧rawを前提とした訓練を着手条件にしない |

## 実装単位・依存関係・担当

主担当はCodexの統合担当。サブ担当は下表の独立レビューを上限4分で行い、共有ファイルは編集しない。実装・ビルド・探索・学習は一系統ずつ実行する。

| 順序 / ID | 問いと成果物 | 依存 | 完了条件 | 担当 |
| --- | --- | --- | --- | --- |
| 1 / D7-01 | 27点法のroot宣言を、USI・CSA・対局判定・棋譜保存まで正しく通せるか | 規則と棋譜の宣言方式の確定 | 境界判定、合法/不正宣言、保存→再読込みの勝者一致、旧API互換が成功 | 主担当。サブ担当は勝敗反転・時計境界をレビュー |
| 2 / D7-02 | 宣言可能局面へ向かう手と、相手に宣言を許す手を探索内で区別できるか | D7-01 | ルール伝播、反復・null move・TT・PV・得点表示の整合性、OFF回帰を確認 | 主担当。サブ担当はTTと終局得点をレビュー |
| 3 / D7-03 | 終盤でも期限と`stop`に応答し、子taskまで終了するか | D7-01の終局型。D7-02が長引けば先行可能 | 登録した遅延基準と全task終了を満たす、または原因を持つ再現例と限定修正を保存 | 主担当が資源を管理 |
| 4 / D7-04 | 自前の実戦から、同じ原因で改善できる失着を複数得られるか | 独立した入力と教師由来、既存診断の除外 | 元対局をまたぐ2件以上で、合法な代替手と再現条件を確認。満たさなければその入力源を停止 | 主担当。サブ担当は由来・split・代替手の根拠を確認 |
| 5 / D7-05 | r6と同じHalfKPで、弱点の順位学習が別局面へ転移するか | D7-04、独立した訓練/開発/確認集合 | adapterの整数整合、未学習局面での順位・重大誤り・同時間品質を確認 | 主担当。サブ担当はexportと符号をレビュー |
| 後続 / D7-06 | 局面別出力層または小さい定跡の方が、次の費用対効果で優れるか | 前段の結果から再順位付け | 一方だけを新ID・別契約で選び、最小診断を固定 | 主担当 |

D7-01のroot宣言は機能修正として完結でき、D7-02以降の棋力結果を待つ必要はない。いずれも既存統計試行の設定を書き換えない。

## D7-01: 宣言を通常の指し手と分ける

対象は平手の27点法と無効設定だけに絞る。[第7回本戦の基本規則](https://denryu-sen.jp/denryusen/dr7_prd/dr1_live.php)は27点法、最大512手、先手3分・後手10分に各手2秒加算。[CSA大会規則の第25条](https://www.computer-shogi.org/wcsc36/rule.pdf)を盤面条件の参照にする。大会固有の最大手数と終局の優先順は、利用する規則・server設定を実装前fixtureに記録する。24点法・駒落ち・トライルールは今回扱わない。

1. coreに`entering_king.rs`を追加する案とし、`EnteringKingRule`、手番側に対する純粋な成立判定、点数・敵陣駒数・不成立理由を返す。盤面・hash・NNUE・履歴は更新しない。既存の駒、持駒、敵陣マスク、王手判定を使う。既存依存で同機能が提供されるかは実装開始時に再確認する。
2. USI/CSAでは検索・定跡取得の前に判定し、内部の終局型で`DeclareWin`を表す。`Move`に偽の移動を追加しない。時計、手番、残り時間、送信済み状態はadapter側で検証する。ponder中の応答保留、`ponderhit`、`stop`との競合は既存状態機械に接続する。
3. USIは`bestmove win`を一度だけ返す。CSAは`%KACHI`送信後にserverの終局応答を待ち、ローカル判定だけで勝敗を確定しない。`ThinkResult`の「手なし」を投了と決めつけない。
4. match runnerは宣言を同じcore規則と時計で検証する。不正宣言は勝利に数えず、専用理由で反則終局とし、通常の学習WDLへ混入させない。正当な宣言をCSAへ保存する際は、宣言者・手番中宣言か着手後server判定かを追跡できる情報を残す。既存Floodgate例の解釈を一括反転しない。方式を確定できない入力は学習ラベルを生成せず理由を返す。
5. `EnteringKingRule=NoEnteringKing/CSARule27`を明示設定する案。旧API・一般起動の既定はOFFを維持し、27点法用の新規対局設定でONにする。規則IDを対局manifestへ記録する。公開struct `SearchConfig` / `SearchInfo`への必須field追加は避け、最初はadapterのroot判定として実装する。

検証は先後の点数閾値前後、敵陣の玉を除く駒数9/10、玉の内外・不在、王手、持駒のみの点数、敵陣外駒、龍馬と成小駒、時計切れを含める。持駒は点数に入るが敵陣の10枚には入らない。USI→runner→CSA→学習readerの先後両方のround-trip、宣言後の重複応答、通常手なしとの区別を確認する。既存の[USI競合テスト](../../crates/sekirei-usi/tests/usi_thread_race.rs)、CSA受信fixture、runnerの棋譜再生テストを拡張する。

成果物は焦点を絞ったソース変更、規則fixture、局所テスト結果、保存・再生のreceipt。棋力上昇の宣言は含めない。

## D7-02: 探索内の終局・TT・表示を揃える

新規のrules-aware検索APIと終局型を追加し、旧APIはOFFを委譲する設計を第一候補とする。変更の中心は`search.rs`、`search/v2.rs`、`tt.rs`とprotocolの得点表示。検索開始時の不変な規則をV1/V2、qsearch、Lazy SMP、投機子taskへ渡す。通常の静的評価・TT cutoffより前に宣言可能性を扱い、反復判定との優先順をfixtureで固定する。`Dfpn` / `SharedMcts`はD7-01のroot宣言だけの対応と明示し、探索途中の対応済みとは表示しない。新しいrules-aware APIの対象には加えない。

宣言勝ちを無条件にmate-scoreへ流すと、USIが「詰み」と表示する。内部の終局種別、通常評価と区別した得点表現、TT保存・復元、PV表示を一組で設計する。rootの宣言は`bestmove win`、先の宣言は合法手だけのPVと別の終局情報で表し、詰みの証明として扱わない。通常評価上限・mate帯との衝突を設計時に点検する。対応するTT表現を安全に用意できるまでは、宣言由来の結果をTTへ公開しない。

新しいrules-aware sessionは自身の規則専用TTを内部所有し、外部の任意`Arc<Tt>`を受け取らず、旧OFF APIとも共有しない設計を優先する。session内の規則切替は全worker停止・join後にTTを無効化する。公開の既存`Searcher`へ別規則を注入できる形にしない。同じTTを別Searcherが異なる規則で同時利用できるなら、自分のworkerをjoinするだけでは不十分である。規則を棋譜反復用board hashに混ぜない。仮想null moveからの宣言を実戦の確定勝ちとして公開しない。必要な検証探索が設計できるまでは、その経路の宣言判定を抑止する。詰将棋APIの連続王手・詰み証明の意味は変更しない。

一手先の自己宣言、相手宣言の回避、TTを温めた後のOFF/ON切替、null move、反復、停止、複数workerを確認する。OFF時は同一sourceの既定探索署名が不変であることを固定節点で確認する。ONの速度と棋力は別途評価し、1Tの結果から4T/8Tへ外挿しない。

## D7-03: 応答と子taskの終了を別々に測る

[dlshogiの今年の停止不具合](https://github.com/TadaoYamaoka/DeepLearningShogi/issues/89)は着想であり、Sekireiに同じ欠陥があるという診断ではない。既存Budget、RAIIによるjob終了、USIのabort/joinを使い、必要な観測だけを追加する。

最初のrelease診断D7-03aは、通常局面・応手多数・詰み直前・宣言直前・ponderhit直後・探索開始直後の6ケース。各3回、自然期限100msと外部stopの2条件で最大36呼出し、`SearchMode=Auto`、`Threads=1`、`MultiPV=1`、`SpecTopN=0`、総60秒、出力16MiB、Hash 64MiB、RSS上限512MiB。起動・ロード時間は探索応答時間から分けて記録する。この段階は逐次探索の応答だけを判定し、並列子taskの終了を確認したとは扱わない。

開始前に局面・送信時点・バイナリ・時計を固定し、暫定受入基準を`stop→bestmove ≤100ms`、自然期限超過`≤100ms`とする。これは2秒加算を想定した工学的な初期目標で、大会の保証ではない。強制停止した呼出しは成功に数えない。

並列終了の確認にはD7-03bを別attemptで必ず行う。`SearchMode=LazySMP`、`Threads=4`、`MultiPV=1`、`SpecTopN=0`を固定し、同じ36呼出し・60秒上限、RSS上限1GiBで逐次実施する。投機探索も確認対象にする場合はD7-03cとして`SearchMode=Speculative`、`Threads=4`、`MultiPV=1`、`SpecTopN=1`を同じ上限で追加する。各backendで実際にworker/taskが参加した証拠と、guard/join/既存活動カウンタによる全task終了を保存する。暫定基準は`stop→全task終了 ≤200ms`で、次の探索へ遅れた書込みがないことも確認する。`bestmove`だけを終了証拠にしない。設定名・受理された値は対象binaryの`usi`と照合して固定する。

ホスト状態が測れない場合やstrict条件がない場合は、機能・固定節点診断まで進め、遅延の合格は出さない。4T枠が確保できなければ並列終了は未確認で残す。違反があれば観測された経路だけを修正し、同じ再現例と別の確認例で検証する。基準を結果に合わせて緩めない。

## D7-04〜05: 有用な教師が得られたときだけHalfKP順位学習を作る

D7-04の初回は保存済みの自前実戦を対象に、由来が分かる別対局から開発8局面・未使用確認8局面を結果の観測前に選ぶ。局面不足なら収集までで止める。Lishogiの掲載解答は教師の重みidentityが不明なのでラベルへ使わない。Suisho5の実行ファイル・重み・評価・派生ラベルも使わない。新データはr6の失われた元rawとは別identityにする。

開発側を1T、各10万/100万節点、総60秒で診断する。不一致は直ちに失着とせず、最大4局面について選択手と代替手を同じ教師・共通の完了深さ・反復計測で比べる。教師段階は別枠で総120秒、各100万節点、最大16呼出し、全出力32MiB。詰みは別検証器による証明、通常評価は同じ深さの再現可能な順位に限定する。予算中断や未比較の手は未解決のまま残す。2対局以上で同種の救済可能な失着を確認できなければ候補を作らない。

D7-05は、上記の開発・確認対局と重ならない訓練対局を別に用意する。初回は最大16対局・32親局面の小さい教師集合でadapterを確認する。既存の完全root集合/事前固定候補和集合の契約と、[順位pair生成](../../scripts/build_root_rank_pairs.py)、[元対局分割](../../scripts/split_root_rank_pairs.py)、[手選択比較](../../scripts/compare_selected_nnue_moves.py)を再利用する。旧scriptの入力契約を偽装して通さず、必要なadapterと検証を追加する。

HalfKPの`--init-bin`からr6を読む経路へ、親手番に揃えた候補手順位損失を追加する。新しい全面的な学習器は作らない。初期重み、整数exportの尺度24、量子化、do/undoと全再計算の一致を先に検査する。更新なしのimport/exportで整数parameterが一致しない場合は学習を開始しない。Rust順位学習器へr6を無理に読み込ませたり、SEKIRW候補の結果をr6の改善と呼んだりしない。

最初の学習上限案は1候補・1seed・1epoch、5分、1 CPU thread（MPS使用時も他学習と排他）、RSS 2GiB、出力512MiB。小標本への当てはまりだけでは進めない。学習率・損失重み・初期値・訓練分割・進行規則は入力が揃ってから結果を見る前に別の実験manifestへ固定する。本書だけで統計試行の登録が完了したとは扱わない。

候補選択では整数正しさに加え、開発集合の順位改善/悪化、重大誤り、同時間の手選択、未解決件数、最大改善例を除いた感度を確認する。独立確認8局面は候補と規則を固定してから一度だけ開く。改善が開発例に限られれば停止する。対局へ進む場合は最大500色反転ペアの新規screenを別登録し、進行・futility・時間上限を先に決める。通過後のfresh-root確認と採用は別段階。既存M03や#280の結果とは合算しない。

## D7-06: LayerStackと定跡は、必要な方だけ選ぶ

- 局面分類ごとの誤りがD7-04〜05で再現し、同じ教師で改善余地が残る場合、共有feature transformer＋2つの小さい出力側networkを最初のLayerStack候補とする。1-head対照と教師・学習量・seedを揃え、分類式は観測前に一つへ固定。型/format識別、学習とRust推論の選択一致、整数do/undo、1T同時間品質までを実装単位とする。[SFNNヘッダ検査](../../crates/sekirei-core/src/external_eval.rs)だけでは推論対応ではない。巨大L1、Threat入力、GPU探索の同時追加は行わない。
- 定跡を選ぶ場合は、既存[lineprior adapter](../../crates/sekirei-train/src/book.rs)を使い、自前の頻出序盤の一分岐に生成費用を限定する。新しい独立20到達局面、1T・生成/再評価計5分を初回上限案とする。利用率、節約時間、離脱直後の手の品質を記録し、利用数だけで採用しない。旧0/20試験を延長・上書きしない。

どちらもまだ実装・学習契約は未登録。前段の結果と資源を見て一方を選び、入力と判定を新IDで固定する。[参加者申告](https://denryu-sen.jp/denryusen/dr7_prd/members.html)や[氷彗の5月の開発報告](https://saihyou.hatenadiary.jp/entry/2026/05/21/002515)は仮説の出典で、Sekireiでの効果の証拠ではない。公開rshogiは設計比較に使い、コードの転載を前提にしない。

## 資源・検証・停止条件

2026-11-15の計画作成中は空き約3.5GiB、16GiB RAM・free表示74%。`ps`はsandboxの制約で一覧を取得できず、競合jobの不在は確認していない。この情報は実行許可の代わりにならない。文書作成にエンジン計算は不要。

実装時の新規ビルド枠は`CARGO_BUILD_JOBS=1`、1回30分、target増加4GiB以内を初期上限とし、開始時6GiB以上の空き、実行中2GiB以上を確保する。既存の一致する成果物が使えれば再ビルドを省く。出力上限・RSS・時間は既存supervisorで監視し、既存機能で足りない上限だけ最小の外側監視を付ける。他job・メモリ圧迫・容量不足ならその計算を止め、原因と再開条件を残す。資源回復後のattemptは前の子process終了を確認して別IDにする。

各実装単位で影響するcore/USI/CSA/runner/trainの焦点テスト、fmt、Clippyを実施する。統合時はREADME記載のworkspace検証、公開API互換、適用される98% coverageを確認する。計測付きcoverageは探索の時間証拠にしない。全workspace検証・CIの資源枠は開始前に別登録し、局所成功だけで統合完了としない。文書だけの今回にengine testやcoverageは走らせない。

試験出力はGit対象外の`data/runs/denryu7-<ID>-<attempt>/`へ置き、source/binary/weight/規則/input SHA、設定、上限、開始・終了、停止理由をmanifestへ保存する。小さい結果表と判定を公開可能な実験記録へ残す。未資格重み・raw・認証情報は公開しない。

## 初期台帳と次の一手

| ID | 実装状態 | 検証・判定 | 次に必要なもの |
| --- | --- | --- | --- |
| D7-A00 | 調査・計画のみ完了 | sourceと既存結果文書を照合。独立レビューを反映し、文書リンク・公開境界・既存内容とindexの保全確認が成功 | D7-01へ。実装・棋力の検証は未実施 |
| D7-01 | core・USI・runner・CSA client/readerの局所実装・回帰検証完了 | core入玉4件、USI競合26件、runner本体98件とCLI1件、CSA全122件、train CSA10件、対象5 crate Clippy通過。先後の自分・相手宣言をfake serverで確認 | 実大会serverと棋力は未検証。明示ONのまま保持 |
| D7-02 | 終局得点型と規則専用の逐次参照探索を局所実装 | 対照局面の宣言回避、root/子node宣言、反復優先、abort、合法PV・盤面復元の6 core testとClippy通過 | OFF署名を凍結後、USIとV1/V2・並列経路へ安全に接続。同時間品質は未測定 |
| D7-03 | 1T・4T LazySMP・投機探索の登録済み局所診断を実施 | 観測可能な期限・stop応答は暫定基準内。4Tは36/36で4ワーカー、投機は36/36でtask起動と返却前残数0を確認。詰み近傍の自然期限は早期終局 | 実大会負荷・詰み近傍の期限は未確認。局所診断は完了 |
| D7-04 | 自前棋譜と除外を監査し、新しいr6開幕16群を固定 | 旧自己対局3run・計296局を外部archiveに確認。v039全局面は開発専用に封印。新開幕は訓練8・開発4・確認4群へ分割済み、合法性・既存identity除外を通過。自己対局は0局 | 資源事前検査が`REFUSE`（CPU/swap/競合job不明）。再事前検査と外部監督契約の後に収集 |
| D7-05 | 条件待ち | r6順位adapter未実装 | 救済例と独立した訓練/開発/確認入力 |
| D7-06 | 後続候補 | 実験未登録 | 前段の結果による再順位付け |

次の実装はD7-02のルール専用探索sessionと終局表現。runnerの`--entering-king-rule csa27`は判定側の設定であり、Sekireiを対局させるときは各エンジンへ`--engine-option1 EnteringKingRule=CSARule27`と`--engine-option2 EnteringKingRule=CSARule27`も渡す。実験開始前に両者の規則IDが一致することをmanifestで確認する。未確認のバックグラウンドjobが動くとは扱わない。

### D7-01a 実行記録（2026-11-15）

source HEAD `baa77c1db3b8a1b8f10fad15db7aae28e6d9ae2c` で `crates/sekirei-core/src/entering_king.rs` と `lib.rs` を追加。盤面を変更せず手番側の敵陣内駒点＋持駒点、玉を除く敵陣駒数、王手を判定する。最初のsourceレビューで「敵陣外の盤上駒も点数へ加算する」誤りを検出したため敵陣maskを適用し、その反例をtestへ追加した。先後閾値、9/10枚、無効設定、玉の内外・不在、王手、敵陣外の金、成駒を局所testに含めた。Cargoのテスト成否は未確認。

開始前の空きは430MiB、作業中も957MiB～2.0GiBで変動し、新規ビルド開始条件6GiBを満たさない。`ps`による競合job不在も確認できないためエンジンbuild/searchは起動しなかった。`rustfmt --edition 2024`、`git diff --check`は成功。D7-04の入力所在だけを読み取り監査したが、公開CSA集合や`data/selfplay`には外部由来素材が混在する。自前実戦8開発・8独立確認の由来を確定しておらず、集合はまだ凍結していない。既存NM03/NM04/NM05の並行作業ファイルは編集していない。

### D7-01b 継続記録（2026-11-15）

空きが一時11GiBに回復しCargo lock保持者も観測されなかった時点で、単一jobの局所試験を実施。core宣言4件、USI `go infinite→stop→stop` 1件、runnerの先後CSA保存と不正宣言2件、train readerの先後再読込み1件が成功した。readerの最初の0手fixtureは既存仕様で拒否されたため、合法な1手を入れ、runnerが生成したCSAと同じ共通fixtureをreaderで検証した。旧注記なし`%KACHI`の解釈は変更していない。

CSA clientは明示`--entering-king-rule csa27`でroot宣言を送信し、serverの`#WIN`前に学習用`%KACHI`を確定しない経路を追加した。runnerは不正`win`を宣言者の勝利にせず、CSAを出さず、独立した`InvalidDeclaration`として記録する。独立レビューで、秒読みを超えた応答と相手の手番中宣言を見つけたため、経過時刻判定とserver終局後の宣言者注記を追加した。判別不能なserver`%KACHI`はreaderが拒否する`unknown`注記へ送る。後二つの修正は未実行試験であり、正しさはまだ確認していない。

CSA clientの旧通信テストはコンパイル成功後、sandboxのlocalhost bindが`Operation not permitted`で失敗した。環境による未実施として扱い、機能失敗や成功に読み替えない。別のCargoが`target/debug` lockを保持した際はこちらの待機testを中断し、重複buildを避けた。空きは再び2.7GiBへ低下。次は6GiB以上・Cargo lock不在で新規のrunner時計、USI ponder、CSA純粋判定、共通reader試験と適用crateのClippyを順番に実施し、loopback fake serverを使う送受信は通信可能な環境でserver結果ごとの棋譜を照合する。既定変更・実戦・学習・棋力判定はなし。

文書検証: `git diff --check`、`check_documentation_references.py`、`check_public_surface.py`、編集3文書のローカルリンク検査を実施。日本語lintはSudachiPy未導入のため実行できず、natural-japaneseの手動チェックリストで確認した。外部の計算・コード試験の成功を意味しない。

### D7-01c 継続監査（2026-11-15）

空き容量は2.1GiBで新規ビルド開始条件6GiBを満たさず、Cargo lock保持者は観測されなかった。最新差分を静的に点検し、`git diff --check`は成功した。前記D7-01bの後に加えたCSAの相手手番中`%KACHI`注記、runnerの秒読み超過・不正宣言WDL除外、USIの`ponder infinite`試験はまだコンパイル・実行していない。次の空き容量回復時には、まずCSA純粋判定とrunner時計・USI ponderの局所試験を単一Cargo jobで実施し、続いて許可されたloopback環境でfake serverの先後宣言とserver結果を照合する。結果が揃うまでD7-01完了、既定採用、棋力改善とは判定しない。

D7-04の入力所在も再点検した。`data/runs/`の旧対局ディレクトリ88件は外部SSDへ退避済みで、現存する`data/selfplay/res/U4Qa/games/`の例はSekirei対YaneuraOuである。現存する対局ファイルの数だけで、自前実戦の由来、既存holdoutとの非重複、開発8・確認8局面の独立性は証明できない。集合を凍結せず、対局元manifestと既存除外台帳を照合できる入力を再開条件にする。

### D7-01d 局所検証とD7-02着手（2026-11-15）

空きが52GiBへ回復しCargo lock保持者がいないことを確認して、単一jobで未検証箇所を実行した。CSAの独自初期配置は旧readerが`P1`〜`P9`を読み飛ばすため、clientの棋譜にbase SFENを着手より前に注記する修正を追加。保存側とreader側の局所試験が通過した。localhost bindは通常sandboxで拒否されたため、loopbackだけのfake server試験を承認済みの実行環境で実施。先後×自分/相手の4経路で`%KACHI`送受信、serverの`#WIN/#LOSE`、宣言者注記、重複なしを確認した。

適用5 crateの`cargo clippy --all-targets -- -D warnings`、CSA全122件、runner本体98件とCLI1件、USI競合26件、core入玉4件、train CSA10件が通過。runnerの秒読み超過、USI `ponder infinite`、旧注記なし`%KACHI`の解釈も対象試験に含む。これは局所実装・回帰の判定であり、実大会server、探索内の宣言、速度、棋力、既定採用の証拠ではない。D7-02のsource監査では旧`Searcher`が外部`Arc<Tt>`を受け、V1/V2/qsearch・投機探索が整数mate帯とTTを共有するため、単純な宣言得点追加は不可と確認した。次は規則専用session、型付き終局、TTへ公開しない境界を先に固定する。

### D7-02a 設計入口（2026-11-15）

対象source `baa77c1db3b8a1b8f10fad15db7aae28e6d9ae2c`。`Searcher`と`SpeculativeSearcher`は外部`Arc<Tt>`を受け、`SearchState`が各workerへ共有する。TTは64-bit data wordの下位4bitをgenerationに使用し、`TtEntry`は得点・深さ・bound・指し手だけを持つ。V1/V2のmate distance、qsearch、TT cutoff、USI score表示が整数scoreを前提にする。宣言をmate帯の整数へ直接写すと詰み表示になり、別規則の共有TTも汚す。

最初の実装単位は、旧OFF APIを維持したまま規則専用TTを内部所有するsessionと、宣言を詰みから区別する終局表現の契約・試験とする。成功条件は、別規則のTTを渡せない型、先後の一手先宣言・相手宣言回避・OFF固定節点署名のfixture、null moveでは宣言を確定しない反例を先に示せること。次にV1/V2/qsearch/並列workerへ不変の規則を通す。得点の予約帯だけで種別を推測する設計や、どこか一経路だけへ数値を足す変更は採用しない。初回は1T・固定節点・1局面最大10万節点、合計6局面・60秒、Hash 64MiB、出力16MiBまで。新しい強さの統計対局は別の事前登録と独立入力が揃うまで開始しない。

独立したsource監査で、V1/V2のnodeと両qsearchはいずれも整数を返し、TTのboundも整数得点だけを保持すると確認した。したがって規則専用TTを作るだけでは、宣言由来の値が親のcutoffやPVで通常評価/詰みへ変わる問題を防げない。次の編集前に、数値と宣言由来の種別を一緒にnegamax再帰へ運ぶ型、Mate距離との順序、abort時の無効値を具体化する。初回ON実装ではTT得点probe/storeとnull moveを抑止し、V1/V2/qsearch・別実装の`spec_alpha_beta`へ伝播してから限定的に有効化する。TTの空きbound code一つでは宣言由来のLower/Upperまで表せないため、既存64-bit TTへ安易なbit割当はしない。`Dfpn`/`SharedMcts`はroot限定のまま区別して報告する。

### D7-02b 境界fixtureと実装分割（2026-11-15、試験前登録）

現行sourceで`Searcher::search_impl`→`root_search_with_window`→`alpha_beta`を確認。`alpha_beta`は子節点の最初にV2へ分岐し、V1/V2のqsearchと投機専用`spec_alpha_beta`も別の戻り値・TT経路を持つ。単一の判定箇所に整数得点を足す案は棄却した。最初の低費用checkは、先手が次手番で27点宣言でき、後手の静かな玉移動では宣言を許す一方、飛車の王手なら防ぐ1局面・2合法手をcore fixtureに固定すること。固定SFENは`5+B+B+R+R/4K+S+S+P+P/7GG/9/r8/9/9/9/4k4 w 2P 1`、比較手は`5i6i`と`9e5e`。二つの着手後の`DeclarationCheck`とundo後のhash/整数NNUE/手番を検査する。これが失敗すればfixtureを修正し、探索試験へ進めない。このcheck自体は探索の改善証拠ではない。

次の実装順は、(1) 規則を作成時に固定した専用sessionとTT所有境界、(2) abortと通常評価/詰み/宣言を区別してnegamax・rootへ運ぶ結果型、(3) V1/V2と両qsearchで反復・宣言の優先順を固定し、ONのTT得点とnull moveを抑止、(4) Lazy SMPと投機子taskへ同じ規則を伝播して停止・PV・表示を確認、(5) OFFの固定節点署名とONの同時間品質を別に確認する。各段階の接続が終わるまではONの探索途中対応を公開しない。最初の機能診断は既定OFF・1T・最大6局面×10万節点、総60秒、Hash64MiB、出力16MiBとし、棋力試験は別契約とする。

### D7-02b fixture結果と得点型の境界（2026-11-15）

固定SFENの`5i6i`・`9e5e`はいずれも合法で、前者の後の先手は宣言可、後者の後は`InCheck`で宣言不可。両枝でundo後のhash・整数NNUE二系統・手番が元に戻る。core test `one_ply_opponent_declaration_can_be_prevented_by_check`と対象Clippyが通過。fixture source SHA-256 `dc87613aa30b8a102e470e86834c96d70315d0a73ddc309c7f023fbbaf586442`。この結果は探索が`9e5e`を選ぶ証拠ではない。

USIの既存score表示は`±899000`以上をmateとして扱い、HalfKP推論は整数値をそのまま返す。宣言に予約整数を与えるだけでは、通常評価の上限、mate距離調整、alpha-beta window、TT boundとの衝突を排除できない。次の実装前に、通常評価・詰み・宣言・中断を明示型で順序付け、親への符号反転と同点時の扱いをfixtureで固定する。ONのTT得点を無効化した状態でもPVを合法に保持できる経路を設計する。数字だけをタグの代わりにしない。

この時点で空き容量は約27GiB、`memory_pressure`のfree表示64%。前回確認した33GiBから約6GiB減ったが原因は未確認で、sandboxの`pgrep`もprocess一覧を取得できない。自分のCargo/engine jobは終了済み。D7-04用runner/probeの新規release buildと自己対局はこの節では起動せず、先にD7-02の得点型・TT境界を進める。新規自己対局を始めるときは、空きと稼働jobを再確認し、開幕生成・game-level分割・時間/RSS/出力上限を別IDで観測前に登録する。

### D7-02c 終局得点型の最小契約（2026-11-15）

`search/outcome.rs`に`RuleScore`（有限centipawn、詰み・宣言・連続王手千日手由来の勝敗と手数）、`RuleBound`（Exact/Lower/Upper）と`RuleNodeResult::Aborted`を追加した。勝ちは通常評価より高く、負けは低い。勝ちは短い終局、負けは遅い終局を優先し、同手数の種類は決定的な順で比較する。合法手1手を親へ戻すと勝敗とLower/Upperが反転し手数が1増え、abortは得点にならない。`i32::MIN`は有限評価として作れない。source SHA-256 `e5864240e026121ca66e0acb8ae07ec9fe3deca2f8a13225a3bd9d63479ff3cf`、対象3 testとcore Clippy `-D warnings`通過。これは結果型と順序の契約だけで、現行V1/V2/qsearchやUSIのON探索へは未接続。TTの宣言由来得点公開、null move、PV、並列workerは従前どおり未対応。接続前にOFF固定節点署名を取り、ON専用sessionのTT所有境界を設ける。

追加の読み取り監査ではroot aspiration/PVS/root split、V1/V2と両qsearch、投機taskの`AtomicI32`結果がいずれも整数窓に依存する。型だけを子再帰へ渡してもcutoff下限を確定した宣言・PVとして表示してしまう。最初はOFF経路を保持したまま、専用TTを所有するONの逐次・型付き参照探索を別経路に作り、TT得点・null move・aspiration・PVS・root split・投機taskを使わず、合法手PVと中断を端から端まで検査する。これを正解照合用に置き、V1/V2・両qsearchを順に対応させて同時間品質を別に測る。参照経路の遅さをON本番の許容性能と読み替えない。現在のUSI `EnteringKingRule=CSARule27`はroot宣言だけで、探索途中のON対応を表示していない。

次の`D7-02d`は参照経路の正しさだけを調べる実装診断とする。対象は既登録の一手先宣言回避fixture、先後の自己宣言、反復、通常開始局面、停止の最大6局面。1T・局面ごと最大2万節点・深さ3・Hash64MiB・総60秒・出力16MiB。実行前にsource/binary/input identityを保存し、root/子nodeの宣言、反復が宣言より優先すること、abortが未完の値を確定結果にしないこと、undo後のboard/hash/NNUE、PVの全手合法性を検査する。参照探索からの同時間品質や棋力の推定はしない。既登録のD7-02b実装診断上限（6局面×10万節点）を拡張・流用した判定にはしない。

`D7-02d`の実装・局所確認: `search/rules.rs`に規則固定・専用TT所有の`RuleSearchSession`を追加した。現段階ではTT得点probe/storeを行わず、逐次の型付き全展開と捕獲/王手応答qsearchを参照経路として用いる。root宣言は手なしの`Win{Declaration,0}`、一手先の相手宣言は親で`Loss{Declaration,1}`、反復は宣言より先、節点上限によるabortは未完得点を公開しない。対照fixtureで参照探索全体が宣言を許す静かな`5i6i`を避け、合法PVとundo後のhash/NNUE復元も確認した。対象6 test、core Clippy `-D warnings`、`cargo fmt --all -- --check`、`git diff --check`が通過。`rules.rs` SHA-256 `14fcd4c2bab1307cab7a2759c495e5a45e646409862e63fbc3cdf7999b8d856a`。まだUSI・V1/V2・LazySMP/投機へ接続しておらず、同時間品質も未測定。これはON本番対応やD7-02の完了ではない。計画内の6局面すべてを独立release binaryで計測した診断でもなく、上記はcore局所testの範囲に限る。

ユーザーの「キリが良いところで止めましょ」を受け、上記6 testと静的チェックを区切りに作業を停止。再開時は現在の空き約11GiBと競合jobを再確認し、USI接続前にOFF固定節点署名を凍結する。D7-04自己対局は0局で、D7-05/06も未着手のまま。

### D7-03a 初回応答診断（2026-11-15）

現行release binary SHA-256 `e21b4c042de583dd532798553a572d6f05d8f4a1ec5ed3d535875d97af097902`。結果観測前に`data/runs/denryu7-D7-03a-20261115/contract.json`（SHA-256 `f82070d62eeb6ca6995f2b42f45e1caa39782c5039365f1e76c9efa8f9600c8e`）へ6ケース×2条件×3回、1T、Hash64MiB、60秒・RSS512MiB・出力16MiBを登録。観測36件、総1.23秒、最大RSS196,526,080 bytes、出力9,803 bytes、強制終了なし。外部stopの有効観測12件は最大0.186ms、6件は`stop`より前に終わった。自然期限18件はすべて100msより前に終了し、登録条件では期限応答の証拠にならない。結果は`PARTIAL_STOP_OBSERVED_NATURAL_UNOBSERVED`。raw `observations.json` SHA-256 `636478d28e38d1f23db8d75dd593326f51bf7467a79e9130f19d77947aa118ff`、transcript SHA-256 `554ea73a1616beb7447cdde3b338c1d1211ab198ce5efb9b9157242f848b6526`。1Tの応答は並列task終了の証拠ではない。

原因は実測後にsourceで確認した`MoveOverhead`既定50msで、`go movetime 100`の実効期限が50msになること。初回を合格に読み替えず、別ID `D7-03a2-20261115-a1`で`MoveOverhead=0`、同じ100ms基準・36呼出し上限を結果前に再登録した。応手多数の局面は自玉を守る既存合法fixtureに替え、詰み近傍のstopは即時送信にした。二attemptは合算しない。a2も自然期限が成立しなければ観測不能のまま残す。

### D7-03a2 実効100msの別attempt（2026-11-15）

契約SHA-256 `934fa0a757a33d1e72715d5d9b7a5be97b47ec693829f1bac3bcc65abef7ae78`、binaryはD7-03aと同一。36呼出し、総2.05秒、最大RSS196,935,680 bytes、出力10,270 bytes、強制終了なし。自然期限18件のうち15件は実際に100msまで探索し、最大超過1.806ms。詰み直前3件は0.16〜0.20msで早期終局し、期限応答を測れなかった。stop18件はすべて`stop`後に応答し、最大0.198ms。`1T_STOP_AND_OBSERVED_DEADLINE_PASS_WITH_EARLY_MATE`とする。これは測れた呼出しの工学的基準だけで、詰み近傍の期限、4T/投機子task終了、実大会の負荷、棋力を証明しない。raw `observations.json` SHA-256 `82971fe85545d49491427d5bf600acffaee652255bf646748385962c1b9ab60c`、transcript SHA-256 `02a9584bf09991c2e95770c81a70133b6727b4e0ddeedbbf74bd5292f4444a63`。

次はD7-03bの4Tを別契約に固定して、全workerが参加した記録と`stop`後のjoinを確認する。a1/a2の数値は合算しない。

### D7-03b 4T LazySMPの別attempt（2026-11-15）

観測前登録の契約SHA-256 `4a775ef14d3d9634645bcd6a6fcdefbf9497870191c932e4a4b24887479ca0bb`。D7-03a2と同一binary、規則OFF、`LazySMP`・4T・`MoveOverhead=0`、36呼出し、総60秒・RSS1GiB・出力16MiBを上限とした。実測36件、総2.89秒、最大RSS309,182,464 bytes、出力22,325 bytes、強制終了・protocol errorなし。自然期限18件のうち15件で100msまで探索し、最大超過26.667ms。詰み近傍3件は約1.1〜2.5msで早期終局。stop18件はすべてstop後に応答し、最大12.467msだった。

transcriptには全36件で`w0`〜`w3`の節点数が正値として記録され、各診断行は対応する`bestmove`より前に出た。`LazySmpSearcher::search_with_history`は4 workerの並列iteratorを`collect`で待ってからworker結果を構成し、USIはその戻り値を受けて`bestmove`を出す。従って、この試行で観測された`stop→bestmove`は4 workerの終了時間の上界でもあり、登録した100ms/200msの暫定基準内。直後の`isready`にも古い出力はなかった。これはOS上の各スレッド消滅時刻の独立測定ではなく、探索呼出し内の参加workerのjoinと出力順の証拠である。判定は`4T_LAZYSMP_OBSERVED_STOP_AND_DEADLINE_PASS_WITH_EARLY_MATE`。raw `observations.json` SHA-256 `b4657166e7a1f4d942cf216839e8542699daff4d3ec645adbde61e763e4f8aed`、transcript SHA-256 `5424c729e6588e5cf6965ac3f31acbde9a96738a76e3206dd29aaa6f01aaefdc`。1T、4T、初回の結果は合算しない。

### D7-03c 投機子taskの終了境界（2026-11-15、実装前）

source監査では`SpecGroup::drop`は非昇格taskへabortを通知するだけで、昇格taskと非昇格taskの終了を待たない。`SpeculativeSearcher::search_with_history_on_worker`は最後に共有Budgetをabortするが、その後すぐ結果を返し得る。仮説は、この境界で古いtaskが`bestmove`後も実行中となり、次の`usinewgame`/TT clearと競合し得ること。最小修正は検索呼出しごとのin-flight countをspawn前に増やし、task本体のRAII guardで必ず減らすこと。最終abort後に0を待ってから結果を返し、既存のTT・評価・探索方針は変えない。まずunitで待機とpanic時のguardを確認し、次に単一release診断を新IDで固定して`Speculative`・`SpecTopN=1`、100ms自然期限/stop各18件、総60秒・RSS1GiB・出力16MiB、`stop→bestmove ≤100ms`・全task終了≤200msを確認する。USIの`Threads=4`はこのbackendのpool数を決めず、実poolは`SpecTopN+1=2`であることを明記する。修正後の計測を修正前の4T結果へ合算しない。

### D7-03c 修正・投機探索の別attempt（2026-11-15）

`SpecState`へ検索呼出し専用のtask起動数とin-flight countを追加し、各taskのRAII guardで終了を記録。最終Budget abort後に残数0を待ち、PVを読む前に全taskがTTへのアクセスを終える。USIは`speculative_tasks spawned/remaining/join_wait_us`を`bestmove`前に出す。待機が実際に動作する試験、panic時にcountが残らない試験、既存の投機11試験、期限試験（task起動数正・返却時残数0）が通過。これは別規則のTT安全性や棋力の証拠ではない。

release binary SHA-256 `a1305ee1db70c928bbe38013458130242e50bc3165d810ae6fdd8a6cd201641e`、観測前契約SHA-256 `e6ad0cf1d8fa3250b217918776b3a9f703321d387b3e29f376f6dc270a800103`。36呼出し、最大RSS197,558,272 bytes、出力9,117 bytes、error/強制終了なし。自然期限18件中15件を100msまで観測し、最大超過1.909ms。詰み近傍3件は約0.16〜0.19msで早期終局。外部stop18件はすべて応答し、最大0.190ms。診断行は36件とも対応する`bestmove`より前、起動数は1〜21で全件正、返却前残数は全件0、最終待機時間は最大6μs。`SpeculativeSearcher`は`SpecTopN=1`で2-thread専用poolを作り、`Threads=4`はこのpool数へ作用しない。返却時刻は全task終了の上界となるため、登録した100ms/200msの暫定基準内。判定`SPECULATIVE_OBSERVED_STOP_DEADLINE_AND_TASK_DRAIN_PASS_WITH_EARLY_MATE`。raw `observations.json` SHA-256 `aac98202df930351efd881f4b50f5d139fad77f5c83759c9540f0d6d0ec3c6bd`、transcript SHA-256 `ccb5cfa9e39b3393de743c0cf14a5a98dfdd28c4f86512fe2957ea331bb6ddeb`。初回・1T・4T結果と合算しない。詰み近傍の自然期限、実大会の長時間・高負荷は未確認。

計測後に`search.rs`の追加assert一行だけをrustfmtの推奨改行へ整えたため、現在の作業treeのSHAは契約内の計測時source SHAと異なる。動作式・release binary・raw attemptは変更していない。以後の計測では現行sourceを改めて固定する。

### D7-04a 入力所在の再監査（2026-11-15）

外部archive `/Volumes/ESD-EPA/Projects/sekirei_data/runs-through-20260922.tar.zst`の目録から、Sekirei同士の自己対局を3run、計296局確認した。2026-09-18の二runは旧v0.3.38重みの96局と100局、09-19のmaterial-only runは100局。後者は既存の定跡試験で1〜80局を学習、81〜100局を0/20 coverageの未使用確認に割り当てているため、D7-04の新しい独立確認として再利用しない。現在のr6に対する教師有効性、他のNNUE学習・holdoutとの重複、旧二runの個別棋譜identityは未監査で、8開発＋8独立確認局面はまだ凍結していない。次はarchive内のmanifest/CSAハッシュを既存除外台帳へ照合し、利用可能な入力だけを固定する。旧教師をそのままr6の正解ラベルと扱わない。

### D7-04b 旧対局の独立性判定（2026-11-15）

archive内の`local_selfplay_20260918T1204Z_v039_review/REPORT.md`と`validation-exclusion-ledger.json`を再読込した。v039の100棋譜は74一意、31開始局面群・5,033 canonical着手前局面が全て`sealed_development_only`。旧v038の96棋譜と9件のCSA SHAが完全一致し、さらに739局面・19開始局面群を共有する。v039の9局面はさらに旧holdoutと重なる。したがってrun名で分割しても独立確認にならず、v039は開発用の既知例に限定する。

archiveの196 CSAについて09-19の定跡100局との完全file SHA一致は0。v039再生位置と現行r6の確認できた開幕除外集合、NM01〜05の64位置は照合範囲で一致0。ただしNM06以降、r6訓練位置、元対局単位の全重複を覆う検証ではない。既存r6学習manifestの訓練元は別のgen50〜65 gensfen対局と記載されているが、位置一致までは未検証。この旧archiveだけから独立8確認局面を作る案は`NO_FRESH_CONFIRMATION_INPUT`で停止し、開発局面の選定も新しいgame-level除外manifestを固定してから行う。再開条件は新しいr6自己対局または由来を照合できる別の自前棋譜である。

### D7-04c 新しい自前入力の準備契約（2026-11-15、実行前）

旧archiveを確認用へ転用しない。現行r6重みをSekireiのみに読み込ませ、既存の`run_local_selfplay.py`で新しい自己対局を収集する案。まずその実行に必要な`sekirei-match`と`nnue_probe`を単一Cargo jobずつrelease buildし、bin/weight SHAとNNUE activationを固定する。各buildは30分・target増加4GiB以内、開始時の空き6GiB以上、実行中2GiB以上。buildは対局の結果観測に含めず、失敗や資源不足なら対局を開始しない。

対局前に、Sekirei r6のMultiPV上位3手から固定seedで分岐した12手前後の開幕を最大16群作り、合法性・canonical SFENの一意性・既存確認/学習除外との重複を先に検査する。16群が得られなければ収集を停止する。群単位で訓練8・開発4・独立確認4へ先に割り当て、各群を先後2局ずつ、最大32局、1T・Hash64MiB・`SpecTopN=0`・定跡OFF・r6 absolute・27点法OFFで逐次実施する案。対局予算は各手200ms、最大192手、総25分、RSS1GiB、出力512MiB。別のraw契約へ開幕/実行ファイル/重みhash、停止条件、分割IDを結果前に登録してから開始する。使用可能な別対局が8/8/16を下回れば不足をそのまま記録し、分割後に局面を移さない。これは教師候補の入力収集であり、棋力試験でも採用根拠でもない。

準備結果: `sekirei-match` SHA-256 `f798515d3513547c48981db46e8f5ce80d4cef8fac1a31d1414f99920482c246`、既存engine `a1305ee1db70c928bbe38013458130242e50bc3165d810ae6fdd8a6cd201641e`、r6重み `634bdf527e7306045c8278df1d5311777561dfb09c1ebe6829ff3f4d883cc469`。旧`nnue_probe`はSEKIRW01専用でr6 HalfKP 64,216,957 bytesを拒否したため、対局を開始せず、私有HalfKP networkの構造・局面出力・再読込を確認する経路を追加した。修正後の現行release probe SHA-256は`05fcfcd4f6c3072ec6c4fdcdbef26016e15b34e5ce581e3494297cd847600ada`。`--json --strict --nnue-output absolute`で通過し、出力幅1916cp、再読込一致。既存engineのUSI handshakeはreturncode 0、`readyok`、HalfKP読み込み確認あり。HalfKPのstrictはファイル構造・標本の非定数/感度・再読込一致までで、旧SEKIRW01の層多様性検査を主張しない。この時点では開幕除外・分割・対局契約は未固定だった。

開幕生成attempt `D7-04c-openings-a1`を結果前に固定する。`scripts/generate_d7_r6_openings.py`、上記engine/r6重み/validator、2026-10-20の`r6-current-main-a60be2c-20261020/openings-4t/exclusions/0`〜`5`の完全scope manifest、seed `20261115`、最大64 trajectory、各12手、1手500節点、3候補からseed付き一様選択、総180秒・USI出力16MiB。16個の最終canonical identityと6手時点のfamily identityを一意にし、各6〜12手のidentityを除外集合へ照合する。選抜順の最初8群を訓練、次4群を開発、最後4群を確認へ結果前に割り当てる。`sekirei-match validate-positions`を全prefixへ通し、全入力hashを再検査する。16群未満・検証失敗ならこのattemptは不完全として対局へ進まない。この除外はidentity単位であり、全対局の将来の合流や未記載holdoutとの非重複を証明しない。

`D7-04c-openings-a1`は16/16群を選び、6手時点のfamilyと12手の最終identityはいずれも16個一意。268回の500節点探索、12 trajectoryを重複等で除外、USI出力94,119 bytes。`sekirei-match validate-positions`は16局面有効。driver SHA-256 `0ddfc759eba8df9b0e6ed132fbe6025849e7496c27ef22c854aeafef34d6d9cd`、契約 `be66fbf382a64e5af31c279f66e8d3c01d5af7e642b65a3c6260dc4a77ddaaaa`、manifest `8badd722aa9317bc204df24d9ef67e983043ed340231a9029cb5fb74f53f8c0e`、全開幕 `e267476841c6f4c71955918b9d890466fdd2bd96ceaf778d7dc40248a119c178`。各splitは訓練8・開発4・確認4で凍結。新しい対局は0局。

`run_local_selfplay.py --dry-run`の訓練split事前検査では位置、HalfKP strict probe、engine activationが通過（このdry-runのprobeは上記現行build前のSHA `e221af6b6888568ffdf00a227774bf22e83dc13a390ac9b07a99f9d8a6cda763`）。現行buildでもstrictを再確認済み。ただし併記された資源判定は`formal_gate_ready=false` / `REFUSE`。このsandboxではCPU数・swap・競合jobの観測が`UNKNOWN`、1分load 4.43、空きメモリ5.14GB、空きディスク23.0GBだった。形式的な対局gateでなく入力収集であっても、他の重いjobとの競合を排除できないため、このattemptでは対局を開始しない。再開条件は競合jobと資源が観測できる状態での独立した再事前検査、25分・1GiB・512MiBの外部監督契約固定。
