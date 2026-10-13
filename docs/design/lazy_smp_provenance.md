# Lazy SMP provenance and patent-screen record

Status: engineering review completed 2026-10-01 for issue #68. This is a
bounded prior-art and claim screen, not a legal opinion or a freedom-to-operate
conclusion.

## Implemented scope

The implementation is an independent safe-Rust composition of existing
Sekirei components:

- independent root workers with private boards and move-ordering state;
- the existing atomic shared TT and one shared cancellation flag;
- persistent worker searchers, helper depth skew of one ply, and main-worker
  cancellation of helpers;
- YBW disabled inside Lazy SMP workers so the same Rayon pool is not nested;
- deterministic result selection by completed depth, score, and move encoding;
- `SearchMode=Auto`: sequential search on one thread, Lazy SMP on multiple
  threads, and the existing speculative backend for `MultiPV > 1`.

No third-party dependency, source code, pseudocode, or test corpus was added.
The implementation uses no `unsafe`. General concepts were checked against
Hyatt and Mann, [“A Lockless Transposition-Table Implementation for Parallel
Search”](https://doi.org/10.3233/ICG-2002-25104), and Kaneko,
[“Parallel Depth First Proof Number Search”](https://doi.org/10.1609/aaai.v24i1.7551).
Neither paper was used as a code template.

## Primary-registry claim screen

The concrete claim target was narrower than “parallel search”: multiple
independent iterative-deepening root workers, a generic shared atomic TT,
one-ply worker depth staggering, and cancellation when the main worker ends.

Queries were run on 2026-10-01:

| Registry | Queries | Result relevant to this implementation |
|---|---|---|
| [USPTO Patent Public Search](https://ppubs.uspto.gov/pubwebapp/) | exact `lockless transposition table` / `shared transposition table` with chess or game; exact `depth staggering` / `depth scheduling` with chess or game tree | no matching family for the concrete scheme |
| [European Patent Register](https://register.epo.org/regviewer) | exact `lockless transposition table`, `shared transposition table`, and `depth staggering` + `game tree` | no results |
| [J-PlatPat](https://www.j-platpat.inpit.go.jp/s0100) | `transposition table chess`, `置換表 将棋`, and `並列探索 ゲーム木` in patents/utility models | no results |

The broader USPTO query `"transposition table" AND chess` returned six
documents in four families. The only current game-playing application that
needed a claim review was US 2026/0138030 A1, “Inference-Based Move Selection
Using Predictive Control for Game-Playing Applications.” Its independent
claims require generating a predicted opponent response for each legal move
with a nominal opponent engine and selecting from those evaluated responses.
Sekirei's Lazy SMP workers independently search the same root; they do not use
that nominal-opponent-engine scheme. The other reviewed hits concerned tensor
decomposition, medical treatment planning, and storage-device scheduling.

No reviewed live claim plausibly covered the concrete implementation above.
This was a keyword-and-claim screen, not an exhaustive professional patent
search; a materially different parallel-search design should repeat the gate.

## Correctness evidence and claim boundary

The repository tests cover board preservation, a one-worker sequential
control, repeated shared-TT A/A result stability, and shared-versus-isolated TT
agreement under a fixed budget. USI tests cover mode selection and stop/quit
handling. These establish bounded correctness only. They do not establish an
Elo gain, 64-core contention below 5%, or a patent non-infringement conclusion.

The release-mode diagnostic was rerun on 2026-10-01 from startpos with two
workers and fixed depth 4. Across three repeats, shared and isolated TT modes
selected the same move (`7g7f`), score (0), and depth (4). Isolated TT used
3,866 nodes in every repeat; shared TT used 3,759--3,804 nodes. The node-count
difference records work sharing only and is not Elo or wall-clock evidence.

```bash
cargo run --release -p sekirei-core --example lazy_smp_diagnostic
```

On 2026-10-10, `LazyFlags` bit 128 was added as a diagnostic-only USI control.
It keeps the other selected policy bits but gives each worker a private TT;
the configured total `Hash` budget is divided among those tables. A
20-position ABBA measurement used four workers, 300 ms, material evaluation,
and the same Sekirei-generated openings. Across the resulting 40 paired
observations, shared TT reached a greater selected depth 20 times, tied 15,
and lost 5, for a mean advantage of 0.45 ply (median 19 versus 18). A helper
supplied the selected result in 28 of 40 shared-TT searches and none of the
isolated controls. Worker-duration spread was 0 ms at millisecond resolution
in every sample. These results support retaining TT sharing, but they do not
measure Elo or identify which individual TT hits crossed worker boundaries.

On 2026-10-12, the diagnostic path gained a bounded 1/256 node sampler that
distinguishes revisits by the same worker from visits by another worker. The
ordinary search path does not allocate the sampler or execute its atomics.
The four-worker shared-TT measurement estimated 22.3% cross-worker visits,
versus 43.5% for the isolated-TT control. See the
[work-overlap report](../experiments/lazy_smp_work_overlap_v0.3.72/README.md)
for the fixed conditions and claim boundary.

The same opt-in path now also samples committed TT stores and attributes a
later sampled hit to the last known committing worker. This distinguishes
confirmed cross-worker TT reuse from repeated search-tree visits. USI output
reports local, foreign-worker, and unknown-writer sampled hits for each worker,
so measurements can distinguish global sharing from sharing that reached the
worker whose root result was selected. It also reports every non-selected
worker's root nodes and the divergent subset whose final move differs from the
selected worker. The first four-worker
measurement found 45.7% foreign-worker hits among sampled hits with a known
writer and 75.1% discarded root-result nodes. These figures selected the
existing root-vote policy for an SPRT; they are not themselves an adoption
criterion. See the
[useful-work report](../experiments/lazy_smp_useful_work_v0.3.72/README.md).

`LazyFlags` bit 256 is a diagnostic-only depth-ladder experiment: with bit 4
also enabled, four workers use depth offsets `(0, 1, 2, 3)` instead of
`(0, 1, 0, 1)`. It is disabled by default. Its first 64-game material screen
was neutral, so it is not strength evidence and was not adopted.

Root-move ordering is reset at the start of every search. The implementation
uses thread-local storage only to carry ordering between iterative-deepening
passes within that search. Without the reset, a Rayon thread could retain the
order from a different logical Lazy SMP worker or engine instance on a later
`go`, making results depend on scheduler placement.

Time-capping helper workers at 75% and 50% was screened and rejected before
SPRT. The stricter cap reduced discarded root-result nodes but also reduced
median completed depth. See the
[helper-budget report](../experiments/lazy_smp_helper_budget_v0.4.0/README.md).

`LazyFlags` bit 512 is an opt-in root-order diversity experiment. Worker 0
keeps the normal root order; helper worker `n` rotates only its first root
iteration by `n` moves. Once a worker completes a root iteration, its ordinary
iterative-deepening feedback replaces that seed. The flag is disabled by
default and is not strength evidence until it passes the fixed diagnostic and
an independently preregistered material-only SPRT.

`LazyFlags` bit 1024 is an opt-in result-selection experiment. It first fixes
the eligible set to workers that completed the maximum depth. It then uses all
workers' reported moves only to break ties between moves in that set. Unlike
the full vote in bit 16, a majority of shallower workers cannot lower the
reported completed depth. This bit changes neither worker search nor the
default policy and requires an independent paired SPRT before adoption.
