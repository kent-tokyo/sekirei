# Official NNUE v1, Tier 2 — dataset sizing measurement

Answers the preregistration's (`docs/experiments/official_nnue_v1_preregistration.md`, PR #55) own explicit
"Dataset sizing plan" section, which deliberately deferred picking a corpus size until real per-game cost was
measured. Base SHA: `99806a5` (main tip after PR #59). Branch: `experiment/official-nnue-v1-tier2-sizing`.

## Deviation from the preregistration's original sketch (justified, not silent)

The preregistration's own sketch suggested measuring cost via `--games <subset> --export observations.jsonl
--depths 2,4,6,8` (the Quietset multi-depth export step). **Not used.** Tier 2's actual candidate recipe does
not route through Quietset at all -- it uses `--games` directly with `--wdl-lambda 0.7 --label-depth 4`, a
single search depth, not the 4-depth Quietset-stability export. Measuring the Quietset step's cost would not
have been a real proxy for Tier 2's actual cost. Instead, this measurement runs the **exact Tier 2 recipe
shape** (single seed, 1 epoch, real `--min-ply 20 --min-rate 1800 --wdl-lambda 0.7 --label-depth 4`) on a
small, deterministic subset -- both sizing the cost and mechanically dry-running the `--games` path for the
first time, which the preregistration separately flagged as untested-by-any-prior-real-checkpoint in this
project.

## Method

Deterministic subset: the first 100 files, sorted by filename, from `data/csa/2023/` (symlinked into a scratch
directory, not copied -- `--games <dir>` reads non-recursively via a single `fs::read_dir`, confirmed by
reading `crates/sekirei-train/src/main.rs` directly, not assumed).

```sh
cargo run --release -q -p sekirei-train -- \
  --games /tmp/csa_subset_100 \
  --label-depth 4 --wdl-lambda 0.7 \
  --min-ply 20 --min-rate 1800 \
  --validation-ratio 0.15 --split-seed 42 --shuffle-seed 7 --init-seed 42 \
  --epochs 1 \
  --output /tmp/tier2_sizing_weights.bin
```

## Measured (real numbers, not estimated)

| Quantity | Value |
|---|---|
| Raw CSA files in subset | 100 |
| Games surviving `--min-rate 1800` filter | 82 (82%) |
| Train / valid games (game-level split) | 71 / 11 |
| Total labeled positions | 2,565 (2,216 train + 349 valid) |
| Total wall-clock (labeling + 1 epoch train + valid) | **1,143.85s (19.06 min)** |
| Of which label search time | 1,141.9s (99.8% of wall-clock -- training itself is near-free by comparison, matching Tier 1's own finding) |
| CPU utilization | ~102% (`1169.05s user / 1143.85s real` -- effectively single-threaded, same as Tier 1) |

Derived rates:
- **11.44 s / raw CSA file** (1,143.85s ÷ 100)
- **13.95 s / post-filter game** (1,143.85s ÷ 82)
- **0.446 s / labeled position** (1,143.85s ÷ 2,565)
- **25.65 positions / raw file, 31.28 positions / post-filter game**

Search-time variance is real and large, not a rounding artifact: several individual searches took 5-24s
(logged as `slow search:` lines) against a ~0.35-0.5s typical case -- correlated with high `legal_moves` counts
(124-337) on complex mid-game positions. A larger corpus's total time is a sum over this real distribution, not
a clean multiple of the mean.

## New operational risk found (relevant to how Tier 2 should actually be executed, not just how big)

**The teacher-label cache is written to disk only once, at the end of the epoch that computed it** (confirmed
by reading `crates/sekirei-train/src/main.rs` -- same "after epoch 1: merge new entries into cache" pattern
already documented for the `--positions` path). A process killed mid-epoch -- and this session has an
unresolved, reproducible pattern of exactly that happening to backgrounded `cargo run` processes at roughly
the ~30-minute mark, cause undiagnosed, `tasks/lessons.md`'s 2026-08-21/22 entry -- loses **all** labeling
progress for that epoch, not just the tail end. This measurement's own 19-minute run completed inside that risk
window without incident, but a larger single-pass corpus extrapolated from the rate above would not.

## Corpus size options (time estimated from the measured rate; disk is not the binding constraint here --
## CSA files are read in place, not copied, and the teacher cache/checkpoints are small)

| Raw CSA files | Est. post-filter games | Est. positions | Est. single-pass labeling time |
|---|---|---|---|
| 100 (this measurement) | ~82 | ~2,565 | ~19 min (measured) |
| 300 | ~246 | ~7,700 | ~57 min |
| 500 | ~410 | ~12,800 | ~95 min (1.6h) |
| 1,000 | ~820 | ~25,600 | ~190 min (3.2h) |

None of these have been run -- all but the 100-file row are extrapolated from the single measured rate above,
not independently verified at that scale.

**Recommendation, not a decision**: given the unresolved ~30-minute background-kill risk and the all-or-nothing
epoch-end cache write, a single labeling pass safely inside that risk window (i.e. the 100-300 file rows) is
the conservative choice for the *first* real attempt -- not because a bigger corpus wouldn't be better for the
final model, but because losing an hours-long unattended run's entire labeling progress to an undiagnosed kill
is a real, demonstrated failure mode, not a hypothetical one. A larger corpus becomes lower-risk once either
(a) the kill's cause is diagnosed/fixed, or (b) the trainer gains incremental teacher-cache checkpointing
(flagged below as discovered work, not implemented here -- out of scope for a sizing measurement).

**3-seed cost multiplier**: Tier 2 runs 3 separate `--init-seed` values. Teacher labels do not depend on
`--init-seed` (only on `--label-depth` and the position itself) -- routing all 3 seed runs through the *same*
`--teacher-cache <path> --reuse-teacher-cache` file means only the **first** seed run pays the full labeling
cost measured above; seeds 2 and 3 read the cache instead of re-searching. **Not yet used in this
measurement** (this run had no `--teacher-cache` flag at all) -- strongly recommended for the actual Tier 2
execution, since it turns a ~3x labeling cost into ~1x.

## Discovered work (not implemented here, flagged for whoever picks this up)

- Incremental teacher-cache checkpointing (write periodically during the epoch, not only at its end) would
  directly de-risk any single-pass corpus larger than what safely fits inside the ~30-minute kill window --
  same spirit as PR #58's periodic progress logging, but for durability, not just visibility.
- The ~30-minute background-process kill itself remains undiagnosed (`tasks/lessons.md`).

## What this measurement does not decide

The actual corpus size for Tier 2's real 3-seed run. That is presented above as options with real time
estimates, not resolved by this document -- a product/risk-tolerance call, not a technical one.
