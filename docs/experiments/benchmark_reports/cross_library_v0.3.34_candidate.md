# Cross-library benchmark: v0.3.34 candidate

> Measurement correction (2026-09-12): the v6
> `state_roundtrip_search_no_nnue` sequence undid each move immediately instead
> of applying all six plies first. Those timings/ratios are **invalid**, for
> both libraries. Other state rows include initialization; the legal-root row
> also includes buffer creation, move generation and Sekirei NNUE updates.
> They do not measure isolated board speed. Move-generation rows include a
> representation-dependent raw-encoding checksum. Do not compare them directly
> with v7, which observes the output slice without encoding. Perft implementations
> differ in leaf counting and state maintenance. Use `cross_library --components`
> for separated measurements with legality/restoration checks and raw samples.

This is a local throughput diagnostic, not a playing-strength or general
engine-speed claim. The candidate includes uncommitted move-generation and
state-update optimizations. Results are from the pinned `rsshogi` revision on
the same process, with setup excluded where stated.

## Environment

- Revision at measurement: `6bc75a5eed7adde799da4d4064e716e1c5fa8aa6`
- OS/architecture: Darwin 25.5.0, arm64
- Rust: `rustc 1.97.0`
- Command: `SEKIREI_BENCH_ITERATIONS=5000 cargo run --release -p sekirei-bench --bin cross_library`
- Samples: 7; paired alternating order
- Reference: `rsshogi` 1.2.3, revision `a1dbc020e0711574ba0bec6a7b123c411ccdd625`
- Representation sizes: `Move` 5 bytes, `PackedMove` 4 bytes, `NarrowMove` 2
  bytes, rsshogi `Move32` 4 bytes

## Median results

Times are nanoseconds per iteration unless noted.

| Operation | Sekirei | rsshogi | Faster side |
|---|---:|---:|---|
| Legal moves, startpos, `Vec` | 121 | 125 | Sekirei 1.03x |
| Legal moves, startpos, packed | 108 | 113 | Sekirei 1.05x |
| Legal moves, startpos, narrow | 102 | 109 | Sekirei 1.07x |
| Legal moves, midgame with hands, packed | 244 | 260 | Sekirei 1.07x |
| Legal moves, midgame without hands, packed | 105 | 106 | Sekirei 1.01x |
| Legal moves, drop-only, packed | 565 | 657 | Sekirei 1.16x |
| Legal moves, drop-only without pawn, packed | 475 | 518 | Sekirei 1.09x |
| Perft(2), startpos | 2.336 us | 4.059 us | Sekirei 1.74x |
| Perft(3), startpos | 74.250 us | 129.071 us | Sekirei 1.74x |
| Perft(2), midgame with hands | 16.916 us | 34.116 us | Sekirei 2.02x |
| Full state update, six moves | 710 | 1,065 | Sekirei 1.50x |
| Search state update, no NNUE | 594 | 1,045 | Sekirei 1.76x |
| Search state roundtrip, no NNUE | 602 | 1,078 | Sekirei 1.79x |
| Legal-root state roundtrip | 3.657 us | 3.780 us | Sekirei 1.03x |

## Interpretation

This run is materially more stable than the earlier 100-iteration probe, but
Storage Management was still active during measurement and some rows have
visible outliers. Treat the medians as candidate evidence, not a clean release
claim. The current candidate now wins the packed start-position legal-move
case and most packed/multi-ply cases. It remains behind on ordinary legal
generation in some developed positions and on the narrow no-hands case by a
negligible margin. Repeat on an idle, controlled machine before claiming
overall superiority.

After this run, `FixedMoveList` received the same duplicate-capacity-assert
removal already used by the packed and narrow lists. That change passed the
full core regression suite but is not included in the numbers above; rerun the
benchmark before attributing any further speed change to it.

`shogi_core` is not included because it does not provide a legal move
generator or Perft implementation. The state-update comparison with rsshogi
does not exercise NNUE in rsshogi; it is a rules-state diagnostic only.

## Follow-up probe after FixedMoveList assert removal

The same command was rerun after the FixedMoveList change. The host was still
under background load, so these values are a confirmation probe rather than a
replacement for the table above.

