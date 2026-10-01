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
