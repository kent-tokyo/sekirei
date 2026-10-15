# Deferred quiet HalfKP edge

This records the historical main-prefix prototype. The current disabled
implementation extends through QS; see the [QS experiment](../quiet_halfkp_qs_v0.4.1/README.md)
for its completed independent 4T result.

`V2_LAZY_QUIET_HKP` remains **0 by default**. Speed is unqualified; no strength claim.

For a clean active HalfKP board, the candidate defers one non-king,
non-promoting quiet move's accumulator delta through the main node prefix.
TT/repetition/abort returns can undo geometry without applying the delta.
Evaluation and eager descendants materialize it once; a consumed non-Copy
private token pairs make/undo. Captures, drops, king moves, dirty accumulators,
SEKIRW01 and king-relative evaluation retain the existing path. QS stays eager.
Public Board/MoveToken signatures and layouts are unchanged; native diagnostics
are absent. Training and evaluator selection are unchanged.

The frozen a7eff experiment passed 419 ordinary core tests (8 ignored),
144 fixed-depth searches with identical bestmove/score/PV/nodes/bounds and
restoration, 10M legal-move differential visits, a separate 10M quiet-edge
make/materialize/undo differential, fmt and default/tune release clippy.
Visits are selected legal-move attempts, not 10M unique positions. The fresh
native pair passed 31 exact 1T pairs, four isolated 4T worker-zero pairs and
two lifecycle cases (72 search processes). Current-main integration includes
the disabled CONT candidate: on `505f18e`, 1,044 workspace tests (422 core),
four isolated checks including a bounded 100k differential and 144 identities,
and workspace/tune clippy passed. After rebasing onto `4c46fdf`, 1,054
workspace tests (431 core), workspace/tune clippy, the actual mechanism and
144 identities passed on 148 frozen compiler inputs. B runtime/test files
remained byte-identical. Hosted coverage is checked separately after push.

| Independent own 4T attempt | Accepted / actual calls | Result |
| --- | --- | --- |
| 1 | 68 / 92 | Host-monitor ENOSPC; interrupted |
| 2 | 88 / 112 | Three host-rejected quartets at one position; interrupted |

Neither completed the required 128 accepted calls. No partial NPS statistics
were calculated or pooled. All 1,354 frozen inputs were unchanged after each
stop. The candidate was not used for private YaneuraOu comparison or default
adoption. These interruptions are neither a speed gain nor a measured regression.

H1 used fresh own32 seed 202610169, excluding development and all prior corpora:
Threads/Rayon 4, Hash 64, own synthetic HalfKP/FV16, Auto, SpecTopN 0,
LazyFlags 15, MoveOverhead 0, diagnostics off, and fresh-process per-position
ABBA at one second.
The gate retains the 512MiB floor, five quiet samples, 600s wait, three whole
quartet host retries, 700 ms whole-quartet filter, minimum eight eligible
states, and bootstrap 95% lower bound above 1. The generator's two historical quiet-TT
paths were repaired and independently reviewed before generation or timing;
old tools were archived, and all 13 exclusions checked against real files.

`results.json` binds the source/native/correctness proofs and interrupted raw
captures retained in the experiment worktree. Only audited unused compiler
caches were deleted; raw records, binaries, models and coverage evidence remain.
