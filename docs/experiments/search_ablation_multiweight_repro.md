# `search_ablation` multi-weight mixup: minimal repro and blast-radius assessment

Status: **investigation only — no fix applied**. Per the standing instruction,
this P0 (`tasks/todo.md`) is fixed together with the EvalFile-reload
implementation, not today. This document records the repro, the corrected
understanding of where the bug actually lives, and what past results it does
(and does not) affect.

## 1. The bug, confirmed by code trace

`crates/sekirei-core/src/nnue.rs:198-206`:

```rust
pub fn load_weights(path: &Path) -> io::Result<()> {
    let w = read_weights(path)?;
    if WEIGHTS.set(w).is_ok() {
        NNUE_ACTIVE.store(true, Ordering::Relaxed);
    } else {
        eprintln!("[nnue] weights already loaded; ignoring duplicate load");
    }
    Ok(())
}
```

`WEIGHTS` is a process-global `static OnceLock<NnueWeights>` (`nnue.rs:167`).
`OnceLock::set` only ever succeeds once per process. A second `load_weights(path_b)`
call: reads and parses `path_b` from disk (real I/O, real CPU), discards the
result, logs one line to stderr, and returns `Ok(())` — the caller has no
way to detect from the return value alone that the load was a no-op.
`weights()` (`nnue.rs:180-184`) goes on serving the *first* file's values for
the rest of the process's life.

This is deterministic and requires no live run to establish — it's a direct
reading of four lines with no branching on external state. A minimal repro
demonstrating it is written at
`crates/sekirei-core/examples/repro_multiweight_onelock.rs` (writes two
trivially-distinct weight files to a temp dir, loads both in one process,
reports whether the second took effect). **Execution deferred**: swap was at
~90% and another project's job was pinning a CPU core at the time this was
written (see the session's resource notes); running even this cheap example
requires a `cargo run` compile. The repro's outcome is not in doubt — the code
path has no conditional logic that could make the second load succeed — but
it has not been empirically executed this session. Run it once load clears:
`cargo run -p sekirei-core --example repro_multiweight_onelock`.

## 2. Corrected understanding: where this does *not* apply

`docs/design/evalfile_reload.md` (committed earlier this session, `63dba1b`)
originally characterized `search_ablation.rs` as calling `load_weights()`
"in-process, once per arm" — that claim was checked against the actual source
during this task and found to be **wrong**. It has been corrected in that
document (see its §1). The actual facts:

- `search_ablation.rs`'s `load_weights_and_fingerprint` (`search_ablation.rs:569-585`)
  is called **exactly once**, in `main()` (`search_ablation.rs:887`), driven by
  a single `--weights` CLI flag. Every arm (A–E: seq-AB / seq-PVS / PVS+YBW /
  PVS+YBW+spec / PVS+spec), every profile (production/controlled), every
  thread count dispatched within one process invocation shares that one
  loaded weight file.
- The tool's own module doc states this as a deliberate design guarantee
  (`search_ablation.rs:24-29`): *"Every process invocation loads NNUE weights
  exactly once (or falls back to the engine's built-in default), so 'same
  NNUE weights' holds automatically across every arm/profile/thread-count
  measured by that invocation."*
- There is no `--weights`-per-arm flag, no per-arm weight map, nothing in
  `Cli`/`Arm`/`main()` that could trigger a second `load_weights` call in a
  single `search_ablation` run.

**Conclusion: no past PVS/YBW/speculation ablation result is contaminated by
this bug.** `search_ablation` never attempted to load two different weight
files in one process, so the `OnceLock` collision never had an opportunity to
fire in any historical run of that tool. This is a correction to this
session's own earlier (mistaken) framing, not a new finding about the
ablation data itself — the ablation reports already on record stand as they
were.

### The Phase A2 B1-vs-A gate: also unaffected, for a different reason

`scripts/gate_phase_a2_weight_ab.py` compares two *different* weight files
(B1 candidate vs. A/v011 baseline) — but it does so across **separate OS
processes**, not in one process. Each shard spawns a fresh `sekirei-match`
subprocess (`gate_phase_a2_weight_ab.py:155`, `subprocess.Popen`), which in
turn spawns two fresh `sekirei` engine child processes (`--engine1`/`--engine2`,
`--args1`/`--args2` carrying the two weight paths). Separate processes mean
separate address spaces, so each side gets its own independent `OnceLock` —
no collision possible. This was true of the gate's one (suspended, 0-game)
attempt and remains true of any future attempt run the same way.

## 3. Where the bug *does* matter

- **`sekirei-train --eval-only`** already had to route around it:
  `nnue.rs:213-220`'s doc comment on `read_weights` (the side-effect-free
  parse-without-activating variant) explains it exists specifically because
  a caller that wants to score a checkpoint's weights without redirecting the
  process's one global `WEIGHTS` needs this path instead of `load_weights`.
  This is existing, working evidence that the project already knew about the
  hazard and has one precedented workaround for it.
- **The EvalFile-reload design itself** (`docs/design/evalfile_reload.md`)
  is the actual place this bug is the motivating concern: its whole point is
  letting one *running* USI process load a *second* weight file later in its
  life (a runtime reload) — which is exactly the "second `load_weights` call
  in one process" shape this bug breaks today. That design doc's §2–§4
  already proposes the fix (an `Engine` struct holding a swappable
  `Arc<NnueWeights>`, no process-global state) and its §5 test plan already
  includes "two `Engine` instances in one process hold independent weights."
  No new design work needed here; this document exists to correct the
  motivating example and confirm the historical blast radius, not to design
  the fix a second time.
- Any **future** in-process multi-engine tool (e.g., a hypothetical unified
  in-process ablation-plus-strength comparator) would need to either spawn
  separate processes (today's working pattern) or wait for the `Engine`
  refactor — using bare `load_weights` twin in one process for two
  "instances" is the one path guaranteed to silently do the wrong thing.

## 4. Summary for the completion report

| Question | Answer |
|---|---|
| Is the `OnceLock` bug real? | Yes — confirmed by direct code trace, `nnue.rs:198-206`. |
| Does a second `load_weights()` call in-process take effect? | No — `OnceLock::set` fails silently after the first success; only an stderr log line signals it. |
| Did `search_ablation` ever load two weight files in one process? | No — confirmed from source (`search_ablation.rs:569-585`, `887`); one `--weights` flag, no per-arm mechanism. Earlier design-doc claim to the contrary was wrong and has been corrected. |
| Are past PVS/YBW/speculation ablation results affected? | No. |
| Is the Phase A2 B1-vs-A gate affected? | No — separate OS processes per side, separate `OnceLock`s. |
| Where does the bug actually matter? | Future in-process multi-instance code, principally the EvalFile-reload implementation itself. |
| Fixed today? | No — P0 deferred to the EvalFile-reload implementation session, per standing instruction. |
