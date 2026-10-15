# Deferred quiet HalfKP through QS prefixes

`V2_LAZY_QUIET_HKP` remains **0 by default**. The independent own 4T speed
gate rejected this candidate; no YaneuraOu comparison or strength claim follows.

The experiment extends the [main-prefix prototype](../quiet_halfkp_edge_v0.4.1/README.md)
through both quiescence prefixes. TT returns can undo the pending quiet edge
without updating the accumulator. Evaluation and eager descendants materialize
it once. Legacy QS retains its evaluation at qply10, including in check; QS2
retains its original in-check zero at its depth caps. Internal QS moves stay
eager. Public Board/MoveToken APIs and layouts are unchanged; native counters
are absent. CONT rows and cooperative prestart cap remain disabled.

| Independent own 4T H1 | Result |
| --- | --- |
| Accepted calls / eligible positions | 128 / 32 |
| Actual quartets / rejected by host load | 34 / 2 |
| Candidate / baseline aggregate nodes per wall second | 1.00001061 |
| Bootstrap 95% interval | 0.99093789–1.00907677 |
| Decision | Improvement not established; default OFF |

The source is based on `46b5f6917d50ce52fd0a95e15e2094bc53956e38`.
Both native arms use the same checkout/build flags, with only the compile-time
candidate default changed. H1 uses fresh disjoint own32 seed 202610171:
Threads/Rayon 4, Hash 64, own synthetic HalfKP/FV16, Auto, SpecTopN 0,
LazyFlags 15, MoveOverhead 0, diagnostics off, and fresh-process per-position
ABBA at one second.
The complete result was evaluated once; it was not pooled with earlier attempts
or rerun to rescue a negative verdict. This is an own-engine speed comparison,
not a current YaneuraOu speed ratio.

Source qualification passed 431 ordinary core tests (8 ignored), 288 fixed-depth
searches with identical bestmove/score/PV/nodes/bounds and restoration, the
24-row/64-call QS boundary test, 10M legal-move differential visits and a
separate 10M quiet-edge differential. Visits are not unique positions. The
actual native binaries passed 72 fresh correctness/lifecycle processes.
Default/tune release clippy and fmt passed. Integration and hosted CI are
recorded separately in `results.json`.

The plan's stale driver expectation was corrected before H1, with the old bytes
archived and an actual input-freeze regression added. The final 1,365 frozen
inputs remained unchanged. `results.json` binds the proofs and durable raw
captures; training, evaluator selection and previous evidence are preserved.
