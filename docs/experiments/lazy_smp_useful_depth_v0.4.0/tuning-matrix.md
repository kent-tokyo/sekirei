# One-factor tuning after useful-depth qualification

Use one frozen tune executable, the current baseline defaults, material
evaluation and unused Sekirei-generated colour-paired openings. The paired
runner refuses multiple changed side options and multi-bit LazyFlags changes.
No parameter below has been adopted by this implementation.

| Factor | Baseline | Separate candidate values |
|---|---:|---|
| `T_V2_SORT` | 0 | 1 |
| `T_V2_HINDSIGHT` | 0 | 1 |
| `T_V2_FH_BLEND` | 0 | 1 |
| `T_V2_SHAPE` | 0 | 1, 2, 4, 8, in four independent runs |
| `T_V2_PVQ` | 0 | 1 |
| `T_TT_KEEP_DEPTH` | 2 | 1 or 3, independently |
| `T_TT_AGE_WEIGHT` | 6 | 5 or 7, with retained-TT generation replay |

`T_ROOT_ORDER=1` is not a useful independent candidate under the current
`SEARCH_V2=1`, `V2_ROOT=1` baseline. The inspected root code retains iteration
order whenever either ROOT_ORDER or V2_ROOT is enabled. Switching ROOT_ORDER
alone therefore leaves this policy active on both sides. Do not send this
no-op to SPRT or silently change V2_ROOT to invent a different baseline.

Fresh-engine ABBA diagnostics intentionally clear prior-search TT contents.
They can compare same-search replacement pressure, but cannot establish the
benefit of generation aging: all fresh entries have the same generation.
An aging experiment must retain the TT across a frozen sequence of at least
20 completed `go` commands, never send `usinewgame` between positions, and
report per-go TT hits, unknown prior-go provenance, attempted/committed
writes, rejected shallower writes and collision replacement. Repeat complete
sequences in ABBA, keep hash capacity fixed, and exercise the 16-generation
packed-age wrap. Treat the existing topology example as a mechanism probe;
it is not a replacement for this retained-TT benchmark. No TT generation
change is justified by the fresh-position results in this report.

After complete diagnostics select at most one active factor for a new paired
H0=0/H1=10 Elo SPRT, alpha=beta=0.05, hard cap800 games. Invalid records or
incomplete artifacts invalidate the test. H1 is required for adoption. Do not
pool factors, previously observed openings or exploratory results.

## Later v0.4.1 screens

The isolated [V2_SORT screen](../v2_quiet_sort_v0.4.1/README.md) completed
counter-enabled fixed-depth activity/PV checks and a separately qualified
1T/4T 300ms ABBA run. It established no paired useful-depth benefit and does
not advance; V2_SORT remains zero. The already queued aspiration candidate
was not changed.

The separate [deferred losing-sort check](../deferred_losing_sort_v0.4.1/README.md)
confirmed all 20 one-worker depth-8 signatures on the frozen v0.4.1 engine.
Its timed run was blocked before searching by compiler load, so no speed gain
is established. It is not pooled with V2_SORT or earlier blocked runs.

The opt-in [cooperative wait-drain candidate](../cooperative_root_wait_v0.4.1/README.md)
is a separate two-worker prototype factor, T_COOP_DRAIN=0/1 with
COOP_PERSISTENT=0 held fixed. All 80 depth-10 searches completed, but the forty ON searches had zero
wait-phase confirmations. Its prospective activation requirement failed, so
300 ms timing was not started. It stays disabled, does not advance, and does
not replace the queued independent aspiration candidate.

The separate [advice-latency observer](../cooperative_advice_latency_v0.4.1/README.md)
measures helper publication phase and worker-0 progress before confirmation.
It is diagnostics-only, defaults to zero, and does not interrupt main scouts
or establish a policy candidate. Instrumented timings are unqualified.

The separate [cooperative candidate-priority experiment](../cooperative_candidate_priority_v0.4.1/README.md)
compares the old ascending confirmation pass with highest-observed-ready
selection plus rescan. It does not interrupt a main child or adopt helper
scores. Outcome diagnostics classify actual beta cuts by publication phase.
The flag stays disabled; activity, useful-depth timing and independent H1
remain separate gates, and the queued aspiration candidate is unchanged.

The priority mechanism screen activated50 reorders with7 alpha raises and1
verified cut. Its completed300ms run was invalidated by post-run outside load;
observed median depth delta was0 overall and-0.25 for quiet positions. It does
not advance and remains disabled. No completed-result retry was performed.
