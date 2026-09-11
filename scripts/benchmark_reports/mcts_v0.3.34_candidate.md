# MCTS benchmark: v0.3.34 candidate

This report records the internal effect of routing MCTS legal-move expansion
through the thread-local fixed move buffer. It is not a cross-library result.

## Measurement contract

- Revision: current working tree, version `0.3.34`
- Host: the developer macOS host; background load may affect absolute values
- Build: release Criterion benchmark
- Command:
  `CARGO_TARGET_DIR=/tmp/sekirei-target cargo bench -p sekirei-bench --bench movegen -- root_mcts_256_simulations --exact --warm-up-time 1 --measurement-time 2 --sample-size 30`
- Criterion sample: 30 samples for the Root MCTS row

## Result

| Benchmark | Median | Criterion change |
|---|---:|---:|
| Root MCTS, 256 simulations | 20.521 us | -51.018% vs stored baseline |
| Tree MCTS, 256 simulations, depth 4 | 356.41 us | -26.310% vs stored baseline |

The Tree MCTS result was measured separately with 50 samples and a three-second
measurement window. The baseline is Criterion's local stored baseline, not an
`rsshogi` measurement. The large Shared Tree change observed in a loaded run is
not treated as evidence because that run had severe outliers.

## Interpretation

Tree, Shared Tree, Root MCTS, and speculative search now obtain legal moves
through the reusable fixed-capacity path where their consumers can work with a
`&[Move]`. This removes a temporary general-purpose `Vec<Move>` at expansion
boundaries while preserving the public `generate_legal_moves` API.

The remaining direct `rsshogi` gap is the public `Vec<Move>` output on some
hand-heavy positions. See `cross_library_v0.3.34_candidate.md`; the packed and
fixed output paths are the relevant high-throughput comparison for engine
internals.

## Speculative NarrowMove pilot decision

A pooled `NarrowMoveList` pilot was measured with the decode cost included.
Its first 30-sample probe measured `373.10 us`, but the repeat probe measured
`457.74 us` and Criterion reported a `+14.791%` regression against that stored
baseline. Because the two probes were not run on a fully idle host and no
matched FixedMoveList baseline was collected in the same window, the result is
not a portable performance claim. The pilot was nevertheless reverted: it did
not provide reproducible evidence of a gain, while the existing fixed-buffer
path has established correctness and speed evidence.

## Policy candidate selection optimization

`policy::top_n` now generates into a fixed-capacity list and partially selects
the requested top-N moves instead of allocating and fully sorting a temporary
Vec. A fresh 100-sample Criterion run of speculative depth-4 startpos measured
a median of `691.28 us` (range `663.27–720.55 us`). This is a current-working-
tree measurement; it is not directly comparable to the earlier stored baseline
because the target was rebuilt and host load was not isolated. The change is
retained because it removes unnecessary allocation and full sorting, while a
future paired run is required before claiming a wall-clock improvement.

## Follow-up after score-once top-N selection

For the usual `top_n <= 16` case, candidate scores are now kept in a small
fixed score array while the top moves are inserted in order. This avoids
recomputing `policy_score` during the final sort. The isolated
`policy_top_n_startpos_n2` benchmark measured a median of `388.28 ns` (range
`380.42–398.06 ns`) and reported a `-64.359%` change against its local stored
baseline (95% interval `-68.138%` to `-60.411%`, `p < 0.05`).

The corresponding speculative depth-4 benchmark measured a median of
`688.74 us` (range `665.15–715.96 us`) and reported a `-30.486%` change against
its local stored baseline (95% interval `-33.545%` to `-27.242%`, `p < 0.05`).
These are local Criterion results, not an rsshogi comparison.

## Follow-up after fixed-buffer pool reuse

The policy fixed-list path now reuses the existing thread-local fixed-buffer
pool instead of allocating a 600-entry list on every call. A fresh 100-sample
Criterion run of speculative depth-4 startpos measured a median of `436.88 us`
(range `423.31–449.80 us`) and reported a `-34.545%` change against the local
stored baseline (95% interval `-36.757%` to `-32.279%`, `p < 0.05`). This is
strong local evidence that the allocation removal improves speculative search;
it is not an rsshogi comparison and remains host-specific.