| Operation | Sekirei | rsshogi | Faster side |
|---|---:|---:|---|
| Legal moves, startpos, `Vec` | 132 | 139 | Sekirei 1.05x |
| Legal moves, startpos, packed | 123 | 128 | Sekirei 1.04x |
| Legal moves, startpos, narrow | 113 | 121 | Sekirei 1.07x |
| Legal moves, midgame with hands, packed | 538 | 548 | Sekirei 1.02x |
| Legal moves, midgame without hands, packed | 227 | 230 | Sekirei 1.01x |
| Perft(3), startpos | 75.945 us | 134.112 us | Sekirei 1.77x |
| Full state update, six moves | 1,585 | 2,404 | Sekirei 1.52x |
| Search state roundtrip, no NNUE | 1,362 | 2,422 | Sekirei 1.78x |
| Legal-root state roundtrip | 4.641 us | 5.068 us | Sekirei 1.09x |

The probe confirms that the earlier start-position packed win was not an
isolated 100-iteration artifact. It does not establish portable or overall
superiority; repeat on an idle controlled host before release claims.

## Second follow-up probe after inline rollback

After reverting the stack-overflowing broad inline experiment, the benchmark
was rerun with 5,000 iterations and seven paired samples. Background load was
still present, but the principal rows were stable enough to confirm direction:

| Operation | Sekirei | rsshogi | Faster side |
|---|---:|---:|---|
| Legal moves, startpos, `Vec` | 157 | 156 | Effectively tied |
| Legal moves, startpos, packed | 143 | 162 | Sekirei 1.13x |
| Legal moves, startpos, narrow | 142 | 159 | Sekirei 1.12x |
| Legal moves, midgame with hands, packed | 238 | 247 | Sekirei 1.04x |
| Legal moves, midgame without hands, packed | 106 | 109 | Sekirei 1.03x |
| Legal moves, midgame without hands, narrow | 106 | 109 | Sekirei 1.03x |
| Perft(3), startpos | 74.977 us | 129.938 us | Sekirei 1.73x |
| Perft(2), midgame with hands | 14.766 us | 28.567 us | Sekirei 1.93x |
| Full state update, six moves | 702 | 1,065 | Sekirei 1.52x |
| Search state roundtrip, no NNUE | 604 | 1,067 | Sekirei 1.77x |
| Legal-root state roundtrip | 2.168 us | 2.240 us | Sekirei 1.03x |

The ordinary start-position `Vec` row is within measurement noise. This probe
does not replace an idle-host run or establish portable overall superiority.

## Current probe after Vec batch-writer specialization

The `Vec<Move>` sink was given the same bulk target reservation behavior as the
compact sinks. This release-mode probe used 5,000 iterations and seven paired
samples. It was still run on a host with background load, so it is diagnostic
evidence rather than a release-grade ranking.

| Operation | Sekirei | rsshogi | Faster side |
|---|---:|---:|---|
| Legal moves, startpos, `Vec` | 142 | 137 | rsshogi 1.04x |
| Legal moves, startpos, fixed buffer | 126 | 130 | Sekirei 1.03x |
| Legal moves, startpos, packed | 111 | 119 | Sekirei 1.07x |
| Legal moves, midgame with hands, `Vec` | 317 | 240 | rsshogi 1.32x |
| Legal moves, midgame with hands, fixed buffer | 252 | 236 | rsshogi 1.07x |
| Legal moves, midgame with hands, packed | 241 | 245 | Sekirei 1.02x |
| Legal moves, midgame without hands, packed | 103 | 103 | Tie |
| Perft(3), startpos | 75.404 us | 126.687 us | Sekirei 1.68x |
| Perft(2), midgame with hands | 13.590 us | 26.392 us | Sekirei 1.94x |
| Full state update, six moves | 697 | 1,051 | Sekirei 1.51x |
| Search state update, no NNUE | 592 | 1,050 | Sekirei 1.77x |
| Search state roundtrip, no NNUE | 603 | 1,087 | Sekirei 1.80x |
| Legal-root state roundtrip | 2.032 us | 2.026 us | Tie |

The result identifies the remaining gap more precisely: the public, general
`Vec<Move>` path is still slower on hand-heavy positions, while the packed
path is essentially tied there and the search/perft paths are clearly faster.
The next high-value optimization should therefore target the ordinary
hand-generation path or its API contract, not more broad inlining. A clean,
idle-host rerun is still required before claiming portable superiority.

## Current probe after `Move::raw` benchmark normalization

The benchmark consumer now uses the same inline raw encoding method on
Sekirei's public `Move` and compact outputs. This run used 5,000 iterations and
seven paired samples. It remains a candidate diagnostic because host load was
not fully controlled.

