# CSA対局履歴とr6実行設定（2026-11-14）

## 対象と事前確認

CSA対局では探索ごとに現局面だけから履歴を作っていたため、サーバーが渡す初期着手と対局中の着手が千日手・連続王手の判定へ届かなかった。実戦用r6の評価器、尺度、Rayon worker数も一つの起動記録で固定できなかった。既定の材料評価と公開既定は変更しない。

## 変更

- 初期着手を合法再生し、以後の自手と相手手を王手情報付きで一度ずつ`PositionHistory`へ追加する。主探索とroot候補診断へ同じ履歴を渡す。
- CSAに`--threads 1..=64`を追加し、起動時のRayon worker数を表示・manifestへ記録する。
- r6用の有界LaunchAgentテンプレートを追加する。`--eval nnue --fv-scale 24 --threads 4`、重みSHA-256 `634bdf527e7306045c8278df1d5311777561dfb09c1ebe6829ff3f4d883cc469`、明示的な局数上限を固定する。資格情報はKeychainから起動時に取得し、plistに保存しない。テンプレートは自動起動しない。

## 確認と判定

公開`origin/main`の`0ee9d00`基点へ今回のCSA差分だけを適用した隔離worktreeで、CSA全117テスト、`cargo clippy -p sekirei-csa --all-targets -- -D warnings`、manifest検証スクリプト、LaunchAgent起動ヘルパーのテスト、Rust整形とplist構文検証が通過。合法な四回反復と連続王手、初期着手の不正入力、自手の二重記録を回帰テストで扱った。

別のローカル隔離HEAD `b86bee8`では、公開r6重みのSHA一致、release CSAバイナリでのHalfKPロード、尺度24、Rayon worker 4、有効な起動manifest、Keychainラッパーの非通信smokeを確認した。このバイナリは上記`origin/main`基点のソースと同一ではない。保存先はGit管理外の`data/runtime/csa-r6-profile-20261114/`で、状態は`ready_not_started`。

実サーバー対局、起動後の終局、棋力効果は未確認。今回の判定はCSA履歴の局所的な正しさとr6実行設定の準備完了に限る。実運用時はその時点のソースから再ビルドしたバイナリ・重み・尺度・worker数を固定し、1局有界の実通信と終局を別に確認する。
