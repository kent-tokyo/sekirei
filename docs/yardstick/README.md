# 固定の開始局面

[標準の棋力テスト](../strength_testing.md)で使う開始局面です。一度だけ生成し、
以後は変更せずに使い回します。測定用の固定セットで、独立確認用のコーパスではありません。

| ファイル | 局面数 | 用途 |
|---|---:|---|
| `sprt.sfen` | 500 | 変更の採否（SPRT） |
| `gauge.sfen` | 100 | 版ごとのYaneuraOuゲージ |

駒得評価の`gate_openings`（seed 20261127、1,600本）から、盤上と持ち駒の駒割りが
±200cp以内の局面を生成順に選びました。2つのセットに同じ局面はありません。
各行は`startpos moves ...`の手順付きなので、千日手の判定にも実際の履歴が使われます。
生成条件・ハッシュ・選んだ軌跡番号は`manifest.json`にあります。

再生成する場合（通常は不要）:

```bash
cargo build --release -p sekirei-core --example gate_openings
python3 scripts/freeze_yardstick_openings.py --seed 20261127 --out-dir docs/yardstick
```
