# Own-HalfKP compiler PGO: v0.4.4, 8T

The dedicated local PGO binary exceeded the pinned YaneuraOu build by
**3.09% aggregate nodes per measured wall second** on Apple M4. The
position-bootstrap 95% interval is **+0.76% to +5.40%**. Independent QA
recomputed the statistics and checked input, raw, host and protocol bindings.
This qualifies the tested artifact and workload. Ordinary Cargo builds do
not automatically use this profile.

| Completed comparison | Ratio | Bootstrap 95% interval |
| --- | ---: | ---: |
| Fresh own HalfKP speed32, PGO/native, 8T | 1.02411821 | 1.01623498–1.03127459 |
| Pinned YaneuraOu, common evaluator, quick31, 8T | 1.03088520 | 1.00764017–1.05401927 |

The source is `4afc8608e43646743ae78564ec45a3e98540d4a4`; its Rust engine,
Cargo and `.cargo` files match baseline main
`adb4745e6eb33ee5102d857f17b18fa4196d2653`. The change is compiler profiling,
without search-policy or NNUE-weight changes. Binary, corpus, evaluator and
local receipt hashes are in [results.json](results.json). Local receipt
hashes bind retained evidence; private raw files are not downloadable artifacts.

## Build and own gate

Rust 1.97.0 / LLVM 22.1.6 used native CPU, optimization level 3, Thin LTO,
one codegen unit and panic abort, with separate native/instrumented/profile-use
target directories. The fresh profile searched 64 tracked positions for
1,000 ms each at 8T using the read-only own r3 HalfKP file and its tracked
public candidate card. Actual load acknowledgements, model/card hashes,
FV_SCALE=24 and normal process exits were checked for every training call.
Only training-process profiles were merged; build-script profiles were kept
separate. Full symbol/Mach-O audits found no instrumentation in timed binaries.
The profile-use log had 135 missing-function warnings discarding zero counts
and two Cargo summary warnings, without nonzero/hash/CFG mismatch warnings.

This is runtime compiler profiling, not NNUE training or candidate-model
qualification. No external evaluator was used for profiling, tuning,
candidate selection or default selection. Build reproduction from the pinned
clean checkout uses the card-matching own file supplied by its owner:

```bash
python3 scripts/build_pgo_lane.py --output results/own-halfkp-pgo-build \
  --training-positions docs/experiments/pgo_native_v0.4.1/compiler-profile-training.sfen \
  --threads 8 --count 64 --movetime 1000 \
  --training-evalfile /path/to/own-r3.bin \
  --training-eval-card weights/candidates/sekirei-halfkp-r3.json
```

The [own speed corpus](own-speed32.sfen) has 16 positions per side, generated
with material-only legal playouts, seed 20261018081. Tracked experiment SFENs
and quick31 were excluded before choosing the first 16 eligible states per
side, without timing/score selection. This is a speed corpus, not an
owner-attested NNUE training or strength holdout.

Fresh 1T depth-12 ABBA searched these 32 positions: all 128 calls matched
bestmove, score, PV, nodes, completed depth and bounds exactly, including only
identical legitimate early mates. CPU-quiet admission was removed for semantic
identity, before any identity call, because elapsed time was not compared;
the amendment and two earlier zero-call attempts are retained. Timed gates
retained their original quiet admission and host guards.

The own 8T gate had 128 accepted / 152 actual calls and all 32 eligible
positions. Six host-rejected whole quartets were retained. An earlier
600-second quiet timeout made zero calls and was not pooled. Candidate
selection was frozen only after own PASS and independent QA, before private
reference-evaluator access.

## Direct reference gate

Both engines used 8T, Hash 64 MiB, 1,000 ms, FV_SCALE=24 and the same private
HalfKP 256x2-32-32 file. Sekirei used Rayon=8, Auto, SpecTopN=0, LazyFlags=15,
diagnostics off, MoveOverhead=0 and absolute NNUE output. The reference used
its native hash/thread options, book off, delays zero, MinimumThinkingTime=0
and RoundUpToFullSecond=false. Load acknowledgements and ready responses were
verified; file identity does not establish bitwise evaluator-output equivalence.
The Apple M4 has four performance and six efficiency cores; affinity was not
fixed. Reference source `c1b80eaa09fe13d5f12b1599d1ae4d53c224de30` used Apple
clang 21, APPLEM1/NNUE, NEON dot-product and native CPU flags.

Eight fresh White/Black smoke calls passed before the quick31 ABBA run
(all 31 reference positions are White). All 31 quartets completed, with 124
accepted / 128 actual calls and one host-rejected whole quartet retained.
The preregistered <700 ms rule excluded position 28 as a whole, leaving 30
eligible positions; it did not trigger a retry. Each position divides the
candidate arm mean of nodes/wall time by the reference arm mean. Ratios are
geometrically aggregated; 5,000 position-bootstrap draws use seed 20261018.
The gate requires at least 20 eligible positions, point ≥1 and lower bound >1.

A persistent 1 Hz host guard required five quiet samples, rejected external
compilers at ≥1% CPU, single outside processes at ≥70%, aggregate outside CPU
at ≥100%, and free disk below 1 GiB. At most three whole-quartet attempts were
allowed for host interference; no NPS value entered retry decisions. Independent
QA replayed every host decision across 268 samples (maximum cadence gap 1.30 s),
checked the exact 521 raw files and 2,315 input bindings, and recalculated the
complete-capture interval. No partial speed statistics were used.

All 128 actual external PVs, including host-rejected calls, replayed as legal;
the illegal 5i5a negative control failed as expected. Replay made no search
calls. Every measured process quit normally, returned zero and was reaped.
The protocol sends `usinewgame→isready/readyok→position→go`; the reference
resets its board on `isready`. Wall timing begins at go send, so queued
position work can enter that interval.

Cross-engine nodes count different trees. This is raw throughput for the
pinned binary and corpus, not useful-depth, strength, Elo, 4T, another CPU or
ordinary-release superiority. The earlier [8T study](../compiler_pgo_8t_v0.4.2/README.md)
is separate; its ratio is not pooled or multiplied with this result. New
Finny in-place code is absent from this measured binary. External scores/PVs,
weights and teacher labels stay private and are not used for training or
selection. Search/Cargo defaults and NNUE weights remain unchanged.
