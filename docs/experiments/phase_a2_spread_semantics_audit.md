# Phase A2 B1-vs-A gate: spread/diversity semantics audit (read-only)

Status: **read-only audit**, produced while the formal `results/phase_a2/b1_vs_a`
run is suspended. No code was changed, no games were run, no hash was
recomputed, no JSONL was bulk-reloaded to produce this document — every
number below was copied from an already-written artifact
(`SUSPENDED.md`, `state.json`, `progress.log`) or read directly from
`scripts/gate_phase_a2_weight_ab.py` and
`docs/experiments/phase_a2_b1_vs_a_formal_gate_preregistration.md`.

## 1. Current run snapshot — TWO distinct run_ids exist, corrected after operator clarification

**Correction:** an earlier version of this document treated
`results/phase_a2/b1_vs_a` (0 games, in this repo's main worktree) as *the*
formal run, and reported no artifact showing "367 completed pairs." That
was wrong — the actual formal launch used a separate, pinned worktree,
`../sekirei-phase-a2-run2` (a `git worktree` sibling of this repo,
detached at a merged-main commit), with `--outdir
results/phase_a2/b1_vs_a_run2`. That artifact exists and has real game
data. Both run_ids are real; they are simply different `run_id`s and must
never be merged (preregistration §1 "Resume rule" — each `run_id` is
self-contained, exactly this project's standing convention for e.g. the
burn-in vs. the suspended attempt, applied here to a second sibling too).

| | `results/phase_a2/b1_vs_a` (this repo) | `results/phase_a2/b1_vs_a_run2` (worktree `../sekirei-phase-a2-run2`) |
|---|---|---|
| Nature | Preregistration-era placeholder attempt — launched before the permutation feature (commit `310113a`) existed in this worktree's checkout, immediately resource-paused, never actually played a game | **The actual formal run.** Launched from a dedicated worktree pinned to a merged-main commit, with the permutation feature already in place |
| Games | 0 | 756 |
| Completed pairs | 0 | 378 |
| Status | `SUSPENDED` (operator-confirmed, see nuance below) | Stopped (`stop_launching: true`, `decisive_verdict: null`, no process running — see nuance below; loosely what the task brief called "PAUSED_BY_OPERATOR," though no such literal status string exists in the code on either worktree) |
| `manifest.toml` / `permutation_order.json` | Neither exists for this run_id | Both exist |

The rest of this section covers `b1_vs_a_run2` — the real formal run — in
detail. `results/phase_a2/b1_vs_a`'s own state (0 games, suspended,
no manifest) is unchanged from the previous audit and is not repeated here.

### 1.1 `b1_vs_a_run2` snapshot (as recorded / lightly derived from recorded scalars — no bulk JSONL reload, no hash recompute)

