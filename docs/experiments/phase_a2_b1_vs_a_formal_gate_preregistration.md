# B1 vs A formal gate: methodology pre-registration (design only, not executed)

Status: **pre-registration only**. No permutation was generated, no code was
written or changed, no gate was run to produce this document. This
pre-registers the methodology the *next* formal Gate Step 1 attempt must
follow, given two findings from the 2026-07-25/26 exploratory burn-in:

1. The gate's own stop-on-decisive logic can trigger well before the corpus
   is exhausted (166 games, 83/100 shards, at the 100-position cap).
2. Because shards are launched from the corpus in plain file order, an
   early stop concentrates entirely on whichever positions sit at the front
   of the file — which is exactly the "lucky/unlucky draw from a narrow
   slice" failure mode `phase_a2_seeded_init_preregistration.md`'s 1707-position
   corpus was chosen to avoid in the first place (see `data/README.md`'s note
   on why a `startpos`-only match collapses, and the parallel concern here).

This document does not modify `phase_a2_seeded_init_preregistration.md`
(frozen) or `docs/experiments/phase_a2_b1_vs_a_gate_preflight.md` (already
covers binary/weight/corpus hash and resource-monitor readiness) — it covers
the one gap neither of those documents closes: how the corpus is *ordered*
and how "enough diversity was actually seen" is defined, independent of
whether SPRT's bare LLR happens to cross its boundary early.

Recorded status, unchanged by this document:

| Item | Status |
|---|---|
| Operation/protocol burn-in | PASS (2026-07-26) |
| Exploratory strength signal | decisive positive (B1 over A) |
| Formal Gate Step 1 | **PENDING** |
| Production champion promotion | not done |
| Gate Step 2 (B1 vs C) | not started |

## 1. Opening order: deterministic permutation

### Why not file order

`data/gate/openings_gateB.sfen` has no documented internal ordering
guarantee (its own header note in `data/README.md` only describes the
filtering that produced it — ply ≥ 8, deduplicated from
`positions_opening.jsonl` — not that it was shuffled afterward). Launching
shards sequentially from position 0 means an early SPRT stop always samples
a contiguous prefix, never a spread. A fixed-seed permutation, generated
once and hashed into the manifest, fixes this without needing to change
`gate_phase_a2_weight_ab.py`'s shard-launch order logic — the script already
launches shards in `shards` array order; permuting *which corpus position*
each shard index maps to is enough.

### Algorithm

Reuse the exact Fisher-Yates + xorshift64 pattern already implemented and
used elsewhere in this codebase (`sekirei-bench/src/bin/search_ablation.rs`'s
`shuffled_arm_order`/`xorshift64`) rather than introduce a new PRNG or a
`rand`-crate dependency — this project already has one deterministic,
seeded shuffle implementation, reusing it is both less code and consistent
with an existing, already-reviewed pattern:

```
fn xorshift64(s: &mut u64) -> u64 {
    *s ^= *s << 13; *s ^= *s >> 7; *s ^= *s << 17; *s
}

fn deterministic_permutation(n: usize, seed: u64) -> Vec<usize> {
    let mut order: Vec<usize> = (0..n).collect();
    let mut s = seed | 1;  // avoid the all-zero xorshift64 fixed point
    for i in (1..order.len()).rev() {
        let j = (xorshift64(&mut s) as usize) % (i + 1);
        order.swap(i, j);
    }
    order
}
```

Applied once, offline, over the corpus's 1707 line-indices (not the SFEN
text itself) — the output is an ordering of *indices into the existing,
unmodified* `openings_gateB.sfen`, not a rewritten corpus file. This keeps
`openings_gateB.sfen`'s own sha256 (`816fdf76...`) as the stable identity of
"which 1707 positions," while a separate, small permutation-order file
carries "in what order to draw them."

### Fields to record in the run manifest (schema in
`docs/design/gate_manifest_schema.md`)

