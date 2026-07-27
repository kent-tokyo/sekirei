# Phase A2 B1-vs-A gate: shadow replay — implementation spec, test plan, and resume checklist

Status: **design only, not implemented, not executed**. Written so that,
once CPU-heavy work is authorized again, this can be implemented directly
at this level of detail without further design work. No code was written
or run to produce this document.

## 0. Input data availability (corrected — real formal-gate data exists, but stays untouched by this spec)

An earlier version of this document reported no real formal-gate data
existed anywhere. That was based on an incomplete search that missed a
sibling git worktree (`git worktree list` was not checked) — see
`phase_a2_spread_semantics_audit.md` §1 for the correction. Real data
**does** exist:

`../sekirei-phase-a2-run2/results/phase_a2/b1_vs_a_run2/` (a separate
worktree from this repo) has 378 completed pairs (756 games) as of its
last recorded activity, with `combined.jsonl` (37KB, per-game records) and
`manifest.toml`/`permutation_order.json` both present — i.e. everything
§2's input table needs.

**This spec still does not read or replay that data.** Per this task's
explicit CPU-work constraints, `combined.jsonl` was not bulk-loaded, no
per-pair records were parsed, and no LLR/decile recomputation was run
against it — only small top-level scalars from `state.json`/
`combined.json`/`combined.verdict.json`/`manifest.toml` were read (see the
audit doc §1.1). The implementation and test plan below (§2-§5) remain
build-and-unit-test-only today, against the synthetic fixtures in §5;
pointing this implementation at `b1_vs_a_run2`'s real `combined.jsonl` is
explicitly deferred to the checklist's step 5, once CPU-heavy work is
authorized. The exploratory burn-in
(`results/phase_a2/b1_vs_a_burnin/`, 166 games / 83 pairs) remains a third,
separate data source, and preregistration §1's Resume rule still means its
games are never countable toward `b1_vs_a_run2`'s (or `b1_vs_a`'s)
`completed_pairs`.

## 1. Purpose

A shadow replay reconstructs, from a run's raw per-pair records, the
sequence of diversity/SPRT states the run passed through — without
re-running any games — so that:
- The current live gate script's stopping decision can be independently
  cross-checked.
- Methodology amendments (e.g. `phase_a2_spread_amendment_draft.md`) can be
  evaluated retroactively against already-played data, without touching
  the live run.

## 2. Inputs

Per completed pair, in the order pairs actually completed (not
necessarily permuted-rank order, if parallel shards complete out of
sequence before `confirmed_prefix` catches up — though for `b1_vs_a`
specifically, `confirmed_prefix` only advances through a *contiguous*
completed prefix, so for this gate the confirmed-order sequence and
permuted-rank order coincide by construction):

