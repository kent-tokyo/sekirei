# 開発と検証

変更に関係するテストから始めます。文書の修正だけならビルドや対局は不要です。

## 文書・公開情報を確認する

```bash
python3 scripts/check_documentation_references.py
python3 scripts/check_release_metadata.py --allow-planned-release-manifest
python3 scripts/check_release_manifest_layout.py
git diff --check
```

参照チェックは英語・日本語READMEのローカルファイルを対象とします。
他文書のリンクや見出しへのリンクは別途確認してください。
メタデータ確認はローカルの整合性検査で、レジストリでの公開確認ではありません。

## Rustを変更する

```bash
cargo test -p sekirei-core --test halfkp_incremental
cargo fmt --all -- --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

最初のコマンドはNNUE増分更新の例です。変更箇所に合わせて対象を選びます。
並行して重いビルドを起動せず、このMacの空き容量・メモリ・他の実行中処理を確認します。

CIのRust行カバレッジは98%以上です。ローカルで同じ確認を行う場合：

```bash
bash scripts/check_rust_coverage.sh
```

`cargo-llvm-cov 0.8.7`、`llvm-tools-preview`、`jq`が必要です。
CIでは`sekirei-core`の公開APIをv0.3.73と比較します。
WASM固有の検証は[WASM README](../crates/sekirei-wasm/README.md)を参照してください。

## 実験・公開の記録

速度・指し手品質・棋力の確認は[ベンチマーク](benchmarks.md)で区別します。
計画の現在の扱いは[棚卸し](plans/README.md)、資源・データ・採用のルールは
[AGENTS.md](../AGENTS.md)にまとめています。

リリースごとの変更は[CHANGELOG](../CHANGELOG.md)、配布物と公開確認は
[リリース記録](releases/README.md)へ記録します。文書チェックだけで
リリースの公開完了や棋力改善を判定しません。
