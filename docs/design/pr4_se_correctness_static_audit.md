# PR #4 static correctness re-audit: singular-extension verification search

Status: static, read-only. No builds, no runtime measurement.
Source: `sekirei-next-strength`@`9b61ed4` (PR #4 rebased onto `main`@`0bb4221`),
`crates/sekirei-core/src/search.rs`.

Scope note: PR #4's actual diff is a single condition change (line 611,
`&& skip_move.is_none()` added to the TT-cutoff guard) plus one regression
test. This audit evaluates semantic correctness of the surrounding SE
machinery as exercised by that change — it does not propose expanding PR #4's
diff. Any issue found here is reported separately per the task's instruction
("実害の可能性がある未対処問題を見つけた場合は、PR #4へコメントまたは別issue
として記録してください。PR #4のdiffへ直接追加せず").

## What PR #4 actually changed

`alpha_beta`'s TT-probe cutoff (`search.rs:611`):

```rust
if entry.depth >= depth as u8 && skip_move.is_none() {
    // ... early-return / alpha-tightening from the cached entry
}
```

Before PR #4, this condition lacked `&& skip_move.is_none()`. A singular-
extension verification search (`search.rs:777-787`, called with
`skip_move = Some(tt_mv)` at the *same* hash, before any move is made) would
immediately re-hit the very TT entry that made it SE-eligible in the first
place (guaranteed deep enough and non-`Upper` by the eligibility filter,
`search.rs:769-774`), short-circuiting to a single node. `sing_ext` could
never observe a fail-low, so it was structurally always `0` — dead code. This
is a real, well-isolated fix, and the added regression test
(`singular_extension_verification_does_not_short_circuit_on_tt_hit`,
`search.rs:1878-1902`) correctly proves the verification search now performs
real work (`nodes() > 1`) instead of an instant TT hit.

## Finding: verification-search results are still written to the shared TT, unguarded — but a pre-existing replacement policy narrows this to a near-non-issue at current constants

This is the specific distinction the task asked to confirm — "TT cutoffを
避けること" (the read side, which PR #4 fixed) vs. "skip-move verificationの
結果を通常TT entryとして保存しないこと" (the write side). **Confirmed: these
are separate concerns, and only the first has an explicit guard.** However,
a first pass at this (since corrected — see below) overstated the practical
severity by not checking `Tt::store`'s actual replacement policy before
concluding the write path was unconditionally dangerous. It isn't, at
current constants — but the reason it isn't is incidental, not an enforced
invariant, which is itself worth recording.

### The mechanism

Every `store_tt(...)` call inside `alpha_beta` is unconditional with respect
to `skip_move` — none of the 5 call sites guard on it:

| Line | Context |
|---|---|
| `search.rs:840` | first-move beta cutoff |
| `search.rs:858` | single-legal-move (no siblings) terminal store |
| `search.rs:957` | YBW parallel-pass beta cutoff |
| `search.rs:1062` | sequential-tail beta cutoff |
| `search.rs:1079` | end-of-node store (no cutoff, full move loop completed) |

`store_tt` itself (`search.rs:1474-1492`) takes no `skip_move` parameter and
has no awareness of it — it's a plain, unconditional write helper.

A verification search (`skip_move = Some(tt_mv)`) runs the *entire* function
body of `alpha_beta` with `tt_mv` filtered out of `ordered`
(`search.rs:757-761`) — it is not a special-cased, read-only probe. It
therefore reaches one of the 5 `store_tt` sites above just like a normal
search node, and stores its result **under the same `hash`** as the
unrestricted position, since excluding one candidate move doesn't change the
board or its hash. The resulting `TtEntry` (`score`, `depth`, `bound`, `mv`)
carries no marker that it was computed with `tt_mv` excluded — it is
indistinguishable, to any future reader, from a normal, complete-move-set
search result.

### Why PR #4 is what activates this, even though the write code predates it

Before PR #4's fix, a verification search always hit the TT-cutoff at line
611 and returned *immediately*, at the top of the function, before reaching
any of the 5 `store_tt` sites (all of which are past the move-generation and
move-loop code). The poisoning pathway existed in the source but was
**unreachable in practice** for `skip_move.is_some()` calls — the dead-code
bug PR #4 fixes and the TT-poisoning gap this finding describes were, before
PR #4, two bugs that cancelled out from an external-behavior standpoint (the
verification search never ran long enough to poison anything, because it
never ran at all). **PR #4's fix makes the verification search actually
execute — which is exactly what newly exercises the unguarded `store_tt`
calls on every SE-eligible node.** This is not a defect introduced by PR #4's
diff itself (the `store_tt` calls are pre-existing, unmodified by PR #4), but
it is a regression risk specifically *activated* by PR #4's change, and
should be evaluated alongside it rather than filed as a fully independent,
unrelated concern.

### `Tt::store`'s replacement policy — the mitigant I initially missed

`Tt::store` (`crates/sekirei-core/src/tt.rs:174-188`) is **depth-preferred**,
not always-replace:

```rust
if existing_key ^ existing_data == hash {
    let existing_depth = ...;
    if entry.depth < existing_depth {
        return;   // reject shallower writes to an occupied slot
    }
}
```

A write is rejected outright (no score, bound, *or* `mv` is stored) whenever
the incoming `entry.depth` is strictly less than what's already in that slot.

Cross-checking this against the verification search's own depth (`depth/2`,
`search.rs:782`) and the SE-eligibility floor that must already hold for the
verification search to run at all (`tt_se_depth >= depth - 3`,
`search.rs:774`): for **every** `depth >= SE_MIN_DEPTH` (`= 8`,
`search.rs:70` — the only depths at which SE, and therefore a verification
search, ever fires), `depth/2 < depth - 3` holds:

