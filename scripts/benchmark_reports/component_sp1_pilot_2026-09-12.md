# SP1 component pilot — 2026-09-12

This is a bottleneck-discovery run, not the rsshogi speed gate. It used the
debug `cross_library` binary, synthetic deterministic LCG NNUE weights, 21
samples per case, and a 10 ms calibration window. The capture recorded an
Apple M4 with load averages 3.42 / 4.54 / 8.34 before measurement, so these
values must not be used as a release or cross-product performance claim.

Capture: `/tmp/sekirei-component-capture-sp1` (immutable binary snapshot,
provenance, raw samples, and validated summary). The runner now rejects any
capture that does not contain the fixed 46-case set.

Selected p50 diagnostics (rsshogi / Sekirei; greater than 1 means Sekirei was
faster for that row):

| Operation | Ratio | Note |
|---|---:|---|
| SFEN initialization | 0.18x | Sekirei includes its NNUE refresh; not an equal evaluation-state workload |
| Start-position legal generation (Vec) | 0.38x | Vec output vs rsshogi Move32 output |
| Start-position legal generation (Fixed/Packed) | 0.38x / 0.39x | Output representation alone does not close the gap |
| Midgame legal generation (Vec) | 0.58x | Vec output vs rsshogi Move32 output |
| Midgame legal generation (Fixed/Packed) | 0.58x / 0.61x | The bottleneck is not removed by output packing |
| Drop-only legal generation (Packed/Narrow) | 1.17x / 1.19x | Compact sinks help this drop-heavy case |
| Six-ply no-NNUE roundtrip | 47.14x | Rule-state work is explicitly not identical; diagnostic only |

The isolated king-safety scan was 5.00 us on startpos and 6.74 us on the
midgame fixture, compared with 7.96 us and 9.37 us for full Fixed legal
generation in the same capture. This is a bottleneck signal, not a directly
additive decomposition because the full path also computes and emits moves.

The only immediate actionable signal is that ordinary and midgame legal
generation remain behind rsshogi in this protocol, while compact output helps
only the drop-heavy case. Narrow is the best of the tested Sekirei sinks in
this pilot, but still trails on startpos and midgame. The next SP1 task is to split generation into
constraint calculation, piece emission, and output conversion on the same
legal corpus, then rank end-to-end candidates. No optimization is accepted
from this pilot alone.

The isolated four-direction ray probes were 0.576/0.606 us (rook/bishop) on
startpos, 0.581/0.598 us on midgame, and 0.531/0.578 us on drop-only. Ray
lookup/arithmetic is therefore measurable but not large enough by itself to
explain the full-generation gap; the next candidate should focus on piece
iteration and promotion emission rather than replacing the bitboard layout.

The 32-case correctness smoke corpus passed Sekirei/rsshogi legal-USI-set,
Perft(2), and per-move do/undo checks before this pilot.

## First candidate change

The attack predicate now exits after step-attacker checks when the opponent
has no lance, bishop/horse, or rook/dragon. This is a safe endgame/drop-heavy
fast path and leaves the normal sliding path unchanged. The focused
before/after debug captures were noisy: king-safety p50 changed from
5.00→5.30 us on startpos, 6.74→6.59 us on midgame, and 0.225→0.222 us on
drop-only. It is retained as a correctness-preserving micro-optimization,
but no stable speed gain is claimed from these captures.

The next candidate replaced full ray-mask materialization in king safety with
first-blocker inspection. Against the pre-change debug capture, three
post-change sessions gave these p50 ratios (pre/change):

| Case | Session 1 | Session 2 | Session 3 |
|---|---:|---:|---:|
| Startpos king safety | 1.157x | 1.138x | 1.135x |
| Midgame king safety | 1.054x | 1.022x | 0.999x |
| Startpos full Fixed generation | 1.025x | 1.032x | 1.048x |
| Midgame full Fixed generation | 1.007x | 1.013x | 1.025x |

Drop-only was neutral within noise (0.984x–0.995x). The candidate is retained
for the next paired gate because it is correct and improves startpos in these
diagnostics, but it is not yet a formal 1.05x result: the captures use debug
builds and do not satisfy the planned ten-session confidence gate.

The first-blocker helper was then tightened to classify the blocker through
the existing piece bitboards instead of a mailbox read. In one follow-up
capture, full Fixed generation changed by 1.053x / 1.010x / 1.018x on
startpos / midgame / drop-only relative to the preceding candidate. The
isolated safety probe varied in both directions, so this second change is
retained as a correctness-preserving implementation detail but still has no
formal speed claim.

## SP3 roundtrip pilot

The runner also measures applying and undoing every legal move from a fixed
position, with the legal list prepared outside the timed operation. The
debug capture `/tmp/sekirei-component-capture-sp3-roundtrip` reported:

| Position | Legal moves | p50 per whole iteration | Approx. per move |
|---|---:|---:|---:|
| Startpos | 30 | 2.106 us | 70.2 ns |
| Midgame | 169 | 12.281 us | 72.7 ns |
| Drop-only | 525 | 37.235 us | 71.0 ns |

This is a Sekirei-only diagnostic because the current rsshogi component
runner does not expose the same all-legal-move roundtrip contract. It isolates
the do/undo cost per move but does not yet prove a cross-library advantage.

The cross-library diagnostic also records the current debug layout: `Board` is
1904 bytes with 16-byte alignment, `Move` is 5/1, `MoveToken` is 16/8,
`PackedMove` is 4 bytes, and `NarrowMove` is 2 bytes. These are layout facts,
not release-performance claims; the release-profile pilot below confirms the
same layout contract in the optimized build.

## Release-profile fixed pilot

