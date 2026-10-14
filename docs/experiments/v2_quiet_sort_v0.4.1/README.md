# Isolated quiet-sort screen

V2_SORT=1 is an existing disabled search-policy candidate, not the separate
behavior-preserving deferred losing-tactical sort. No search code, parameter
default, evaluator, NNUE weights or candidate adoption changed in this screen.
The already queued independent aspiration candidate remains fixed.

## Mechanism and protocol

Under SEARCH_V2=1, interior nodes use the V2 picker even with V2_ROOT=1.
The baseline fully sorts quiet moves with sort_unstable_by_key. The candidate
selects the first four quiet moves by strict-greater scans and rotations, then
stably sorts the remaining tail. Equal-key move order can differ, changing
LMR, pruning, histories and subsequent search results. Direct partial-pick or
tail-sort counters are unavailable in this frozen USI executable.

The prospective protocol was saved before searches. Both arms used the same
clean v0.4.1 tune build at 4344202, SHA-256
18b427475993da42ed0c2c411b03e8785e8fe6b1c4f03bfd046ff4dc2b0e08a2.
Build receipt and retained artifact hashes are bound in [results.json](results.json).
This is the deliberately frozen prior engine, not a binary built from the
latest main. The V2 picker/parameter source at main 02fdaf7 matches the frozen build.

A uses T_V2_SORT=0; B uses 1. SearchMode=LazySMP, material evaluation,
Hash=64, LazyFlags=15,
MoveOverhead=0, T_ASP_VOLATILITY=0, no book and SpecTopN=0 are held fixed.
The existing score-blind diagnostic corpus contains ten quiet and ten tactical
positions (SHA-256 de70936824cd695f67a03093faa0980a00913544043057931941788fd0eaf5ad).
It is not an independent strength corpus and must not be reused as one.

The mechanism lane used one worker, depth 8, counters and two repeats of
per-position ABBA: 160 fresh-process searches. Within-arm signatures must
match exactly; cross-arm bestmove/score/PV/nodes/depth identity is observed,
not required. Instrumented timing is unqualified.

## Observations

All four observations per arm matched exactly at every position. The two arms
differed in 17 of 20 position signatures, including two bestmoves and 16
scores. This establishes a reproducible effect on actual searches, without
claiming direct counts of picker activity. All 1,180 PV moves replayed legally
through the frozen Sekirei rules tool, with no bestmove/PV-prefix violation.
This is a same-rules check, not an independent rule oracle or mate-score proof.

| Category | Positions | Median paired B/A nodes at depth 8 |
| --- | ---: | ---: |
| All | 20 | 1.000000 |
| Quiet | 10 | 0.998610 |
| Tactical | 10 | 1.019688 |

The node result is mixed and does not establish better search quality. Score
or bestmove changes cannot be judged as improvements without an independent
reference or playing evidence. These are development positions and a shallow
material screen, not a representative strength estimate.

The planned counter-free 1T/4T, 300ms, two-repeat ABBA lane was blocked by the
idle-host preflight before any search. Compilation and another CPU-heavy job
were observed during preparation. No processes were stopped and no allow-busy
override was used. The blocked attempt remains separate; it yields no NPS,
completed-depth or order-direction comparison. A later run must use a new
output directory and retain this record rather than pool/restart it.

V2_SORT stays disabled and does not advance to SPRT. Qualified 300ms paired
depth benefit, both order directions and quiet/tactical subgroup checks remain
necessary before any promotion; independent H1 is required for adoption.

## Reproduction

From the same frozen executable and corpus, using a new output directory:

```sh
python3 scripts/paired_search_diagnostics.py \
  --baseline <frozen-v041-tune-engine> --candidate <same-engine> \
  --positions docs/experiments/lazy_smp_useful_depth_v0.4.0/diagnostic.sfen \
  --output <new-depth8-directory> --threads 1 --count 20 --repeats 2 \
  --depth 8 --option LazyFlags=15 --option MoveOverhead=0 \
  --a-option T_V2_SORT=0 --b-option T_V2_SORT=1 \
  --diagnostics --policy-diagnostic --source-ref 4344202
```

For counter-free timing remove the two diagnostic flags and replace depth 8
with movetime 300 and threads 1,4. Let the default idle-host guard decide
whether a timed run can begin. Validate every completed run with
scripts/validate_paired_pv.py. Full position-paired observations, protocol,
raw-artifact hashes, build receipt hash and the blocked attempt hash are in
results.json. The main checkout's unrelated pending README was preserved.