| depth | verification store depth (`depth/2`) | eligibility floor (`depth-3`) | verification store wins? |
|---|---|---|---|
| 8 | 4 | 5 | no |
| 9 | 4 | 6 | no |
| 10 | 5 | 7 | no |
| 12 | 6 | 9 | no |
| 15 | 7 | 12 | no |
| 20 | 10 | 17 | no |

Since SE eligibility *requires* a genuine entry already at that exact hash
with depth `>= depth - 3` (that entry's `mv` is literally what `tt_mv` is —
`search.rs:598-604`, `769-774`), the verification search's own store at
`depth/2` can never out-rank the entry that made it eligible in the first
place, at any depth SE can actually trigger at. **The "same-slot self-
poisoning" scenario I originally wrote up does not occur in practice at
current constants** — the depth-preferred policy rejects it every time,
before the write's `score`/`bound`/`mv` are packed or stored at all.

### Why this protection is incidental, not an enforced invariant — and the narrower exposure that remains

The inequality above holds only because `SE_MIN_DEPTH = 8`, the verification
depth formula is `depth/2`, and the eligibility floor is `depth - 3` — three
independently-tunable constants/formulas that happen to keep the
verification store permanently behind the eligibility entry. Nothing in the
code documents this as a load-bearing relationship. If a future change lowers
`SE_MIN_DEPTH`, loosens the eligibility floor (e.g. `depth - 5`), or changes
the verification depth formula (e.g. to reduce SE's own cost), this
protection can silently stop holding — with no test to catch the regression,
since the current regression test doesn't touch TT contents after the call
(see below).

Two narrower, still-real exposures remain even given the table above:

1. **A different, unrelated hash bucket collision.** `Tt::store`'s rejection
   only applies when the occupied slot's key matches (`existing_key ^
   existing_data == hash`). If some other position's entry occupies that
   *slot index* at store time (a generic hash-table collision, not specific
   to SE), the depth check compares against that unrelated entry instead —
   this is a pre-existing hazard of any fixed-size TT design, not something
   SE-specific, and out of scope here.
2. **The `mv` field as a degraded ordering hint, only in the narrow window
   where a store *does* win.** The table above covers the entry that
   established eligibility; it doesn't cover every possible entry state.
   Should the verification store ever win against a *weaker* pre-existing
   entry than the eligibility one (e.g., the slot's content changed between
   the SE probe and the verification store — a cross-thread YBW-sibling
   transposition into the same hash during that window is the realistic
   mechanism, given `alpha_beta`'s own parallel dispatch), the parent node
   still overwrites the slot again at its own full depth `D` shortly after
   (`search.rs:1079` or an earlier cutoff site) once its move loop completes
   — `D` is always `> depth/2`, so the parent's own store always wins the
   depth check and restores a complete entry. **The exposure window is
   therefore bounded to "between the verification store and the parent
   node's own store, for probes arriving on a different thread than the one
   computing node `H`"** — not "until the entry ages out," and not the
   broad, systemic issue the first draft of this doc described.

### Why the existing PR #4 regression test doesn't cover this either way

`singular_extension_verification_does_not_short_circuit_on_tt_hit` seeds a
genuine entry (`store_tt(&state, hash, 50, 4, Bound::Exact, Some(tt_mv), 0)`)
and asserts only that more than one node was explored
(`state.budget.nodes() > 1`). It asserts nothing about the TT's contents
afterward. Per the analysis above, in this test's own specific setup (seeded
depth 4, verification search also depth 4 — the test mirrors the SE call's
*shape*, not a full depth-8+ SE-triggering search) the seeded entry's depth
equals the verification call's own `depth` argument, so `entry.depth <
existing_depth` is false (equal, not less) and **the seeded entry in this
particular test would in fact be overwritten** — this test's own numbers
don't reflect a real `depth >= 8` SE trigger, they're a simplified
direct-call shape chosen to exercise the dead-code fix cheaply. It's a
reasonable test for what it was written to prove (the read-side fix); it
doesn't extend to prove anything about TT contents post-call.

### Minimal fix (design-only, not applied to PR #4's diff in this pass)

Given the risk is now understood to be narrow rather than systemic, the case
for a code change is about closing an *incidental* protection into an
*explicit* one, not about fixing an active bug. Two options, in order of
surgical minimality:

1. Guard every `store_tt` call site with `skip_move.is_none()` (5 call sites,
   mechanical, makes the invariant explicit regardless of future constant
   tuning).
2. Thread `skip_move` into `store_tt` itself and early-return when it's
   `Some(_)` (1 call site, DRY, less error-prone against a future 6th call
   site being added without the guard).

Either preserves the verification search's own internal alpha-beta pruning
(it still needs its own `return`s to terminate normally at `beta` — nothing
above proposes removing those, only the *store* half of each cutoff site).

### Test needed

A regression test that: (a) seeds a genuine TT entry at a realistic SE-
triggering depth (`>= SE_MIN_DEPTH`, not the simplified depth-4 shape the
existing test uses), (b) runs the verification-search-shaped call, (c)
re-probes the same hash with `skip_move = None` afterward, and (d) asserts
the entry is unchanged. This both documents the depth-preferred policy's
current protection *as an explicit, checked invariant* (so it fails loudly if
`SE_MIN_DEPTH`/the verification-depth formula/the eligibility floor ever
drift out of their current safe relationship) and, if a `skip_move` guard is
added per the fix above, would keep passing independent of constant tuning.

### Status

**CORRECTED after review**: the original static-only read (this doc's first
draft) concluded the write path was unconditionally dangerous by reading
`store_tt`'s wrapper but not `Tt::store`'s actual replacement policy. Having
now read `tt.rs:174-188` and checked it against `SE_MIN_DEPTH = 8`, the
same-slot self-poisoning scenario is **not exercised at current constants** —
confirmed by direct arithmetic, not merely inferred. What remains is (a) an
*incidental*, undocumented protection rather than an enforced invariant, and
(b) a narrower cross-thread-transposition exposure window bounded by the
parent node's own subsequent store. Severity downgraded accordingly — see
summary table.

## Other Phase 4 checklist items — no issues found

- **`skip_move.is_some()` used elsewhere?** No — grep confirms its only
  producer is the SE verification call (`search.rs:786`) and its only
  consumers are the TT-cutoff guard (line 611), the ProbCut guard (line 664,
  `skip_move.is_none()` — correctly also excludes verification-search nodes
  from ProbCut), the SE-eligibility filter itself (line 770), and the
  move-list filter (line 757-761). Single, well-contained purpose.
- **Move-ordering info (`tt_mv`) still used inside a verification search?**
  Yes — `tt_mv` is still read from the TT probe and used for `order_moves`
  (line 746-754) even when `skip_move.is_some()`; only the excluded move
  itself is filtered out of the move list afterward. This looks correct and
  intentional (better ordering of the remaining candidates).
- **Exact/Lower/Upper bound semantics**: internally consistent given the
  excluded move set — the actual defect is that the exclusion isn't recorded
  on the stored entry (covered above), not that the bound classification
  logic itself (`orig_alpha` comparison, `Bound::Lower` on cutoff) is wrong.
- **Mate-score ply normalization**: `score_to_tt`/`score_from_tt` are applied
  uniformly on every store/probe path, including verification-search calls —
  no PR #4-specific issue found.
- **Repetition / draw / mate-adjacent handling**: unmodified by PR #4's
  single-line diff; out of scope, no finding.
- **Verification window and `SE_MARGIN`**: `se_beta = (se_score -
  SE_MARGIN).max(alpha)`, verification window `(se_beta-1, se_beta)` — a
  standard null-window singularity probe, consistent with common SE
  implementations. No issue found.
- **Extension recursion / stacking cap**: nested SE-within-SE is structurally
  prevented — the eligibility filter requires `skip_move.is_none()`
  (`search.rs:770`), so a verification-search node never computes its own
  `sing_ext`. Whether extensions can still *stack* across successive
  *different* nodes along one forcing line (each independently qualifying for
  its own +1) has no explicit cap in what was read — flagged as a lower-
  confidence design observation (**NEEDS_RUNTIME_VALIDATION**, not a proven
  bug), since alpha-beta cutoffs and the outer iterative-deepening
  `max_depth` bound naturally limit runaway growth in practice.

## Summary

| Finding | Severity | Confidence | Action |
|---|---|---|---|
| `store_tt` has no `skip_move` guard; same-slot self-poisoning is currently prevented only *incidentally*, by the depth-preferred `Tt::store` policy combined with the specific values of `SE_MIN_DEPTH`/verification-depth/eligibility-floor, not by an explicit invariant. A narrower cross-thread-transposition exposure remains, bounded by the parent node's own subsequent store. | Low-Medium — not an active bug at current constants; a latent fragility that would silently reactivate if those constants are tuned independently in the future | High for the "not currently exercised" claim (direct arithmetic against `tt.rs`); Medium for the residual cross-thread window (plausible mechanism, not runtime-measured) | File as a tracked issue recommending an explicit `skip_move` guard + a depth-realistic regression test, so the protection stops being incidental; do not fold into PR #4's diff; PR #4 stays draft |
| Extension stacking across nodes, no explicit cap | Low / informational | Low-medium | No action; note for future search-tuning work |
| Everything else audited | No issues found | — | — |
