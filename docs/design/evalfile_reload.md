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
- **A real, pre-existing bug this design should reference**: `sekirei-bench/src/bin/search_ablation.rs:577` calls `nnue::load_weights()` in-process, once per arm being compared. Because `WEIGHTS` is one process-global `OnceLock`, comparing two arms with *different* weight files in a single `search_ablation` run today silently keeps whichever weights loaded first — the "ignoring duplicate load" log line is the only symptom. This is the concrete motivating case for "independence between engine instances."
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
Today's design is one process-global `OnceLock` — fine for the normal case (one USI process = one engine instance), but it's exactly why `search_ablation.rs` can't compare two weight files in one process today (see §1). Recommend introducing an `Engine` struct:
```
Engine
├─ Arc<NnueWeights>      // swappable; replaces the OnceLock global
├─ Evaluator             // thin wrapper resolving weights from the Engine, not a bare global
├─ Searcher
└─ local Rayon ThreadPool
```
so that constructing two `Engine`s in one process (as `search_ablation` already effectively wants to do) gives each its own independent weights, with no process-global state at all. `sekirei-usi`/`sekirei-csa` each still construct exactly one `Engine` per process — no behavior change for the normal CLI path.

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
- Two `Engine` instances constructed in one process (the `search_ablation` in-process-multi-arm case) hold independent `Arc<NnueWeights>` — loading a second, different file into instance B does not affect instance A's evaluation output.
