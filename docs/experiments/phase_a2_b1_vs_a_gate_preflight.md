# B1 vs A formal gate: preflight (checklist only — gate not launched)

Status: **preflight document only**. No burn-in, no Elo/SPRT games, no
training, no benchmark, no push, no version bump. This document assembles
and verifies what a future launch of `scripts/gate_phase_a2_weight_ab.py`
needs, reusing the previous (suspended, 0-game) attempt's manifest where it
still holds and flagging, explicitly, everywhere it no longer does.

## 1. Weight hashes (verified this session, zero-compute I/O only)

| Role | File | sha256 |
|---|---|---|
| Candidate (B1, `--weights1`) | `data/runs/phaseA2_20260724/checkpoints_b1/weights_b1_seed42.bin` | `019d13f284447b6afc3905dfccb7a5a570e4e3d3b08655a7f3a7b43b174a1385` |
| Baseline (A / v011, `--weights2`) | `data/weights_v011_opening_combined.bin` | `a45be6099c0936283e79f34d380a4dbc7ba681796bb0bb56b2cd743c2c786ea6` |

Both re-hashed independently this session (`shasum -a 256`) and both match
`results/phase_a2/b1_vs_a/SUSPENDED.md`'s recorded values exactly — the
weight files have not changed since the suspended attempt. See
`docs/experiments/phase_a2_seeded_init_audit.md` for the fuller per-seed
audit (file size, format check, weight variance, teacher-cache fingerprint).

## 2. Binary hash — **BLOCKED, not just stale**

`SUSPENDED.md` recorded `target/release/sekirei` sha256 `646408b6…` and
`target/release/sekirei-match` sha256 `ef84b524…`, against git commit
`af5d6d4`. Checked this session:

- `HEAD` is now `40e1d3e` (three commits ahead of `af5d6d4` — all
  test/docs changes from today's earlier work; no engine source under
  `crates/sekirei-core`, `crates/sekirei-usi`, or `crates/sekirei-match-runner`
  changed). A rebuild would *likely* reproduce byte-identical binaries.
  That is an inference, not a verification — the entire reason a gate
  manifest records a binary hash is to not have to rely on that inference.
- More concretely: **neither binary exists on disk right now.** The
  `cargo clean` run earlier this session (245 MB removed, `target/` cleared)
  deleted both. `ls target/release/sekirei target/release/sekirei-match` →
  "No such file or directory" for both, confirmed this session.

**This is a hard preflight blocker, not a soft one.** A `cargo build
--release` is required before this gate can launch at all — there is
nothing to hash, let alone gate with. That build is explicitly out of scope
today (banned by the standing light-day constraint, and the machine has
another project's job pinning a CPU core with swap at ~90% as of this
writing). Do not treat "no engine code changed" as equivalent to "verified
identical binary" — rebuild and re-hash before launch, record the new hash
in this file or its successor, and only then proceed.

## 3. Opening corpus hash

`data/gate/openings_gateB.sfen`, 1707 positions, sha256
`816fdf7661989b348bf1c2e078fd6b5748ff9cfc14fa0aed3b83c6df39d56545` — re-verified
this session, unchanged from `SUSPENDED.md`.

## 4. Candidate / baseline labels

`--engine1`/`--weights1` = B1 (candidate); `--engine2`/`--weights2` = A/v011
(baseline). `gate_phase_a2_weight_ab.py`'s `relabel_and_merge` already maps
`candidate_win`→B1, `baseline_win`→A directly (no label-swap step needed,
per that script's own module doc) — confirmed by reading the source, not
assumed.

## 5. Speculation off — confirmed satisfied

`cfg["options"]` in the previous attempt's `state.json` is `[]` (no
`--option` flags were passed). Checked `crates/sekirei-usi/src/main.rs:155`:
`UseSpeculation` USI option defaults to `false`, and `main.rs:85` initializes
`use_speculation = false` before any `setoption`. With no override, the
gate's engines run with speculation off by default — no explicit
`--option UseSpeculation=false` is strictly required, but adding it anyway
(defense against a future default-flip) costs nothing and is recommended for
the next launch command.