| Operation | Sekirei | rsshogi | Faster side |
|---|---:|---:|---|
| Legal moves, startpos, `Vec` | 127 | 116 | rsshogi 1.09x |
| Legal moves, startpos, fixed buffer | 119 | 121 | Sekirei 1.02x |
| Legal moves, startpos, packed | 106 | 116 | Sekirei 1.09x |
| Legal moves, startpos, narrow | 104 | 111 | Sekirei 1.07x |
| Legal moves, midgame with hands, `Vec` | 337 | 267 | rsshogi 1.26x |
| Legal moves, midgame with hands, fixed buffer | 277 | 294 | Sekirei 1.06x |
| Legal moves, midgame with hands, packed | 263 | 262 | Tie |
| Legal moves, midgame without hands, packed | 112 | 114 | Sekirei 1.02x |
| Legal moves, midgame without hands, narrow | 116 | 118 | Sekirei 1.02x |
| Perft(3), startpos | 78.347 us | 128.116 us | Sekirei 1.64x |
| Perft(2), midgame with hands | 16.633 us | 29.952 us | Sekirei 1.80x |
| Full state update, six moves | 711 | 1,096 | Sekirei 1.54x |
| Search state update, no NNUE | 611 | 1,085 | Sekirei 1.78x |
| Search state roundtrip, no NNUE | 620 | 1,138 | Sekirei 1.84x |
| Legal-root state roundtrip | 2.393 us | 2.365 us | rsshogi 1.01x |

This probe confirms that the compact output paths are the practical
high-throughput interface: they match or exceed rsshogi on the tested legal
move fixtures, while the public `Vec<Move>` path still pays for its larger
representation on hand-heavy positions.

## Fresh comparison run after release-target rebuild

The comparison was rerun on the same host with 5,000 iterations and seven
paired samples. The release target was rebuilt before the run. The medians
below are the current evidence for the speed-gap question; host scheduling
was not isolated, so they are not a portable ranking.

| Operation | Sekirei | rsshogi | Faster side |
|---|---:|---:|---|
| Legal moves, startpos, `Vec` | 121 ns | 115 ns | rsshogi 1.05x |
| Legal moves, startpos, fixed buffer | 128 ns | 126 ns | rsshogi 1.02x |
| Legal moves, startpos, packed | 105 ns | 116 ns | Sekirei 1.10x |
| Legal moves, startpos, narrow | 102 ns | 110 ns | Sekirei 1.08x |
| Legal moves, midgame with hands, `Vec` | 319 ns | 241 ns | rsshogi 1.32x |
| Legal moves, midgame with hands, fixed buffer | 250 ns | 239 ns | rsshogi 1.05x |
| Legal moves, midgame with hands, packed | 239 ns | 240 ns | Sekirei 1.00x |
| Legal moves, midgame without hands, packed | 103 ns | 103 ns | Tie |
| Legal moves, drop-only, fixed buffer | 562 ns | 608 ns | Sekirei 1.08x |
| Legal moves, drop-only without pawn, fixed buffer | 475 ns | 527 ns | Sekirei 1.11x |
| Perft(3), startpos | 76.484 us | 130.384 us | Sekirei 1.71x |
| Perft(2), midgame with hands | 13.989 us | 27.352 us | Sekirei 1.96x |
| Full state update, six moves | 715 ns | 1,075 ns | Sekirei 1.50x |
| Search state update, no NNUE | 609 ns | 1,078 ns | Sekirei 1.77x |
| Search state roundtrip, no NNUE | 625 ns | 1,108 ns | Sekirei 1.77x |
| Legal-root state roundtrip | 2.684 us | 2.414 us | rsshogi 1.11x |

The principal remaining product-level gap is the public `Vec<Move>` path on
hand-heavy positions: approximately 1.32x slower in this run. The compact
packed path is tied there, and the deeper state-transition/perft paths are
faster. This does not establish that either engine is fastest overall because
the benchmark covers only the listed operations and fixtures.

## Adaptive fixed-sink pilot

The legal `Vec<Move>` API now uses the reusable fixed sink when at least two
drop-piece kinds are present, then copies the completed slice into the caller's
Vec. A same-command diagnostic run measured:

| Operation | Sekirei | rsshogi | Faster side |
|---|---:|---:|---|
| Legal moves, midgame with hands, adaptive Vec | 672 ns | 616 ns | rsshogi 1.09x |
| Legal moves, midgame with hands, fixed then Vec | 585 ns | — | Sekirei diagnostic path |

