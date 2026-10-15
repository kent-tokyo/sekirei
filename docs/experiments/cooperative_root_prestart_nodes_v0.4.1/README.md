# Root prestart under a shared node budget

Prospective diagnostic, 2026-10-15. The preceding
[prestart experiment](../cooperative_root_prestart_v0.4.1/README.md) completed
mechanism/PV checks but both permitted300ms attempts blocked before search.
That timing protocol stays closed. This separate experiment measures work per
shared node budget, with no wall-time qualification or strength gate. Other
CPU jobs remain running. No NNUE, training or hold-out data is used.

## Frozen comparison

Use the existing clean non-PGO native tune executable built from
`d7d4a5bc857ebf9d17432780841e46bf7e614207`, SHA-256
`17f71bfbe029130dad9dfc0e650dd2a73018908744699982efc61d2f8483031b`.
Both arms use that exact executable. This is a diagnostic of the pinned
historical candidate, not a current-main throughput or strength claim.
Current integration base is `a7eff77e4142ad89929e44a6e50c2e2ee958ebb5`;
its additional default-off quiet-TT experiment is outside the measured binary.

Use all20 existing development positions (10quiet/10tactical), SHA-256
`de70936824cd695f67a03093faa0980a00913544043057931941788fd0eaf5ad`.
Threads1/2/4/8, Hash64, material evaluation, no book, fresh engine per slot,
ABBA per position, two repeats, **20000 shared nodes per go**:640rows.
Common options: SearchMode=LazySMP, SpecTopN0, LazyFlags16399, MoveOverhead0,
T_COOP_MULTI1, T_COOP_PERSISTENT1, T_SEARCH_V2=1;
T_COOP_DRAIN/PRIORITY/LATENCY/HELPER_LMR/PRESTART_DIAG all0.
Only T_COOP_PRESTART differs, A0/B1. Do not add --depth, --movetime,
--diagnostics or --allow-busy. Node mode always has timing_qualified=false.

The shared cooperative parent budget counts main and helper nodes together.
Independent-worker fallback would allocate separate budgets and is rejected
by actual group activation checks. Node increments precede the abort check,
so parallel overshoot is recorded rather than requiring exactly20000 nodes.
Under-budget rows, including early mate/terminal results, are retained.
USI does not expose completed_bound or abort_reason: an absent bound tag is
not proof of Exact, and a shortfall does not prove a particular stop reason.
Depth0 is retained as no completed iteration; PV legality is still required.
No positions, repeats or directions are discarded.

Commit this protocol and measurement-tool changes before search. Before the
first search, preserve these protocol bytes, executable/build receipts,
corpus/category manifest, legal-replay tool, original prerequisite reports,
all measurement/analysis scripts and their hashes in a new presearch binding.
The original prestart reports and blocked attempts remain separate.
Source_ref identifies the historical native source; driver_source separately
identifies the new committed tool/protocol source.

## Analysis and stopping

Require all640 ABBA coordinates, legal replay of every PV, identical binary
hashes, exact assigned options and node-budget provenance. Require40 exact
1T control blocks (bestmove/score/PV/nodes/completed depth). Threads2 is an
unchanged-policy scheduling control; its nondeterminism is recorded. Both
4T/8T arms must activate persistent groups with3/7 helpers, reuse physical
threads, report zero spawn failures, join all threads and satisfy root node/
job census. B4/8 prestart passes must be positive in aggregate; A and1/2T
must have no prestart record. Observer records are prohibited.

For each count report actual node/overshoot distributions, completed depths,
paired depth deltas, quiet/tactical medians, both ABBA directions, per-position
means over both repeats, root/helper census and within-arm variation.
Position-bootstrap95-percent intervals use5000 draws with seed20261015 and
sorted percentiles at floor(p*(draws-1)).4T/8T remain independent.
Wall NPS and elapsed time are retained in raw logs only, not used in analysis.

One node run only. Never retry a completed/interrupted run, change the node
budget/corpus/options after observation, or pool it with timing evidence.
An incomplete run retains raw rows with timing_qualified=false and cannot
yield a completed analysis. Record failures and stop; do not overwrite files.
Even a positive depth-per-node signal cannot adopt a default or establish
Elo/YaneuraOu superiority: qualified timing and independentH1 are still needed.

```sh
python3 scripts/paired_search_diagnostics.py \
  --baseline target/cooperative-root-prestart-build/tune/engine \
  --candidate target/cooperative-root-prestart-build/tune/engine \
  --positions docs/experiments/lazy_smp_useful_depth_v0.4.0/diagnostic.sfen \
  --output target/cooperative-root-prestart-nodes-run \
  --threads 1,2,4,8 --count 20 --repeats 2 --nodes 20000 \
  --option LazyFlags=16399 --option MoveOverhead=0 \
  --option T_COOP_MULTI=1 --option T_COOP_PERSISTENT=1 \
  --option T_SEARCH_V2=1 --option T_COOP_DRAIN=0 \
  --option T_COOP_PRIORITY=0 --option T_COOP_LATENCY=0 \
  --option T_COOP_HELPER_LMR=0 --option T_COOP_PRESTART_DIAG=0 \
  --a-option T_COOP_PRESTART=0 --b-option T_COOP_PRESTART=1 \
  --source-ref d7d4a5bc857ebf9d17432780841e46bf7e614207
```

Replay with the prior frozen legal tool (SHA-256
`dfbe08d01ced743c0bbd409fe0ca816599d53daa4ef6dcbaa447e381c8d9dc00`)
and analyze with the newly frozen node-analysis driver. Store receipts and
negative observations in this experiment's results.json. Defaults remainOFF.

## Observed result

The single640-row run completed. All40 one-thread ABBA blocks retained exact
bestmove/score/PV/nodes/depth identity. All5255 recorded PV moves replayed
legally; no rows were excluded. Both4T/8T arms activated and reused all
persistent helpers, with zero spawn failures and all physical threads joined.
Every row consumed at least the requested node budget and had depth>0.

| Completed-depth delta, B minus A | 4T | 8T |
|---|---:|---:|
| Overall median | -0.5 | 0.0 |
| Quiet median | -0.75 | -0.75 |
| Tactical median | -0.5 | 0.0 |
| Mean of position means | -0.6375 | -0.3875 |
| Position-bootstrap95% interval | [-1.0875, -0.2375] | [-0.7625, -0.0375] |
| Maximum node overshoot A / B | 2 / 3 | 6 / 7 |

The frozen candidate did not improve completed depth per requested node in
this development corpus. The number of main confirmations increased from
851 to2302 at4T and from938 to2633 at8T. Helper nodes increased from257220
to494368 and319270 to510246 respectively. This is consistent with extra
work from probing against the weaker pre-first aspiration lower bound, but
the experiment does not isolate that cause. No elapsed-time/NPS result was
used to select a candidate or claim speed.

Keep COOP_PRESTART=0. Do not send this unbounded-pre-first-work candidate to
H1 on the basis of these observations. A future separate candidate could
cap work before the first child returns; it needs its own prospective
comparison. These results concern the historical native source, under the
recorded host load, and do not prove current-main strength or a loss in Elo.
The prior blocked timing protocol remains closed.

Tool validation:85measurement-tool tests and10analysis-contract tests passed.
The commit hook passed fmt/workspace Clippy. [results.json](results.json)
binds the complete diagnostics and57hashed artifacts, including separately
archived historical measurement drivers. No NNUE or evaluator changes.
