# B1 vs A: exploratory burnin decisive signal (NOT a formal gate result)

**Result identifier: `exploratory_burnin_decisive_pass`. This is explicitly
NOT `formal_gate_pass`.**

## Status

| Item | Status |
|---|---|
| Operation/protocol burn-in | **PASS** |
| Exploratory strength signal | **decisive positive** (B1 over A) |
| Formal Gate Step 1 (B1 vs A, pre-registered) | **PENDING** — not concluded by this run |
| Production champion promotion | not done |
| Gate Step 2 (B1 vs C) | not started |

## Why this does not count as the formal Gate Step 1 result

This run used only the **first 100 of 1707** positions in
`data/gate/openings_gateB.sfen`, drawn strictly in file order (not a random
or stratified sample). `phase_a2_seeded_init_preregistration.md` specifies
the full 1707-position corpus specifically for position diversity, so that
a decisive verdict can't be an artifact of an unrepresentative slice.
Promoting a decisive result from the first 100 positions alone to "formal
Gate Step 1 PASS" would retroactively weaken that pre-registered diversity
guarantee. The signal is real and strong, but it answers "does B1 beat A on
these 100 openings" — not "does B1 beat A across the pre-registered
corpus's full diversity," which is what Gate Step 1 was designed to answer.

## What actually ran

- **Purpose at launch**: pipeline/protocol burn-in (illegal moves, engine
  errors, weight-load reliability, resource-monitor sanity) — not intended
  to reach a strength verdict, but the SPRT check the gate runs at every
  confirmed shard reached a decisive boundary crossing before the 100-position
  cap was exhausted.
- **Outdir**: `results/phase_a2/b1_vs_a_burnin/` (deliberately separate from
  both the suspended `results/phase_a2/b1_vs_a/` attempt and any future
  formal run — none of these are mixed).

## Results

- **W/D/L**: B1 (candidate) 122 — A (baseline) 44 — draws 0, out of 166
  games played (83/100 shards completed before the run's own stop-on-decisive
  logic halted further launches).
- **Elo/LOS/LLR**: `elo_diff = +177.16`, `los = 100.0%`, SPRT
  `H0(elo<=0) vs H1(elo>=20)`, `alpha=beta=0.05`, `llr = 3.123` (bounds
  ±2.944 — upper bound crossed, i.e. H1 accepted at that error rate).
- **Confidence interval**: not computed at this aggregate level —
  `gate_phase_a2_weight_ab.py`'s `relabel_and_merge` does not compute an
  overall CI (only `sekirei-match`'s per-shard JSON has an `elo_ci_95`
  field, and at n=2 games/shard those are far too wide to be meaningful
  individually). Recorded here as a known gap, not glossed over.

### Note: why 162 and 166 both appear in this run's logs

`progress.log`/`state.json`'s `sprt_history` show the DECISIVE PASS first
firing at `confirmed_prefix=81` (**162** games, `llr=2.966` crossing the
upper bound `2.944`) at 2026-07-26 00:13:51. Setting `stop_launching=True`
at that instant only stops *new* shard launches -- shards 82 and 83 were
already running and were allowed to finish rather than being killed
mid-game, per this orchestrator's existing "pause/stop launching, never
kill in-flight work" design. Those two shards completed 80 seconds later
(00:15:11), advancing `confirmed_prefix` to 83 (**166** games,
`llr=3.123`), which is the final number `combined.json`/`combined.jsonl`
hold and the one used throughout the rest of this record. 162 and 166 are
two timestamps of the same monotonically-growing series, not two different
datasets -- no game was lost, duplicated, or recomputed between them.

## Operational/protocol burn-in criteria (all passed)