```text
permutation_algorithm   "fisher_yates_xorshift64" (this project's existing pattern, reused verbatim)
permutation_seed        a fixed u64, chosen and recorded BEFORE generating the
                         permutation or launching any shard — e.g. a dated
                         constant such as 20260726, following this project's
                         existing convention (search_ablation's --seed
                         20260722, B1/B2/B3's init_seed 42/43/44) of small,
                         documented, non-tuned integers, not a "nice-looking"
                         or post-hoc-chosen value
input_corpus_sha256     816fdf7661989b348bf1c2e078fd6b5748ff9cfc14fa0aed3b83c6df39d56545
                         (openings_gateB.sfen, 1707 positions -- identifies
                         WHAT was permuted)
ordered_output_sha256   sha256 of the generated index-order file itself
                         (computed once the permutation is actually generated
                         -- not today). Pins WHICH permutation was used, so a
                         re-run with the same seed/algorithm against the same
                         corpus can be verified byte-for-byte reproducible
                         rather than trusted on the seed alone.
```

### Pairing rule

`sekirei-match --games-per-position 2` already plays each corpus position
twice — candidate-as-black/baseline-as-white, then candidate-as-white/
baseline-as-black (confirmed this session: `shard_0000_kifu/game0001.txt`
and `game0002.txt` show exactly this color swap for the same starting SFEN).
**A "pair" is these two games for one corpus position.** Formalized rule:

- A pair is **complete** only when both color-orientations for that
  position have a recorded result (win/loss/draw), regardless of order.
- If a run stops (decisive verdict, resource pause-then-abort, or
  contamination — see §3) with one game of a pair finished and the other
  still `running`/`pending`, **that pair does not count toward
  `completed_pairs` or `unique_openings_represented`** in the diversity
  check (§2). It's not discarded data — the finished game's raw record
  stays in the shard's output — it's just not counted as a *complete* pair
  for diversity purposes, so a run can't get partial credit for half-tested
  positions.

### Global game index rule

Today's `relabel_and_merge` computes `global_pos = shard["start_pos"] +
local_pos` — i.e., an index into the corpus's *original* line order. Under
a permutation, this needs one more level of indirection:

```
permuted_rank = the position's 0-indexed rank in the permutation output
                (i.e. permutation[permuted_rank] = original corpus line index)
global_game_index = permuted_rank * 2 + color_index
                     (color_index: 0 = first-played orientation, 1 = second)
```

`shard["start_pos"]`/`shard["end_pos"]` continue to mean "this shard covers
permuted-ranks [start_pos, end_pos)" — shard construction, launch order, and
`confirmed_prefix` bookkeeping in `gate_phase_a2_weight_ab.py` are unchanged;
only the *lookup* `positions[shard["start_pos"]:shard["end_pos"]]` in
`load_positions`/`launch_shard` needs to go through the permutation array
first (`positions[permutation[start_pos]:permutation[end_pos]]` conceptually
— in practice, materializing a permuted copy of the position list once at
load time, so every other line of the script is untouched, is the smaller
diff). Not implemented today — recorded here as the shape of the eventual
one-line change.

### Resume rule

- Resuming an in-progress run must use the **exact same** `ordered_output_sha256`
  as recorded in that run's manifest at creation. If the permutation
  algorithm, seed, or input corpus changes, that is a **new `run_id`**, never
  a resume of the old one — mirrors this session's existing convention (a
  fresh outdir per genuinely new attempt, established for the burn-in vs.
  the suspended `b1_vs_a` attempt).
- A suspended/aborted run's completed shards are valid data for *that*
  `run_id` only. They are never merged into a different `run_id`'s
  `confirmed_prefix`/diversity count — carrying this session's established
  rule (burn-in and the suspended `b1_vs_a` attempt were kept in separate
  directories, never combined) forward as a standing rule for all future
  attempts, not a one-off.

## 2. Minimum-diversity gate (in addition to the bare SPRT boundary)

**A decisive SPRT LLR crossing alone no longer finalizes PASS/FAIL.**
Finalization requires the SPRT boundary crossing *and* all of:

```text
minimum_unique_openings   >= N (see trade-off analysis below)
all counted openings' color-reversed pair complete (per §1's pairing rule)
positions drawn from >= K distinct sections of the permuted corpus
                          (a simple, cheap proxy for "not clustered":
                          divide the corpus into e.g. 10 contiguous permuted-
                          rank deciles, require representation from at
                          least, say, 7 of them -- exact K left as an
                          implementation-time tuning choice, not fixed here)
