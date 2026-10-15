# Ordered correction-key expansion (default off)

Source base: `89f34715ff72dfb989c73a29a1113938fb5600b4`.
`correction_key_unroll` is an opt-in Cargo feature, forwarded by the USI
package. It expands the twelve non-pawn/non-king piece kinds in their existing
ascending order. Empty bitboards still leave the hash unchanged. Every key,
correction table, learning rule and evaluator is preserved; no Board cache,
search counter or pruning rule is added.

The native Rust 1.97 baseline retains loop control, pawn/king exclusions and
variable bitboard offsets in `CorrKeys::of` (356 bytes). `from_u8` decoding
already disappears in this build. The expansion targets the remaining loop
work, at the cost of more instructions and possible instruction-cache pressure.
Neither source-level work removal nor code size proves a search speed gain.

## Prospective development protocol

Commit source and this protocol before native builds or searches. Freeze clean
standard native release builds at the same source, compiler, target path and
environment: A without features, B with only `correction_key_unroll`. Save
hashes, build logs and receipts; no tune, PGO or diagnostic counters. Compare
the resulting function size and machine code as mechanism evidence.

Use the existing twenty development positions (10 quiet, 10 tactical), SHA256
`de70936824cd695f67a03093faa0980a00913544043057931941788fd0eaf5ad`.
Material, Hash64, MoveOverhead0, SpecTopN0, LazyFlags15, SearchMode=LazySMP,
no book and a fresh process per search. This corpus cannot qualify strength.

1. Fixed depth 8, 1T/4T/8T separately, one per-position ABBA repeat (240 searches).
   Require exact 1T bestmove/score/unit/bound/PV/nodes/completed-depth identity.
   Report parallel scheduling variation separately. Require legal returned PVs.
2. Only after correctness, fixed time 300 ms, 1T/4T/8T, two per-position ABBA
   repeats (480 searches), counters off. Use existing 25%-CPU pre/post guards.
   At most two attempts; attempt 2 is permitted only if attempt 1 blocks before
   any search, with a >=60 s gap. No retry after a completed/interrupted run,
   busy-host override or termination of other tasks.

The 8T development screen requires geometric mean NPS ratio >=1.005,
position-bootstrap 95% lower bound >1 (5,000 draws, seed 20261015), and
nonnegative median completed-depth deltas overall and for both categories.
1T/4T each require nonnegative overall median depth and NPS ratio >=0.99.
Keep all positions and both ABBA directions. A failed host/correctness check
cannot be rescued by a speed/depth observation.

This is a development screen, not an independent H1 or YaneuraOu comparison.
The feature remains default off; later adoption requires a separately pinned
independent gate. The queued aspiration strength candidate is unchanged.
