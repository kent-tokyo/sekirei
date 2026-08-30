# PR #5 post-merge static audit: speculative-search pool isolation

Status: static, read-only. No builds, no runtime measurement — every claim
below cites the specific source lines it's derived from
(`sekirei-spec-fix`@`8e6a145`, tree-identical to `main`@`0bb4221`:
`crates/sekirei-core/src/speculative.rs`, `crates/sekirei-core/src/search.rs`,
`crates/sekirei-usi/src/main.rs`).

## Thread-pool lifecycle

- One `SpeculativeSearcher` (and its one dedicated `spec_pool:
  Arc<rayon::ThreadPool>`) is created **once per engine process lifetime**,
  at USI startup (`main.rs:57`, `let mut searcher = make_searcher(hash_mb)`).
  It is **not** recreated per game, per move, or per `go` — confirmed by
  reading every call site of `make_searcher` (`main.rs:57`, `main.rs:163`);
  the only other call site is the `Hash` setoption handler.
- `usinewgame` (`main.rs:207-217`) and `go` (`main.rs:254-262`) both
  explicitly abort the in-flight search's shared `external_abort` flag and
  `join()` its driving OS thread before mutating shared state (`board`,
  `searcher.clear_tt()`, starting a new search). The `Hash` setoption handler
  (`main.rs:159-163`) does **not** — see Finding 2 below.
- `Arc<ThreadPool>` drop: the dedicated pool is only ever dropped when
  `searcher` is reassigned (Hash change) or the process exits — not after
  every search — so no per-search pool churn exists to begin with.

## Resource sizing — Finding 1 (the standout result of this pass)

**Real per-engine-process concurrent compute-thread demand is
`Threads + top_n`, not `Threads` alone, and `top_n` is currently a fixed,
un-configurable constant (`3`).**

- `SpeculativeSearcher::new(tt, top_n)` sizes the dedicated pool as
  `num_threads(top_n.max(1))` (`search.rs:1262-1265`).
- `make_searcher` hardcodes `SpeculativeSearcher::new(Tt::new(hash_mb), 3)`
  (`usi/main.rs:492`) — no USI option exposes `top_n`; every engine process
  always carries exactly 3 dedicated spec-pool worker threads, independent of
  the `Threads` option.
- `Threads` (`main.rs:164-171`) sizes a **separate** pool: rayon's *global*
  pool, via `rayon::ThreadPoolBuilder::new().num_threads(n).build_global()`.
  This is the pool `alpha_beta`'s own YBW dispatch
  (`work.into_par_iter()...collect()`) runs on.
- If `Threads` is never explicitly set (option default is `0` = "unset", per
  the comment at `main.rs:81`), the global pool is never sized via
  `build_global()` and lazily initializes to rayon's own default —
  `num_cpus` — on first use. On this project's 10-core dev machine, an
  engine process that never receives a `Threads` setoption would silently run
  a 10-worker global pool **plus** the 3 fixed spec-pool workers = 13 OS
  threads capable of concurrent CPU work, not the 1–2 an operator might
  assume from an unset/default option.