| Field | Value | Source |
|---|---|---|
| Artifact path | `../sekirei-phase-a2-run2/results/phase_a2/b1_vs_a_run2/` (a **separate git worktree**, not a subdirectory of this repo — `git worktree list --porcelain` shows `worktree /Users/k_tanabe/Documents/Documents/oss_rust/sekirei-phase-a2-run2`, `HEAD 2feadf5e15b49cb41df9e15273c3596278027390`, detached) | `git worktree list --porcelain` |
| Status | Not currently running (`ps aux` shows no `gate_phase_a2`/`sekirei-match`/`target/release/sekirei` process); `state.json`: `stop_launching: true`, `decisive_verdict: null`, `resource_paused: false` | direct process check + `state.json` |
| Total games | 756 | `combined.json`: `"games": 756`; matches `state.json`'s last `sprt_history` entry |
| Completed pairs | 378 | `state.json`: `confirmed_prefix: 378`; `combined.json`: `"completed_pairs": 378` (378 × 2 = 756 games, consistent) |
| Incomplete / pending pairs | 1322 of 1700 shards not done (1321 `"pending"` + 1 `"running"` in `state.json`, though no OS process backs that "running" entry right now — see nuance below) | `state.json` shard status counts |
| Current `spread_ok` | `false` | `state.json` last `sprt_history` entry; `combined.json` |
| Covered deciles | **3 of 10** (deciles 0, 1, 2) — *derived*, not stored as an explicit field: `confirmed_prefix=378` is a strictly contiguous done-prefix (by construction of `cmd_run`'s confirmation loop), so ranks 0-377 are exactly what's covered; `decile(rank 377) = min(9, 377*10 // 1700) = 2`. Reaching decile 3 needs rank ≥ 510 (132 more completed pairs past the current prefix). | arithmetic on `confirmed_prefix=378` and `num_positions=1700` (both already-recorded scalars) — no per-game data read |
| Current LLR | `-0.506142`, bounds `[-2.944439, 2.944439]` — within bounds, not crossed | `combined.verdict.json` |
| Last computed SPRT check | `INCONCLUSIVE` (i.e. "keep testing" as of the last confirmed prefix — **not** a final gate verdict; `state.json`'s authoritative `decisive_verdict` is `null`) | `combined.verdict.json`; `state.json` |
| 6 contamination counters | All zero: `illegal_moves=0`, `engine_errors=0` (covers `stale_bestmoves`), `weight_load_failures=0`, `protocol_errors=0`, `material_fallbacks=0`, `time_forfeits=0` — clean as of `confirmed_prefix=378` | `state.json` last `sprt_history` entry |
| `manifest.toml` | Exists, `[immutable]` section present | directory listing |
| `permutation_order.json` | Exists (`permutation_sha256 = a3ae8bb7fed8ae8e...`, seed `20260726` — same seed as the preregistration doc specifies, consistent with `b1_vs_a`'s never-generated permutation being for the *same* intended methodology) | `manifest.toml` |
| Start time | `2026-07-27 06:29:23` (first `progress.log` line: "initialized: 1700 shards...") | `progress.log` |
| Last recorded activity | `2026-07-27 09:27:26` (last `confirmed_prefix=378` log line; matches `resource_log.jsonl`'s last snapshot timestamp) | `progress.log`, `resource_log.jsonl` |
| Explicit pause/stop timestamp or log line | **None found** — see nuance below | `progress.log` (1296 lines, grepped case-insensitively for `stop\|pause\|operator\|kill\|abort\|terminat`: zero matches) |
| Resume count | No such field exists in `state.json` — not tracked by this script | `state.json` key listing |
| Manifest hashes already recorded (copied verbatim, not recomputed) | git commit (this worktree's HEAD) `2feadf5e15b49cb41df9e15273c3596278027390`; engine binary sha256 `f0f92bfef712b1acd65ff829b6c1fd20115c0ad875aaa2364cd0fc75d071e508`; match runner sha256 `7a3db8a07896b3dae24f2458f740f05722de5455d6fab18254151a8ca32f2cee`; B1 weights sha256 `019d13f284447b6afc3905dfccb7a5a570e4e3d3b08655a7f3a7b43b174a1385` (same weight file/hash as `b1_vs_a`'s manifest — expected, same candidate); A weights sha256 `a45be6099c0936283e79f34d380a4dbc7ba681796bb0bb56b2cd743c2c786ea6` (also same as `b1_vs_a`); corpus sha256 `816fdf7661989b348bf1c2e078fd6b5748ff9cfc14fa0aed3b83c6df39d56545` (also same). **Note the engine/match-runner binary hashes differ from `b1_vs_a`'s recorded `646408b6...`/`ef84b524...`** — expected, since `b1_vs_a_run2` is a separate worktree pinned to a different (merged-main) commit, i.e. a genuinely different binary build, not a discrepancy to be alarmed by. | `manifest.toml` `[immutable]` section |
| Resume command | Not separately recorded in this run_id's own artifacts (no `SUSPENDED.md`-equivalent report exists here) — reconstructable from `manifest.toml`'s `[immutable]` fields plus the standard `run` invocation shape, but not copied verbatim from an existing document the way `b1_vs_a`'s was | — |

**Nuance on "why did it stop" — reported carefully, not asserted as fact:**
`state.json` shows `stop_launching: true` with `decisive_verdict: null`.
In this script version, every code path that sets `stop_launching = True`
(`decide_verdict` returning `PASS`/`FAIL`/`CONTAMINATED`/`NOT_READY`) *also*
sets `decisive_verdict` to a non-null value in the same block — so this
combination (`stop_launching=true`, `decisive_verdict=null`) is not
produced by any of the script's own automatic stop conditions visible in
the code. Nor is there a resource-pause log line (`resource_paused: false`,
and no "PAUSING new shard launches" text anywhere in `progress.log`) — this
run never hit the load/swap threshold, unlike `b1_vs_a`. Combined with one
shard still marked `"running"` in `state.json` despite no backing OS
process, the most consistent read of the evidence is an **external stop**
(the process was terminated — and/or `state.json` was hand-edited to set
`stop_launching`) **rather than** an automatic verdict, contamination, or
resource-monitor pause. This is consistent with what the task brief called
an operator-initiated pause, but no artifact in this run_id explicitly
records the operator's action or reasoning the way `b1_vs_a/SUSPENDED.md`
does — worth writing a `SUSPENDED.md`-equivalent for `b1_vs_a_run2` before
relying on "why it stopped" for anything beyond this best-effort read.

**On the reported "367 pairs / ~734 games":** the actual recorded state is
**378 completed pairs / 756 games** — 11 pairs / 22 games ahead of the
reported figures. Since no process is running now but `progress.log`'s
last activity (`09:27:26`) is recent, the most likely explanation is that
the run continued advancing in the background after the 367-pair figure
was last observed, and stopped (however that happened) somewhat later, at
378. Not an alarming discrepancy, but worth noting precisely rather than
silently substituting one number for the other.

**Status nuance:** there is no `PAUSED_BY_OPERATOR` (or any status-enum)
string anywhere in `gate_phase_a2_weight_ab.py`. The pause is two layered
facts, not one flag:
1. **Mechanical cause** — the resource monitor's own pre-launch check
   (`should_pause_launching`) held every shard launch back from the very
   first loop iteration: first on `load1=18.3 > 10cores*1.5`, then (after
   load cleared) on steady swap ~85.7%, above the default `--max-swap-pct
   50` threshold. This was automatic, not a human clicking "pause."
2. **Operator confirmation** — `SUSPENDED.md` is a human-written report
   (this session, 2026-07-25) formalizing "we are stopping here, no
   auto-resume is configured" after confirming via `ps`/`pgrep` that no
   process was left running. It explicitly (and correctly) notes this
   wasn't a decisive verdict or a contamination stop.

So "resource-monitor-paused, then operator-confirmed-suspended" is the
accurate compound status — not a single code-level enum.

### Where the earlier "367 does not appear anywhere" claim went wrong

The earlier version of this audit searched only this repo's own working
tree (`grep -rl "367"` across `*.md`/`*.py`/`*.sh`/`*.log`/`*.json`/
`*.jsonl`, `find` scoped to `results/` and `data/runs/`) and concluded no
matching run existed. That search never looked outside this checkout —
`git worktree list` was not part of the original investigation, so the
sibling worktree `../sekirei-phase-a2-run2` (a different directory tree
entirely, one level up from this repo) was never in scope. The number
itself turned out to be close but not exact (378, not 367 — see the
nuance note in §1.1) rather than nonexistent. Lesson for future audits of
this kind: `git worktree list --porcelain` should be a standard first step
before concluding an expected run_id doesn't exist, not an afterthought.

The untracked scripts sitting in this repo's working tree
(`scripts/gate_orchestrator.py`, `scripts/extend_gate_sprt.sh`,
`scripts/tee_engine_b.sh`/`tee_engine_c.sh`, `scripts/analyze_loadtest.py`,
`scripts/analyze_confirmatory.py`, `scripts/tally_kifu_outcomes.py`) are
still correctly identified as belonging to **other, unrelated
experiments** — a B-vs-C YBW search-option gate (`results/elo_gate/`), a
`conflict_ft`-vs-`control` sprint gate (`sprint_gate_runs/20260718_...`), a
`search_ablation` confirmatory analysis, and throwaway load-test
instrumentation. None of that changes; they still don't touch either
`b1_vs_a` or `b1_vs_a_run2`.

## 2. Static audit: what `spread_ok` actually measures

### 2.1 The code path

`compute_diversity_and_counters` (`scripts/gate_phase_a2_weight_ab.py`,
around line 405-476):

```python
global_pos = global_offset + local_pos   # global_offset = shard["start_pos"]
decile = min(DIVERSITY_DECILES - 1, gp * DIVERSITY_DECILES // max(1, num_positions))
decile_hits.add(decile)
spread_ok = len(decile_hits) >= DIVERSITY_MIN_DECILES_COVERED
```

`cmd_run` (around line 610-620) builds the position list this way, in
order:

```python
raw_positions = load_positions(args.corpus)                       # file order
order, perm_meta = load_or_create_permutation(args.outdir, args.corpus, len(raw_positions))
positions = [raw_positions[i] for i in order]                     # <- permuted BEFORE shard creation
num_positions = min(len(positions), args.max_positions)
positions = positions[:num_positions]
...
shards = make_shards(num_positions, args.shard_positions)         # sequential 0..num_positions over the ALREADY-PERMUTED array
```

`make_shards` chunks `0..num_positions` sequentially
(`start_pos = 0, shard_positions, 2*shard_positions, ...`) — over the
*length* of the already-permuted `positions` array. `launch_shard` slices
`positions[shard["start_pos"]:shard["end_pos"]]`. Shard dispatch itself is
strictly sequential in `start_pos` (`cmd_run`'s launch loop always picks the
lowest-`shard_id` `"pending"` shard).

**Conclusion: `global_pos` is the position's 0-indexed rank in the fixed,
seeded permutation (`permuted_rank`), not its line index in
`openings_gateB.sfen`.** The original corpus line for a given `global_pos`
is `order[global_pos]` — a separate, one-more-indirection lookup that the
decile computation never performs (nor needs to; see below).

This matches the preregistration doc's own vocabulary exactly (§2):
"positions drawn from >= K distinct sections of **the permuted corpus**...
divide the corpus into e.g. 10 contiguous **permuted-rank** deciles" — the
doc frames deciles as sections of permuted rank space, not raw file order,
from the outset.

**Framing note:** nothing in §2 below should be read as "the code has a
bug." The permuted-rank decile check matches the preregistration doc's own
specification exactly (§2.3 quotes the doc verbatim: "permuted-rank
deciles" is the doc's own term, not an inference). What §2.2-2.4 describe
is a possible **tension between two design goals** — "stop launching
shards as soon as a defensible sample has been seen" (favored by a
progress-coupled check, which forces a real minimum amount of the
permutation to be worked through) vs. "directly verify the *specific*
positions played were not clustered" (which a progress-coupled check only
guarantees indirectly, via the permutation having done its job) — not an
implementation defect to be fixed. See `phase_a2_spread_amendment_draft.md`
for why resolving that tension by simply swapping to a direct measurement
is not obviously an improvement either.

### 2.2 Why this is a *progress* proxy, not an independent diversity re-measurement

Because dispatch is strictly sequential over `start_pos` (permuted rank),
and `spread_ok` is computed only from **completed** shards
(`compute_diversity_and_counters` is called with `all_confirmed =
[shard_by_id[i] for i in range(cp)]`, i.e. the confirmed *prefix*), the set
of covered deciles at any moment is mechanically determined by how far the
confirmed prefix has advanced through the fixed permutation — not by any
fresh sampling or re-measurement of which corpus content was actually
drawn. In other words: **at any given moment, `spread_ok` answers "has the
run progressed far enough into its fixed permuted order," which is a
different question from "is the completed sample itself representative."**
The permutation is what makes progressing through rank-space *also* mean
scattering across the original file (see 2.4) — but mechanically, the
check itself is a progress gate, dressed as a diversity gate.

The preregistration doc is aware of this and treats it as intentional: §1
"Why not file order" explains the permutation exists specifically so that
sequential dispatch (which the script's shard-launch logic was never
rewritten to avoid) still produces a spread sample rather than a contiguous
prefix. The permutation is the fix; the decile check on top of it is a
correctness proxy that the fix actually worked, not an independent
diversity measurement.

### 2.3 Relation to shard launch order

Identical. `shard["start_pos"]` *is* permuted rank, by construction (§2.1),
and shards launch in increasing `start_pos` order (the `cmd_run` dispatch
loop, and `make_shards`'s sequential construction) — so "shard launch
order" and "permuted-rank order" are the same sequence, not two things that
happen to correlate.

### 2.4 Minimum condition for 7/10 deciles to be covered

Constants (`scripts/gate_phase_a2_weight_ab.py` lines 53-55):
`DIVERSITY_DECILES = 10`, `DIVERSITY_MIN_DECILES_COVERED = 7`. For the
currently-configured corpus, `num_positions = 1700` (min of 1700 canonical
openings and `--max-positions 1707`, which doesn't bite — the 1707 vs 1700
distinction is resolved in the preregistration doc's own §"Resolving 1700
vs. 1707").

Deciles are 0-indexed (`decile = min(9, gp*10 // 1700)`), so the cheapest 7
to touch are deciles **0 through 6**. Decile 6 requires
`gp*10 // 1700 == 6`, i.e. `gp >= 6*1700/10 = 1020`.

- **Theoretical floor**: a single completed pair at permuted rank
  `gp >= 1020` (assuming deciles 0-5 are already each covered by at least
  one earlier-completed pair, which strictly-sequential dispatch makes
  likely but not certain — completion can lag dispatch under parallelism
  and retries) is sufficient. `1020 / 1700 = 60%`.
- **Practical/robust threshold**: because `spread_ok` counts *completed*
  pairs, not merely *dispatched* shards, and completion trails dispatch
  under `--parallel 3` (concurrent shards) plus the retry loop (failed
  shards re-enter `"pending"`, up to 3 attempts), the confirmed prefix
  advancing into decile 6 in practice requires the run to have worked
  through something closer to **~70%** of the permuted corpus, not exactly
  60% — the extra margin absorbs in-flight/retrying shards that haven't
  yet folded into `confirmed_prefix`. This is the source of the "false
  until ~70% completion" framing in the task brief: it is a description of
  typical behavior under this run's concurrency settings, not a constant
  in the code (the code's actual constant is the 60% floor derived above).

### 2.5 Would raw corpus-content diversity look different, if checked directly?

Not verified by execution (out of scope for this audit — no replay was
run), but worth recording as a design note for §3: under a uniformly
random permutation (Fisher-Yates + xorshift64, seed `20260726`), the
*original corpus index* of the pairs completed by the time decile 6 of
*permuted rank* is reached (~60-70% of the corpus) is, by construction,
already scattered roughly uniformly across the *original* file too — that
scattering is exactly what the permutation buys. The permuted-rank decile
check is therefore a **necessary, not sufficient**, proxy: passing it
implies broad original-corpus coverage (given the permutation actually ran
correctly), but the check itself never re-derives or re-confirms that
directly from `order[global_pos]`. See the amendment draft for why
computing original-index deciles directly, instead, is not a strict
improvement.

## 3. Corroborating text (verbatim, for traceability)

`docs/experiments/phase_a2_b1_vs_a_formal_gate_preregistration.md`:

- §1 "Why not file order" (lines 38-49): "`data/gate/openings_gateB.sfen`
  has no documented internal ordering guarantee... Launching shards
  sequentially from position 0 means an early SPRT stop always samples a
  contiguous prefix, never a spread. A fixed-seed permutation, generated
  once and hashed into the manifest, fixes this without needing to change
  `gate_phase_a2_weight_ab.py`'s shard-launch order logic."
- §"Global game index rule" (lines 265-287): "`shard["start_pos"]`/
  `shard["end_pos"]` continue to mean 'this shard covers permuted-ranks
  [start_pos, end_pos)'."
- §2 (lines 341-350): "positions drawn from >= K distinct sections of the
  permuted corpus (proxy for 'not clustered': divide the corpus into e.g.
  10 contiguous permuted-rank deciles, require representation from at
  least, say, 7 of them)."
- §3 (lines 398-403): "SPRT boundary reached BUT completed_pairs < 300 →
  continue launching shards (drawing further into the permuted corpus)
  until completed_pairs >= 300 or the corpus is exhausted... do not
  finalize a verdict (PASS or FAIL) on an under-diverse sample."

`docs/design/gate_report_template.md` §5 defers to the same document for
the definition, adding no independent semantics.

## 4. A resume-path defect worth flagging (found while auditing, not requested, but load-bearing for §1's manifest fields)

**This section is about `b1_vs_a` (the 0-games placeholder in this repo),
not `b1_vs_a_run2`** — `b1_vs_a_run2` already has both `manifest.toml` and
`permutation_order.json` (§1.1), so the defect below does not apply to it.

`results/phase_a2/b1_vs_a/state.json` predates the permutation feature
(commit `310113a`) — `state["cfg"]` has no `permutation_seed` /
`ordered_output_sha256` keys. `cmd_run`'s resume branch (`else:` at line
707) only raises on a permutation mismatch when `recorded is not None`
(line 709) — for this run, `recorded` is `None`, so the check is **silently
skipped**, not violated. Separately, `write_manifest_immutable` — the
function that writes `manifest.toml` and is the source of the
binary/weight-hash immutability check on every subsequent resume (lines
715-738) — only runs inside the `if state is None:` (fresh-init) branch. A
naive resume of this exact `--outdir` as-is would therefore:
1. Silently adopt the current script's permutation feature (deterministic,
   seed `20260726` — safe, since 0 shards have been played so far under
   any position mapping, this is not a "prior data invalidated" problem);
   but
2. **Never gain a `manifest.toml`** — the binary/weight-hash immutability
   safety net stays permanently absent for this specific `run_id`, on this
   and every future resume of the same `--outdir`, since the manifest-write
   only fires on cold-init.

Not a blocker for resuming (nothing has run yet, so nothing is at risk of
being retroactively invalidated), but worth a manual manifest write (or a
one-line script fix) before treating a resumed run's eventual verdict as
carrying the same immutability guarantee the preregistration doc describes
for every other `run_id`.
