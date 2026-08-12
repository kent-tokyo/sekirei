# PR #4 re-gate redesign (design-only — not executed in this pass)

Status: design document. Nothing in this doc has been run. No engine match,
build, or benchmark was executed to produce it — every claim about what the
engine/CLI currently supports is derived from reading
`crates/sekirei-usi/src/main.rs`, `crates/sekirei-core/src/search.rs`, and
`crates/sekirei-core/src/budget.rs`.

Context: 3 consecutive attempts at a 300-pair wall-clock re-gate of PR #4
were all killed by host resource contention before producing usable data
(`docs/experiments/pr4_gate_attempt_index.md`). This redesign adds a cheap,
load-insensitive correctness pre-filter (§5A) ahead of the wall-clock gate
(§5B), and a preflight/continuation check (§5C) meant to catch contention
*before* launch instead of after 3 failed tries.

## 5A. Fixed-depth comparison (correctness pre-filter, not a strength verdict)

### A note on "fixed-node" vs. "fixed-depth"

The task framing asked for a fixed-*node* comparison. Checked against the
actual USI implementation: `parse_go` (`usi/main.rs:495-613`) recognizes
`btime`/`wtime`/`byoyomi`/`binc`/`winc`/`movestogo`/`movetime`/`depth`/
`infinite` — there is **no `nodes` token**, and `SearchConfig`
(`search.rs:254-263`) has no `max_nodes` field. `Budget` (`budget.rs`) tracks
a live node *count* (`nodes()`) but has no node-count-based abort — only a
time-based one (checked every 4096 nodes via `tick()`). A true node-budgeted
`go nodes N` does not exist in this codebase today, and adding it would be a
code change, out of scope for this design-only pass.

**Substitute: `go depth N` already does the job.** When `depth` is given with
no clock tokens, `parse_go` returns `(None, None)` for the time limit
(`usi/main.rs:566-567`, `depth.is_some() && !has_clock` branch) — the search
runs iterative deepening to exactly depth `N` with **no time cap at all**,
making it just as host-load-insensitive as a node budget would be (the
search takes however long it takes, but always reaches the same depth
regardless of how slow or fast the host is that day). This is the standard
technique engines use for load-insensitive dev testing (fixed-depth
"bench"-style comparison), and needs no new code.

**Caveat**: node count *at* a fixed depth can still differ between base and
candidate (that's expected — it's exactly one of the things being measured,
not something being controlled away). And because there's no engine-side
time cap under `go depth N`, an external wrapper-level timeout is required
per position (see "safety wrapper" below) — a pathological position could in
principle run very long at a fixed depth with nothing internal to stop it.

### Design

