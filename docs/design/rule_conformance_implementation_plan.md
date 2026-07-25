# Rule-conformance implementation plan (design only — no code changed)

Status: **design/planning only**. Breaks the `known_missing` items tracked
by `crates/sekirei-core/tests/rule_conformance.rs` and its fixture corpus
into four concrete implementation issues, in priority order. No code was
written or changed to produce this document — every fact below was
confirmed by reading current source (cited by path/line), not assumed.

## Priority order and why

1. Continuous-check/max-moves → core API (smallest, fixes an existing bug,
   no new *rule* to implement, just relocates and completes logic that
   already partially exists).
2. Nyugyoku declaration (genuinely new rule logic, richest existing fixture
   coverage to build against).
3. Jishogi (genuinely new rule logic, but reuses #2's primitives — ordered
   after it deliberately, see its own "dependencies" below).
4. CSA/USI/training/match-runner result-vocabulary mapping (widest-reaching
   change, touches every crate in the workspace — done last so its
   canonical type can include the outcome variants #1–#3 introduce, rather
   than being redesigned once they land).

---

## Issue 1: Consolidate continuous-check/max-moves into a `sekirei-core` API

**Scope**: Move "is this repeated position a continuous-check loss (連続王手
の千日手) vs. an ordinary sennichite draw" and "has the max-move ceiling
been reached" out of `sekirei-match-runner`'s inline game loop into a
reusable, independently-testable `sekirei-core` function.

