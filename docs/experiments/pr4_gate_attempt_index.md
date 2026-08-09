# PR #4 re-gate: 3 attempt index (read-only classification)

Status: static, read-only. Artifacts were only inspected (`ls`/`cat`/`diff`/`grep`),
never modified or deleted. No aggregation across attempts was performed and
none should be — see "Prohibited" at the bottom.

Common config for all 3 attempts (`pr4_regate_match/MATCH_CONFIG.md`):

- Base: `main` HEAD `0bb4221` (== `fix/spec-pool-isolation`@`8e6a145`, PR #5, no SE fix), binary sha256 `3ac3d456b2548c74aca524c5010a4a65b3440a2ce7bf58f0f9a093f8ce557b3b`
- Candidate: `feat/next-strength-candidate`@`9b61ed4` (PR #4 rebased onto `main`), binary sha256 `334623ae971c9795a81cee96a05004477855f9a80ac75a30870833458a267c04`
- Weights: `weights_v011_opening_combined.bin`, sha256 `a45be6099c0936283e79f34d380a4dbc7ba681796bb0bb56b2cd743c2c786ea6` (both sides, identical)
- Opening slice: `positions_300.sfen` = `openings_gateB.sfen` lines 151–450, sha256 `db9cc5e45bdf0b96c282ebec9de155f6e4f48e3831ff36829650b3cbfdd1e3c5` (non-overlapping with `depth_fix_match` [1–100] and `se_on_fix_match` [101–150])
- Engine options: `Threads=2` (attempts 1–2), `Threads=1` (attempt 3 only)
- Byoyomi: 10000ms (attempts 1–2), 5000ms (attempt 3 only)
- Requested scope: cover-all, 6 shards × 50 positions = 300 pairs / 600 games (attempts 1–2); 4 games only (attempt 3, deliberately minimal)

## Attempt 1 — 6-shard parallel

| Field | Value |
|---|---|
| Artifact path | `sekirei-abtest-results/pr4_regate_match/run1_contaminated_load_spike/` |
| UTC start–end | ~2026-08-09T13:52Z – 13:58Z (file mtimes 22:52:57–22:58:11 JST) |
| Requested | 300 pairs / 600 games, 6 parallel shards, Threads=2 each (12 engine processes + 6 match-runner processes) |
| Completed | 38/600 games total (`aa`=7, `ab`=6, `ac`=8, `ad`=7, `ae`=6, `af`=4) |
| TimeForfeit count | 28/38 games (74%) — `aa`=6/7, `ab`=4/6, `ac`=6/8, `ad`=4/7, `ae`=5/6, `af`=3/4 |
| End reason | All 6 background shard processes killed by the harness |
| Load/swap at time | Load average 45–65 on a 10-core machine; `renkin-crowdout-diag` (unrelated job, ~7/10 cores) + several other concurrent Claude Code sessions already running |
| Contamination | Yes — severe. 74% of completed games are TimeForfeit, i.e. CPU-starvation outcomes, not engine-strength outcomes |
| Classification | **CONTAMINATED** (existing verdict vocabulary, `docs/design/gate_manifest_schema.md`) / environment-invalid |
| Reusable | No — not as W/D/L signal. `NOTE.md` in this directory documents the same conclusion; kept as an artifact of the incident, not deleted |

## Attempt 2 — 1-shard sequential retry

| Field | Value |
|---|---|
| Artifact path | `sekirei-abtest-results/pr4_regate_match/shard_aa.log` + `shard_aa_out/` (top level — **not yet archived** into a subdirectory at the time of this audit; left in place, not moved, to avoid modifying artifacts during a read-only pass) |
| UTC start–end | ~2026-08-09T14:05Z – 14:18Z (file mtimes 23:05–23:18 JST, from `game0001.txt`–`game0003.txt`) |
| Requested | Same 50-position shard `aa` (100 games), run alone (no other shards in parallel) to reduce peak footprint |
| Completed | 3/100 games |
| TimeForfeit count | 2/3 games (game 1 and game 3; game 2 completed normally, Engine1 win, 48 moves) |
| End reason | Background process killed by the harness after game 3; engine process was also being transparently retired/relaunched mid-run (`[match] retiring e1/e2 ... after game N (TimeForfeit); launching a fresh process`) — itself a symptom of the same contention, not a separate bug |
| Load/swap at time | Same sustained incident as attempt 1 — swap ~85% of 5GB used throughout, unrelated job + concurrent sessions still active |
| Contamination | Yes — 2/3 games are TimeForfeit even with parallelism reduced to a single shard |
| Classification | **CONTAMINATED** |
| Reusable | No |

## Attempt 3 — 4-game Threads=1 foreground micro-batch

| Field | Value |
|---|---|
| Artifact path | `/tmp/mini_test.sfen` (2 SFEN lines, scratch only) — no result file exists (`mini_test_result.json` was never written) |
| UTC start–end | Started ~2026-08-09T14:29Z; killed at the harness's own 2-minute foreground timeout (exit 143) before any game completed |
| Requested | 4 games, Threads=1, byoyomi=5000ms — deliberately minimal to test whether *any* footprint could complete under current load |
| Completed | 0/4 games (no output was produced before the timeout) |
| End reason | Bash tool's default 120s foreground timeout, not an internal script decision |
| Load/swap at time | Same sustained incident, unchanged |
| Contamination | N/A — no data was produced to contaminate |
| Classification | **CONTAMINATED** (by absence of any usable data under the same root cause) / no data |
| Reusable | No — nothing to reuse. Informative only in that even a minimal footprint could not complete, reinforcing that the bottleneck is host-wide, not match-size-dependent |

## Why these 3 attempts must not be summed

- Attempt 1's 38 games and attempt 2's 3 games do not share a common,
  uncontaminated sampling process — both are drawn from the same
  CPU-starvation regime where the *outcome variable itself* (TimeForfeit vs.
  a real game result) is a function of host load, not engine strength. Adding
  41 TimeForfeit-dominated games together does not shrink the confidence
  interval on a real signal; it just produces a larger number built on the
  same non-signal.
- Games that did complete "normally" inside these attempts (e.g. attempt 2's
  game 2, a genuine Engine1 win) cannot be cherry-picked out and pooled either
  — the selection itself is confounded by load (a game finishes "normally"
  partly as a function of which side happened to get enough CPU time in that
  window), so even the non-forfeit subset is not an unbiased sample.
- No partial pair (an incomplete color-reversed pair, i.e. only one of the two
  colors played) is counted as a full pair anywhere in this index or in
  `search_lineage_after_pr5.md`.

## Status

All 3 attempts: **CONTAMINATED / INVALID_ENVIRONMENT**. Zero valid pairs
collected toward the 300-pair target. The re-gate remains outstanding; see
task #15 and `docs/experiments/gate_redesign_low_load.md` (§5B, once written)
for the retry plan and updated resource-preflight gating meant to catch this
condition *before* launch next time instead of after 3 failed tries.
