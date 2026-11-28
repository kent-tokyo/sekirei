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
| 開始局面 | [固定の開始局面](yardstick/README.md)。SPRTは`docs/yardstick/sprt.sfen`（500局面）、ゲージは`docs/yardstick/gauge.sfen`（100局面） |
| ビルド | `cargo build --release`。比較する2つのバイナリは同じビルド設定で作る |

対局には`scripts/run_parallel_sprt.py`を使います。色を入れ替えた2局を1組として、
複数の組を同時に指します。同時対局数の既定値は「性能コア数 − 1」です（`--concurrency`で変更可）。
`run_ab_match.py`でも同じSPRTを実行できますが、1局ずつなので時間がかかります。

## 1. 変更の採否：SPRT（Sekirei同士）

ベースライン（変更前のmain）と変更後のバイナリを対局させます。

```bash
R6=/absolute/path/sekirei-halfkp-r6-v0.5.0.bin
python3 scripts/run_parallel_sprt.py \
  --engine1 target/candidate/sekirei --engine2 target/baseline/sekirei \
  --option-both EvalFile="$R6" --option-both FV_SCALE=24 \
  --positions docs/yardstick/sprt.sfen --byoyomi 200 \
  --sprt 0,5 --pairs 5000 --out-dir target/sprt/<変更名>
```

- **H1採択**：マージしてよい。
- **H0採択、または5,000組（10,000局）で未決着**：不採用。同じ変更を再実行して合格を狙わない。修正した変更は新しいテストとして最初から実行する。
- 結果（LLR、勝敗分、対局数、両バイナリのコミット）をPRの説明に貼る。

結果は`summary.json`（判定、LLR、勝敗、両バイナリのSHA-256）と`pairs.jsonl`に残ります。

### 強さを変えない変更（非退行テスト）

リファクタリング・単純化・高速化など、強くなることを狙わない変更は
`--sprt=-5,0`（`=`が必要）で実行し、H1採択（弱くなっていない）ならマージします。

## 2. 版ごとの進捗：YaneuraOuゲージ

リリースや大きな変更のあとに、固定した相手との成績でEloを追跡します。

```bash
python3 scripts/run_parallel_sprt.py \
  --engine1 target/release/sekirei \
  --option1 EvalFile="$R6" --option1 FV_SCALE=24 \
  --engine2 /path/to/YaneuraOu \
  --option2 EvalDir=/path/to/r6-evaldir --option2 FV_SCALE=24 --option2 USI_Hash=64 \
  --option2 NodesLimit=160000 --option2 USI_OwnBook=false --option2 BookFile=no_book \
  --option2 NetworkDelay=0 --option2 NetworkDelay2=0 --option2 MinimumThinkingTime=1 \
  --positions docs/yardstick/gauge.sfen --pairs 100 --byoyomi 1000 \
  --out-dir target/gauge/<版>
```

- YaneuraOuのバイナリと`--nodes-limit`は一度決めたら変えません（初回に勝率が50%前後になる値を選ぶ）。
  変えた場合は過去の値と比較しません。
- 結果（勝率、Elo差と95%区間、版、コミット）は版ごとに[ベンチマーク](benchmarks.md)へ1行追記します。
- ゲージは進捗の記録用です。変更の採否には使いません。

| 固定値 | 値 |
|---|---|
| YaneuraOu binary SHA-256 | `83ed10ca6708eda4dd7984096ebdd0adebcee115f4085eb0c003d169a4e5f26c` |
| `NodesLimit` | 160000 |
| `EvalDir` | r6を`nn.bin`という名前で置いたディレクトリ |

`NodesLimit`は、gauge.sfenで各20組（1手1秒、Apple M4・3並列）を試して選びました。
Sekireiの得点率は10000で97.5%、20000で95.0%、40000で97.5%、80000で70.0%、
160000で60.0%でした。35〜65%に入った最初の値です。

## 未決定の項目

なし。ゲージの固定値は2026-11-28に下の表のとおり決めました。

決まったらこの文書を更新し、[AGENTS.md](../AGENTS.md)は変えずに済むようにします。