- Even with `Threads` explicitly set (as `pr4_regate_match/MATCH_CONFIG.md`
  does, `Threads=2`), true peak concurrent compute-thread demand per engine
  process is `2 (global pool) + 3 (spec pool, fixed) = 5`, not `2`. For a
  match-runner shard running 2 engine processes at once, that's `10` real
  compute threads from *one* shard alone.
  This `Threads + top_n` figure is deliberately just the CPU-*budget* number
  (threads that can be doing real, concurrently-schedulable search work). It
  excludes two other threads every engine process also has: the
  `std::thread::spawn`-launched driver thread that runs
  `SpeculativeSearcher::search`/`root_search` itself (mostly blocked on the
  YBW `LockLatch` while dispatching, not itself computing —
  `FINDINGS_INTERIM.md`'s "dedicated OS thread, NOT a rayon pool worker"),
  and the per-search watchdog thread (`search.rs:1318-1324`,
  `std::thread::sleep(lim)` then `abort_now()` — asleep for the entire
  search, not consuming CPU). Both exist and count toward total *thread*
  count (relevant to OS scheduler overhead, `ulimit -u`, memory), but neither
  competes for a CPU core the way the `Threads + top_n` pool workers do — so
  a resource-preflight calculator budgeting *CPU* capacity should use
  `Threads + top_n`, while one budgeting raw OS thread/handle count should
  use `Threads + top_n + 2`.
- **Operational relevance**: this formula (`Threads + top_n` per process,
  `top_n` fixed at 3 in the current binary) directly explains part of the
  load-average spike (45–65 on a 10-core machine) observed in
  `pr4_gate_attempt_index.md`'s Attempt 1 (6 shards × 2 engines ×
  effectively-5-threads-each = up to 60 compute threads), independent of and
  additive to the unrelated `renkin-crowdout-diag` job and concurrent Claude
  Code sessions also present at the time. This was previously
  uncharacterized — prior planning (`--parallel N --threads 2`) implicitly
  assumed `Threads` alone determined footprint.
- **Minimal fix (not implemented in this pass, design-only)**: either expose
  `top_n` as a USI option so it can be dialed down for constrained hosts, or
  — with no code change at all — have any future gate/match preflight
  calculator budget `Threads + 3` per engine process instead of `Threads`.
  The latter requires no engine change and is what
  `docs/design/gate_redesign_low_load.md` §5C adopts.
- **Test needed** (not written in this pass): a fixture asserting the live
  thread count of a running `SpeculativeSearcher` search matches
  `configured_Threads + top_n`, so this formula doesn't silently drift if
  `top_n` or the pool-construction code changes later.
- Status: **NEEDS_RUNTIME_VALIDATION** for the *exact* live thread count
  under real load (this pass didn't run `ps -M`/`sample` to confirm); the
  *sizing formula itself* is a direct, static fact from the constructor code,
  not an inference.

## Resource sizing — other checklist items

- `top_n.max(1)` when `top_n=0`: only affects the dedicated pool's thread
  *count* (always ≥ 1 thread exists even if speculation is fully disabled).
  `policy::top_n(board, tt, 0)` returns zero candidates, so `SpecGroup::spawn`
  produces an empty `tasks` Vec — the idle thread never executes real work.
  Wastes one idle OS thread's memory/scheduling overhead when `top_n=0`; not
  a functional bug. Currently unreachable in the production binary anyway
  (hardcoded `top_n=3`).
- Multiple engine processes in parallel (match-runner `--parallel`): each
  process independently builds its own dedicated pool sized by the same fixed
  formula — no cross-process coordination exists or is expected (each is a
  separate OS process). The multiplication (`shards × engines_per_shard ×
  (Threads + 3)`) is exactly what Finding 1 above quantifies.

## Cancellation and accounting

- **Abort propagation to spec pool**: confirmed correct.
  `spec_alpha_beta` (`speculative.rs:179-197`) checks
  `task_abort.load(Relaxed) || state.budget.should_abort()` at function entry
  and again before every recursive call (`speculative.rs:239-242,
  248-251`), and explicitly treats a post-recursion abort as "the returned 0
  is meaningless — bail without using it," matching the file's own
  documented invariant (`speculative.rs:7-9`).
- **`SpecGroup::drop` / superseded-task termination**: `Drop` sets every
  *non-promoted* task's abort flag (`speculative.rs:165-171`); `promote()`
  removes the winner from that list so it's deliberately left running,
  uncancelled, until it naturally finishes or the shared `budget` deadline
  fires. This is the documented, intended mechanism — post-fix, it's safe
  specifically because the pool it runs on can no longer starve `alpha_beta`'s
  own YBW dispatch (that was the entire point of PR #5).
- **Does `search()` wait for live tasks before returning?** No, and this
  appears to be intentional rather than an oversight: `SpeculativeSearcher::
  search` (`search.rs:1292` onward) never explicitly joins spec-pool tasks;
  it returns as soon as its own iterative-deepening loop ends. A promoted
  task from the final depth iteration can keep running briefly after
  `bestmove` is printed. Termination is still bounded: the next `go`'s
  abort+join sequence (`main.rs:254-262`) sets the *same* `external_abort`
  flag (`abort_flag()` returns a clone of `self.external_abort`,
  `search.rs:1275-1277`, reused — not recreated — across `search()` calls),
  which every live `spec_alpha_beta` frame checks at its next recursion
  point. `search_handle.join()` only waits for the OS thread that ran
  `search()`'s driver loop, **not** for spec-pool tasks to physically stop —
  so there is a small window where old-search spec-pool threads and the
  newly-started search can be concurrently live. Given per-node abort checks,
  this window is bounded by "one unit of work" (roughly one node's move
  generation + eval + TT store) — informative, not a correctness bug.
- **Winner/discarded/physical-node definitions**: consistent with
  `FINDINGS_INTERIM.md`'s usage — "promoted" = kept alive across a depth
  iteration boundary via `promote()`; "discarded" = non-promoted, aborted on
  `SpecGroup` drop; "physical nodes" is used informally in that doc for
  nodes actually visited (as opposed to nodes a stalled search claims to be
  "in" without visiting) and is not a defined field in this codebase —
  flagged only so a future reader doesn't go looking for a `physical_nodes`
  field that doesn't exist.
- **TT sharing vs. pool separation**: no contradiction. `SpecState.tt` is
  the *same* `Arc<Tt>` as the main search's TT (`search.rs:1306-1307`,
  `tt: self.tt.clone()`) — PR #5 changed *scheduling* (which pool runs a
  task), not *data sharing* (which TT a task reads/writes). Every TT write
  site reachable from `speculative.rs` is guarded by the abort check
  immediately before it (lines 118, 257-267, 279). No unguarded write found.

