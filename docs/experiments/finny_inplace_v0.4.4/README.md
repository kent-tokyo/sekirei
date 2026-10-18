# Finny in-place refresh: opt-in native CPU candidate

This candidate removes one 512-byte accumulator copy from a warm Finny
refresh. It is **default OFF** and has no qualified speed or strength gain.
The cache becomes the delta-update destination; the final value is copied
once to the caller. Feature order, wrapping arithmetic and network format
are preserved. The cache entry is invalidated before partial updates and
published as valid only after completion.

```bash
cargo build --release -p sekirei --bin sekirei --features nnue_finny_inplace
```

Without the feature, the existing copy-in/copy-out path remains selected.
The feature forwards from USI to core and requires neither PGO nor `tune`.
In a tune-only build, `T_NNUE_FINNY_INPLACE` defaults to zero and permits a
0/1 diagnostic. Enabling the Cargo feature forces the in-place dispatch;
the tuning parameter cannot disable that compile-time choice. `NNUE_FINNY`
continues to control whether the refresh cache itself is used.

## Correctness and native builds

The measured source is pinned to
`221043cde704f31606de552ab0e2a8be018e7e5b`, based on main
`9204a3577b0e0a0a68930e09dcfea694212964d1`. Its feature-OFF and feature-ON
binaries were built from the same clean commit with release defaults,
native CPU and identical toolchain settings. The recipes clear ambient
Rust/profile/target/build overrides, use locked offline builds and separate
target directories, bind actual command receipts, and check all 149
tracked Rust/Cargo/config inputs before and after. Full symbol/Mach-O audits
found no instrumentation in either binary.

Default HalfKP tests passed 17 cases (3 ignored); feature ON passed 18
(3 ignored). The public refresh path matched full rebuilding across 142
legal-move/undo fixtures and 284 perspectives, including 266 changed warm
entries. Tests cover wrapping, malformed hands, network switching and a
real partial-update panic followed by cache invalidation and successful
rebuilding. The isolated tune test checked 568 refreshes with exact results.
Fmt and core/USI feature clippy passed; normal commit hooks also passed.
A separate CI feature job is configured for the native ON route; its
hosted result is pending.

Fresh 1T depth-12 ABBA made 128 searches on the
[new material-generated speed corpus](own-speed32.sfen). Bestmove, score,
PV, nodes, depth and bounds matched exactly in every quartet; all actual
PVs replayed as legal and the illegal 5i5a control was rejected. Identity
is semantic-only: elapsed time was not used for a speed claim.

## Speed qualification

The fixed own-r3 HalfKP evaluator is read-only, FV_SCALE=24, with explicit
successful path/format acknowledgements. No model was trained or selected.
The new speed corpus uses material-only legal trajectories, seed
20261018091, selecting the first 16 canonical positions per side after
excluding 233 existing states across eight frozen inputs. No score or
elapsed-time selection was performed. This is a speed diagnostic corpus,
not an owner-attested NNUE training or playing-strength holdout.

The 8T plan uses Rayon=8, Hash 64 MiB, one second, fresh processes and
baseline→candidate→candidate→baseline. Auto, SpecTopN=0, LazyFlags=15,
diagnostics off, MoveOverhead=0 and absolute NNUE output are held fixed.
It requires all 32 positions before statistics, excludes whole quartets
with an arm below 700 ms, and needs at least 24 eligible positions. The
primary ratio is the geometric mean of per-position mean(candidate
nodes/wall) / mean(baseline nodes/wall), with 5,000 bootstrap draws, seed
2026101808. Adoption needs point ≥1 and a 95% lower bound >1.

Host monitoring runs at 1 Hz and requires five quiet samples. External
compiler CPU ≥1%, single-process CPU ≥70%, aggregate outside CPU ≥100%
and free disk below 1 GiB reject complete quartets. At most three host-only
attempts are allowed per position. Rejected or incomplete captures are
retained and never pooled; no partial speed statistics are computed.
Normal shutdown, input hashes and every actual PV are checked.

Four 8T captures stopped on three consecutive host-rejected quartets.
Their actual-call counts were 16, 60, 36 and 160; their accepted-position
counts were 1, 5, 3 and 28. None completed the corpus. All summaries remain
null and are never pooled. All 272 actual PVs were independently replayed
as legal, with the negative control rejected. The latest retry checked all
731 bindings, raw protocol hashes and every actual evaluator acknowledgement.
Its host log showed actual outside load, with no owned engine misclassified.
An earlier no-compiler/30-second-quiet launch guard hit its 600-second ceiling
without starting an engine. Operational launch guards do not change speed
acceptance or statistics. No candidate speed value or confidence interval
is reported.

The [results summary](results.json) records the actual qualification state
and local evidence hashes. Private raw reports and own evaluator files
remain local. No private reference evaluator, external scores/PVs or
teacher labels enter this candidate's design, screening or default choice.
The candidate is separate from compiler-PGO throughput results; it remains
disabled until independent speed evidence justifies adoption.
