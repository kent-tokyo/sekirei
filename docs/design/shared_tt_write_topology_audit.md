# Shared-TT write topology audit (issue #32)

Status: **static, read-only.** No engine matches, no fixed-depth gate runs,
no benchmarks, no NNUE training, no PR #17 implementation changes, no PR #4
re-evaluation, no rewrite of `speculative.rs`. Every claim below cites the
specific source lines it's derived from. `main`@`77cff19`
(`crates/sekirei-core/src/tt.rs`, `search.rs`, `speculative.rs`); PR #17
(`feat/qsearch-tt`@`e6d50e1`) read separately, not merged, cited explicitly
where it differs from `main`.

Motivating evidence (both already recorded in
`docs/experiments/fixed_depth_gate_run_index.md`):
1. PR #16 (merged) removed `SpecGroup::spawn`'s parent-hash closure store — a
   confirmed last-writer-wins race. Its own repeatability evaluation left 3
   positions unchanged (not newly regressed, not improved): `unique_bestmoves=2`
   both before and after the fix, at `king_danger_nyugyoku_full_army`,
   `_insufficient_pieces`, `_insufficient_points`. A second, unidentified
   source was inferred at the time.
2. PR #17's repeats-mode re-evaluation against post-PR#16 `main` moved the
   primary metrics in the unfavorable direction (bestmove-variance 2/21 →
   4/21), newly destabilizing 3 previously-stable positions. A `SpecTopN=0`
   control was fully deterministic on both sides.

## 1. Executive summary

- **Finding 0 (new, confirmed — not a race, no concurrency required):**
  `root_search_inner` and `alpha_beta`'s two mid-function loops (`nw_results`
  processing, and the sequential tail) `break` on `state.budget.should_abort()`
  and then fall through to an **unconditional** final `store_tt` call that
  classifies `Bound::Exact` vs. `Bound::Upper` purely from `best_score >
  orig_alpha`, with no awareness that the break was abort-triggered rather
  than a natural loop exhaustion. This can store an entry labeled `Exact`
  (unconditionally trusted by every consumer, §3) that was never actually
  informed by every legal move at that node — some were simply never reached
  before the deadline. `spec_alpha_beta` gets this right (`return 0` on
  abort at both check points, `speculative.rs:192-194` and `237-239`+`246-248`,
  never falling through to its own store — the file's own header comment
  states this as an explicit invariant, `speculative.rs:7-9`). `search.rs`'s
  two producers do not follow the same discipline. Unlike every other
  finding in this audit, this one requires no transposition, no second
  producer, and no live trace — it follows directly from the control flow
  of a single call, deterministically, whenever a search is aborted after
  at least one child has already updated `best_score`/`best_move`. See §7
  Finding 4 for the full derivation and §11 for a proposed (not implemented)
  fix.
- **`Tt::store`'s equal-depth replacement policy is not "last-writer-wins at
  one specific removed call site" — it is the store function's own general
  behavior, unconditionally, for every producer.** `Tt::store` (`tt.rs:175`)
  rejects a write only when `entry.depth < existing_depth`; at
  `entry.depth == existing_depth` it always overwrites, with **zero**
  preference for `Bound::Exact` over `Lower`/`Upper`, no generation counter,
  no producer identity, no tie-break of any kind. PR #16 removed *one*
  guaranteed-collision instance of this (SpecGroup siblings racing on the
  same parent hash). The underlying permissiveness in `Tt::store` itself was
  not touched and still governs every other producer pair.
- **The XOR-trick storage format (`tt.rs:1-23`) is genuinely safe against
  cross-writer field-mixing.** A concurrent race can produce a torn/miss read
  (safe — silently treated as a cache miss) or a clean single-writer entry
  (whichever writer's `key` store physically lands last, provided no further
  `data` write follows it) — but never a Frankenstein entry with e.g. one
  writer's score and another's move. This is confirmed by construction, not
  merely plausible (§4).
- **7 real (non-test) producer call sites exist on `main`** (1 in
  `root_search_inner`, 5 in `alpha_beta` via the `store_tt` helper, 2 in
  `spec_alpha_beta`). **PR #17 adds 5 more**, all in `quiescence()`, all at
  the literal depth `0`.
