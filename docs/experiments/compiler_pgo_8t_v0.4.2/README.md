# Compiler PGO: 8T evidence draft

**Independent final QA passed.** The completed capture reports the
source-identical PGO binary above the pinned YaneuraOu reference by **4.6537% in
aggregate nodes per measured wall second**, with a bootstrap 95% interval of
**+2.5487% to +6.6855%**. This concerns the dedicated local binary; ordinary
Cargo/release builds do not automatically use its profile.
The integration base is newer (`f86485ad`); this result remains pinned
to the tested `7a23c91` artifact, not a latest-main or rebuilt release binary.

| Completed measurement | Ratio | Bootstrap 95% interval |
| --- | ---: | ---: |
| Own independent 8T holdout, PGO/native | 1.04511808 | 1.03612136–1.05448512 |
| Pinned reference 8T, PGO/YaneuraOu | 1.04653707 | 1.02548662–1.06685475 |

## Build and own qualification

Source `7a23c91bcfe0fb01284e02737946afffab97bc82` (v0.4.2) is unchanged:
all 148 compiler inputs match the fresh native control. The core remains
Pure and Safe Rust. Rust 1.97.0 / LLVM 22.1.6, target-cpu=native, opt-level=3,
Thin LTO and one codegen unit were held constant. Native, instrumented and
PGO builds used the same checkout, target directory and command.
All 19 build phases passed, including 434 core tests (9 ignored) and six
full-symbol/Mach-O runtime audits. Timed binaries have no profile hooks.

Fresh atomic compiler profiles used 64 material and 64 own synthetic
positions, each searched for one second with eight workers. Old profiles
and build-script profiles were not merged. This is compiler profiling,
not NNUE training. Binary/profile, source, toolchain, corpus, model and
local evidence hashes are recorded in [results.json](results.json).

All 384 fresh one-thread depth-12 searches matched bestmove, score, PV, nodes, depth
and bound exactly. The independent own gate used 32 new legal-playout
states (seed 2026101768), excluding training64, quick31 and 18 historical
corpora. Its [holdout](own-holdout32.sfen) and portable
[provenance](own-holdout-provenance.json) allow future exclusion.
Missing tail32/256 exclusion files were restored with their original SFEN
hashes. The old development manifest was not recovered; a new hash-bound
bridge preserves all exclusions. Historical helpers were not executed.

Own testing used synthetic HalfKP/FV_SCALE=16, fresh-process ABBA, 128 accepted
calls and 32 eligible positions. Two host-rejected quartets were retained
separately. All 590 frozen bindings matched after the run and independent QA.
Actual protocol traces verify normal quit, exit 0 and reap without fallback
kill. PV-replay exit 0 is enforced by the audited controller's passed record;
its stdin/stdout/stderr were independently checked, without a separate raw
exit-code sidecar. Selection was frozen before private reference comparison.

## Pinned reference comparison

Both engines used eight threads, Hash=64 and the same private HalfKP
256x2-32-32 file, FV_SCALE=24 and one-second searches. Sekirei used Auto,
SpecTopN=0, LazyFlags=15, MoveOverhead=0, diagnostics off and Rayon=8. The M4
has four performance and six efficiency cores; affinity was not fixed.
YaneuraOu source `c1b80eaa09fe13d5f12b1599d1ae4d53c224de30` used Apple clang
21.0.0, NNUE/APPLEM1, NEON dot-product and -mcpu=native. File identity does
not prove bitwise equivalence of loaded evaluator outputs.

Eight White/Black smoke calls preceded the fresh 31-position quick-corpus
study (all White). Reference→Sekirei→Sekirei→reference quartets produced
124 accepted calls; one additional whole quartet was rejected for host load.
Position 28 fell below the preregistered 700ms floor, leaving 30 eligible
positions, with no retry for that exclusion. All 128 search calls have actual
normal-shutdown sidecars. Independent QA checked all 32 PV replays/negative
controls, 386 host samples and all 2,128 frozen bindings (1,919 unique files).
PV replay exit proof uses stored markers/frozen code, not raw exit sidecars.

Each per-position ratio divides the two arm means of `1000*nodes/elapsed_ms`.
Geometric aggregation and 5,000 position-bootstrap draws use seed 20261014.
The reference gate requires at least 20 eligible positions, point≥1 and a
95% lower bound>1. Five quiet samples, a 512MiB floor, 600s quiet waits and
three host-only quartet attempts remain unchanged. Preregistered extra
30s continuous launch quiet and an outer 60s game-idle guard only schedule
launches; they do not alter acceptance/statistical guards. The outer guard
uses PID identity without exact process birth-time binding. Its 59 samples
span 60.854s; 28 metadata paths matched. Original status payload bytes were
not retained; launch sample age is reconstructed from UTC plus elapsed.

Secondary engine-reported NPS was 1.04722061 (95%: 1.02615923–1.06755917),
separate from the wall-NPS primary gate. Earlier partial captures stay unpooled;
their partial
statistics were not used. External scores/PVs, private weights and derived
teacher labels are not published or used for selection or training.

## Protocol and scope

The reference resets its board on `isready`; both engines therefore receive
`usinewgame→isready/readyok→position→go`. Elapsed time starts at go send;
queued position work can enter this interval. Older comparisons using the
incorrect position-before-ready order are not reused.

The own plan retains a copied “4T-only artifact” description-only erratum;
actual commands/training/traces/statistics are 8T. This result is independent of
4T measurements: ratios are neither pooled nor multiplied.

Cross-engine nodes count different trees. This is raw-throughput evidence
for the pinned workload, not useful-depth, playing-strength, Elo or ordinary
release-binary superiority. Search/Cargo defaults and NNUE weights are
unchanged. Raw reports and profiles stay local; hashes are evidence bindings,
not downloadable raw artifacts or hosted-CI proof.