## 6. Threads effective value — confirmed satisfied

`launch_shard` (`gate_phase_a2_weight_ab.py:117,120`) always appends
`--engine-option{1,2} Threads={cfg['threads']}` explicitly for both engines.
Previous attempt: `threads=2` in `cfg`. Effective Threads is therefore always
explicit, never left at the engine's own default (`main.rs:145`'s USI
default is `0`/unset) — confirmed by reading the launch command construction,
not assumed from the cfg value alone.

## 7. Fresh-process operation — confirmed satisfied

Each shard launches a fresh `sekirei-match` subprocess
(`gate_phase_a2_weight_ab.py:155`, `subprocess.Popen`), which itself spawns
two fresh `sekirei` engine child processes per game (standard USI
architecture: one process per engine side). No engine process is reused
across shards or across the two sides of one shard — each gets its own
address space, its own fresh `nnue::WEIGHTS` `OnceLock`, its own fresh
search state. This is also why the multi-weight `OnceLock` bug documented in
`docs/experiments/search_ablation_multiweight_repro.md` does not affect this
gate (see that document §2).

## 8. Material fallback — confirmed disallowed

Weight-load failure aborts the engine process (`exit(2)`, commit `92c7ce4`,
locked by `crates/sekirei-usi/tests/evalfile_load_failure_aborts.rs`) rather
than silently falling back to material evaluation. `verify_weights_loaded`
(`gate_phase_a2_weight_ab.py:169-183`) also actively polls each shard's
stderr for `"NNUE weights loaded"` (expects ≥2, one per engine) and
`"weight load failed"`/`"FATAL"`, killing the shard on a detected failure
rather than letting it silently run on whatever the engine fell back to.
Two independent layers agree: no material-fallback path is possible here.

## 9. Illegal / stale bestmove / protocol-error logging

Already implemented in `sekirei-match-runner`, not something this preflight
needs to add:
- `EndReason::IllegalMove` (`main.rs:463`) — logged per-move
  (`main.rs:439`, `"[match] illegal move {mv_str:?} by {mover_name} ..."`)
  and recorded as the game's `end_reason` in its JSON/JSONL output.
- `EndReason::EngineError` (`main.rs:390`) — covers stale/malformed engine
  output.
- `fatal_protocol_error` (`main.rs:253`) — a hard-abort path for a genuine
  USI protocol violation (distinct from an in-game illegal move), invoked at
  `main.rs:329,337,485`.

**Recommended addition, not yet done**: `relabel_and_merge`
(`gate_phase_a2_weight_ab.py:186-236`) currently tallies only
`candidate_win`/`baseline_win`/draw counts into `combined.json` — it does
not separately surface how many of those wins/losses were via
`EndReason::IllegalMove` or `EndReason::EngineError` rather than a normal
decision. A gate could technically reach a decisive SPRT verdict while
one side is winning largely because the other keeps producing illegal
moves — a real bug signal, not a strength signal. Before trusting a future
decisive verdict, cross-tabulate `end_reason` across all confirmed shards'
JSONL records, not just the win/loss/draw counts `combined.json` already
has. This is a recommendation for the burn-in review (§11 below), not a
code change made today.

## 10. Load / swap / RSS monitoring

Already implemented: `resource_snapshot`/`log_resource_snapshot`
(`gate_phase_a2_weight_ab.py:316-344`) record `load1/5/15`, swap used/total,
tracked child RSS, tracked process count, and zombie count every loop
iteration (~20s) to `resource_log.jsonl`, and `should_pause_launching`
(`:332-339`) pauses new shard launches (without killing in-flight ones) on
either threshold. This is what actually held the previous attempt at 0
games — see §12.

## 11. New run ID — do not reuse `results/phase_a2/b1_vs_a`