| Field | Value |
|---|---|
| Base SHA | `0bb4221` (`main`, = PR #5, no SE fix) |
| Candidate SHA | `9b61ed4` (`feat/next-strength-candidate`, PR #4 rebased) |
| Weights | `weights_v011_opening_combined.bin`, sha256 `a45be609...` (same both sides) |
| Position corpus | A **fresh, non-overlapping** slice of `data/gate/openings_gateB.sfen` — lines 451–550 (100 positions; 1–450 already claimed by `depth_fix_match`/`se_on_fix_match`/`pr4_regate_match` per `search_lineage_after_pr5.md` §5) |
| Position count | 50 (subset of the 100-line slice above; smaller than a strength gate needs, since this is a correctness/cost pre-filter, not an Elo measurement) |
| Depth budget | `go depth 10` — deep enough to exercise SE (`SE_MIN_DEPTH=8`) and let speculation run a few iterations; shallow enough to keep worst-case per-position time bounded on a contended host |
| Threads | `Threads=1` — deliberately, to remove YBW's own rayon work-stealing as a source of run-to-run nondeterminism (a fixed-depth comparison is more useful if repeatable). Real per-process compute-thread demand is still `1 + 3 (spec pool, fixed) = 4`, per `pr5_pool_isolation_static_audit.md` Finding 1, not 1 — the spec pool cannot be disabled without a code change (`top_n=3` is hardcoded, see that doc). |
| Speculation | Fixed at `top_n=3` (hardcoded in `usi/main.rs:492`, no USI option) — cannot be varied without a code change; note this as a limitation rather than a configurable axis |
| Seed | N/A — no RNG in the search path found; the only run-to-run nondeterminism risk is scheduling-order-dependent (rayon work-stealing at `Threads>1`, or spec-pool/main-search TT write races) — mitigated, not eliminated, by `Threads=1` above |
| Safety wrapper | Each single-engine, single-position `go depth 10` invocation must run under an external hard timeout (e.g. `timeout 120s` per position) — the engine itself imposes none under a depth-only `go` |

### Metrics — split by what's already instrumented vs. what needs new counters

Already available without any code change, via `SpecSearchInfo`
(`search.rs:1221-1244`) / existing USI `info` output / match-runner's
existing per-game `EndReason` counters (`docs/design/gate_manifest_schema.md`):

- Legal-bestmove rate — via match-runner's existing `IllegalMove` counter
- `score`, final `depth` reached, `elapsed`, `hashfull`
- `nodes` (aggregate main-search node count — **excludes** spec-pool nodes,
  see below)
- `spec_hits` / `spec_total` (existing speculation-hit-rate proxy)
- `bestmove_changes` (existing PV-stability counter — directly usable for
  "PV安定性")
- PV list (`pv_list`) for tactical/mate-solve spot checks against known
  positions, if a small hand-verified tactical suite is added to the corpus

**Would require new instrumentation (not added in this pass, flagged as
out-of-scope for a CPU-light, no-build design pass):**

- "Discarded speculative nodes" / "total physical nodes" separate from
  `nodes` — confirmed via reading `speculative.rs` that `spec_alpha_beta`
  never calls anything equivalent to `budget.tick()`/increments any node
  counter at all (matches `FINDINGS_INTERIM.md`'s own note: "`spec_alpha_beta`
  never ticks the shared node counter"). Spec-tree nodes are currently
  **entirely uncounted**, anywhere. Adding this is a small, well-scoped
  instrumentation change, not a design gap — just not free today.
- "SE eligibility" / "actual extensions" counts — a similar counter existed
  once as uncommitted, local-only telemetry on the `audit/speculative-depth-
  stall` branch (per `FINDINGS_INTERIM.md`, "will be `git checkout --`
  reverted before any fix commit") and was in fact reverted; it is **not**
  present in the current `main`/PR #4 tree. Would need to be re-added.

### What this pre-filter is for for and isn't

Per the task framing: this is meant to screen for correctness regressions
and extreme search-cost blowups *before* spending a load-sensitive wall-clock
budget on a strength gate — not to produce an Elo verdict. A useful
pass/fail read from this pass alone: zero illegal bestmoves, no PV
instability outliers, no >Nx node-count blowup at the fixed depth between
base and candidate, no timeouts under the safety wrapper.

## 5B. Time-controlled paired match (execute only once §5C passes)

| Field | Value | Notes |
|---|---|---|
| Base / candidate SHA | `0bb4221` / `9b61ed4` | unchanged from the 3 failed attempts |
| Binary hashes | must be re-verified fresh at run time, not assumed from `pr4_regate_match/MATCH_CONFIG.md` (that build is now ~1 day stale relative to whenever this actually re-runs) | |
| Weights / opening hash | `weights_v011_opening_combined.bin` (`a45be609...`); reuse `pr4_regate_match/positions_300.sfen` (lines 151–450, sha256 `db9cc5e4...`) — still valid and unused (0 valid pairs collected against it across all 3 attempts) | |
| `Threads` | **1**, revised down from the prior attempts' `2` | See sizing below |
| `--parallel` | **1** (sequential shards only) | Attempt 2 already showed that shrinking to 1 shard at `Threads=2` was *not* sufficient on its own — the host was still saturated by an unrelated job regardless of our own footprint. The fix for that is §5C's preflight gate, not further shrinking; `Threads=1`/`--parallel 1` here is a genuinely smaller footprint than what was already tried and failed, offered as an additional margin, not the primary fix. |
| Byoyomi | 10000ms, unchanged (comparable with `depth_fix_match`/`se_on_fix_match` precedent) | |
| Real thread demand | Per engine process: `Threads(1) + top_n(3) = 4` (CPU-competing) `+ 2` (driver + watchdog, not CPU-competing) — per `pr5_pool_isolation_static_audit.md`. Per shard (2 engines): `8` CPU-competing threads. At `--parallel 1`: `8` total — comfortably under 10 physical cores *if* §5C confirms nothing else is running. | |
| Contamination counters | Rolling `TimeForfeit` rate per shard, checked at the completion of each game (not polled at fixed short intervals — computed from already-flowing shard output, no extra process spawned to check) | |
| Minimum completed pairs | 240/300 (80%) with **< 5%** TimeForfeit rate, or the run is `CONTAMINATED` regardless of how many pairs nominally "completed" | Mirrors the classification vocabulary in `pr4_gate_attempt_index.md` / `docs/design/gate_manifest_schema.md` |
| Stopping rule | Fixed N = 300 pairs, report Elo/95% CI/LOS via the existing `aggregate.py` — no SPRT early-stopping. A full SPRT + diversity-gate machinery already exists for the *formal* B1-vs-A gate (`scripts/gate_phase_a2_weight_ab.py`, `docs/experiments/phase_a2_b1_vs_a_formal_gate_preregistration.md`) — reusing that machinery for this PR-validation-scale engineering gate would be a heavier redesign than this task warrants; noted as a possible future unification, not adopted here. | |
| Artifact path / run ID | `sekirei-abtest-results/pr4_regate_match/run2_<UTC-start-timestamp>/` — deliberately distinct from `run1_contaminated_load_spike/` (attempt 1) and the still-loose top-level `shard_aa.log`/`shard_aa_out` (attempt 2, per `pr4_gate_attempt_index.md`). Before starting `run2`, archive attempt 2's loose files into `attempt2_contaminated_sequential/` for consistency (a small, safe file-move, not a deletion) — deferred to whoever actually launches the retry, since this pass is read-only. | |
| Continuation-abort rule | See §5C | |

## 5C. Resource preflight — launch-refusal vs. continuation-abort (design + dry-run-only calculator)

Design principle from the task: detect contention **before** launch, not
after 3 failed tries. Two separate rule sets, as requested.

### Launch-refusal conditions (checked once, before any shard starts)

| Check | Command | Threshold |
|---|---|---|
| 1-minute load average | `uptime` | Refuse if `>= physical_cores - 2` (i.e. `>= 8` on this 10-core machine) |
| Swap usage | `sysctl vm.swapusage` | Refuse if used/total `> 30%` (well below the ~85% seen during the incident — real margin, not a threshold tuned to the exact failure already observed) |
| Free physical memory | `vm_stat` | Refuse if free `< 2GB` |
| Disk free | (existing standing criterion, reused as-is) | Refuse if `< 10GB` |
| Named contention jobs | `pgrep -fl renkin` (or any other project-specific heavy-job name known to run on this host) | Refuse if any match found — this exact job caused 2 of the 3 incidents |
| Concurrent Claude Code sessions | `pgrep -c -x claude` (approx.) | Warn at `> 1`, refuse at `> 2` — the incident had 5 |
| Predicted total CPU-competing threads | `parallel_shards × 2 × (Threads + 3)` (formula from `pr5_pool_isolation_static_audit.md` Finding 1) | Refuse if `> physical_cores - 2` |

A **single** failed check is enough to refuse launch — this list is
deliberately conservative (multiple independent tripwires) rather than a
single composite score, so any one dimension of contention blocks the run
without needing the others to agree.

### Continuation-abort conditions (checked periodically while shards run)

Checked from data already flowing (shard stdout, already being written) plus
one lightweight system check per interval — **not** a tight poll loop; check
interval should be minutes, not seconds, consistent with the standing
anti-high-frequency-polling guidance.

| Check | Trigger |
|---|---|
| Rolling TimeForfeit rate (this run) | `> 15%` of games completed so far in a shard → abort that shard, mark `CONTAMINATED` |
| Swap usage | `> 70%` at any periodic check → abort all running shards |
| Load average | Above the launch-refusal threshold for **2 consecutive checks** (avoids a single noisy sample causing a false abort) → abort |
| A new named contention job appears | Same job-name check as launch-refusal, re-run each interval → abort |

### Dry-run-only calculator

A small, read-only script implementing exactly the launch-refusal table
above (system inspection only — `uptime`/`sysctl`/`vm_stat`/`ps`/`pgrep`; no
dependency on `sekirei-match` or any engine binary, and **no code path that
can launch a match**) is provided at `scripts/gate_resource_preflight.py`.
It prints a PASS/REFUSE verdict with per-check detail and exits non-zero on
refuse, for scripting. It does not implement the continuation-abort loop
(that belongs inside whatever actually drives the match run, since it needs
access to live shard output) — this script covers the launch-refusal half
only, safely, in a form that can be run right now without violating the
current CPU-light constraint (its own resource cost is a handful of `ps`/
`sysctl` calls, not a build or a match).

## 5D. Swap-percentage inversion under macOS dynamic swap-file resizing (found 2026-08-12, design-only)

Status: design-only, like §5C — nothing in this section has been coded yet.
Found while resuming the B-vs-C YBW gate (`results/elo_gate/t2`,
`ROADMAP.md` §1.5), not during this design pass specifically, but it directly
invalidates a numeric assumption both §5C's launch-refusal/continuation-abort
tables and `gate_orchestrator.py`'s own separate, simpler resource monitor
(`DEFAULT_MAX_SWAP_PCT = 50.0`, unrelated code path from
`gate_resource_preflight.py`, duplicating a weaker version of the same idea —
worth unifying eventually, not attempted here) both make: that swap
used/total (%) moves *with* contention.

### The empirical finding

Two `sysctl vm.swapusage` snapshots taken ~11 minutes apart on this same
host, while independently confirmed idle (swapins +16 pages over a 20s
window, swapouts flat, load average stable) via the 5-signal check in
[[sekirei_resource_resume_criteria]] / this repo's standing pre-build
checklist:

| Time | `total` | `used` | used/total |
|---|---|---|---|
| 06:02 | 8192.0 MB | 6483.56 MB | 79.1% |
| 06:13 | 7168.0 MB | 6378.56 MB | 89.0% |

`used` barely moved (-105 MB, noise-level). `total` dropped 1024 MB. The
*percentage* rose 10 points **while the machine got quieter, not busier** —
macOS shrinks the dynamic swap file itself once paging pressure eases, and
does so independently of how much is still resident in it. A fixed
used/total threshold is measuring the wrong thing here: it can trip (or
stay tripped) precisely when contention is easing, and — the more
important direction for a launch-refusal gate — it can also silently
loosen under *rising* pressure if `total` grows faster than `used` during
a spike, though that direction wasn't directly observed today.

### Why this matters for both existing designs

- §5C's launch-refusal table (30%) and continuation-abort table (70%) are
  both used/total-based (`parse_swap_used_fraction`,
  `scripts/gate_resource_preflight.py:126-136`) — same failure mode.
- `gate_orchestrator.py`'s independent `should_pause_launching` (50%
  default) hit exactly this today: raised to 85% mid-session as a
  workaround, still false-tripped once `total` shrank further. No fixed
  percentage is safe against a moving denominator — this isn't a threshold-
  tuning problem, it's a metric-choice problem.

### Revised signal design

Load average and free physical memory (both already in §5C's table)
don't have this problem — `uptime`'s load average is contention-derived
directly, not swap-file-size-derived, and `vm_stat`'s free-page count is an
absolute quantity, not a ratio against a moving total. Only the swap
signal needs to change:

| Signal | Current (§5C) | Revised | Why |
|---|---|---|---|
| Load average (1min) | `>= physical_cores - 2` refuses | **unchanged** | Already absolute-denominator (core count doesn't move), not affected by this finding |
| Free physical memory | `< 2GB` refuses | **unchanged** | Already an absolute quantity |
| Swap | used/total `> 30%` (launch) / `> 70%` (continuation) | **absolute swap `used` (MB), not a fraction** — e.g. refuse if `used` climbs `> N` MB above a *session-start baseline* reading, rather than any fixed absolute number (this host's steady-state idle `used` has itself ranged ~6.1-6.5 GB across sessions per today's and prior logs — a fixed absolute cutoff would need the same re-tuning problem as a fixed percentage unless it's baseline-relative) | `used` was empirically flat (±105 MB) while genuinely idle today; a moving-total ratio isn't. A delta-from-baseline avoids hard-coding today's ~6.4GB idle level as if it were universal. |

Concretely: `collect_swap_usage()`/`parse_swap_used_fraction()` in
`scripts/gate_resource_preflight.py` would gain a sibling
`parse_swap_used_mb()` (trivial — the regex already captures `used` before
dividing by `total`; the fraction division is the only part to drop), and
the launch-refusal/continuation-abort tables' swap rows would key off
`used_mb - baseline_used_mb` (baseline captured once at process/session
start) instead of `used / total`. `gate_orchestrator.py`'s own
`should_pause_launching`/`resource_snapshot` would need the equivalent
change, or — simpler — could stop duplicating this logic and shell out to
`gate_resource_preflight.py`'s (revised) checks instead; not decided here,
flagged as the unification noted above.

### Interim workaround (recommended, not yet applied)

Per the user's own explicit choice when this was first hit mid-session
2026-08-12, the recommended immediate stopgap for `gate_orchestrator.py` is
`--max-swap-pct 100` (swap gate effectively disabled, `--max-load-mult`
left as the sole brake) — but the T2 run that hit this actually reached
its decisive verdict on its own (swap happened to drop back under the
already-raised 85% threshold before anyone acted on the 100% choice), so
this flag has not actually been used in a real run yet. It's a pragmatic
stopgap either way, not an implementation of the design above. This
section is the from-scratch redesign that stopgap stands in for;
implementing it (the `parse_swap_used_mb`/baseline-delta change above) is
a small, scoped code change, not attempted in this pass per the "no CPU,
design only" framing this section was written under.

## Exact commands for the next retry (once §5C passes and is re-run manually)

```bash
# 5C preflight (safe to run anytime, including right now)
python3 scripts/gate_resource_preflight.py --parallel 1 --threads 1

# 5A fixed-depth pre-filter (50 positions, ~1-2 min/position worst case
# under the external timeout wrapper — still run only after 5C passes,
# since it's real CPU work, just bounded and load-insensitive in its result)
# (exact invocation TBD at execution time — needs per-position `go depth 10`
# driven directly over USI, not via sekirei-match, since sekirei-match has
# no depth-only mode; a small wrapper script would be needed and is not
# written in this pass)

# 5B time-controlled gate (only after 5A shows no correctness red flags)
./target/release/sekirei-match \
  --engine1 <base sekirei binary> --engine2 <candidate sekirei binary> \
  --args1 data/weights_v011_opening_combined.bin \
  --args2 data/weights_v011_opening_combined.bin \
  --engine-option1 Threads=1 --engine-option2 Threads=1 \
  --byoyomi 10000 \
  --positions sekirei-abtest-results/pr4_regate_match/positions_300.sfen \
  --games-per-position 2 \
  --output sekirei-abtest-results/pr4_regate_match/run2_<timestamp>/result.json \
  --json
```
