# Lazy SMP sampled-work overlap diagnostic

Status: diagnostic completed on 2026-10-12. No playing default changed.

This experiment measured repeated position work across Lazy SMP workers. It
used the source candidate based on `38925bfe0aac2a376e54a33cde48554ba0444565`
on arm64 macOS 27.0.1, material evaluation, `Hash=64`, six positions from
`data/gate/openings_standard.sfen` (SHA-256
`2240cc495e02d50befd45b445f4218d84ae6cdff355db952bdf776f82ad1a1e5`),
300 ms per position, and two repeats.

The sampler observes 1/256 mixed position hashes in a bounded lock-free table.
It runs only in the opt-in Lazy SMP diagnostic path. Percentages below are
pooled over the sampled visits or retained fingerprints; they are estimates,
not exact tree coverage.

## Shared and isolated transposition tables

| Threads | TT policy | Median depth | Cross-worker visits | Shared fingerprints |
|---:|---|---:|---:|---:|
| 1 | shared | 15.5 | 0.0% | 0.0% |
| 2 | shared | 15.5 | 10.5% | 7.3% |
| 4 | shared | 15.5 | 22.3% | 13.5% |
| 2 | isolated | 15.0 | 7.5% | 4.4% |
| 4 | isolated | 15.0 | 43.5% | 61.2% |

The shared table prevents a large part of the duplicated work: the four-worker
isolated control revisited most retained sampled fingerprints from more than
one worker. The remaining 22.3% cross-worker visit rate in the shared
configuration is large enough to justify worker-diversity experiments. TT
physical collision writes remained too rare to explain the loss.

## One-factor diversity screens

`LazyFlags=271` keeps the normal policy but changes four workers' iterative
depth offsets from `(0, 1, 0, 1)` to `(0, 1, 2, 3)`. It reduced the four-worker
cross-worker visit estimate to 19.6% and shared fingerprints to 12.8%, but
median selected depth fell from 15.5 to 14.0. A 64-game, colour-reversed,
material-only screen at 50 ms ended 31-30-3 (about +5 +/- 85 Elo). This is
neutral evidence, so bit 256 remains experimental and disabled by default.

A second temporary candidate rotated helper workers' root move lists after
normal ordering. Its diagnostic values improved only slightly, while the same
64-game screen ended 22-40-2 (about -100 +/- 90 Elo). The candidate was
rejected and its implementation removed.

A third temporary candidate kept worker zero's aspiration window unchanged and
scaled the three helpers to 200%, 50%, and 400%. Four-worker cross-worker
visits fell to 20.3% and shared fingerprints to 12.7%, but median depth fell to
14.5. Its 64-game screen ended 31-33-0 (about -11 +/- 87 Elo). This candidate
was also rejected and removed. Together, these screens show that reducing the
overlap estimate alone is not an adoption criterion; completed depth and paired
match results still control the decision.

## Reproduction

The ordinary shared-table lane is:

```bash
python3 scripts/nps_threads.py target/release/sekirei \
  --threads 1,2,4 --movetime 300 \
  --positions data/gate/openings_standard.sfen --count 6 --repeats 2 \
  --lazy-diagnostics --option LazyFlags=15 \
  --json target/diagnostics/lazy-overlap-shared.jsonl
```

Use `LazyFlags=143` for the isolated-TT control and `LazyFlags=271` for the
depth-ladder candidate. Counter-enabled runs add atomic work and must not be
used as ordinary NPS baselines. The small position set, short clock, and wide
match intervals do not establish an Elo gain or a general scaling result.