The 21-sample release-profile pilot was run with the same fixed component set.
The benchmark contract now targets a 50 ms timing window per sample (with a
20 ms minimum), reducing timer noise for the smallest move-kind cases.
For legal move generation, the rsshogi p50 divided by Sekirei p50 was about
1.03--1.10x on startpos, 1.04--1.14x on midgame, and 1.09--1.20x on the
drop-only fixture across Vec/Fixed/Packed/Narrow. The strongest rows were
Packed/Narrow in startpos and midgame, and Fixed in drop-only. The pilot does
not establish a formal ten-session gate or an end-to-end search lead; do/undo
and NNUE rows remain separate because their work contracts differ.

## SP4: rules-only SFEN initialization

The explicit `Board::from_sfen_rules_only` path was measured in the same
release-profile component pilot. `init_sfen_warm` was 1358.0 ns p50, while the
rules-only path was 434.8 ns p50, or approximately 3.12x faster for this
initialization contract. The rules-only board preserves the reconstructed hash
but intentionally has no NNUE-ready accumulator; this is an initialization
result, not a claim about recursive search throughput.

## SP4/SP6: isolated NNUE refresh

The revised release pilot adds `sekirei_nnue_refresh` separately from the
existing `sekirei_nnue_forward` case. Under the 50 ms target and 20 ms minimum
sample contract, refresh p50 was 909.47 ns for startpos, 910.40 ns for the
midgame fixture, and 398.78 ns for the drop-only fixture (12 samples each,
except startpos with 11 due to the fixed case schedule). The capture is stored
at `/tmp/sekirei-component-capture-sp6-refresh` and validated as a 57-case
pilot. These values explain the initialization/evaluation split; they are not
an end-to-end search result and do not establish a cross-library speed lead.

The same 66-case capture also isolates explicit-weight evaluation and cloning.
`sekirei_nnue_evaluate_with_weights` p50 was 2744.37 ns, 2734.62 ns, and
2135.26 ns for startpos, midgame, and drop-only. The rules-only board followed
by explicit evaluation was 2753.42 ns, 2740.50 ns, and 2140.06 ns. A direct
`Board::clone` was 17.27 ns, 17.37 ns, and 17.23 ns. Weight construction and
SFEN parsing were outside the timing window. The full capture was validated
from `/tmp/sekirei-component-capture-sp4-state-boundary-v2`.

## SP5: release assembly check

The release AArch64 assembly was inspected after the split prototype tests.
The relevant bit operations lower to native `rbit`/`clz` sequences for least
significant-bit extraction, while population-count paths use NEON `cnt` where
vectorized. No evidence currently justifies replacing the production u128
mapping; the `[u64; 2]` prototype remains a measured-later alternative rather
than an adopted representation.

## Measurement-window revision

The component runner now targets 50 ms per sample, rejects samples below 20 ms,
and permits up to `1<<27` iterations. A 54-case release pilot passed the
validator with observed sample durations from 47.83 ms to 103.71 ms. Earlier
10 ms-window captures are retained as historical diagnostics and are not mixed
with the revised A/A contract.

## SP3: move-kind do/undo baselines

The release pilot also exercised one legal move per transition kind using
fixed fixtures and the same 21-sample timing contract. p50 was 7.30 ns for a
quiet move, 11.97 ns for a capture, 7.51 ns for a drop, and 9.77 ns for a
promotion. These are Sekirei-internal baselines: the current rsshogi adapter
does not expose the same single-move token contract, so no cross-library ratio
is inferred from them.

## SP7: same-binary A/A noise floor

Ten captures of the same release binary were stored under
`results/rsshogi-speed/component-sp7-aa/`, including raw samples, validated
summaries, provenance, and the frozen executable. Across all 54 cases, the
five adjacent A/A pairs produced an overall p50 geometric mean of 1.0025x;
case medians ranged from 0.9964x to 1.0126x. Individual microcase ratios still
range from 0.9265x to 1.3785x, so this remains a measurement-noise pilot rather
than a candidate comparison or a formal performance gate.

The release correctness follow-up also passed the ignored
`random_perft_mated_10m` test (10,000,000 cases, 31.93 s) and the startpos
Perft(5) test. These establish regression evidence for the current movegen
changes; they do not replace the remaining formal performance gate.

## Revised 50 ms A/A follow-up

The revised 50 ms contract was repeated for 10 same-binary captures (five
adjacent pairs) under `results/rsshogi-speed/component-sp7-aa-50ms/`. The
overall geometric mean was 1.0014x, pair geometric means ranged from 0.9966x
to 1.0060x, and case medians ranged from 0.9896x to 1.0607x. The longer window
reduced the earlier noise, but initialization and aggregate microcases still
need separate treatment before a formal candidate gate.

The release workspace correctness run also completed successfully across the
core, USI race, CSA, match-runner, training, and benchmark targets. The core
library reported 139 passed and 2 ignored, and the training target reported
158 passed; the ignored core checks were run separately in release mode.

## SP7: 66-case A/A rerun

The expanded 66-case contract was captured ten times in one monitored session
under `/tmp/sekirei-component-sp7-aa-66ms/`. Every capture passed the preflight
and sample validator. The five adjacent pair geomeans were 0.9976x, 1.0068x,
1.0064x, 0.9056x, and 1.0110x; the aggregate geomean was 0.9846x and case
medians ranged from 0.9765x to 1.0522x. The fourth pair shows a broad
one-direction drift rather than a stable binary effect, so this is recorded as
an inconclusive noise-floor run, not as a candidate performance result.
Using the five pair geomeans, the small-sample log-ratio t-approximation 95%
interval is 0.9288x..1.0438x and is reported by the aggregator alongside the point estimate. Because
the fourth pair is affected by directional drift, the interval is diagnostic
and is not a release gate.