**Current duplication**: Today, `crates/sekirei-match-runner/src/main.rs`
resolves a repeated position to `(Outcome::Draw, moves, EndReason::Repetition)`
unconditionally (confirmed: no continuous-check branch exists in that
function at all) and a max-move ceiling to
`(Outcome::Draw, moves, EndReason::MaxMoves)` — both inline in match-runner,
neither exposed as a `sekirei-core` API. `rule_conformance.rs`'s own module
doc already documents this exact gap ("`sekirei-match-runner`'s repetition
handling ... always resolves a 4-fold hash repeat to `Outcome::Draw` — it
has no continuous-check special case at all"). The corpus already has two
engine-verified continuous-check fixtures
(`continuous_check_sennichite_black_checks`/`_white_checks`) whose
`expected_continuous_check_side`/`expected_result` fields document the
*correct* ruling but cannot be checked against a real decision function,
because none exists.

**Proposed core API**:
```rust
pub enum RepetitionVerdict {
    OrdinaryDraw,
    ContinuousCheckLoss(Color),  // the side that was continuously checking, and loses
}
pub fn classify_repetition(
    history: &[(u64 /* hash */, bool /* mover gave check this move */)],
) -> RepetitionVerdict;

pub const MAX_MOVES: u32 = 512;  // named constant, replacing scattered literal
                                  // 512s (search_ablation.rs, match-runner) with
                                  // one source of truth
pub fn max_moves_reached(ply: u32) -> bool { ply >= MAX_MOVES }
```

**Affected crates**: `sekirei-core` (new module/functions), `sekirei-match-runner`
(replace inline logic with calls to the new API), `crates/sekirei-core/tests/rule_conformance.rs`
(promote existing continuous-check cases from "raw facts only" to "assert
the real ruling").

**Required fixtures**: the two existing continuous-check cases are directly
reusable once the API exists — no new SFEN construction needed, just a new
assertion against `classify_repetition`'s output. Consider one additional
"ordinary sennichite, no check at all" negative case to confirm the API
doesn't over-fire (the corpus's original `plain_sennichite_no_check` case
already covers this from the raw-fact side; extending its assertion the
same way costs nothing extra).

**Test strategy**: reuse `rule_conformance.rs`'s existing move-replay harness
(`recorded_move_histories_are_legal_move_by_move`'s pattern) to build the
`(hash, gave_check)` history, then assert `classify_repetition` against
`expected_continuous_check_side`/`expected_result`.

**Compatibility risk**: low-medium. Purely additive at the `sekirei-core`
level; the risk is entirely in match-runner's *observable behavior change*
— a continuous-check repetition that today incorrectly resolves to a draw
will, after this change, correctly resolve to a loss for the checking side.
Any existing recorded match/gate results that happened to include such a
position would be retroactively "wrong" under the old rule — worth an
explicit note in whatever changelog/report accompanies this fix, not a
silent behavior change.

**Estimated change size**: small–medium (~100–150 new lines in
`sekirei-core` plus tests; ~20–30 line diff in match-runner replacing inline
logic with API calls).

**Dependencies**: none — can land independently and first.

---

## Issue 2: Nyugyoku (entering-king declaration) judgment

**Scope**: Implement the JSA-style 27/28-point entering-king declaration
rule: declaring side's king is in the enemy camp, not in check, has ≥10 of
its own pieces physically present in that camp, and meets the point
threshold (major piece = 5, minor = 1; hand pieces always count toward the
total, board pieces only if physically located in the enemy camp).

**Current duplication**: none — this logic does not exist anywhere in the
codebase (confirmed during the Sprint 1 provenance/USI audit and
reconfirmed during the Sprint 2 fixture-corpus work). Five dedicated
placeholder fixtures already exist and are deliberately, carefully
constructed (avoiding accidental stalemate and double-check, per
`tasks/lessons.md`'s 2026-07-25 entry): `nyugyoku_declaration_win_eligible`,
`nyugyoku_insufficient_points`, `nyugyoku_insufficient_pieces_in_enemy_camp`,
`nyugyoku_king_outside_enemy_camp`, `nyugyoku_in_check_cannot_declare` — all
currently asserted only for SFEN-parses/legal-moves-exist/in-check-matches-claim,
never for the actual eligibility verdict.

**Proposed core API**:
```rust
pub struct DeclarationEligibility {
    pub eligible: bool,
    pub points: u32,             // computed total, for debuggability
    pub pieces_in_camp: u32,     // computed count, ditto
    pub king_in_camp: bool,
    pub in_check: bool,
}
pub fn nyugyoku_declaration_eligible(board: &Board, side: Color) -> DeclarationEligibility;
```
Returning the computed intermediates (not just a bool) matters for
debugging fixture failures and for any future USI-level "why can't I
declare" diagnostic message — cheap to include now, expensive to retrofit
later.

**Affected crates**: `sekirei-core` (new logic), `sekirei-usi` (a real
"declare win" path needs *some* UI-level trigger — base USI has no
standard declare-win command; this needs its own small design decision,
flagged here as an open question rather than resolved by this plan),
`sekirei-match-runner` (recognize and score a legitimate declaration during
automated play), `rule_conformance.rs` (promote the 5 existing cases).

**Required fixtures**: the 5 existing cases are reusable as-is. Recommend
adding 2 tight boundary cases before implementing (exactly-10-pieces-in-camp,
exactly-at-the-point-threshold) — boundary conditions are exactly where a
hand-derived expected value is most likely to be wrong, so these should be
engine-verified (once the function exists) rather than hand-asserted.

**Test strategy**: unit tests per boundary condition (camp membership,
check status, piece count, point threshold) independent of the corpus,
plus the corpus-driven integration assertions.

**Compatibility risk**: medium. This is a wholly new way for a game to end;
any pipeline that doesn't expect it (self-play data generation, existing
gate/match tooling) needs to explicitly handle a new outcome kind or it
will silently mis-record it. Must be gated behind an explicit trigger
(not auto-detected mid-search) to avoid surprising existing automated runs.

**Estimated change size**: medium (~200–300 lines: camp-membership check,
point calculation, USI/match-runner integration points) plus the fixture
promotion.

**Dependencies**: none blocking; benefits from Issue 1's "extract core rule
logic, expose from sekirei-core" pattern but isn't blocked by it.

---

## Issue 3: Jishogi (mutual impasse) judgment

**Scope**: Determine mutual-impasse draw conditions — both kings effectively
"safe" in their own camps with neither side able to force a decisive
result, distinct from nyugyoku's single-side declaration. Thinnest existing
coverage: exactly one placeholder fixture
(`jishogi_mutual_impasse_boundary`, `expected_declaration_eligibility:
"pending_implementation"`).

**Current duplication**: none, doesn't exist.

**Proposed core API**: builds directly on Issue 2's primitives —
```rust
pub enum JishogiVerdict { NotApplicable, MutualImpasseDraw, OneSidedOnly(Color) }
pub fn jishogi_check(board: &Board) -> JishogiVerdict;
// internally: nyugyoku_declaration_eligible(board, Black) and
// nyugyoku_declaration_eligible(board, White), then the mutual/one-sided
// combination logic on top
```

**Affected crates**: same set as Issue 2 (`sekirei-core`, `sekirei-usi`,
`sekirei-match-runner`, `rule_conformance.rs`).

**Required fixtures**: only one exists today — this is the weakest-covered
category in the whole corpus. **Recommend expanding fixtures as a
prerequisite step**, before implementation, not as an afterthought: at
least one case per side's eligibility combination (both eligible → mutual
draw; one eligible/one not → not a mutual impasse, falls through to
whichever single-side rule applies; neither eligible → not applicable at
all).

**Test strategy**: same shape as Issue 2, composed from its per-side
primitive.

**Compatibility risk**: medium, same reasoning as Issue 2 (new way for a
game to end; needs explicit integration, not silent auto-detection).

**Estimated change size**: medium, but smaller than Issue 2 *if implemented
after it* — mostly the mutual/one-sided combination logic on top of
already-built point/camp primitives, rather than rebuilding them.

**Dependencies**: **depends on Issue 2 being implemented first.**
Implementing jishogi before nyugyoku would mean duplicating the point/camp
primitives now and reconciling two implementations later — strictly worse
than the reverse order.

---

## Issue 4: CSA/USI/training/match-runner result-vocabulary mapping

**Scope**: A single canonical mapping layer between the four independent
"how did the game end" vocabularies that exist in this codebase today, with
zero conversions between any pair of them currently.

**Current duplication (four, confirmed by reading each definition)**:

| Vocabulary | Location | Variants |
|---|---|---|
| `sekirei_csa::protocol::GameResult` | `crates/sekirei-csa/src/protocol.rs:58` | `Win, Lose, Draw, Aborted` |
| USI `gameover` | `crates/sekirei-usi/src/main.rs:602` | literal no-op: `"gameover" => {}` — carries no information at all today |
| `sekirei_train::csa::GameResult` | `crates/sekirei-train/src/csa.rs:27` | `BlackWin, WhiteWin, Draw, Unknown` (parsed from CSA log tokens `%TORYO`/`%TSUMI`/`%KACHI`→decisive, `%JISHOGI`/`%SENNICHITE`→`Draw`, `%CHUDAN`/`%ILLEGAL_MOVE`/`%TIME_UP`→`Unknown`) |
| match-runner `Outcome`/`EndReason` | `crates/sekirei-match-runner/src/main.rs:206,213` | `Outcome: E1Win, E2Win, Draw`; `EndReason: Resign, Win, IllegalMove, Repetition, MaxMoves, EngineError` |

No pairwise conversion exists between any two of these today.

**Proposed core API**: a canonical `sekirei_core::result::GameOutcome`
covering every real-world ending observed across all four vocabularies —
including the new variants Issues 1–3 introduce (continuous-check loss,
nyugyoku win, jishogi draw) — plus explicit, named conversion functions
to/from each of the four existing types (this project's own convention
elsewhere in `sekirei-core` favors explicit functions over blanket
`From`/`Into` trait webs where the mapping is lossy in one direction, e.g.
`sekirei_train::csa::GameResult::Unknown` has no unique corresponding
`GameOutcome` — several distinct core outcomes could have produced it).

**Affected crates**: **all of** `sekirei-core`, `sekirei-csa`, `sekirei-usi`
(implement the currently-no-op `gameover` using the canonical type),
`sekirei-train`, `sekirei-match-runner` — the widest-reaching of the four
issues.

**Required fixtures**: a new table-driven fixture enumerating every known
real ending (illegal move, ordinary repetition draw, continuous-check loss,
nyugyoku win, jishogi draw, resignation, time forfeit, protocol/engine
error) with its expected round-trip through each of the four external
vocabularies where a corresponding case exists.

**Test strategy**: table-driven round-trip tests
(canonical → vocabulary → canonical); explicitly test and document the
*lossy* directions rather than only the lossless ones (e.g.
`sekirei_train::csa::GameResult`'s `Unknown` collapses several distinct
canonical outcomes — the round-trip test for that direction should assert
"maps to `Unknown`," not attempt a bijection that doesn't exist).

**Compatibility risk**: highest of the four issues — touches every crate,
changes call sites throughout the workspace. Requires incremental rollout:
canonical type + conversions first (purely additive, no call-site changes),
then migrate one crate's call sites per subsequent commit, only removing
any of the four legacy vocabularies once nothing still depends on it.

**Estimated change size**: large in total scope, but decomposable into
small, independently-reviewable, always-compiling steps (mirrors the
"maintain a working state at every step" discipline used in
`docs/design/evalfile_reload.md`'s migration plan).

**Dependencies**: benefits from Issues 1–3 landing first, so the canonical
type's variant list is complete on first design rather than needing
revision once continuous-check/nyugyoku/jishogi outcomes exist. Recommend
implementing this **last**, matching the priority order requested and
independently justified by this dependency analysis.
