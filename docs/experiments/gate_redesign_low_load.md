# PR #4 low-load re-gate redesign

Status: **historical design record**. The proposal followed three wall-clock
gate attempts interrupted by host contention. The old SHAs, paths, and numeric
resource thresholds below were never a completed match and must not be reused
as current settings. Current admission logic lives in
`scripts/gate_resource_preflight.py`.

## 5A. Fixed-depth correctness pre-filter

Purpose: find crashes, illegal moves, score explosions, or severe search-cost
changes before starting a long match. It is not an Elo gate.

Frozen design at the time:

| Item | Historical value |
|---|---|
| Base / candidate | `0bb4221` / `9b61ed4` |
| Weights | identical `weights_v011_opening_combined.bin` |
| Corpus | fresh non-overlapping 50-position slice |
| Search | `go depth 10`, `Threads=1`, then-hard-coded speculative top-N 3 |
| Safety | one process and one position at a time, external hard timeout |

Fixed-depth and fixed-node tests answer different questions. Fixed depth
compares work needed to finish the same nominal iteration; fixed nodes compare
decisions under the same resource budget. Neither alone establishes strength.

The later run series found `SpecTopN=3` noise-dominated even with
`Threads=1`. Current deterministic pre-filters must therefore use
`SpecTopN=0`; parallel behavior requires repeated A/A controls. See
[`fixed_depth_gate_run_index.md`](fixed_depth_gate_run_index.md).

## 5B. Time-controlled paired match

The proposed match used one thread per engine, one sequential shard, identical
weights and openings, 10-second byoyomi, and 300 color-reversed pairs. It
required at least 240 completed pairs and under 5% time forfeits. The result
states were deliberately separate:

- valid statistical result;
- `INCONCLUSIVE` at the fixed cap;
- `CONTAMINATED` by resource pressure or time forfeits;
- invalid execution or provenance.

No result from the three interrupted attempts was promoted to candidate
strength evidence.

## 5C. Resource admission and continuation

The durable design rule is to separate **launch refusal** from
**continuation abort**:

- Before launch, record CPU topology, load, memory pressure, swap used, free
  disk, named competing jobs, expected engine processes, and real worker count.
- During a run, monitor sustained load and memory pressure, a rise in swap used
  relative to the start, new competing jobs, and rolling time-forfeit rate.
- A transient sample should warn; repeated or severe pressure should stop new
  games and preserve the current record.
- Resource relaxation is diagnostic-only and cannot qualify a formal gate.

The old estimate was:

```text
CPU-competing workers = parallel_shards × 2 engines × (Threads + speculative workers)
```

Modern code must derive this from the actual selected backend and options;
hard-coded `+3` is historical.

## 5D. macOS swap lesson

The original design used `swap_used / swap_total`. On macOS, the total swap
file allocation can shrink while used bytes remain nearly flat, making the
percentage rise even though pressure improved. A fixed percentage therefore
produced false refusals.

Use absolute used bytes and change from a session-start baseline, together
with `memory_pressure`, load, and free-memory signals. Do not infer recoverable
RAM from swap percentage alone, and do not tune a threshold after seeing a
candidate result.

## Current execution boundary

For a new run:

1. Create a new run ID and freeze revisions, binary hashes, weights, options,
   corpus, statistical rule, and artifact directory.
2. Run `scripts/gate_resource_preflight.py` and retain its JSON output.
3. Run a deterministic fixed-depth or fixed-node pre-filter with A/A evidence.
4. Start the paired gate only if correctness and resources pass.
5. Preserve partial output and classify contamination instead of silently
   restarting or combining runs.

This file records why the safeguards exist. It is not a current launch recipe
and contains no unreported benchmark or match result.
