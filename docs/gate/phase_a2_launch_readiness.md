# Phase A2 B1-vs-A gate: launch-readiness engineering

Self-contained reference for what this PR implements in
`scripts/gate_phase_a2_weight_ab.py` and
`crates/sekirei-match-runner/{main.rs,engine.rs}`. Written so it doesn't
depend on any other not-yet-merged document — every rule below is either
implemented in this PR's own code or independently derivable from it.

**No formal Phase A2 game has been played by this work. This PR does not
launch the gate, does not lift any prior suspension, and does not create a
formal run directory.** It only implements and verifies the prerequisites
for a future, separately-decided launch.

## 1. Deterministic permutation

The opening corpus is drawn in a fixed-seed permuted order rather than
sequential file order, so an early SPRT stop still samples from across the
whole corpus instead of concentrating on whichever positions sit at the
front of the file.

- Algorithm: Fisher-Yates (back-to-front), driven by an xorshift64 PRNG
  (`state ^= state<<13; state ^= state>>7; state ^= state<<17`, `state|1`
  initial seed, plain `%` modulo — see `deterministic_permutation` in
  `gate_phase_a2_weight_ab.py`).
- Seed: `20260726`, confirmed and fixed.
- Generated once per run, persisted to `<outdir>/permutation_order.json`,
  hashed into the manifest's `permutation_sha256`. Resuming a run reloads
  this file rather than regenerating it, and verifies the hash still
  matches before proceeding.

## 2. Minimum-diversity gate

A bare SPRT LLR boundary crossing does not by itself finalize a verdict.
In addition:

```text
completed_pairs >= 300     (a pair = both color-orientations of one
                             corpus position both finished)
AND positions drawn from >= 7 of 10 deciles of the permuted corpus
```

Both conditions apply symmetrically to a PASS-direction and a FAIL-direction
boundary crossing — an early stop in either direction still needs this
much diversity before finalizing.

## 3. Six operational counters and the stop rule

| Counter | How it's observed |
|---|---|
| `illegal_moves` | `EndReason::IllegalMove` prints a `" (illegal)"` tag on the per-game summary line, captured in the shard's stdout log |
| `engine_errors` (manifest field: `stale_bestmoves`) | `EndReason::EngineError` prints `" (engine error)"` the same way; this binary has no separate stale-bestmove variant, so the two are one counter |
| `time_forfeits` | `EndReason::TimeForfeit`, prints `" (time forfeit)"`. Distinguished from a dead/crashed engine process by `engine.rs`'s `map_recv_result`, which separates a genuine `go()`-deadline timeout (engine alive, too slow) from the reader thread ending because the process disconnected (a real fault, stays `EngineError`) |
| `weight_load_failures` | Detected in real time by polling each shard's stderr for `"NNUE weights loaded"`/`"weight load failed"`; a detected failure kills and retries the shard |
| `protocol_errors` | Always 0 for data that reaches a confirmed shard — a fatal protocol error exits the whole match-runner process before any shard output is written, so it's caught upstream as a shard failure/retry rather than a counter on confirmed data |
| `material_fallbacks` | Always 0 by construction — a weight-load failure aborts the engine process; there is no fallback-to-material-evaluation code path to trigger |

Stop rule (`decide_verdict`):

```text
any counter unobservable for this run           -> NOT_READY
any observed counter nonzero                    -> CONTAMINATED (halt, quarantine)
SPRT boundary crossed AND completed_pairs >= 300
    AND >= 7/10 deciles covered                 -> PASS or FAIL (whichever boundary)
SPRT boundary crossed but diversity not yet met  -> keep launching shards
corpus exhausted without a qualifying boundary   -> INCONCLUSIVE
```

`NOT_READY` takes priority over every other outcome, and `CONTAMINATED`
takes priority over a clean-looking SPRT boundary — a dirty or unobservable
counter always wins over what the win/loss numbers alone would suggest.

On `CONTAMINATED`, the run directory is renamed with a `_contaminated`
suffix rather than deleted or left in place — the completed shards may
still be useful evidence for root-causing the contamination, but the
directory can never again be resumed as if it were clean; a fresh run is
required.

## 4. Resume safety

Resuming an existing `--outdir` re-verifies, before doing anything else:

- the persisted permutation order's hash still matches what the manifest
  recorded;
- the engine binary, match-runner binary, and both weight files still hash
  to what the manifest's `[immutable]` section recorded.

Either mismatch raises `SystemExit` rather than silently continuing.
Resuming an already-finished run is idempotent — it detects nothing left
to do and exits without reprocessing games, re-deciding the verdict, or
appending a duplicate manifest snapshot.

## 5. A note on an earlier exploratory run's numbers

An earlier exploratory burn-in run (uncounted toward any formal result,
used only to shake out protocol/process issues before this diversity-gate
design existed) produced two different game counts in its own logs: its
SPRT check first crossed a decisive boundary at 162 games, but 2 shards
that were already in flight at that instant were allowed to finish rather
than being killed mid-game — bringing the run's final total to 166 games.
Both numbers describe the same run at two different timestamps, not
inconsistent data; no game was lost, duplicated, or recomputed between
them. That run used only 100 of the corpus's positions in sequential
order and predates the minimum-diversity gate described above — it does
not, and was never treated as, a formal gate result.

## 6. Scope of this PR

This PR includes exactly one prerequisite from outside its own new work:
`fix(match-runner): harden USI game-boundary protocol against stale
bestmove leaks` — the `TimeForfeit`/retirement logic this PR adds extends
a retirement-on-fault mechanism that commit already introduces, and
doesn't exist on `main` without it. No other unrelated commit is bundled
in. This PR is not a merge of any other unmerged work beyond that one
prerequisite.