The fixed-then-Vec row is not a direct public-API result yet; it is an
isolated probe showing the potential of batch writing. The run also showed
large host-load variation in unrelated rows, so this pilot is not a release
benchmark. Regression tests and Clippy pass. The implementation is retained
as a bounded candidate, but it does not yet satisfy the rsshogi speed target.

## Native-instruction-set comparison

Both crates were rebuilt in release mode with `RUSTFLAGS='-C target-cpu=native'`.
The run used 5,000 iterations and seven paired samples. Values are medians;
the comparison is still host-specific and does not represent a portable
ranking.

| Operation | Sekirei | rsshogi | Faster side |
|---|---:|---:|---|
| Legal moves, startpos, `Vec` | 325 ns | 304 ns | rsshogi 1.07x |
| Legal moves, startpos, packed | 276 ns | 293 ns | Sekirei 1.06x |
| Legal moves, midgame with hands, `Vec` | 997 ns | 791 ns | rsshogi 1.26x |
| Legal moves, midgame with hands, packed | 866 ns | 891 ns | Sekirei 1.03x |
| Perft(3), startpos | 354.480 us | 610.746 us | Sekirei 1.72x |
| Full state update, six moves | 3.391 us | 4.135 us | Sekirei 1.22x |
| Legal-root state roundtrip | 9.427 us | 7.581 us | rsshogi 1.24x |

The native build does not remove the two remaining gaps: the public `Vec<Move>`
path on hand-heavy positions and the legal-root full-state roundtrip. It does
confirm that compact output and deeper traversal are already competitive or
faster under the same CPU-specific optimization. The next optimization target
is therefore the allocation-compatible `Vec<Move>` API and the full-state
roundtrip's NNUE/history work, followed by an idle-host rerun.

## Vec pool-borrow optimization follow-up

The adaptive `Vec<Move>` path now borrows one pooled `FixedMoveList` for the
whole generation-and-copy operation. This removes the per-call pool pop/push
pair without changing the public API or move ordering. A fresh native release
run used 5,000 iterations and seven paired samples.

| Operation | Before native probe | After | rsshogi after | Result |
|---|---:|---:|---:|---|
| Legal moves, midgame with hands, adaptive `Vec` | 997 ns | 792 ns | 746 ns | rsshogi 1.06x |
| Fixed generation plus `Vec` copy diagnostic | 923 ns | 683 ns | — | Sekirei diagnostic |

The adaptive path is now close to rsshogi on this hand-heavy fixture, while
remaining host-sensitive. Start-position values varied substantially between
runs, so this change is not credited with a start-position improvement.

A 20,000-iteration rerun also showed strong scheduling variance (1,291 ns for
Sekirei versus 1,185 ns for rsshogi on the hand-heavy `Vec` row). It is kept as
diagnostic evidence only; an idle-host repeated run is required before making
a release-level performance claim.

## Vec reserve-call reduction

The `Vec<Move>` sink no longer calls `Vec::reserve` once per target group. The
caller-owned vector now relies on its amortized `push` growth, avoiding repeated
capacity checks while preserving behavior for callers with arbitrary initial
capacity. One additional native release run (5,000 iterations, seven paired
samples) measured the following:

| Operation | Sekirei | rsshogi | Result |
|---|---:|---:|---|
| Legal moves, midgame with hands, adaptive `Vec` | 1,039 ns | 1,393 ns | Sekirei 1.34x |
| Legal moves, midgame without hands, `Vec` | 433 ns | 415 ns | rsshogi 1.04x |

The same run had large outliers on the start-position row, so only the
hand-heavy result is recorded as a directional signal. Core tests and Clippy
remain green after this change; an idle-host rerun is still required for a
stable release claim.

## Root ordering allocation reduction

Search move ordering now uses a stable cached-key insertion sort backed by
fixed arrays when a node has at most 64 moves. Larger lists retain the
allocation-backed cached sort. This preserves one key calculation per move
and avoids a temporary heap allocation at ordinary search nodes.

The existing `search_depth4_startpos` Criterion benchmark measured:

| Revision | Median | Change |
|---|---:|---:|
| Before fixed ordering | 7.541 ms | baseline |
| After fixed ordering | 6.307 ms | -16.4% |

Criterion reported `p < 0.05`; two mild/severe outliers were observed. The
result is a local fixed-depth signal, not an rsshogi comparison, but it is a
direct improvement to the engine's root and alpha-beta ordering path.

## ProbCut capture-buffer follow-up