## Finding 2 (minor) — `setoption Hash` skips the codebase's own defensive pattern

- `main.rs:159-163` reassigns `searcher` (rebuilding the TT and the dedicated
  pool) without the abort+join sequence that both `usinewgame` and `go`
  apply before mutating shared state.
- **Repro**: send `go` with a long byoyomi, then before `bestmove`, send
  `setoption name Hash value N`.
- **Impact**: not a demonstrated correctness bug — the in-flight search
  thread holds its own `Arc<SpeculativeSearcher>` clone and keeps running
  against its own (old) TT/pool until it finishes or the shared abort fires;
  the board position is unaffected, so its eventual `bestmove` is still
  legal. The main visible effect is an inconsistency with the codebase's own
  established pattern, and a transient extra idle pool (new, from
  `make_searcher`) alongside the still-draining old one.
- **Minimal fix**: mirror the `usinewgame`/`go` abort+join sequence in the
  `Hash` branch before calling `make_searcher`.
- **Test needed**: an integration test sending `go` then `setoption Hash`
  mid-search, asserting exactly one `bestmove` is emitted and no panic
  occurs.
- Status: **NEEDS_RUNTIME_VALIDATION** — real USI GUIs conventionally don't
  send `setoption` mid-search, so this is a defensive-robustness gap rather
  than an observed live failure. Low priority.

## Summary

| Finding | Severity | Confidence | Action |
|---|---|---|---|
| 1: real thread demand = `Threads + top_n(=3)`, uncharacterized in prior gate planning | Significant for resource planning; not a functional bug | High (direct from constructor code) | Feed into `gate_redesign_low_load.md` §5C preflight formula; consider exposing `top_n` as a USI option in a future PR |
| 2: `setoption Hash` doesn't abort+join like `go`/`usinewgame` | Low | High (direct from code) | Track as a small follow-up fix + test, not urgent |
| Cancellation/accounting/TT-sharing | No issues found | — | — |
