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

## Completed observations

Clean native source: `72706632774c15e6b3690d1847dd0fc465a643ca`.
A SHA256: `59d99aabf1aaade97e742f994d3d919cd81a464aed40313da28a48ec6c448be4`.
B SHA256: `bedd89c31f348896df4a576a6243b376b62bdcbe74be849a9a2a6ccbceeca741`.
[results.json](results.json) binds receipts, raw captures, machine code,
test logs, presearch input hashes and PV validation.

Both core configurations passed 381 tests with 4 ignored. Default workspace
Clippy and feature-enabled core Clippy passed. The native machine code uses
fixed bitboard offsets and removes the kind-loop/exclusion branches; the
function grows from 89 instructions / 356 bytes to 234 instructions / 936 bytes.
These sizes describe static code, not dynamically executed instruction counts.

All 20 1T fixed-depth ABBA blocks matched bestmove, score/unit/bound, PV, nodes
and completed depth. The 4T and 8T lanes each varied within arms in all 20
blocks and had no exact cross-arm blocks; no parallel identity is claimed.
The 240 searches replayed 1,484 PV moves with zero legality violations through
the frozen same-rules validator.

Both timed attempts stopped before any search because the host exceeded the
25%-CPU threshold. Attempt 1 recorded a Sekirei data-generation process;
attempt 2 recorded two Rust compiler processes and began more than 60 seconds
after attempt 1. No timed search ran, no process was stopped, no
busy-host override was used and no third attempt is permitted by this protocol.
There is no qualified 1T/4T/8T NPS or completed-depth result. The development
screen does not advance; no independent H1 or YaneuraOu comparison was opened.
The feature remains default off.