Recommend a fresh `--outdir` for the next launch, e.g.
`results/phase_a2/b1_vs_a_run2` (or a timestamped variant). Resuming the
existing `b1_vs_a` directory is functionally a cold start
(`confirmed_prefix=0`, all shards still `pending`) — but that directory's
`resource_log.jsonl` (167 snapshots) and `progress.log` still carry the
abandoned attempt's pause history. Mixing a genuinely new attempt's
resource/progress logs into that history is exactly the ambiguity a fresh
run ID avoids. `state.json`/`SUSPENDED.md` in the old directory should be
left as-is (an honest record of what was tried and suspended), not
overwritten by a new attempt.

## 12. Swap-pause threshold — open decision, not resolved here

The previous attempt used the default `--max-swap-pct 50` and paused
immediately at load1=18.3, then stayed paused at a steady ~85.7% swap that
never cleared before the user suspended it. **Swap is worse right now**
(~90%, this session, with another project's job actively running) than it
was during that entire suspended attempt. Launching the next attempt with
the same default will almost certainly re-enter the identical paused state
immediately. `SUSPENDED.md` notes a ~92% figure was discussed but never
applied. This preflight deliberately does **not** pick a new threshold —
that is a decision for whoever launches the gate, informed by actual
conditions at launch time, not a value to bake into a document written
under today's (elevated, unrelated-project-driven) resource state.

## 13. Burn-in plan (uncounted toward the SPRT record) and pass criteria

Purpose: catch process/protocol/crash issues cheaply before committing to
the full 1707-position, up-to-3400-game SPRT run. Proposed, not executed:

- **Scope**: 100–200 games (i.e. 50–100 shard-positions at
  `--games-per-position 2`, the gate's fixed value), drawn from the *start*
  of `openings_gateB.sfen` for simplicity, or a stratified sample if the
  corpus has known category structure — either is fine for a burn-in, since
  its job is protocol/crash detection, not strength measurement.
- **Explicitly excluded from the decisive record**: run to a separate
  `--outdir` (e.g. `results/phase_a2/b1_vs_a_burnin`), never merged into the
  real run's `combined.json`/SPRT history. A burn-in game is not a discarded
  "warm-up" in the `search_ablation` sense (that concept doesn't apply to
  match play) — it is simply a small, disposable dry run of the exact same
  pipeline.
- **Pass criteria** (all required to proceed to the full gate):
  1. Zero `EndReason::EngineError` and zero `fatal_protocol_error` aborts
     across all burn-in games.
  2. Zero `EndReason::IllegalMove` games for *either* side (one or two,
     rarely, might indicate a genuine, narrow rules-engine edge case worth
     investigating before a 3400-game run amplifies it — but the target is
     zero).
  3. `verify_weights_loaded` reports `True` (≥2 "NNUE weights loaded" lines,
     zero "weight load failed"/"FATAL") for every shard, every retry.
  4. No shard exhausts its 3 retries (`gate_phase_a2_weight_ab.py:426`) —
     a shard needing a retry at all during burn-in is worth understanding
     before scaling up, even if it eventually succeeds.
  5. `resource_log.jsonl` shows the configured `--max-swap-pct`/
     `--max-load-mult` actually pausing/resuming sanely (at least one
     pause-then-resume cycle observed, or confirmation that resources
     stayed clear the whole burn-in) — i.e. the monitor itself is verified
     working on this machine's current conditions, not merely present in
     the code.
  6. Manual inspection of a handful of burn-in kifu (the `--output` PGN/kifu
     directory) confirms games look sane (no obviously-degenerate repeated
     openings, no suspiciously-short games beyond genuine early
     resignations).
- **If any criterion fails**: stop, diagnose, fix, and re-run burn-in from
  scratch — do not patch around a failure mid-way and continue into the full
  gate on an unverified pipeline.

## 14. Verdict: is the formal gate ready to launch?

**No.** Blocked on, in order:
1. §2 — rebuild `target/release/{sekirei,sekirei-match}` and record their
   sha256 (currently: the binaries don't exist at all, post `cargo clean`).
2. §12 — decide a swap-pause threshold appropriate to actual conditions at
   launch time (not today's, which are elevated by an unrelated project's
   job).
3. §13 — run and pass the burn-in.

Only after all three does "run the full B1-vs-A SPRT gate" become an
actionable next step, and even then it remains a separate, explicit decision
— not something this document authorizes on its own.
