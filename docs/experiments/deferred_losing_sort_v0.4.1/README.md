# Deferred losing-tactical sorting: v0.4.1 check

This checks an existing behavior-preserving optimization separately from the
V2_SORT quiet-order policy candidate. No search code, parameter default, NNUE
model or strength adoption changed here. Deferred losing sorting is already
the default; this screen does not introduce a new default.

The protocol was saved before searches. Both arms used the same clean frozen
release/tune engine built at 4344202, SHA-256
18b427475993da42ed0c2c411b03e8785e8fe6b1c4f03bfd046ff4dc2b0e08a2.
The measured V2 picker and parameter source at main 02fdaf7 matches that
build. Hash=64, LazyFlags=15, MoveOverhead=0, material evaluation, no book,
SearchMode=LazySMP, SpecTopN=0, V2_SORT=0 and ASP_VOLATILITY=0 were held fixed.
Only T_V2_EAGER_LOSING_SORT differed: A=1 (eager), B=0 (deferred).

Twenty existing score-blind development positions (ten quiet, ten tactical)
ran depth 8, one per-position ABBA repeat, separately at one and four workers.
All 20 one-worker blocks matched bestmove, score with unit/bound, PV, nodes
and completed depth in all four observations. Four workers had within-arm
nondeterminism in all 20 blocks and no exact cross-arm blocks; no four-worker
node-identity claim follows. Shared-TT scheduling variation is not a waiver
for illegal PVs or scores. All 1,100 recorded PV moves from 160 searches
replayed legally through the frozen rules tool, with no bestmove/PV-prefix
violation. This is a same-rules check, not an independent rule oracle.

The prospective counter-free 1T/4T, 300ms, two-repeat ABBA run used a ten-second
settling delay, then was blocked before any search by concurrent compiler
load. Its zero-row manifest is retained. No external process was stopped, no
allow-busy override was used, and no timing data was pooled or substituted.
These fixed-depth signatures support sample-wise behavior preservation;
NPS improvement remains unconfirmed. No depth, Elo or YaneuraOu-superiority
claim follows from this screen. It is not an independent strength corpus.

Reproduce into new output directories with:

```sh
python3 scripts/paired_search_diagnostics.py \
  --baseline <frozen-v041-tune-engine> --candidate <same-engine> \
  --positions docs/experiments/lazy_smp_useful_depth_v0.4.0/diagnostic.sfen \
  --output <new-depth8-directory> --threads 1,4 --count 20 --repeats 1 \
  --depth 8 --option LazyFlags=15 --option MoveOverhead=0 \
  --a-option T_V2_EAGER_LOSING_SORT=1 \
  --b-option T_V2_EAGER_LOSING_SORT=0 --source-ref 4344202
```

For timing use movetime 300 instead of depth 8 and two repeats, with the same
idle-host guard. Validate every completed run with validate_paired_pv.py.
[results.json](results.json) binds the protocol, raw rows, signatures,
corpus, build receipt, PV validation and blocked timing attempt hashes.