illegal_moves             == 0
protocol_errors           == 0
stale_bestmoves           == 0
time_forfeits              == 0
material_fallbacks        == 0
```

### Choosing the minimum-unique-openings threshold: 200 vs. 300 vs. 400

Corpus has 1707 positions total. Each threshold implies a **minimum** game
count (threshold × 2, since a pair is 2 games) before finalization is even
eligible — SPRT could still run longer than that minimum if the boundary
isn't crossed yet, but can never finalize with fewer completed pairs than
the threshold.

| Threshold | % of 1707-position corpus | Min. games before eligible | Trade-off |
|---|---|---|---|
| **200** | 11.7% | 400 | Cheapest to reach. Risk: at only ~12% of the corpus, even a permutation-drawn sample could still land disproportionately within one or two "clusters" if the corpus has any structural grouping by opening type (unverified either way — no category metadata exists for this corpus, unlike `search_ablation`'s own test corpus, which does tag `category`). The corpus-section-spread check (K deciles above) partially compensates, but a small N makes that check itself less powerful (fewer samples to spread across 10 deciles). |
| **300** | 17.6% | 600 | A meaningful step up in coverage for a proportionally modest additional cost (200 more games than the 200-threshold, i.e. +50% games for +50% more unique openings). Balances confidence against the fact that `openings_gateB.sfen`'s positions were already filtered for "requires real play to resolve" (ply ≥ 8, per `data/README.md`) — they are not random noise positions, so 300 of them is a reasonably rich sample without needing exhaustive coverage. |
| **400** | 23.4% | 800 | Meaningfully more robust against localized bias (nearly a quarter of the whole corpus), at double the 200-threshold's minimum game count. Justified if a future audit finds the corpus *does* have structural clustering (e.g., if someone adds category metadata later and finds openings aren't evenly distributed through the file) — until that's known, 400 is precautionary rather than evidence-driven. |

**Recommendation: 300**, as a middle-ground default — enough to make a
single-cluster artifact implausible without doubling the compute cost
relative to 200, and proportionate to what's actually known about this
corpus (filtered-for-real-play but otherwise unstructured, per the available
documentation). This is **not** fit to today's burn-in's own numbers: the
exploratory burn-in used only 100 positions and reached a decisive LLR
crossing at 83 of them — this pre-registration deliberately does not treat
"83 was apparently enough to see a huge effect" as evidence for what
threshold to require going forward, since B1's true margin over A
(elo≈+177 in the exploratory signal) may not represent every future
candidate/baseline pair this gate design will ever be used for. The
threshold should be picked for the gate's general reliability, not
retrofitted to make today's particular result look sufficient.

## 3. Stop rules

```text
SPRT boundary reached AND minimum-diversity conditions met (§2)
    → decisive (PASS or FAIL per which SPRT boundary)

SPRT boundary not reached AND max games / corpus exhausted
    → INCONCLUSIVE (not a verdict either way -- matches SPRT's own
      standard "ran out of budget without a signal" case)

SPRT boundary reached BUT minimum-diversity conditions not yet met
    → continue launching shards (drawing further into the permuted
      corpus) until diversity is met or the corpus is exhausted; if the
      corpus is exhausted first, fall through to INCONCLUSIVE above --
      do not finalize a verdict on an under-diverse sample just because
      the corpus ran out

protocol/illegal-move contamination detected (any of the zero-tolerance
counters in §2 above becomes nonzero)
    → STOP launching new shards immediately; do not finalize any verdict
      from this run_id. Quarantine: rename/tag the run directory (e.g.
      append `_contaminated`) rather than delete it -- the completed,
      clean-looking shards before contamination may still be useful
      evidence for root-causing the contamination itself, but the run_id
      as a whole is disqualified from ever producing a formal verdict.
      A fresh run_id, not a resume, is required after the root cause is
      fixed.

binary, weight file, or engine-option config changes after a run_id starts
    → that run_id is retroactively invalid for a formal verdict (the
      manifest's immutable fields, §"Gate manifest schema," would no
      longer describe what was actually played throughout). Start a new
      run_id from scratch. This is why the manifest's binary/weight/config
      fields are immutable once a run starts (see gate_manifest_schema.md)
      -- there is deliberately no "update the binary hash mid-run" path.

burn-in games or any other run_id's games
    → never counted toward this run_id's `completed_pairs`,
      `unique_openings_represented`, W/D/L, or SPRT state. Each run_id is
      self-contained; cross-run_id merging is not supported by this design.
```
