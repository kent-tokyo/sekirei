# Design: `EvalFile` hot-reload

Status: **design only — not implemented**. No behavior of the USI binary changes as a result of this document. Written 2026-07-25 as light, CPU/memory-free work while heavier processes (training, gates, benchmarks) are paused on this machine.

## 1. Current behavior (as of `HEAD`, confirmed by reading source)

- **Global weight storage**: `static WEIGHTS: OnceLock<NnueWeights>` (`crates/sekirei-core/src/nnue.rs:167`), read through `weights() -> &'static NnueWeights` (`nnue.rs:180-184`). A second `OnceLock<NnueWeights>` (`DEFAULT_WEIGHTS`, LCG fallback) exists purely so an early `weights()` call — e.g. `Board::startpos()` at process start, before `isready`/`setoption EvalFile` run — can't accidentally pin `WEIGHTS` to garbage (`OnceLock::set` only ever succeeds once).
- **`load_weights(path)`** (`nnue.rs:198-206`): parses the file, then calls `WEIGHTS.set(w)`. On a *second* call it just logs `"weights already loaded; ignoring duplicate load"` (`nnue.rs:203`) and returns `Ok(())` — **this is today's entire "reload" behavior: a silent no-op**.
- **`setoption name EvalFile value <path>`** (`crates/sekirei-usi/src/main.rs:248-254`): only stores the path string into a local `eval_file: Option<String>`. It never calls `load_weights` itself.
- **Loading actually happens at `isready`** (`main.rs:163-188`), and only `if !weights_active()` (`main.rs:165`) — i.e. **once weights are active for this process (including via the CLI-arg load path at `main.rs:41-57`), every later `EvalFile` + `isready` is already a no-op today**, independent of anything this design changes.
- **Failure handling** (`92c7ce4`, both load sites: `main.rs:42-53` at startup, `main.rs:167-186` at `isready`): print `info string FATAL: weight load failed...` and `std::process::exit(2)`. Deliberate — the comment at `main.rs:45-51` explains why (a failed load would otherwise leave `weights_active()` false and silently degrade to material eval with no external signal). Locked by `crates/sekirei-usi/tests/evalfile_load_failure_aborts.rs`.
- **Format header**: 8-byte magic, `SEKIRW01` current / `JANOSW03` legacy (identical binary layout) accepted, `read_weights` (`nnue.rs:221-298`) rejects anything else or the wrong byte length before touching any global state. Architecture (`INPUT`/`L1`/`L2`) is compile-time `const`, not stored per-file.
- **`Threads` runtime-reconfig precedent** (`b8bd40e`, `main.rs:217-239`): builds a fresh `rayon::ThreadPool`, stores it as `Option<Arc<ThreadPool>>` (`main.rs:118`), and the *old* `Arc` is simply dropped once every in-flight clone (captured by value in the currently-running search's `pool.install(...)` closure) finishes — no lock, no abort signal, because the change only needs to affect the *next* spawned search. This does **not** fully transfer to weights: weights are read from deep inside a *running* search's per-node accumulator (`nnue.rs:469`, `478`, called on every make/unmake), not just once at spawn time.
- **`Board`** (`board.rs:51-66`) derives `Clone` and owns its own `acc: NnueAcc` (`board.rs:65`); `NnueAcc::new()`/`refresh_acc()` (`board.rs:282-292`) both resolve `weights()` at call time, not at some earlier point — a *freshly constructed or refreshed* `Board` already picks up whatever `weights()` currently returns.
- **Correction (2026-07-25, re-verified against source after this doc's first draft)**: an earlier version of this section claimed `search_ablation.rs` calls `nnue::load_weights()` once *per arm*. That is wrong. `load_weights_and_fingerprint` (`sekirei-bench/src/bin/search_ablation.rs:569-585`) is called exactly **once**, in `main()` (`search_ablation.rs:887`), from a single `--weights` CLI flag — every arm (A–E), profile, and thread count within one process invocation shares that one loaded weight file, by explicit design (the module doc at `search_ablation.rs:24-29` states this directly: "Every process invocation loads NNUE weights exactly once ... so 'same NNUE weights' holds automatically across every arm/profile/thread-count measured by that invocation"). **No past PVS/YBW/speculation ablation result is contaminated by the `OnceLock` behavior below** — search_ablation never attempted to load two different weight files in one process, so the bug never had an opportunity to fire there.
- **The real bug, and where it actually would bite**: `WEIGHTS` is one process-global `OnceLock` (`nnue.rs:167`, `load_weights` at `nnue.rs:198-206`) — a *second* `load_weights(path)` call in the same process silently keeps whichever weights loaded first (the "already loaded; ignoring duplicate load" log line, `nnue.rs:203`, is the only symptom; the second call's parsed weights are discarded). This is a live foot-gun for any *future* in-process multi-engine-instance code — most concretely, the `Engine` struct this very design doc proposes in §2, and any tool built to compare two weight files without spawning separate OS processes. It is not, historically, a `search_ablation` bug: that tool side-steps it entirely by loading once per process and relying on separate process invocations (or the OS shell) to vary the weight file across comparisons. The gate script `scripts/gate_phase_a2_weight_ab.py` sidesteps it the same way at a different layer — each shard's `sekirei-match` subprocess spawns two fresh `sekirei` engine child processes (`subprocess.Popen`, `gate_phase_a2_weight_ab.py:155`), each with its own address space and thus its own independent `OnceLock`.
- No `sha2`, `arc-swap`, or `parking_lot` dependency exists anywhere in the workspace today (checked every crate's `Cargo.toml` + `Cargo.lock`) — std-only.

## 2. Target behavior

### `setoption name EvalFile` / `isready`
Keep `isready` as the point where loading (initial or reload) must complete before `readyok` — no change to that contract. The gate changes from `!weights_active()` to "no reload currently pendingApply and no search in flight" (see below), so a *second* `EvalFile` + `isready` in the same process actually takes effect instead of silently no-opping.

### Reload vs. in-flight search
Recommend: accept the new path immediately into a pending slot on `setoption EvalFile`. Do **not** swap the active weights while a search is running (mirrors the `Threads` precedent's "changes affect only the next search" semantics, which needed no locking there for the same reason). Apply the swap at the next `isready` (parse + validate the file there, so a bad path/format is reported before the next `go`, not discovered mid-search) — i.e. validation happens eagerly at `isready`, activation is deferred only if a search is currently in flight, applied at that search's completion.

### Accumulator rebuild
No per-node accumulator surgery needed **if** weights become an `Arc<NnueWeights>` resolved once at `Board` construction/`refresh()` time rather than baked into long-lived incremental deltas against a bare `&'static`. Since `position` reconstructs `Board` fresh each time (confirmed: `main.rs:91` initializes one `Board`, and the `position` command handler replaces it), swapping the active `Arc` before the next `position`+`go` is sufficient. The one spot needing an explicit `refresh_acc()` call today is the *live* `board` local when a reload completes at `isready` with no intervening `position` — already precedented at `main.rs:172`.

### Format/version validation
Unchanged: magic check + strict byte-length check, both before any global state changes. A reload that fails validation must reject cleanly (see below), never partially apply.

### SHA-256 display
Requires adding a `sha2` crate dependency (none exists today) to `sekirei-core` or `sekirei-usi`. On successful load/reload, print `info string EvalFile sha256=<hex> path=<path>` — this closes the loop with the existing offline `scripts/verify_weights_registry.py`, letting a human cross-check a live engine's loaded weights against `docs/weights_registry.toml` without stopping the process.

### Reload failure handling — **deliberate departure from startup policy**
Startup load failure aborts the process (`92c7ce4`) — correct for startup, wrong for a runtime reload: killing a process mid-tournament on a bad reload path is strictly worse than rejecting the reload. Recommend: reload failure logs `info string error: EvalFile reload failed (<reason>); keeping previously active weights`, keeps serving the old `Arc<NnueWeights>` unchanged, and `isready` still answers `readyok` normally (this engine instance remains fully functional, just not updated). This is a new, explicit policy this design doc introduces — call it out as such in review, since it's the one place behavior for the *existing* code path (startup) and the *new* code path (reload) deliberately diverge.

### Material fallback
Stays categorically disallowed, unchanged from `92c7ce4`'s existing policy. A rejected reload is never a fallback-to-material path — only "reject and keep the old (already-validated, already-active) weights."

### Multiple `Engine` instances' independence
Today's design is one process-global `OnceLock` — fine for the normal case (one USI process = one engine instance), but it means no code path in this workspace can construct two independently-weighted engine instances in a single process today (see §1's correction — `search_ablation` and the Phase A2 gate script both currently avoid this by construction, not by relying on the `OnceLock`). Recommend introducing an `Engine` struct:
```
Engine
├─ Arc<NnueWeights>      // swappable; replaces the OnceLock global
├─ Evaluator             // thin wrapper resolving weights from the Engine, not a bare global
├─ Searcher
└─ local Rayon ThreadPool
```
so that constructing two `Engine`s in one process gives each its own independent weights, with no process-global state at all. `sekirei-usi`/`sekirei-csa` each still construct exactly one `Engine` per process — no behavior change for the normal CLI path.

### Back-compat
Keep accepting legacy `JANOSW03` under reload too — same magic check, no special-casing needed.

## 3. API changes required (none applied today)

1. New `Engine` struct (above) replacing the bare `OnceLock<NnueWeights>` + `NNUE_ACTIVE` statics.
2. `weights()`'s `&'static NnueWeights` return type replaced by an explicit `&NnueWeights`/`Arc<NnueWeights>` parameter threaded from `Engine` down through `Searcher`/`NnueAcc` construction — this also happens to be the fix for the hot-loop concern noted in §1 (a lock/atomic-load belongs at "once per search," not inside `add_col`/`sub_col`).
3. `load_weights(path)` replaced by `Engine::reload_weights(path) -> Result<Arc<NnueWeights>, ReloadError>`, side-effect-free until the caller decides to swap (mirrors today's `read_weights`/`load_weights` split, just generalized to support "validate now, apply later").
4. New `sha2` dependency + a small hashing helper.
5. A "reload pending, search in flight" guard (a simple `bool`/`Option` alongside the existing `search_abort`/`search_handle` locals in `main.rs`, not a new synchronization primitive).

## 4. Migration steps

1. Wrap today's `OnceLock` behind an `Engine`-shaped API with **no behavior change** (mechanical refactor, single engine instance, same global-like semantics).
2. Migrate internal call sites (`nnue.rs`'s `add_col`/`sub_col`/`evaluate`/`NnueAcc::new`/`refresh`) from the bare global to an explicit reference passed down from `Engine`.
3. Make the storage swappable (`Arc<NnueWeights>` behind a simple `Mutex`/single-writer-single-reader cell — no need for `arc-swap` given reload is rare and already synchronized at `isready`/search boundaries).
4. Wire `setoption EvalFile` to the new pending-then-apply-at-isready path instead of one-shot `load_weights`.
5. Add sha256 display + the reload-failure-keeps-old-weights behavior.
6. Tests (below).

## 5. Test plan

Reuse the existing test file shapes as direct precedents (`evalfile_load_failure_aborts.rs`, `threads_reconfigurable.rs`, `usi_thread_race.rs`):
- Reload while idle (no search in flight) takes effect on the very next `position`/`go`.
- Reload requested while a search is running is deferred; the in-flight search completes uninterrupted using the old weights; the new weights are active for the *next* search only.
- Reload with bad magic or wrong byte length is rejected at `isready` with an `info string error` line; `readyok` still follows; old weights remain active; process does **not** exit.
- Legacy `JANOSW03` still loads successfully under reload, identically to today's startup path.
- The sha256 printed at load time matches an independent `sha256sum`/`scripts/verify_weights_registry.py` recompute of the same file.
- Two `Engine` instances constructed in one process hold independent `Arc<NnueWeights>` — loading a second, different file into instance B does not affect instance A's evaluation output. (See `docs/experiments/search_ablation_multiweight_repro.md` for a minimal, code-traced demonstration of the current `OnceLock` behavior this test plan supersedes.)

## 6. Commit-split plan (added 2026-07-26 — design only, no code written)

§4's six migration steps, broken into individually-landable commits. Each
row states what the commit changes, confirms the codebase compiles and
passes existing tests in that exact intermediate state (no "half-migrated"
commit), and its real dependencies — reordered from a naive 1-8 reading
where the dependency analysis below found a better order.

| # | Commit | What changes | Compiles/tests pass at this point? | Depends on |
|---|---|---|---|---|
| 1 | Loader API returns format metadata | Extend today's side-effect-free `read_weights` (or add a thin wrapper) to also return/expose magic string and computed dimensions, without changing its parsing or error behavior | Yes — purely additive function, no existing call site touched | none |
| 2a | `Engine` struct wraps today's storage, **no behavior change** | Introduce the `Engine` struct (§2's sketch); internally it still holds a single-init cell equivalent to today's `OnceLock` — mechanical wrap, not yet swappable | Yes — behavior is bit-for-bit identical to today, this is a pure refactor | #1 (uses its metadata-returning loader internally) |
| 2b | Make `Engine`'s storage actually swappable (`Arc<NnueWeights>` + a simple `Mutex`/single-writer cell) | The storage itself becomes replaceable; nothing yet *triggers* a replacement (no caller does it) | Yes — dead capability until #4 wires a caller to it; existing single-load-at-startup path is unaffected | #2a |
| 3 | Migrate internal call sites (`add_col`/`sub_col`/`evaluate`/`NnueAcc::new`/`refresh`) from the bare `weights()` global to an explicit reference threaded from `Engine` | Every read of the global becomes a read of an explicit parameter/field instead | Yes — mechanical, one call-site category at a time; `sekirei-usi`'s one `Engine` instance behaves identically | #2b |
| 4 | Multi-instance independence test | Add the permanent regression test: two `Engine` instances in one process, load different weights into each, assert independence | Yes, and **this is the point where the P0 `OnceLock` bug is actually fixed and locked in** — recommend landing this test right after #3, not at the end (see note below) | #3 |
| 5 | Wire `setoption EvalFile` to `Engine::reload_weights` (validate-now, apply-later) instead of one-shot `load_weights` | `setoption EvalFile` starts actually doing something (storing a *pending* validated reload) rather than just recording a path string | Yes — validation happens, but nothing applies yet without #6 | #2b, #3 |
| 6 | Apply pending reload at `isready` (deferred while a search is in flight) | The reload takes effect for real, following §2's "Reload vs. in-flight search" rule | Yes — this is the first commit where a *second* `EvalFile`+`isready` in one process actually changes behavior | #5 |
| 7 | Accumulator rebuild confirmation + sha256 display | Confirm/add a regression test that a fresh `Board`/`refresh_acc()` after a swap picks up the new weights (§2's "Accumulator rebuild" already argues this needs no new code, just verification); add the `sha2` dependency and `info string EvalFile sha256=...` line | Yes | #6 |
| 8 | `search_ablation` weight-isolation option (optional, not required by the P0 fix) | Give `search_ablation` a way to construct two independent `Engine`s with different weights in-process, *if* a future comparison ever wants that (today it doesn't — see `search_ablation_multiweight_repro.md` §2) | Yes — purely additive, `sekirei-bench`-local change | #3 (needs `Engine` to exist; does not need #5–#7's USI-specific plumbing) |

**Reordering note vs. a naive 1-8 reading**: the independence test (listed
7th in the original example numbering) is moved to **immediately after
`Engine` becomes real and injected (#3)** rather than left until the end.
The property it verifies — two `Engine`s, two independent weight sets — is
fully expressible as soon as `Engine` exists and is actually used for
reads, which is `#3`, not after the USI reload command is wired up (`#5`-`#6`).
Landing it early means the actual P0 fix (independence) is locked in by a
real test at the earliest possible point, rather than being implicitly
assumed correct while three more USI-plumbing commits land on top of it.
Commit #8 (`search_ablation` isolation) similarly doesn't need to wait for
the USI-specific commits (#5-#7) — it only needs `Engine` to exist (#3) —
so it can land in parallel with #5-#7 rather than strictly after them.