| Field | Type | Source (once real data exists) |
|---|---|---|
| `pair_order` | int, 0-indexed sequence position in confirmation order | derived: enumerate `all_confirmed` shards in `confirmed_prefix` order |
| `original_corpus_index` | int, 0..1699 | `order[permutation_rank]` — requires `permutation_order.json`, which already exists for `b1_vs_a_run2` (§0; `permutation_sha256 = a3ae8bb7fed8ae8e...` recorded in `manifest.toml`) and can be loaded directly, no regeneration needed |
| `permutation_rank` | int, 0..1699 | `shard["start_pos"] + local_pos` (today's `global_pos`, per `phase_a2_spread_semantics_audit.md` §2.1) |
| `pair_result` | enum: `b1_win` / `a_win` / `draw` per color orientation, both games of the pair | shard's `.jsonl` records, per §1 pairing rule (both color orientations must be present) |
| `llr_increment` | float, this pair's contribution to the running SPRT LLR | not directly stored today — `run_sprt_check` recomputes LLR from the whole `combined_json` each time, rather than tracking a per-pair delta; a shadow replay implementation must either (a) call the same LLR computation incrementally, pair-by-pair, or (b) recompute cumulative LLR at each `pair_order` step from the growing combined result set. Recorded here as an open implementation choice, not decided by this spec. |
| `contamination_counters` | dict of the 6 zero-tolerance counters at this pair | per-shard `.stdout.log` text markers (`" (illegal)"`, `" (engine error)"` — covers `stale_bestmoves`, `" (time forfeit)"`) plus `state["weight_load_failures"]`; `protocol_errors` and `material_fallbacks` are structurally always 0 for any shard that reached `"confirmed"` status, per `compute_diversity_and_counters`'s own docstring (`phase_a2_spread_semantics_audit.md` §1 table) |

## 3. Outputs

| Field | Definition |
|---|---|
| `covered_original_corpus_deciles` | set of deciles (0-9) touched by `original_corpus_index` across all pairs completed so far — the amendment draft's candidate metric, computed alongside (not instead of) the live permuted-rank version, so the two can be compared pair-by-pair |
| `pair_count_per_decile` | for both the permuted-rank keying (today's live definition) and the original-corpus-index keying (the draft amendment's candidate) — a 10-element histogram each, so the amendment draft's "near-vacuous" concern (§ headline open question) can be checked empirically once real or synthetic data is run through this |
| `first_spread_eligible_pair` | the `pair_order` value at which `spread_ok` first becomes true under the *live* (permuted-rank) definition — i.e. first pair after which `len(covered_permuted_rank_deciles) >= 7` |
| `first_eligible_upper_bound_crossing` | the `pair_order` value, at or after `first_spread_eligible_pair` AND at or after `completed_pairs >= 300`, at which the running LLR first crosses the upper (PASS) SPRT boundary — `None` if never reached |
| `first_eligible_lower_bound_crossing` | same, for the lower (FAIL) boundary |
| `stopping_pair` | `min(first_eligible_upper_bound_crossing, first_eligible_lower_bound_crossing)` (whichever is non-`None` and smaller — if both are `None`, replay reaches corpus exhaustion / `INCONCLUSIVE`, matching §3's stop rules in the preregistration doc) |
| `status` | `PASS` / `FAIL` / `INCONCLUSIVE` / `CONTAMINATED` / `NOT_READY`, mirroring `decide_verdict`'s existing outcome set (`scripts/gate_phase_a2_weight_ab.py` line 479-495) — reused, not redefined, so the shadow replay's status vocabulary matches the live script exactly |

## 4. Sequential decision rule (exactly as specified, to avoid re-deriving it during implementation)

```text
for each pair_order, in confirmation order:
    if completed_pairs(pair_order) < 300:
        verdict = not eligible yet -- keep advancing
        continue

    if not spread_ok(pair_order):          # under whichever decile definition is being evaluated
        verdict = not eligible yet -- keep advancing
        continue

    # From here on: 300+ pairs completed AND spread achieved.
    # Take the FIRST current-LLR boundary crossing at or after this point --
    # never re-scan earlier pair_order values, and never let a LATER pair
    # revisit an earlier crossing.
    if LLR(pair_order) has crossed upper or lower boundary:
        stopping_pair = pair_order
        status = PASS (upper) or FAIL (lower)
        stop replay
```

Two properties this preserves, both worth stating explicitly since they're
easy to get wrong in an implementation:

- **A later pair's LLR reverting back across a boundary never changes the
  stopping decision.** Once `stopping_pair` and `status` are set (the first
  eligible crossing, at or after both preconditions hold), the replay
  stops — it does not keep scanning to see if a subsequent pair's LLR
  trajectory looks different. This directly matches the preregistration
  doc's own stop-rule semantics (§3): the live gate script also stops
  launching new shards once a decisive verdict is reached, so a shadow
  replay that kept scanning past the first crossing would be simulating a
  gate that doesn't exist.
- **Win/loss changes after the stopping pair never retroactively alter
  `spread_ok`/decile coverage that already held at the stopping pair.**
  Decile coverage is a monotonically-growing set (pairs only ever get
  *added* to `decile_hits`, never removed) — once 7+ deciles are covered at
  some `pair_order`, that fact is permanent for all later `pair_order`
  values in the same replay.

## 5. Test plan (fixtures + expected results, no execution)

All fixtures below are **synthetic** (small, hand-constructed
`permutation_rank` → `original_corpus_index` mappings and pair-result
sequences) — none require real formal-gate game data, consistent with §0's
finding that no such data exists yet.

| # | Fixture | Expected result |
|---|---|---|
| 1 | A synthetic run where `permutation_rank` and `original_corpus_index` are deliberately different for every pair (i.e. permutation is not the identity map) | Both decile histograms (`pair_count_per_decile` under each keying) differ from each other; the replay must be reading `original_corpus_index` from the permutation lookup, not silently reusing `permutation_rank` for both |
| 2 | A synthetic run where, under `original_corpus_index` keying, all 10 deciles are covered within the first ~20 pairs (simulating the amendment draft's "near-vacuous" concern), while `permutation_rank` deciles take ~1200 pairs to reach 7/10 | `covered_original_corpus_deciles` reaches 10/10 far earlier than `covered_permuted_rank_deciles` reaches 7/10 — this is the fixture that empirically tests the amendment draft's headline open question |
| 3 | A synthetic run where `completed_pairs` passes 300 *before* `spread_ok` (permuted-rank) is achieved, and an LLR boundary is crossed somewhere in between | `status` stays "not eligible yet" through that early crossing; `stopping_pair` is not set until spread is *also* achieved — verifies spread-not-yet-achieved correctly blocks an early LLR crossing from finalizing a verdict, independent of the 300-pair threshold being met |
| 4 | A synthetic run where `spread_ok` (permuted-rank) is achieved *before* `completed_pairs >= 300` | Same as above, mirrored: an LLR crossing before pair 300 is not accepted even though spread already holds — verifies the 300-pair floor independently of the spread condition |
| 5 | A synthetic run where, after both preconditions hold, the LLR crosses the upper boundary at `pair_order = N`, dips back below it at `N+5`, then crosses again at `N+40` | `stopping_pair = N`, `status = PASS` — the later dip-and-recross must NOT change the result; verifies the "first crossing only" rule (§4) |
| 6 | A synthetic 1700-position corpus where `1700 % 10 != 0` is deliberately tested with a non-round position count (e.g. 1703) to check decile-boundary rounding | `decile = min(9, gp*10 // num_positions)` correctly clamps the top bucket (`gp = num_positions - 1` must land in decile 9, not overflow to 10) |
| 7 | A fixture where pairs are NOT filtered/selected in any way before being fed to the replay (full confirmed-prefix sequence, including pairs that would "look bad" for either side) | Replay result is identical whether computed pair-by-pair incrementally or recomputed from scratch at the final `pair_order` — verifies no selection/survivorship bias is introduced by the replay's own bookkeeping |
| 8 | A fixture where one of the 6 contamination counters (e.g. `illegal_moves`) goes nonzero partway through the sequence, spread and 300-pairs already satisfied beforehand | `status = CONTAMINATED` from that pair onward, regardless of what the LLR trajectory does afterward — verifies contamination takes precedence over an LLR crossing, matching `decide_verdict`'s existing precedence (nonzero counters are checked before the boundary check, `scripts/gate_phase_a2_weight_ab.py` lines 489-494) |
| 9 | A fixture where all 6 counters are present but the replay is fed pre-permutation-feature data (mirroring `b1_vs_a`'s actual `state.json`, which has no `ordered_output_sha256`) | `status = NOT_READY` with `unobserved_counters` populated — verifies the replay correctly refuses to produce PASS/FAIL when it cannot itself confirm which counters this specific historical run was capable of observing (mirrors `decide_verdict`'s existing `NOT_READY` path, lines 486-488) |

## 6. Next-steps checklist (fixed order, for when CPU work is authorized again)

```text
1. Re-confirm the artifact snapshot in phase_a2_spread_semantics_audit.md
   §1.1 is still current for b1_vs_a_run2 (state.json unchanged since this
   audit -- no SUSPENDED.md-equivalent exists for it yet, see step 7 below)
   before doing anything else.
2. Confirm original-corpus-index recoverability: b1_vs_a_run2 already has
   a real permutation_order.json (permutation_sha256 a3ae8bb7...) -- load
   it directly. Only regenerate from PERMUTATION_SEED=20260726 + corpus
   sha256 816fdf76... if that file is ever unavailable, and verify the
   regenerated ordered_output_sha256 matches the recorded one exactly
   before trusting it as a substitute.
3. Implement the shadow replay per §2-§4 above.
4. Run the small synthetic fixtures in §5 (no formal-gate data needed).
5. Point the shadow replay at the real, already-existing data in
   `../sekirei-phase-a2-run2/results/phase_a2/b1_vs_a_run2/combined.jsonl`
   (378 pairs / 756 games as of last recorded activity -- §0; no new run
   needs to be launched for this step, the data already exists) and
   compare its stopping_pair/status against the live script's own recorded
   decisions, as a cross-check.
6. Evaluate the proposed amendment options (A/B/C in
   phase_a2_spread_amendment_draft.md) against that same real data,
   addressing the amendment draft's headline open question (the "near-
   vacuous" concern for option B) empirically instead of analytically.
7. Write a SUSPENDED.md-equivalent report for `b1_vs_a_run2` documenting
   why/how it stopped (phase_a2_spread_semantics_audit.md §1.1 "Nuance" --
   this is currently unrecorded) before treating its pause as settled.
8. Decide whether to resume `b1_vs_a_run2` (continue toward 300+ pairs and
   spread_ok) or start a fresh run_id, and only then actually resume/launch
   -- gated on 1-7 above being satisfactorily closed, and on an explicit
   resource-threshold decision if the resource monitor is expected to be a
   factor (unlike `b1_vs_a`'s swap-threshold issue, `b1_vs_a_run2`'s own
   resource_log.jsonl shows load climbing toward, but not over, its
   pause threshold by the time it stopped -- worth deciding whether to
   adjust before resuming, not just replaying the same settings unchanged).
```
