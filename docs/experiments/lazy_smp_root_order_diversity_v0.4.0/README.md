# Lazy SMP initial root-order diversity (v0.4.0 base)

Status: rejected for adoption on 2026-10-13. The playing default is unchanged.

## Candidate

`LazyFlags` bit 512 keeps worker 0's root order unchanged and rotates helper
worker `n` by `n` moves for its first root iteration. Ordinary root-order
feedback replaces this seed after the first completed iteration. No evaluator,
pruning constant, result selector, time budget, or TT policy changes.

The source base was `origin/main` at
`5699d5fe51bb8d0c5ab4799705b8fdca7df1a96c`. The arm64 macOS 27.0.1 binary
used for the screen had SHA-256
`160b3944841da46860e75581668bad2fbb13d466805afbfea563ece8043b866a`.

## Fixed diagnostic

Both lanes used material evaluation, `Hash=64`, four threads, 300 ms, two
repeats, and the first 20 positions from `openings_standard.sfen` (whole-file
SHA-256 `2240cc495e02d50befd45b445f4218d84ae6cdff355db952bdf776f82ad1a1e5`).
The candidate was `LazyFlags=527`; the baseline was `LazyFlags=15`.

The counter-enabled pass kept median depth 17 and repeated-bestmove stability
70% in both lanes. Divergent non-selected root work fell from 32.9% to 25.3%,
while total non-selected root work was effectively unchanged (75.0% versus
74.9%). Foreign-worker TT reuse also stayed close (40.7% versus 39.7%).

Two counter-free passes reversed execution order:

| Pass | Baseline median depth / NPS / stability | Candidate median depth / NPS / stability |
|---|---|---|
| baseline then candidate | 17 / 5.67M / 82.5% | 18 / 6.20M / 80.0% |
| candidate then baseline | 18 / 7.50M / 70.0% | 18 / 7.00M / 75.0% |

The host throughput changed materially between passes, so the NPS figures are
not pooled. The candidate did not lower median completed depth in either pass,
which was the diagnostic advancement condition.

## Exploratory match

A 32-game, colour-alternating material-only screen used the same binary,
four threads per side, 100 ms per move, `UseBook=false`, and flags 527 versus
15. The candidate scored 15 wins and 17 losses, or -22 Elo with an approximate
95% interval of plus or minus 126 Elo. This only rules out a very large signal;
it is not an adoption gate, and the positions overlap the diagnostic corpus.

## Independent gate outcome

The preregistered gate froze binary SHA-256
`7640b6308d99b2285a9761846ba771c312828f8f8605a0fe20e161b52ee861d5`,
opening SHA-256
`9ba0567b60d92270ad758a3bcd9bd746893b8e967c1fb4812d3d3abd511122af`,
material evaluation, `Hash=64`, four threads, 300 ms, flags 527 versus 15,
colour reversal, H0=0 Elo, H1=+10 Elo and alpha=beta=0.05.

At a complete-pair boundary after 210 games, the candidate was 97-7-106
(-14.9 Elo with an approximate 95% interval of plus or minus 46.5 Elo). The
105 paired observations were 22 wins, 57 draws and 26 losses, for LLR -0.225
against boundaries plus or minus 2.944. This was not a statistical H0 verdict.

The run had already lost the game 78 and game 90 kifu artifacts during a
temporary disk-full condition, so it could no longer become admissible gate
evidence. It was stopped only after completing the current colour-reversed
pair and recorded as `partial/error`. With incomplete evidence and no positive
signal, the candidate is rejected for adoption and will not be rerun or pooled
with another opening set.

## Reproduction

```bash
python3 scripts/nps_threads.py target/release/sekirei \
  --threads 4 --movetime 300 --positions /path/to/openings_standard.sfen \
  --count 20 --repeats 2 --lazy-diagnostics \
  --option SearchMode=LazySMP --option LazyFlags=527 \
  --json target/lazy-root-order-rotate.jsonl
```

Counter-enabled NPS is diagnostic only. The ordinary playing path does not
allocate these observers or execute their per-node atomics.
