# Reuse one exact ProbCut SEE (default off)

This 2026-10-27 candidate removes a duplicate static exchange evaluation
without changing pruning thresholds or move ordering. It has not passed an
independent speed gate. The Cargo feature `probcut_see_reuse` and tune parameter
`V2_PC_SEE_REUSE` both default to off. No evaluator, NNUE weight or default
search policy is selected by this experiment.

ProbCut computes exact SEE for a capture and may then return to ordinary
move classification, which needs the same board-dependent value. Retain only
the first exact value for a non-TT move that is not a capture-up. Capture-up
moves may bypass exact classification; a TT move is searched first and does
not consume that classification. The enabled quiet-TT pending path is excluded.
A scalar `Option<(Move, i32)>` belongs to one parent invocation, and reuse
requires the complete same move after ProbCut children have been undone.
Capture-history keys are recomputed at their original boundary. Move-list
entry layout, generation order, classification, sorting, flags and thresholds
stay unchanged. No map or shared cache is added.

## Correctness and scope

The isolated tune test compares fresh TT/history searches at depths 3, 6 and 8
on eight deterministic legal-playout development roots (four per side), with
material evaluation and an in-memory `HalfKpNetwork::random(11)`. It separately
checks the default configuration, ProbCut SEE-bound enabled, and quiet-TT stage
enabled. All other parameters retain their defaults. These are development
fixtures, not independent holdouts or playing-strength data.

The required signature is exact bestmove, score, PV, nodes, completed depth and
bounds. Every returned PV is replayed against legal generation; root SFEN,
hash, accumulators and occupancy must be restored. Test-only counters must show
OFF writes/hits zero, actual ON writes/hits positive for each configuration,
and exactly one fewer SEE computation for each reuse. Unit tests also cover
nonmatching move fields, disabled/TT/capture-up cases, the first-slot rule,
negative/zero/positive exact values and lazy fallback closure counts.
Counters are absent from production binaries; these runs cannot establish NPS.

The CI feature lane checks compilation and unit boundaries, then runs the
isolated tune comparison without the compile-time feature that would force
both arms on. Coverage invokes that comparison separately, with the existing
98% threshold and exclusion expression unchanged. Local execution results and
hashes will be recorded separately from hosted CI.

## Completed local correctness checks

All 288 searches matched the required signatures and restoration/PV assertions;
all six cache unit tests passed. The three material configurations saved
170, 170 and 158 exact calls; the corresponding synthetic HalfKP configurations
saved 839, 839 and 788. These counts cover the instrumented ProbCut/classification
paths, not all SEE calls in the engine. Standard and feature-enabled core/USI
release Clippy, formatting and diff checks passed. Independent static review
found no blocking issue. [The local record](correctness.json) binds raw receipts
and compiler inputs; hosted CI/coverage remain separate.

The added slot and move comparisons may cost more than the SEE calls they save.
No native speed measurement or default adoption has been made.

## Prospective native development screen

Freeze this source and protocol before native binary builds or timing. Build A
with default features and B with only `probcut_see_reuse`, in the same worktree,
Cargo target directory, stable compiler, native CPU configuration and standard
release profile (ThinLTO, opt3, CGU1, panic abort). Jobs1/incremental0, no tune,
PGO, diagnostics or other candidate feature; save both binaries, build logs,
commands and hashes. Do not reuse the earlier PGO results.

Use the existing 20 public development positions from
`docs/experiments/lazy_smp_useful_depth_v0.4.0/diagnostic.sfen`, SHA256
`de70936824cd695f67a03093faa0980a00913544043057931941788fd0eaf5ad`.
Material evaluation, Hash64 MiB, SpecTopN0, LazyFlags15, MoveOverhead0,
SearchMode=LazySMP, fresh process per call, A/B/B/A per position.

First require exact 1T depth8 signatures and legal PV replay. Then screen 8T
with 1000ms per search, one ABBA quartet per position, counters off. Use the
tracked paired driver plus a full-interval host monitor. A busy/unknown host,
missing raw/normal-exit/PV proof, disk below 1 GiB or failed identity invalidates
timing; retain the attempt without pooling or a statistical rescue. A later
resource-recovery attempt needs a separate recorded output and preserved
limits/verdicts. Do not alter unrelated active jobs without their authorized
owner coordination.

Report aggregate worker nodes per wall second, selected-worker completed depth
and per-direction results separately. The development screen requires at least
16 eligible positions (all arms >=700ms), geometric mean ratio >=1.005,
position-bootstrap 95% lower bound >1 (5000 draws, seed20261027), and
nonnegative median selected completed-depth delta. This screen cannot authorize
default adoption, a YaneuraOu-superiority claim, or Elo. A separate unused-root
independent gate must precede adoption and any conditional external comparison.