| Criterion | Result |
|---|---|
| Illegal moves | 0 (grepped `"illegal move"` across all 83 shards' stderr logs) |
| Protocol errors / FATAL aborts | 0 (grepped `"FATAL"`/`"protocol"` across all stderr logs) |
| Weight-load successes | 166/166 (`"NNUE weights loaded"` count exactly matches 2× completed shards) |
| Weight-load failures | 0 |
| Shard retries | 0 (`state.json`: no shard's `retries` field > 0) |
| Resource-monitor pauses | 0 of 245 snapshots (`resource_log.jsonl`); max swap observed 88.4%, under the `--max-swap-pct 92` threshold used |
| Kifu sanity | spot-checked (`shard_0000_kifu/game000{1,2}.txt`) — legal-looking SFEN, plausible move counts and resignation points |

## Manifest / provenance (saved for continuity into any future formal run)

| Item | Value |
|---|---|
| git commit at run time | `c399a7cfc8fc76882cb968cdb261bca3db314a32` |
| `target/release/sekirei` sha256 | `792dbed130e38dfb8ecdb63a87e4234f4d3d512676cc06a9bf602c01c625f6b1` |
| `target/release/sekirei-match` sha256 | `4ecdbca057e018363be236f755a9205ec8337bca5471010726a8aa60c99bef0e` |
| B1 (candidate) weight, sha256 | `019d13f284447b6afc3905dfccb7a5a570e4e3d3b08655a7f3a7b43b174a1385` |
| A (baseline, v011) weight, sha256 | `a45be6099c0936283e79f34d380a4dbc7ba681796bb0bb56b2cd743c2c786ea6` |
| Full opening corpus, sha256 | `816fdf7661989b348bf1c2e078fd6b5748ff9cfc14fa0aed3b83c6df39d56545` (`data/gate/openings_gateB.sfen`, 1707 raw lines / **1700 canonical valid openings** — see `phase_a2_b1_vs_a_formal_gate_preregistration.md`'s "Resolving 1700 vs. 1707") |
| First-100-positions-used subset, sha256 | **Corrected 2026-07-26**: `6492afec9ac99bedf7f0e444be16c66ba4157871956710285a0292a8e705f389` — saved verbatim at `results/phase_a2/b1_vs_a_burnin/artifacts/first100_positions_used.sfen`. The originally-recorded hash (`7d39a024...`) was wrong: it was extracted via a plain `head -100` on the raw corpus file, which captured the file's 7 leading `#` comment lines plus only 93 real SFEN positions — not the 100 real positions the gate script's own comment-filtering `load_positions` actually used. Regenerated with the correct filter (skip `#`/blank lines, then take 100) and re-hashed; the burn-in's *results* (W/D/L, Elo, LLR) are unaffected — this error was only in which positions this artifact preserved for the record, not in what was actually played. |
| Threads (per engine) | 2 (explicit, `--engine-option Threads=2`) |
| Hash / TT size | 64 MB — **not explicitly set** by the gate script; this is `sekirei-usi`'s compiled-in `DEFAULT_HASH_MB`, applied because no `Hash` USI option was passed |
| Byoyomi | 1500 ms |
| Speculation | off — not explicitly set either; `UseSpeculation` defaults to `false` in `sekirei-usi` and nothing in this run's `cfg.options` (empty list) overrode it |
| Raw game logs | `results/phase_a2/b1_vs_a_burnin/shard_{0000..0099}.{json,jsonl,stdout.log,stderr.log}` and `shard_{0000..0099}_kifu/game*.txt` — all preserved as-is |
| Resource monitor log | `results/phase_a2/b1_vs_a_burnin/resource_log.jsonl` (245 snapshots) |
| Full run state | `results/phase_a2/b1_vs_a_burnin/state.json`, `progress.log`, `combined.json`, `combined.jsonl` |

## What the next formal Gate Step 1 run needs (recorded for continuity, not implemented today)

Discussed but **not implemented or written into `phase_a2_seeded_init_preregistration.md` today** — that pre-registration document remains frozen and unedited by this session, per instruction:

- Draw the 1707 positions via a fixed-seed deterministic permutation rather
  than sequential file order, so early SPRT stopping (which this run showed
  can happen well before the corpus is exhausted) still draws from across
  the full corpus rather than concentrating on whichever positions happen
  to sit at the front of the file. Record the seed, the resulting order's
  hash, and the generation method in that run's manifest. Treat each
  opening's color-reversed pair as one indivisible unit under the
  permutation.
- Do not finalize a formal PASS on SPRT-boundary-crossing alone. A draft
  minimum-diversity gate, to require in addition:
  - at least ~300 unique openings represented,
  - every opening's color-reversed pair completed (not just one side),
  - positions drawn from multiple sections of the corpus, not one cluster,
  - illegal move / protocol error / time-forfeit / material-fallback all
    exactly 0,
  - SPRT LLR past its decision boundary.

## What was not done today

No code change, no formal Gate Step 1 run, no Gate Step 2 (B1 vs C), no
production-champion promotion, no push, no version bump, no edit to
`phase_a2_seeded_init_preregistration.md`. All Sekirei-related processes
(engine, match-runner, gate orchestrator, resource monitor) confirmed
terminated with none remaining before this document was written.