ProbCut capture generation now uses the reusable `MoveBuffer`, filtering and
ordering captures in place. This removes the temporary `Vec<Move>` and its
collection step from the probe path. A subsequent exact Criterion run of
`search_depth4_startpos` measured a 6.059 ms median versus the 6.307 ms
previous sample; Criterion reported no statistically significant change
(`p = 0.51`). The change is therefore retained as an allocation reduction with
no demonstrated fixed-depth regression, not as a claimed speed increase.

## SEE rollback verification

The attempted SEE-recursion buffer change was removed after it regressed the
same benchmark to an 8.373 ms median. With that change absent, a subsequent
exact run measured a 4.430 ms median (100 samples; four outliers). This
confirms that the regression is not present in the retained code. Because the
two runs were separated by rebuild and host scheduling changes, the absolute
difference is recorded as rollback verification rather than attributed solely
to the SEE experiment.

## Cross-library comparison rerun (2026-09-12)

The cross-library harness was rerun on the current working tree with 5,000
iterations and seven paired samples. Both libraries used release builds and
reused their output buffers. The rsshogi reference remains revision
`a1dbc020e0711574ba0bec6a7b123c411ccdd625`. Values below are medians in
nanoseconds per iteration; ratios are calculated as the slower median divided
by the faster median.

| Operation | Sekirei | rsshogi | Faster side |
|---|---:|---:|---|
| Legal moves, startpos, `Vec` | 347 ns | 330 ns | rsshogi 1.05x |
| Legal moves, startpos, fixed buffer | 361 ns | 299 ns | rsshogi 1.21x |
| Legal moves, startpos, packed output | 337 ns | 284 ns | rsshogi 1.19x |
| Legal moves, startpos, narrow output | 263 ns | 285 ns | Sekirei 1.08x |
| Perft(2), startpos | 6.740 us | 11.361 us | Sekirei 1.69x |
| Perft(3), startpos | 263.257 us | 389.542 us | Sekirei 1.48x |
| Full state update, six moves | 1.981 us | 2.954 us | Sekirei 1.49x |
| Search state update, no NNUE | 2.000 us | 3.277 us | Sekirei 1.64x |
| Search state roundtrip, no NNUE | 2.061 us | 3.142 us | Sekirei 1.52x |
| Legal-root state roundtrip | 16.288 us | 6.725 us | rsshogi 2.42x |
| Legal moves, midgame with hands, `Vec` | 815 ns | 673 ns | rsshogi 1.21x |
| Legal moves, midgame without hands, `Vec` | 326 ns | 286 ns | rsshogi 1.14x |
| Legal moves, midgame with hands, fixed buffer | 769 ns | 680 ns | rsshogi 1.13x |
| Legal moves, midgame with hands, packed output | 664 ns | 672 ns | Sekirei 1.01x |
| Legal moves, drop-only | 1.776 us | 1.676 us | rsshogi 1.06x |
| Legal moves, drop-only without pawn | 1.348 us | 1.436 us | Sekirei 1.07x |

This rerun is a bounded local comparison, not a claim of overall engine
superiority. It confirms that the remaining gap is concentrated in the
ordinary legal-move API and the legal-root full-state roundtrip; the deeper
Perft and search-state paths remain faster in Sekirei. The host, compiler
flags, and scheduling state were not independently controlled, so close
results (about 1.05x or less) should be treated as ties until repeated on an
idle host.

## Rejected unstable qsearch sort experiment (2026-09-12)

Replacing qsearch's stable cached tuple sort with `sort_unstable_by` measured
3.407 ms for `search_depth4_startpos` in a 30-sample run. This apparent
improvement was not accepted: changing the order of equal-key moves can change
the explored tree and therefore the node count and selected line. The stable
cached sort was restored so the search benchmark remains comparable and
deterministic.

## Rejected qsearch insertion-sort experiment (2026-09-12)

A second qsearch experiment used a small stable insertion sort that avoided
the fixed 64-entry temporary array by recomputing cheap keys. It was also
reverted after the 30-sample Criterion run measured an 8.009 ms median for
`search_depth4_startpos`, a significant +37.9% change versus the retained
sample (`p < 0.05`).

## Rejected qsearch sort experiment (2026-09-12)

Replacing qsearch's tuple-key cached sort with the small fixed-array i32 sort
was tested and reverted. A Criterion run with 30 samples measured a 7.026 ms
median for `search_depth4_startpos`, a significant +39.0% change versus the
previous retained sample (`p < 0.05`). The experiment is not part of the
working tree; this result is recorded to prevent reintroducing it.
