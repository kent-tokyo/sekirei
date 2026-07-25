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
   slice" failure mode `phase_a2_seeded_init_preregistration.md`'s
   diverse-opening-corpus design was chosen to avoid in the first place
   (the corpus file has 1707 raw lines / 1700 canonical valid openings —
   see the exact resolution of that distinction below; see `data/README.md`'s note
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

### Algorithm — fully confirmed, not illustrative

`permutation_seed = 20260726`. **This is the confirmed value for the next
formal Gate Step 1 run, not an example** — chosen before any permutation is
generated and before any shard is launched, following this project's
existing convention of small, dated, non-tuned integer seeds
(`search_ablation`'s `--seed 20260722`, B1/B2/B3's `init_seed` 42/43/44).

Reuse the exact Fisher-Yates + xorshift64 pattern already implemented and
used elsewhere in this codebase (`sekirei-bench/src/bin/search_ablation.rs`'s
`shuffled_arm_order`/`xorshift64`) rather than introduce a new PRNG or a
`rand`-crate dependency. Every implementation detail below is fixed so that
a Rust implementation and a Python implementation, given the same seed and
the same input file, produce byte-identical output — "reuse the existing
pattern" is not sufficient on its own, since xorshift64 has several
published shift-constant variants and Fisher-Yates has more than one valid
index-generation convention.

**PRNG state and transition** (exactly this project's existing
`search_ablation.rs` definition, no variation):
- State: a single `u64`.
- Initial state: `seed | 1` — bitwise OR with 1, guaranteeing an odd,
  nonzero starting state regardless of the seed's own parity (xorshift64
  has an all-zero fixed point: state 0 maps to 0 forever; `| 1` avoids ever
  landing there without altering the seed's other 63 bits).
- Transition (applied to produce each successive value): left-shift by 13,
  XOR into state; right-shift by 7, XOR into state; left-shift by 17, XOR
  into state; the resulting state is both the new state and the emitted
  value. In order: `s ^= s << 13; s ^= s >> 7; s ^= s << 17;` then emit `s`.
- **Overflow/wraparound**: none occurs and none needs handling. All three
  shift amounts (13, 7, 17) are strictly less than 64 (the type's bit
  width), so `<<`/`>>` on a `u64` are fully defined bit-shifts in both Rust
  and Python (Python ints are arbitrary-precision, so shifting must be
  explicitly masked back to 64 bits after each left-shift — see the Python
  reference below; Rust's `u64` truncates left-shift overflow automatically
  as part of the fixed-width type, no explicit mask needed). `^=` (XOR) has
  no overflow concept at any width.

**Fisher-Yates index generation** — the standard back-to-front shuffle,
iterating `i` from `n-1` down to `1` inclusive (never processing `i=0`,
which has only one possible position and needs no swap):
```
for i in (n-1) down to 1:
    j = xorshift64_next(state) mod (i + 1)
    swap(order[i], order[j])
```

**Modulo bias — decided: plain `% (i+1)`, not rejection sampling.**
`xorshift64_next` returns a full 64-bit value; reducing it via `% (i+1)`
for `i+1` ranging up to 1700 (the canonical opening count — see §"Resolving
1700 vs. 1707" below) introduces a theoretical bias bounded by
`(i+1) / 2^64`, worst case `1700 / 2^64 ≈ 9.2 × 10⁻¹⁷` — undetectable at
any practically achievable sample size, let alone a single 1700-element
shuffle. Rejection sampling would eliminate even this negligible bias, at
the cost of an unbounded (if vanishingly rarely triggered) retry loop and
slightly more code. Given the bias magnitude, added complexity buys nothing
measurable here; **use plain `%`**, matching this project's existing
`shuffled_arm_order` precedent (which also uses plain `% (i + 1)` at a much
smaller n). If this exact permutation function is ever reused for an `n`
approaching `2^32` or beyond, revisit this decision — the bias bound scales
with `n`, and at that scale it would no longer be obviously negligible.

**Input line handling** (defines what "index k" refers to before any
permutation is applied):
1. Read the corpus file line by line.
2. Strip trailing line-ending characters — both `\n` and `\r\n` must
   normalize identically (e.g. Rust's `.lines()` iterator already handles
   this; Python's `.splitlines()` or `line.rstrip('\r\n')`, not bare
   `.rstrip()`, which would also strip meaningful trailing whitespace from
   an SFEN — there is none in practice, but the rule should be exact, not
   "probably fine").
3. Skip a line if, after stripping, it is empty.
4. Skip a line if, after stripping, it starts with `#` (comment/header
   lines) — matching the exact filtering rule `gate_phase_a2_weight_ab.py`'s
   own `load_positions` already uses (`if not line or line.startswith("#"):
   continue`).
5. Every surviving line, in file order, is **canonical opening index k**
   (0-indexed), for `k = 0 .. canonical_valid_opening_count - 1`. The
   permutation is defined over this canonical index space, **never** over
   raw file line numbers (which would require separately tracking which raw
   lines were skipped — avoided entirely by permuting only the
   already-filtered, canonical sequence).

**Reference implementations** (specification, not executed — provided so
"Rust and Python produce the same order" is checkable by inspection, not
asserted):

```rust
fn xorshift64_next(s: &mut u64) -> u64 {
    *s ^= *s << 13;
    *s ^= *s >> 7;
    *s ^= *s << 17;
    *s
}

fn deterministic_permutation(n: usize, seed: u64) -> Vec<usize> {
    let mut order: Vec<usize> = (0..n).collect();
    let mut s = seed | 1;
    for i in (1..n).rev() {
        let j = (xorshift64_next(&mut s) as usize) % (i + 1);
        order.swap(i, j);
    }
    order
}
```

```python
MASK64 = (1 << 64) - 1

def xorshift64_next(s: int) -> int:
    s ^= (s << 13) & MASK64
    s ^= s >> 7
    s ^= (s << 17) & MASK64
    return s & MASK64

def deterministic_permutation(n: int, seed: int) -> list[int]:
    order = list(range(n))
    s = seed | 1
    for i in range(n - 1, 0, -1):
        s = xorshift64_next(s)
        j = s % (i + 1)
        order[i], order[j] = order[j], order[i]
    return order
```

Note the Python version's explicit `& MASK64` after every left-shift
(Python integers don't wrap on their own) and returns the new state
separately from mutating it in place (Python has no `&mut` — the caller
must thread `s` through explicitly) — these are the two concrete places a
naive Python port would silently diverge from the Rust version if not
handled exactly as shown.

Applied once, offline, over the corpus's canonical opening indices (not the
SFEN text itself) — the output is an ordering of *indices into the
existing, unmodified* `openings_gateB.sfen`'s canonical (comment/blank-line-filtered)
sequence, not a rewritten corpus file. This keeps `openings_gateB.sfen`'s
own sha256 (`816fdf76...`) as the stable identity of "which positions,"
while a separate, small permutation-order file carries "in what order to
draw them."

### Fields to record in the run manifest (schema in
`docs/design/gate_manifest_schema.md`)

```text
permutation_algorithm   "fisher_yates_xorshift64" (this project's existing
                         pattern, fully specified above — shift constants
                         13/7/17, seed|1 initial state, plain modulo, no
                         rejection sampling)
permutation_seed        20260726 (confirmed, not an example — see above)
input_corpus_sha256     816fdf7661989b348bf1c2e078fd6b5748ff9cfc14fa0aed3b83c6df39d56545
                         (openings_gateB.sfen -- identifies WHAT file was
                         permuted; 1707 raw lines, 1700 canonical valid
                         openings after comment/blank-line filtering --
                         see next section for why these two numbers differ
                         and which one governs game counts)
ordered_output_sha256   sha256 of the generated index-order file itself
                         (computed once the permutation is actually generated
                         -- not today). Pins WHICH permutation was used, so a
                         re-run with the same seed/algorithm against the same
                         corpus can be verified byte-for-byte reproducible
                         rather than trusted on the seed alone.
```

### Resolving 1700 vs. 1707 (confirmed by direct file inspection, 2026-07-26)

`data/gate/openings_gateB.sfen` has **1707 raw lines** but **1700 canonical
valid openings** — confirmed directly: `wc -l` reports 1707; `grep -c
'^#'` reports 7 comment/header lines; `grep -c '^[[:space:]]*$'` reports 0
blank lines; `1707 - 7 = 1700`. The file's own first header line even
states this explicitly: `"# one SFEN per line -- Gate B opening suite
(1700 positions, ply>=8, ...)"`. This fully resolves the apparent
inconsistency between "1707 positions" (this document's earlier draft,
and `docs/experiments/phase_a2_seeded_init_audit.md`/`phase_a2_b1_vs_a_gate_preflight.md`'s
casual use of the same figure — both refer to the raw line count, not the
canonical count) and the historical `SUSPENDED.md`/`progress.log` figure of
**"1700 shards, 1700 positions, 3400 max games."** That historical figure
was correct all along; this document's own earlier "1707 positions" phrasing
was imprecise and is corrected here.

**Canonical valid opening count: 1700. `maximum_games = 2 × 1700 = 3400`,
matching the historical figure exactly — no inconsistency remains.**

Defined as a formula, not a hardcoded literal, so it stays correct if the
corpus file itself is ever regenerated with a different valid count:
```
maximum_games = 2 × canonical_valid_opening_count
```
where `canonical_valid_opening_count` is computed by the exact line-filtering
rule in the "Input line handling" section above (skip blank lines and lines
starting with `#`), applied to whichever corpus file `input_corpus_sha256`
identifies — never assumed to equal the raw `wc -l` line count, and never
hardcoded as a bare literal separately from that count. `maximum_games` is
even by construction (2 × any integer), so a run can never end mid-pair
purely from hitting the games ceiling.

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

## 2. Minimum-diversity gate (in addition to the bare SPRT boundary) — applied symmetrically to PASS and FAIL

**A decisive SPRT LLR crossing alone no longer finalizes PASS *or* FAIL.**
Diversity is a precondition for treating the result as decisive **in either
direction** — applying it only to PASS (and letting an early FAIL through
unchecked) would make the stop rule asymmetric for no principled reason: an
unrepresentative slice can produce a misleadingly bad result just as easily
as a misleadingly good one.

**Named fields, confirmed values**:
```
minimum_completed_pairs = 300     (see 200/300/400 trade-off below)
minimum_games           = 600     (= 2 × minimum_completed_pairs, by
                                    construction — a "pair" is always 2
                                    games, per §1's pairing rule)
```

**Formal finalization condition** (identical for PASS and FAIL — the SPRT
boundary crossed determines *which* of the two, not *whether* diversity
applies):
```
completed_pairs >= minimum_completed_pairs   (i.e. >= 300)
AND
SPRT LLR has crossed either its upper or lower decision boundary
AND
illegal_moves == 0
AND protocol_errors == 0
AND stale_bestmoves == 0
AND time_forfeits == 0
AND weight_load_failures == 0
AND material_fallbacks == 0
```
All six operational counters at zero is a **precondition for any verdict at
all** (PASS, FAIL, or otherwise) — a nonzero counter routes to the
"contaminated" stop rule in §3 regardless of what the SPRT LLR or diversity
count show.

Also still required, unchanged from the original draft of this section,
as part of what "diversity" means (not just a bare pair count):
```
all 300+ counted openings' color-reversed pairs complete (per §1's pairing rule)
positions drawn from >= K distinct sections of the permuted corpus
                          (proxy for "not clustered": divide the corpus
                          into e.g. 10 contiguous permuted-rank deciles,
                          require representation from at least, say, 7 of
                          them -- exact K left as an implementation-time
                          tuning choice, not fixed here)
```

### Choosing the minimum-completed-pairs threshold: 200 vs. 300 vs. 400

Corpus has **1700 canonical valid openings** (not 1707 — see "Resolving
1700 vs. 1707" in §1). Each threshold implies a **minimum** game count
(threshold × 2) before finalization is even eligible, **in either
direction** — SPRT could still run longer than that minimum if neither
boundary is crossed yet, but can never finalize with fewer completed pairs
than the threshold, whether the LLR trajectory is heading toward PASS or
FAIL.

| Threshold | % of 1700-opening corpus | Min. games before eligible | Trade-off |
|---|---|---|---|
| **200** | 11.8% | 400 | Cheapest to reach. Risk: at only ~12% of the corpus, even a permutation-drawn sample could still land disproportionately within one or two "clusters" if the corpus has any structural grouping by opening type (unverified either way — no category metadata exists for this corpus, unlike `search_ablation`'s own test corpus, which does tag `category`). The corpus-section-spread check (K deciles above) partially compensates, but a small N makes that check itself less powerful (fewer samples to spread across 10 deciles). |
| **300** | 17.6% | 600 | A meaningful step up in coverage for a proportionally modest additional cost (200 more games than the 200-threshold, i.e. +50% games for +50% more unique openings). Balances confidence against the fact that `openings_gateB.sfen`'s positions were already filtered for "requires real play to resolve" (ply ≥ 8, per `data/README.md`) — they are not random noise positions, so 300 of them is a reasonably rich sample without needing exhaustive coverage. |
| **400** | 23.5% | 800 | Meaningfully more robust against localized bias (nearly a quarter of the whole corpus), at double the 200-threshold's minimum game count. Justified if a future audit finds the corpus *does* have structural clustering (e.g., if someone adds category metadata later and finds openings aren't evenly distributed through the file) — until that's known, 400 is precautionary rather than evidence-driven. |

**Recommendation and confirmed value: 300** (`minimum_completed_pairs = 300`,
`minimum_games = 600`) — as a middle-ground default, enough to make a
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
SPRT boundary reached (either upper=PASS or lower=FAIL) AND
completed_pairs >= 300 AND all six operational counters == 0 (§2)
    → decisive (PASS or FAIL, matching whichever boundary was crossed --
      the diversity/cleanliness precondition is IDENTICAL for both, per
      §2's symmetric treatment)

SPRT boundary not reached AND max games (3400 = 2 x 1700 canonical
openings) / corpus exhausted
    → INCONCLUSIVE (not a verdict either way -- matches SPRT's own
      standard "ran out of budget without a signal" case)

SPRT boundary reached BUT completed_pairs < 300
    → continue launching shards (drawing further into the permuted
      corpus) until completed_pairs >= 300 or the corpus is exhausted; if
      the corpus is exhausted first, fall through to INCONCLUSIVE above --
      do not finalize a verdict (PASS or FAIL) on an under-diverse sample
      just because the corpus ran out

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
