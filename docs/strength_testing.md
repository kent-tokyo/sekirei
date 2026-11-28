# 標準の棋力テスト

棋力に影響しうる変更は、この文書の手順だけで採否を決めます（2026-11-27にユーザー承認）。
実験ごとの事前契約、ハッシュ固定の計画書、独立確認集合、ホスト負荷待ちのゲートは不要です。
ルールの要約は[AGENTS.md](../AGENTS.md)にあります。

## 共通の条件

| 項目 | 値 |
|---|---|
| 評価関数 | 両者ともr6（`sekirei-halfkp-r6-v0.5.0.bin`、`FV_SCALE=24`）。入手は[weights/README.md](../weights/README.md) |
| スレッド | 各エンジン1（`--threads-a 1 --threads-b 1`） |
| Hash | 64 MB（既定値） |
| 開始局面 | Sekireiで生成した開始局面。当面は既定の`data/gate/openings_standard.sfen`（ローカル、Git管理外） |
| ビルド | `cargo build --release`。比較する2つのバイナリは同じビルド設定で作る |

`--games-per-position`は偶数にし、「開始局面数 × N」が10,000局以下になるよう選びます
（runnerはこの積が`--games`を超えると起動しません）。

## 1. 変更の採否：SPRT（Sekirei同士）

ベースライン（変更前のmain）と変更後のバイナリを対局させます。

```bash
R6=/absolute/path/sekirei-halfkp-r6-v0.5.0.bin
python3 scripts/run_ab_match.py selfplay \
  --engine-a target/candidate/sekirei --engine-b target/baseline/sekirei \
  --evalfile "$R6" --fv-scale 24 \
  --byoyomi 200 --games 10000 --games-per-position N \
  --sprt 0,5 --paired-sprt \
  --name <変更名> --out-dir target/ab-match/<変更名>
```

- **H1採択**：マージしてよい。
- **H0採択、または10,000局で未決着**：不採用。同じ変更を再実行して合格を狙わない。修正した変更は新しいテストとして最初から実行する。
- 結果（LLR、勝敗分、対局数、両バイナリのコミット）をPRの説明に貼る。

並列対局（`--concurrency`）はmatch runnerへ追加中です。使えるようになったら
同時対局数を「物理コア数 − 1」以下にして付けます。

### 強さを変えない変更（非退行テスト）

リファクタリング・単純化・高速化など、強くなることを狙わない変更は
`--sprt=-5,0`（`=`が必要）で実行し、H1採択（弱くなっていない）ならマージします。

## 2. 版ごとの進捗：YaneuraOuゲージ

リリースや大きな変更のあとに、固定した相手との成績でEloを追跡します。

```bash
python3 scripts/run_ab_match.py yaneuraou \
  --engine-a target/release/sekirei --yaneuraou /path/to/YaneuraOu \
  --evalfile "$R6" --fv-scale 24 --nodes-limit <固定値> \
  --byoyomi 1000 --games 200 \
  --name gauge-<版> --out-dir target/ab-match/gauge-<版>
```

- YaneuraOuのバイナリと`--nodes-limit`は一度決めたら変えません（初回に勝率が50%前後になる値を選ぶ）。
  変えた場合は過去の値と比較しません。
- 結果（勝率、Elo差と95%区間、版、コミット）は版ごとに[ベンチマーク](benchmarks.md)へ1行追記します。
- ゲージは進捗の記録用です。変更の採否には使いません。

## 未決定の項目

- 10,000局に十分な数の開始局面ファイルを固定すること。
- ゲージ用YaneuraOuのビルドと`--nodes-limit`の値。
- このMacの物理コア数（並列対局数の上限）。

決まったらこの文書を更新し、[AGENTS.md](../AGENTS.md)は変えずに済むようにします。