- **No concrete, traced collision is confirmed** — this audit found none of
  the required proof shape ("Producer X and Producer Y wrote the same hash at
  the same depth in the same run, and a consumer used the losing write") for
  any pair, on `main` or on PR #17's branch, because doing so requires a live
  trace, which is out of scope for this pass. Several pairs are **plausible**
  with a specific, non-hand-wavy mechanism (§7); none are **impossible** to
  rule out from static reading alone.
- **The most interesting finding this audit adds beyond what motivated it**:
  PR #17's new depth=0 stores can, in principle, race **with each other** —
  no `SpeculativeSearcher` involvement required — purely via `alpha_beta`'s
  own pre-existing YBW parallelism (`search.rs:868-900`, rayon's *global*
  pool, unrelated to `SpecTopN`). This complicates the "confined to the
  speculative-concurrency channel" reading of PR #17's `SpecTopN=0`-clean
  control (§10) — the control rules out code paths that only execute when
  `SpecTopN>0`, but does not rule out timing/contention effects, since
  `SpecTopN>0` adds real competing CPU load that could be what exposes an
  otherwise-latent YBW-only race rather than the speculative code path being
  the race's actual source.
- **Recommended next action (§11-12)**: this audit does not clear the bar the
  user set for implementation (§9 below, restated: no concrete proven race
  yet). The single highest-value next step is not a code change but an
  **instrumented static replay** — log every `Tt::store` call's
  `(hash, depth, thread_id, bound, wall_clock)` for one `repeats`-mode run at
  `SpecTopN=3` and check the log for same-hash/same-depth pairs with
  overlapping wall-clock windows. That would upgrade the plausible races in
  §7 to confirmed or ruled-out without needing to guess at a fix first. This
  itself would be new code (a logging shim) and a live run, so it is *not*
  done in this pass either — flagged as the recommended action, not taken.

## 2. Complete producer inventory

All `Tt::store` call sites, grep'd from `crates/sekirei-core/src/` with
`grep -rn "tt\.store\|store_tt("` and cross-checked by reading every match's
enclosing function. Test-only call sites (`tt.rs`'s own `#[cfg(test)]`
module, `speculative.rs`'s test-only `spec_state()` helper's callers) are
excluded — they never run outside `cargo test`.

| # | Producer | Source | Hash | Depth | Bound | Move | Thread context |
|---|---|---|---|---|---|---|---|
| P1 | `root_search_inner` | `search.rs:546` | `board.hash()` at root, after all root moves searched | iterative-deepening `depth` (root loop var, always ≥1; single/no-move early-returns bypass this store) | `Lower` (fail-high, `alpha >= hi`) or `Exact` | `Some(best_move)` always (only reached if `best_move.is_some()`) | Driver thread (the thread running `SpeculativeSearcher::search`'s `for depth in ...` loop) — **but** the root-move loop inside `root_search_inner` (`search.rs:522`) is itself sequential, not parallel; concurrency comes from what each `alpha_beta` call it drives spawns internally |
| P2 | `alpha_beta`, first-move cutoff | `search.rs:834` | `board.hash()` at this node | this call's local `depth` (post-IIR adjustment, `search.rs:626-630`) | `Lower` | `Some(first_move)` | Whichever thread is executing this `alpha_beta` invocation — driver thread, or any rayon global-pool worker if this call was itself dispatched from a parent's `into_par_iter()` |
| P3 | `alpha_beta`, single-move terminal | `search.rs:852` | `board.hash()` at this node | local `depth` | `Exact` or `Upper` | `Some(first_move)` | same as P2 |
| P4 | `alpha_beta`, YBW-parallel-pass fail-high | `search.rs:951` | `board.hash()` at this node | local `depth` | `Lower` | `Some(m)`, `m` from the post-`collect()` sequential loop over `nw_results` | Runs on the thread that *dispatched* the `into_par_iter()` (i.e. after the parallel siblings have already joined at `.collect()`, `search.rs:900`) — not concurrent with its own siblings' searches, but concurrent with everything else in the tree |
| P5 | `alpha_beta`, sequential-tail fail-high | `search.rs:1056` | `board.hash()` at this node | local `depth` | `Lower` | `Some(m)`, tail-loop move | same dispatching thread as P4 |
| P6 | `alpha_beta`, end-of-function terminal | `search.rs:1073` | `board.hash()` at this node | local `depth` | `Exact` or `Upper` | `Some(best_move)` (always `Some` — seeded from `first_move` at minimum) | same dispatching thread as P4/P5 |
| P7 | `spec_alpha_beta`, beta-cutoff | `speculative.rs:258` | `board.hash()` at this recursive call's own node (position after however many plies this spec task has played from its root candidate move) | this call's local `depth` (starts at `outer_depth − 1` from `SpecGroup::spawn`'s `depth.saturating_sub(1)`, decrements per recursion; **never 0** — `depth == 0` returns a bare `evaluate()` at `speculative.rs:196-198` before reaching any store) | `Lower` | `Some(best_move)` | `SpecState::pool` — the dedicated speculative-search pool, **not** rayon's global pool (`speculative.rs:91`) |
| P8 | `spec_alpha_beta`, end-of-loop terminal | `speculative.rs:279` | same as P7 | same as P7 | `Exact` or `Upper` | `Some(best_move)` (moves is non-empty by the check at `speculative.rs:228`; `best` starts at `-1_000_000` so the first move's score essentially always exceeds it) | same dedicated pool as P7 |

### PR #17 branch delta (`feat/qsearch-tt`@`e6d50e1`, `search.rs` only — not merged)

PR #17 adds `quiescence()`'s own TT interaction, gated
`let cacheable = qply == 0;` (`search.rs:1201` on the branch). 5 new store
sites, all sharing the literal depth `0`:

| # | Producer | Source (branch) | Hash | Depth | Bound | Move | Gate |
|---|---|---|---|---|---|---|---|
| P9 | `quiescence`, in-check terminal (mate) | `search.rs:1207` | `board.hash()` | `0` | `Exact` | `None` | `cacheable` (`qply == 0`) |
| P10 | `quiescence`, no-capture terminal | `search.rs:1221` | `board.hash()` | `0` | `Exact` or `Upper` | `None` | `cacheable` |
| P11 | `quiescence`, stand-pat-beaten-by-capture cutoff | `search.rs:1251` | `board.hash()` | `0` | `Lower` | `Some(m)` | `cacheable` |
| P12 | `quiescence`, second capture-loop cutoff | `search.rs:1300` | `board.hash()` | `0` | `Lower` | `Some(m)` | `cacheable` |
| P13 | `quiescence`, end-of-function terminal | `search.rs:1321` | `board.hash()` | `0` | `Exact` or `Upper` | `best_move` | `cacheable` |

All 5 run on whatever thread is executing the enclosing `alpha_beta(depth=0)`
call that dispatched into `quiescence` — i.e. the same thread-context
population as P2-P6 (driver thread or any global-pool YBW worker), **never**
`SpecState::pool` (`spec_alpha_beta` never calls `quiescence`; it returns a
bare `evaluate()` at its own `depth == 0`, confirmed unchanged on this
branch — the diff is scoped to `search.rs` only,
`crates/sekirei-core/src/search.rs | 464 +++++++++++++++++++++++++++++++++++++-`
per the branch's own diffstat, `speculative.rs` untouched).

**Topology difference from `main`:** on `main`, depth `0` is a namespace no
producer ever writes into (P1 always ≥1; P2-P6 return early into
`quiescence` before reaching depth `0`, `search.rs:587-588`; P7/P8 return
early into a bare `evaluate()` before reaching depth `0`,
`speculative.rs:196-198`) — so `main`'s depth-`0` TT slots are permanently
empty. PR #17 is the *only* thing that ever writes depth `0`, which is why
its own read-side gate (`qply == 0 && entry.depth == 0`, `search.rs:1144` on
the branch) is sufficient to distinguish "a real qsearch entry" from "a real
search-layer entry" — no other producer can produce a depth-`0` entry to
confuse it. What this read-side gate does **not** protect against is two
*quiescence* stores racing each other (§7, Finding 3).

**Score encoding:** every producer (P1-P13) routes its score through the
same `score_to_tt(raw_score, ply)` / `score_from_tt(stored, ply)` pair
(`search.rs:1439-1474`) before storing or after probing, using its own
local `ply` counter at the time of the call (`root_search_inner`'s root
store uses the literal `0`; `alpha_beta`/`quiescence` use their own `ply`
parameter; `spec_alpha_beta` uses its own recursion-local `ply`, explicitly
commented at `speculative.rs:255-257` as matching the same convention
`alpha_beta`'s `store_tt` uses). This is uniform across every producer and
was already the subject of a dedicated fix (issue #7, mate-score encoding)
earlier in the project's history — not re-audited here beyond confirming
every producer still goes through the same two functions rather than a
local reimplementation.

## 3. Consumer inventory

Every `state.tt.probe(...)` call that uses the result for a score/bound
cutoff (not merely a move-ordering hint):

| Consumer | Source | Depth gate | Trusts which producers |
|---|---|---|---|
| `spec_alpha_beta` probe | `speculative.rs:206-224` | `entry.depth >= depth as u8` | Any producer (P1-P8) — no producer-identity check. On `main` this is the exact shape PR #16's own regression test (`spec_group_tasks_do_not_store_candidate_scores_as_parent_exact_entries`) exists to guard, but only for the *specific* pattern PR #16 removed, not for the general case. |
| `alpha_beta` probe | `search.rs:598-623` | `entry.depth >= depth as u8` | Any producer (P1-P8; P9-P13 on PR #17's branch, but depth `0` can only satisfy `entry.depth >= depth as u8` when the *caller's own* `depth` is also `0` — i.e. this is exactly the `alpha_beta(depth=0) → quiescence` boundary, not a real risk of a depth-0 qsearch entry leaking into a real-depth `alpha_beta` cutoff) |
| `root_search` probe (`tt_mv` only) | `search.rs:416` | none (unconditional) | move-ordering hint only, not a score/bound cutoff — safe by construction regardless of producer |
| `quiescence` probe (PR #17 branch only) | `search.rs:1142-1163` | `qply == 0 && entry.depth == 0` for score/bound; unconditional for `tt_mv` | Score/bound: **only** P9-P13 (itself) — this is the fix that closed the CI-caught sign-flip bug. Move hint: any producer, safe by construction (§ PR #17's own body, "a wrong-layer suggestion can only misorder the move loop, not corrupt the result"). |

## 4. `Tt::store` replacement semantics

Full text of the function (`tt.rs:174-188`):

```rust
pub fn store(&self, hash: u64, entry: TtEntry) {
    let slot = self.slot(hash);
    let existing_data = slot.data.load(Ordering::Relaxed);
    let existing_key = slot.key.load(Ordering::Relaxed);
    if existing_key ^ existing_data == hash {
        let existing_depth = ((existing_data >> 25) & 0x7F) as u8;
        if entry.depth < existing_depth {
            return;
        }
    }
    let data = pack(&entry);
    slot.data.store(data, Ordering::Relaxed);
    slot.key.store(hash ^ data, Ordering::Relaxed);
}
```

### The three depth cases

- **`existing.depth > new.depth`**: rejected. `entry.depth < existing_depth`
  is true → early return, no write happens. This is the only case with any
  protection at all.
- **`existing.depth == new.depth`**: **accepted, unconditionally.**
  `entry.depth < existing_depth` is false, so the function falls through to
  the unconditional write below. **No preference for `Bound::Exact` over
  `Lower`/`Upper`. No generation/producer/thread-identity check. No
  comparison of score, move, or any other field.** Whichever caller's
  `data.store` + `key.store` pair physically completes last is what a
  subsequent `probe` sees, full stop.
- **`existing.depth < new.depth`**: accepted (deeper always wins — correct,
  intended depth-preferred behavior).

This confirms, directly from the source rather than by inference, the
question the user flagged as highest priority: **equal-depth replacement is
last-physical-writer-wins**, with none of last-writer-wins/first-writer-wins/
bound-preference/generation-preference/Exact-preference implemented as an
alternative — "last writer wins" isn't one of several candidate policies to
check for, it is the *only* thing that happens at equal depth, by omission
rather than by an explicit designed choice.

### Read-check-then-write is itself not atomic (TOCTOU)

The `existing_data`/`existing_key` read and the `data`/`key` write are four
**separate** `Ordering::Relaxed` operations, not a single atomic
read-modify-write (no CAS, no lock). Between the depth check and the write
below it, an arbitrary number of other threads can freely write to the same
slot. This means the depth-preferred guarantee itself has a race window: two
threads can both read the same `existing_depth`, both decide their own entry
qualifies (either because both are deeper, or because both are equal), and
both proceed to write — the *check* provides no mutual exclusion, only the
final physical write order decides the outcome. This is consistent with the
function's own doc comment ("depth-preferred: keep deeper results") which
describes the *intent*, not a stronger guarantee the implementation doesn't
actually provide under concurrency.

### Atomic ordering

Every load and store in `tt.rs` uses `Ordering::Relaxed` — no
`Acquire`/`Release`/`SeqCst` anywhere in the file. This is consistent with
the file's own stated design (the XOR-trick is explicitly built to tolerate
torn/reordered reads by detecting them, not to prevent them), but it does
mean there is no memory-ordering guarantee that a `probe` on one thread
observes a `store` from another thread in program order beyond what
`Relaxed` alone provides (no ordering relative to *other* memory operations
around the store/probe). Given the entire entry is reconstructed from a
single `data` word plus a `key` consistency check, and no other shared state
is read/written alongside a TT operation, this looks intentional and
sufficient for the XOR-trick's own correctness — flagged here for
completeness per the user's explicit ask, not as a new finding.

### Can fields from different writers end up mixed in one read? No — proven, not assumed

This was the most safety-critical sub-question to close out. Consider two
concurrent writers A and B to the same slot, each executing
`data.store(their_data)` then `key.store(hash ^ their_data)`. Enumerate the
possible interleavings of the four resulting atomic operations
(`A.data, A.key, B.data, B.key`, all orderings consistent with each thread's
own program order):

- If **no write to `data` occurs after a writer's own `key` store**, the
  final `key` was computed as `hash ^ (that writer's own data value)`, and
  since nothing wrote `data` afterward, the final `data` still equals that
  same value → `key ^ data == hash` → **a complete, internally consistent
  entry from that one writer**, read correctly.
- If **some writer's `data` store lands after another writer's `key`
  store**, the final `key` (computed from an earlier data value) no longer
  matches the final `data` (a later write) → `key ^ data != hash` → **read
  as a miss**, not as a mixed entry.

There is no interleaving that produces a `key`/`data` pair whose XOR
coincidentally equals `hash` while containing fields from two different
writers, because `key` is always `hash ^ (exactly one writer's own data
snapshot)` — never a function of two different data values at once. This
generalizes to N concurrent writers by the same argument (the *last*
`key`-store determines pass/fail, and it only passes if no `data`-store
follows it). **Conclusion: races on `Tt::store` can produce stale entries,
lost writes, or spurious misses — never a Frankenstein entry with e.g. one
writer's score and another's move.** This closes out the field-mixing
concern the user raised as "非常に重要" — it does not happen, by
construction of the XOR-trick, independent of anything found elsewhere in
this audit.

## 5. Concurrent writer matrix

"Concurrently" here means: can genuinely overlap in wall-clock execution,
not merely "both eventually run during the same search." Reasoning, not a
live trace.

| Pair | Concurrent? | Same hash possible? | Same depth possible? | Basis |
|---|---|---|---|---|
| P2-P6 (alpha_beta, node N) vs. P2-P6 (alpha_beta, sibling node) | **Yes** | Only via transposition (distinct first moves from the same parent produce distinct immediate children, but deeper descendants can transpose) | Yes, if the transposed nodes are reached at the same remaining depth | `into_par_iter()` (`search.rs:875-900`) genuinely runs siblings on separate rayon global-pool threads; `board.hash()` is a pure Zobrist function of position only (`board.rs:123-124`, incremental updates keyed off piece/hand/side state, no move-count or path component) — confirmed, not assumed, so transposition-collision at the hash level is structurally real |
| P2-P6 vs. P7/P8 (spec_alpha_beta) | **Yes** | Only via transposition between the main search's own exploration and a live speculative task's subtree | Yes, same mechanism | `SpecGroup::spawn` (called before `root_search`, `search.rs:1338-1340`) runs on a fully separate dedicated pool (`speculative.rs:91`) that overlaps in wall-clock with `root_search`'s own dispatch for the same iterative-deepening depth — this is exactly PR #16's fixed pattern, minus the guaranteed-parent-hash collision that made it provable without a trace |
| P7/P8 vs. P7/P8 (sibling spec tasks, same `SpecGroup`) | **Yes** (each candidate move runs on its own pool worker) | Only via transposition between two *different* top_n candidates' subtrees (their own immediate roots never collide — distinct single moves from the same parent are always distinct positions — but descendants can) | Yes, same mechanism | `SpecGroup::spawn`'s `.map(...)` spawns one `state.pool.spawn(...)` closure per candidate (`speculative.rs:77-137`), all running concurrently on the same 3 (production default) dedicated-pool threads |
| A **promoted** spec task (kept alive past `SpecGroup::drop`, `speculative.rs:143-145`) vs. the **next** iterative-deepening depth's new `SpecGroup` + `root_search` | **Yes** | Same mechanism, wider window since the promoted task can outlive its own depth's iteration | Yes | `SpeculativeSearcher::search`'s loop (`search.rs:1336-1408`) spawns a new `SpecGroup` and calls `root_search` again before necessarily waiting for a promoted predecessor task to finish — nothing in the loop joins on a promoted task before proceeding |
| P9-P13 (quiescence, PR #17 branch) vs. P9-P13 (quiescence, different leaf) | **Yes** | Only via transposition between two concurrently-running `alpha_beta(depth=0)` leaves | Yes — **always** `depth=0` for both, by construction (§2) | `MIN_SPLIT_DEPTH = 3` (`search.rs:49`) means YBW splitting is active from depth 9 down through depth 3, so a depth-9 search tree has many nested levels of real parallelism feeding many concurrent depth-0 leaves; this pair requires **no** `SpeculativeSearcher` involvement at all — see §7 Finding 3 |
| P9-P13 vs. P7/P8 | **Yes** | Only via transposition between a spec task's subtree and a quiescence leaf reached from the main search | **No** — P7/P8 never store at depth `0` (early-return to a bare `evaluate()` before any store, `speculative.rs:196-198`); P9-P13 only ever store at depth `0`. Different depths → `Tt::store`'s depth-preferred rule always resolves this pair the same way regardless of physical write order (whichever has depth ≥1 always wins over the depth-0 entry) — **not** an equal-depth race | Confirmed from the depth values in §2 directly |
| P9-P13 vs. P2-P6 (same alpha_beta subtree, different node) | **Yes**, same as the general YBW case | Only via transposition | **No** — same reasoning as the row above (P2-P6 always depth ≥1) | Confirmed from depth values |

## 6. Race classification (A/B/C/D)

- **A. Correctness race** (unrelated `Exact` entry corrupts a cutoff, wrong
  result): **one confirmed instance that isn't actually a race** — Finding 4
  (§7) is a single-producer, deterministic control-flow bug (abort-time
  `Bound::Exact` mislabeling in `root_search_inner`/`alpha_beta`), proven
  without needing concurrency at all. Among the *concurrency-dependent*
  candidates, none found with a proof of occurrence. The read-side gates in
  place (`alpha_beta`'s `entry.depth >= depth as u8`; PR #17's
  `qply == 0 && entry.depth == 0`) do prevent a *cross-depth* correctness
  bug of the exact shape CI already caught once (that was fixed on PR #17's
  branch already). Whether an *equal-depth* race can still produce a
  correctness-affecting wrong cutoff (as opposed to a search-quality
  difference) depends on whether the two racing producers' scores are
  semantically different for the same nominal position — see Finding 2/3
  below; classified **plausible A/B boundary**, not confirmed A.
- **B. Search-quality race** (`tt_move`/node-count/bestmove shifts by
  completion order, not correctness): this is the well-evidenced category —
  PR #16's fix and PR #17's regression data are both consistent with this
  shape. The mechanism candidates in §7 are classified here.
- **C. Benign nondeterminism** (different writer, same semantic value, no
  real-world effect): the NMP verification search (`search.rs:710-726`) and
  the singular-extension probe (`search.rs:770-781`) both recurse into the
  *same* hash at a strictly *shallower* depth than the enclosing call will
  eventually store — naturally depth-ordered against their own enclosing
  call, not an equal-depth race with themselves. They can still equal-depth
  race with *other* unrelated producers reaching the same shallower depth by
  transposition, which folds back into Finding 1/2/3, not a separate benign
  category of its own.
- **D. Expected parallel-search nondeterminism** (not fixable without
  serialization, not necessarily worth fixing): YBW's own move-order/
  completion-order effects on *which move gets explored first* and thus
  *which alpha/beta window a given node is first reached with* are inherent
  to parallel alpha-beta and pre-date this audit entirely — SpecTopN=0 being
  clean is not evidence this category is absent, only that whatever's in
  this category doesn't manifest as bestmove/node-count variance in the
  measured corpus at that setting. Not in scope to fix.

## 7. Plausible-but-unproven races

Each stated as: mechanism, why it's structurally possible, why it is *not*
elevated to confirmed.

**Finding 1 — spec-vs-main transposition collision (the "PR #16-shaped"
residual).** A live `SpecGroup` task (P7/P8) and the concurrently-running
main search (P2-P6, or P1 at the root) can reach the same board position via
different move orders (transposition) while the position is still within
both producers' respective remaining-depth window, landing at *equal*
`Tt::store` depth (§5 first two rows show this is depth-arithmetically
possible: a spec task starts one full ply "ahead" — `depth+1` at spawn vs.
the main search's `depth` for that iteration — so a transposition reached
`k` main-search plies later and `k+1` spec-task plies later lands both
producers at the same remaining depth). This is the most direct candidate
for PR #16's 3 residual positions (`king_danger_nyugyoku_full_army`,
`_insufficient_pieces`, `_insufficient_points`) — **all three are
`king_danger`/`nyugyoku` (king-safety/impasse-declaration) positions**,
which structurally tend to have unusually dense transposition graphs (few
forcing lines, many reorderable king/general shuffles), consistent with
transposition-driven collisions being more likely there than in tactical or
opening positions. Per the user's explicit instruction, the parent-hash path
PR #16 already removed is **not** re-nominated as the cause here — this is a
distinct, deeper-in-the-tree collision, not the same site.

Not confirmed: no trace exists showing this transposition actually occurred
for these specific positions in an actual run; the argument is structural
(the mechanism is real and reachable) but not position-specific proof.

**Finding 2 — spec-vs-main window mismatch (a variant, not requiring a
"bug").** `spec_alpha_beta` always starts each candidate task with a full
open window (`-1_000_000, 1_000_000`, `speculative.rs:104-112`), while the
main search reaches the same eventual position through whatever
progressively-narrowed alpha/beta window its own path produced
(`search.rs`'s aspiration windows, null-window YBW probes, etc.). Fail-soft
alpha-beta's stored `Bound` and `score` at a given node are a function of
the *search window*, not the position alone — so even two fully-correct,
non-buggy searches of the *same* transposed position at the *same* depth
can legitimately disagree on `Bound`/`score` for that node. `Tt::store` has
no way to know one context is "more informative" than the other; whichever
physically writes last wins regardless. This reframes part of Finding 1:
even without any implementation defect, the *architecture* (spec tasks
always using a wide window) guarantees that spec-vs-main equal-depth
collisions carry semantically different, both-legitimate values — which is
exactly the ideal-finding shape the user described, minus the trace proving
it fired on a specific run.

**Finding 3 — quiescence-vs-quiescence (PR #17-only, and *not*
`SpeculativeSearcher`-dependent).** As established in §2/§5: PR #17's 5 new
store sites (P9-P13) are the *only* producers that ever write depth `0`, and
every one of them is reachable purely through `alpha_beta`'s existing YBW
parallelism (`MIN_SPLIT_DEPTH = 3`, well within the depth-9 evaluation
corpus's range) with **zero** dependency on `SpecTopN` or
`SpeculativeSearcher` being active at all. Two concurrently-running
`alpha_beta(depth=0)` leaves reaching the same position via transposition
(different move orders, `board.hash()` position-only as established in §5)
would race on the exact same equal-depth/no-tiebreak `Tt::store` path.

This has a direct, load-bearing consequence for how PR #17's own
`SpecTopN=0` control result should be read: that control shows the
*speculative-task code path* is not required to reproduce the new variance
— it does **not** show that *speculative concurrency* (as opposed to plain
YBW concurrency) is the enabling factor. `SpecTopN>0` adds real competing
CPU load from `SpecState::pool`'s worker threads running concurrently with
the same physical cores YBW's global-pool workers use; that contention could
change the *timing* of YBW's own internal races enough to expose a
collision that is too rare/fast to hit at `SpecTopN=0`'s lower-contention
baseline, without the *speculative code itself* being where the collision
happens. **This is the audit's own instance of "correlation != proof"
applied back onto the PR #17 write-up's existing claim** — the claim that
the effect is "confined to the speculative-concurrency channel" should be
read as "requires `SpecTopN>0`'s contention profile to manifest," not
"requires speculative *code* to be the colliding producer." Both P2-P6
self-collision (pre-existing, all versions of `main`) and P9-P13
self-collision (PR #17-only) are consistent with the observed
`SpecTopN=0`-clean / `SpecTopN=3`-dirty pattern.

Not confirmed: same caveat as Finding 1 — structurally real, not traced.

**Finding 4 — abort-time `Bound::Exact`/`Upper` mislabeling (confirmed, not
a race).** This one does not belong in the "plausible, unproven" bucket —
it's provable from control flow alone with no concurrency involved, so it's
placed here only because it was found during the same pass, not because it
shares the evidentiary status of Findings 1-3.

`root_search_inner` (`search.rs:511-558`): the move loop breaks on
`state.budget.should_abort()` (`search.rs:527-529`) *before* updating
`alpha`/`best_move` from the in-flight move's score — so a corrupted
0-from-abort score never pollutes `alpha` directly. But moves *after* the
aborted one in `ordered` are never attempted at all, and the function still
falls through unconditionally to:

```rust
if let Some(m) = best_move {
    let bound = if alpha >= hi { Bound::Lower } else { Bound::Exact };
    state.tt.store(board.hash(), TtEntry { score: score_to_tt(alpha, 0), depth: depth as u8, bound, mv: Some(m) });
}
```

`alpha >= hi` being false only tells you *no fail-high was observed among
the moves actually tried* — it does not tell you every move was tried.
Labeling this `Bound::Exact` claims "the true score of this position is
exactly `alpha`," which is only justified once every legal move has been
examined without a fail-high. If the deadline hit after move 3 of 7,
moves 4-7 were never even started: `alpha` reflects only what the tried
moves achieved, so the true score is *at least* `alpha` (an untried move
could still be better) — the honest label for that partial information is
`Bound::Lower`, not `Exact`, and definitely not `Upper` (nothing here
supports "true score ≤ alpha"). The current code cannot express that
distinction because it never checks *why* the loop stopped, only what
`alpha` happened to reach.

`alpha_beta`'s two mid-function loops have the identical shape:

```rust
// nw_results sequential pass (search.rs:903-905)
for (m, nw_score, _idx) in nw_results {
    if state.budget.should_abort() { break; }
    ...
}
// sequential tail pass (search.rs:978-980)
for (j, &m) in rest[seq_start..].iter().enumerate() {
    if state.budget.should_abort() { break; }
    ...
}
// then, unconditionally (search.rs:1068-1073):
let bound = if best_score > orig_alpha { Bound::Exact } else { Bound::Upper };
store_tt(state, hash, best_score, depth, bound, best_move, ply);
```

Same issue, two ways it can under-search: (a) the tail loop can `break`
before ever attempting some of `rest[seq_start..]`, or (b) the `nw_results`
loop's `break` can skip the **full-depth re-search** a null-window-probe
fail-high (`nw_score > alpha`, `search.rs:910-926`) requires to get an
accurate score — the sibling was searched at a null window, but its real
contribution to `best_score` was never resolved. Either way, `best_score`/
`bound` at the final `store_tt` reflects a subset of the legal moves, not
all of them, yet is stored exactly as if the search had run to natural
completion. And here **both** branches of `if best_score > orig_alpha {
Exact } else { Upper }` are unjustified under abort, in opposite
directions: if some tried move already raised `best_score` above
`orig_alpha`, the honest label is `Lower` (some move achieves at least this;
an untried one could still do better), not `Exact`. If no tried move raised
it, `Upper` claims "no move beats `orig_alpha`" — but that's only true of
the moves actually examined; an untried move could still exceed it, so
`Upper` is just as unjustified as `Exact` would have been here. Under a
premature abort, the incomplete data doesn't support *any* of the three
`Bound` variants — the only fully honest response is not to store, which is
exactly what `spec_alpha_beta`'s explicit `return 0` already does.

Contrast with `alpha_beta`'s *own* abort check right after its first child
(`search.rs:815-817`, `if state.budget.should_abort() { return 0; }`) —
that one is correct: it returns before reaching any store. The two
mid-function loops just don't have the equivalent guard before their shared
fall-through to the final store.

**Consequence, and why this is worth taking seriously despite being "just" a
mislabeled bound**: every consumer (§3) treats `Bound::Exact` as
unconditionally trustworthy (`return adj` with no further checking, both in
`alpha_beta`'s probe and `spec_alpha_beta`'s probe). The realistic exposure
is `alpha_beta`'s interior-node store (`search.rs:1073`), not
`root_search_inner`'s root-hash store: the root entry gets displaced almost
immediately by the *next* iterative-deepening depth's own store (strictly
greater `depth`, always wins under `Tt::store`'s depth-preferred rule, §4),
and the root position isn't re-probed within that same search. An interior
node's mislabeled entry has no such automatic replacement guarantee — it
can sit at whatever depth it was stored at until something deeper
transposes into the same hash, and until then it's available to feed a
wrong cutoff in any node (later in the same search, or later in the same
game — the shared TT persists until `usinewgame`'s `Tt::clear()`,
`tt.rs:216-221`) that transposes into it. This is a plain correctness
concern, not a search-quality one (Category A in §6's classification) —
though its practical exposure window is narrow (only fires right at a
deadline, and only when at least one move already completed before the
abort).

## 8. Benign / expected nondeterminism

Covered under Race Classification C/D above (§6) — the NMP-verification and
singular-extension self-recursions are naturally depth-ordered against their
own enclosing call and not a new source; YBW's inherent move-order effects
on which alpha/beta window a node is first reached with are pre-existing and
out of scope to eliminate.

## 9. Relationship to PR #16

PR #16 removed exactly one guaranteed-collision producer
(`SpecGroup::spawn`'s own closure store to the parent hash, before this
audit's P7/P8 existed in their current recursive form) — a genuine, provable
race because every candidate task in a group wrote to the *identical* parent
hash by construction, no transposition luck required. That certainty is why
it was provable without a trace and safe to fix directly. Everything found
in this audit (Findings 1-3) requires a transposition to actually occur,
which this static pass cannot confirm did or didn't happen for any specific
position. The 3 residual positions PR #16's own evaluation left unchanged
are consistent with Finding 1/2 (spec-vs-main transposition collision,
`king_danger`/`nyugyoku` positions structurally favoring dense transposition
graphs) but not confirmed to be caused by it specifically — per the user's
instruction, the already-removed parent-hash path is not re-nominated.

## 10. Relationship to PR #17

Restated from §2/§7 Finding 3: PR #17 adds a new depth-`0` producer
(quiescence) that (a) cannot corrupt or be corrupted by any real-depth
producer thanks to its own `qply == 0 && entry.depth == 0` read gate
(confirmed safe, §3), but (b) **can** race with itself — other concurrently
running `quiescence(qply=0)` calls from transposed positions — through
`alpha_beta`'s pre-existing YBW parallelism alone, independent of
`SpeculativeSearcher`.

Static-topology verdict, per the user's requested trichotomy:
- **Plausible**: quiescence-vs-quiescence self-collision (Finding 3);
  spec-vs-main and spec-vs-spec collisions interacting with quiescence's new
  depth-0 entries indirectly by changing overall TT occupancy/contention
  timing.
- **Impossible**: quiescence's depth-0 entries being corrupted *by*, or
  corrupting, any real-depth (≥1) producer's cutoff decision — ruled out
  directly by the depth values in §2 and the read-side gate in §3, not
  merely unlikely.
- **Confirmed**: none — no trace exists.

**"PR #17 qsearch TT race is the cause" is explicitly not concluded here** —
per the user's own instruction, correlation (worse repeats-mode numbers)
is not proof, and this audit's static reading identifies a *plausible*,
mechanistically well-formed candidate (Finding 3) without confirming it
fired.

## 11. Fix options, ranked

**Finding 4 (§7) is not in this table** — it's not one of the race-shaped
fix options the user asked to rank, and per §9 of the user's instructions
this audit implements nothing regardless. Noted here only because it's the
cheapest, most clearly-justified candidate if/when any fix is greenlit: add
the same `if state.budget.should_abort() { return <without storing> }`
guard `spec_alpha_beta` already uses, to `root_search_inner` and both of
`alpha_beta`'s mid-function loops, before their respective final stores.
Single-file, no new state, no atomics, and unlike Finding 1/2/3 it doesn't
need the instrumented-replay step in §12 to confirm first — the bug is
provable by inspection alone.

| Option | Description | Correctness | Nondeterminism reduction | Memory | Strength risk | Complexity | Hot-path overhead |
|---|---|---|---|---|---|---|---|
| **D. Deterministic equal-depth replacement** | Add an explicit tie-break at `entry.depth == existing_depth` (e.g. `Bound::Exact` beats `Lower`/`Upper`; ties beyond that keep the existing entry, i.e. first-writer-wins, or some other total order) | Same or better (removes the current *arbitrary* choice) | Reduces variance for every producer pair at once, including Findings 1-3, without needing to identify which pair actually fires | None | Very low — a tie-break among semantically-comparable entries can only make the kept entry *more* informative on average, not less | Small — one extra branch in `Tt::store`, no new fields | Negligible (`~free`, comparing 2-bit `Bound` values already unpacked for the depth check) |
| **E. Main-writer priority** | Tag entries by producer class (spare 4 bits exist in the packed `data` word, `tt.rs:22`, currently unused); at equal depth, a speculative-origin entry never displaces a main-search-origin entry | Same or better | Directly targets Finding 1/2 (spec-vs-main); does not address Finding 3 (quiescence-vs-quiescence, no spec involvement) | None (spare bits already reserved) | Low | Medium — needs a producer tag threaded through every `store_tt`/`state.tt.store` call site (13 on the PR #17 branch) and `pack`/`unpack` | Negligible (a few more bits packed/unpacked, no new atomics) |
| **A. Speculative search read-only on shared TT** | `SpeculativeSearcher` reads the shared TT for move-ordering/probing but never writes to it; spec tasks keep their own private result-passing (already true via `SpecTask::result`, unaffected) | Same or better | Eliminates Finding 1/2 entirely (spec never writes) | None | Medium — spec tasks currently *contribute* useful deep entries to the shared TT (e.g. a promoted task's continued work after `promote()`, `speculative.rs:143-145`); losing that could reduce transposition-hit rate for the main search on the *next* depth iteration, a real (if hard to quantify without a gate) strength cost | Low — remove 2 store call sites (P7/P8), no new state | None (removes work) |
| **B. Producer/generation tag gates cutoff use** | Same tagging idea as E, but applied on the *read* side: a consumer only trusts a speculative-origin entry for a cutoff if some freshness/generation check passes, otherwise treats it as ordering-hint-only (same downgrade PR #17 already applies to cross-depth quiescence entries) | Same or better | Targets Finding 1/2 same as E, plus generalizes the pattern PR #17 already proved works (probe-side gating without touching the store side) | Small (a generation counter, or reuse the same spare-bits producer tag as E) | Low | Medium — same tagging plumbing as E, plus a read-side check per consumer (3 consumers, §3) | Negligible |
| **C. Fully separate speculative-only TT** | `SpecState` gets its own `Tt` instance, never shared with the main search | Same or better | Eliminates Finding 1/2 entirely, same as A, and simplifies reasoning (fewer producers on the shared table) | **2x** — a second full-size TT allocation, or a smaller dedicated one that would need its own sizing policy | Same trade-off as A (loses the promoted-task-feeds-main-search benefit) plus loses any speculative-task cache reuse *across* candidate tasks in the same group | Medium — new `Tt` plumbing through `SpecState`, `SpecGroup::spawn` | None on the main search's hot path; doubles TT memory footprint |

**Ranked by "minimum change, maximum race elimination" (the user's stated
preference):** D first — a one-branch change in a single function
(`Tt::store`) that improves every producer pair uniformly, including
Finding 3 which options A/B/C/E don't touch at all, without needing to first
prove *which* pair is actually firing. E and B are natural, low-cost
follow-ups once/if Finding 1/2 specifically needs to be closed further than
D alone achieves. A and C are higher-cost, address only the spec-vs-main
subset, and carry an unquantified strength-tradeoff risk (losing the
promoted-task TT contribution) that would itself need a gate to evaluate —
not preferred as a first move.

## 12. Recommended next action

1. **Do not implement yet** — per §9 of the user's instructions and this
   audit's own findings, no race meets the "proven, not merely plausible"
   bar. Findings 1-3 are mechanistically sound (each cites a specific,
   concrete interleaving and confirms the necessary preconditions — real
   concurrency, real hash collision possibility, real equal-depth
   possibility — directly from source) but none has a trace.
2. **Highest-value next step, if/when resumed**: an instrumented static
   replay — a debug-only logging shim on `Tt::store` recording
   `(hash, depth, thread_id, bound)` per call, run once through the existing
   `repeats`-mode fixed-depth gate at `SpecTopN=3`, then post-processed
   offline for same-hash/same-depth pairs with overlapping wall-clock spans.
   This would either promote a Finding to confirmed (enabling a targeted,
   minimal fix) or rule all three out (redirecting effort elsewhere). Not
   done in this pass — it requires new code and a live run, both outside
   this audit's static-only scope.
3. **If/when a concrete race is confirmed**, Option D (§11) is the
   recommended starting fix regardless of which specific pair turns out to
   be responsible, given it's the only option in the table that also covers
   Finding 3.
4. PR #17 remains draft, unaffected by this audit's outcome either way —
   this audit does not itself change PR #17's merge status.
5. **Finding 4 is a separate decision from the race-hunt this audit was
   scoped for** — it's confirmed, not merely plausible, and cheap to fix,
   but it wasn't what issue #32 asked to investigate and this audit doesn't
   implement anything per §9 of the instructions. Flagged for the user to
   decide whether it's worth a small standalone fix independent of whatever
   happens with Findings 1-3.
