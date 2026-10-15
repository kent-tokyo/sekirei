# Compiler PGO: qualified 4T binary

The source-identical PGO binary exceeded the pinned YaneuraOu reference by
**7.0803% in aggregate nodes per measured wall second** on this Apple M4
4T workload. The bootstrap 95% interval was **+5.0984% to +8.9675%**.
Ordinary Cargo builds do not automatically use this profile.

| Completed measurement | Ratio | Bootstrap 95% interval |
| --- | ---: | ---: |
| Own independent 4T holdout, PGO/native | 1.05786529 | 1.04641433–1.06903566 |
| Pinned reference 4T, PGO/YaneuraOu | 1.07080277 | 1.05098351–1.08967512 |

## Build and own qualification

The measured source is `4dd3a64ca7279dcc0ba07cdbf8e8ce2a60a4e705`; all
148 compiler inputs agree with the fresh native baseline. Rust 1.97.0 /
LLVM 22.1.6, target-cpu=native, opt-level=3, Thin LTO and one codegen unit
were held constant. Baseline, instrumented and profile-use builds used the
same checkout, target directory and command. All 19 build phases passed,
including six binary runtime audits and 434 core tests (9 ignored).
Timed binaries contain no LLVM profile runtime hooks.

Fresh atomic compiler profiles used 64 material and 64 own synthetic
positions, each searched once for one second with four workers. Build-script
profiles were excluded from merging. This does not train an NNUE model.
The [earlier build screen](../pgo_native_v0.4.1/README.md#compiler-profiles-and-boundaries)
documents the compilation flags; this experiment regenerated its profiles
from the current source and the four-worker workload.

All 384 fresh depth-12 searches matched bestmove, score, PV, nodes, depth and
bound exactly. The independent own holdout used 32 fresh positions
(seed 202610175), excluding training, quick and 17 historical corpora.
The [holdout](holdout32.sfen) and [provenance](holdout-provenance.json)
are published so future independent gates can exclude these states.
Its 128 fresh-process ABBA searches used own synthetic HalfKP/FV_SCALE=16,
4T, Rayon=4, Hash=64, Auto, SpecTopN=0, LazyFlags=15, MoveOverhead=0 and
diagnostics off. No host or 700ms exclusions occurred; all 583 frozen inputs
were unchanged. Candidate selection was frozen before external comparison.

Own driver exit/reaping contracts are source-bound; those raw records do
not distinguish normal quit from fallback termination. All 32 separate
no-search PV replays passed the negative control and normal-exit checks.

## Pinned reference comparison

Both engines used four threads, Hash=64, the same private HalfKP
256x2-32-32 file, FV_SCALE=24 and one second per search. The host was an
Apple M4 with 10 physical/logical cores; CPU affinity was not fixed.
YaneuraOu source `c1b80eaa09fe13d5f12b1599d1ae4d53c224de30` was built with
Apple clang 21.0.0, the NNUE edition, APPLEM1, NEON dot-product and
`-mcpu=native`. Binary, evaluator and corpus hashes are in
[results.json](results.json). The evaluator binding proves file identity,
not bitwise equivalence of loaded evaluation outputs.

Before timing, eight actual White/Black smoke searches passed protocol,
full-PV legality and normal-shutdown checks. The speed study then used the
fixed 31-position quick corpus (all White), with fresh-process
reference → Sekirei → Sekirei → reference quartets. It retained 124 accepted
searches; four additional whole quartets were rejected for host load.
One position finished below the preregistered 700ms floor, leaving 30
eligible positions. There was no retry for that exclusion.

The primary per-position ratio compares the two arm means of
`1000 * aggregate_nodes / elapsed_ms`. Geometric aggregation and 5,000
position-bootstrap draws use seed 20261014. The secondary engine-reported
NPS ratio is 1.07149508. The gate required at least 20 eligible positions,
a point estimate at least 1 and a confidence lower bound above 1.
Five quiet host samples, a 512MiB disk floor, a 600s quiet wait and at most
three host-only quartet attempts were fixed in advance.

All 140 actual search calls have full protocol traces, legal PVs and
normal quit/exit-0/reap checks. All 35 PV replay quartets passed their
negative control; all 1,954 frozen inputs were unchanged.
External score/PV outputs remain local and were not used for candidate
selection, tuning, training or defaults. Reference depth is descriptive;
reported score bounds do not prove a completed exact iteration.

## Protocol correction and scope

The pinned reference resets its board on `isready`. The former sequence
`position → isready → go` therefore searched startpos instead of the
requested position. The historical 7c739cb4 and 760d external 4T comparisons
are invalid; their speed statistics are not reused. Own PGO qualifications
are unaffected.

Both engines now receive `usinewgame → isready/readyok → position → go`.
Elapsed time runs from go send to bestmove; queued position processing may
enter this interval. The original failed runs and audit remain local.

This result applies to the recorded binary, host, evaluator and corpus.
Cross-engine nodes count different trees. It does not establish useful
search work, playing strength, 8T performance or release-binary superiority.
Search/Cargo defaults and NNUE weights are unchanged. Raw evidence and
profiles remain local; weights and external score/PV labels are not published.
