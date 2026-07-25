# Design: king-relative NNUE v2 ("Arm B")

Status: **design only — no implementation, no training**. This document specifies the feature set; it does not build or train it. Written 2026-07-25 as light, CPU/memory-free work while heavier processes are paused on this machine.

## 1. Current state (v1, confirmed by reading `crates/sekirei-core/src/nnue.rs`)

- Features are plain **piece-square**, not king-relative at all:
  ```rust
  // nnue.rs:334-337
  pub fn feature_index(sq: Square, kind: PieceKind, piece_color: Color, perspective: Color) -> usize {
      let opp_flag = (piece_color != perspective) as usize;
      sq.index() as usize * (14 * 2) + kind.index() * 2 + opp_flag
  }
  ```
  No king square is an input anywhere in this formula. The project's own `tasks/competitive_analysis.md:150` already names HalfKP as the known gap this document closes: *"HalfKP特徴量 | 現状: PS特徴量（王非依存）"*.
- `BOARD_INPUT = 81*14*2 = 2268`, `HAND_INPUT = 152` (38 thresholds × 2 hand-colors × 2 perspectives), `INPUT = 2420`, `L1 = 256`, `L2 = 32` (`nnue.rs:50-65`).
- `NnueAcc { values: [[i16; L1]; 2] }` (`nnue.rs:343-346`) is **already dual-perspective** — one `[i16; 256]` half per `Color`. `refresh()` (`nnue.rs:358-379`) fully recomputes from a mailbox + hand-count snapshot; `add_piece`/`remove_piece`/`add_hand`/`remove_hand` (`nnue.rs:384-419`) are O(1) and each other's exact inverse. `board.rs:441`'s comment ("no accumulator stack needed — the deltas are symmetric") is the invariant a king-relative scheme breaks (see §4).
- `sekirei-train` imports `feature_index`/`INPUT`/`L1`/`L2` directly from `sekirei_core::nnue` (confirmed: `trainer.rs` uses these at feature-building time) — training and inference share one function; changing it touches both crates in lockstep.
- `Board` (`board.rs:51-66`) derives `Clone`; YBW search splits clone the whole `Board` (accumulator included) at each split point, bounded by `ybw_max_siblings` (default 6) — accumulator-copy cost already scales with split count today, independent of anything this design changes (the accumulator itself stays ~1 KB regardless of `INPUT`'s size, see §3).
- Available building blocks for constructing/handling positions (from a targeted API lookup this session): `Board::pieces(color, kind) -> Bitboard` + `Bitboard::lsb()` to find a king square (no dedicated king-square accessor exists); `Board::hand(color) -> &Hand` + `Hand::get(kind) -> u8` for hand counts; `Bitboard::PROMOTE_BLACK`/`PROMOTE_WHITE` constants for enemy-camp/promotion-zone membership. No nyugyoku-specific point-value table exists (`eval.rs`'s `PIECE_VALUE` is centipawn search weights, not nyugyoku's 5/1 point scale) — irrelevant to this doc directly, but noted since the same exploration surfaced it.

## 2. Feature specification (Arm B)

Per the requested design: **only board-piece features become king-relative; hand-piece threshold features stay exactly as in v1** (global, not king-indexed) — this matches the literal spec (自玉位置×盤上駒 / 敵玉位置×盤上駒 / 持駒閾値特徴 listed as a separate, unchanged bullet).

- **Own-perspective half**: for each board piece, feature = f(own_king_sq, piece_sq, piece_kind, piece_owner-relative-to-perspective).
- **Opponent-perspective half**: same formula, using that perspective's *own* king square (each of `NnueAcc`'s two existing halves is already computed from that color's own vantage point — this generalizes directly, no new accumulator shape needed).
- **Piece kind/owner encoding**: keep v1's existing 28-wide code (`kind.index() * 2 + (piece_color != perspective)`) — 14 kinds × own/opp, unchanged.
- **Hand features**: unchanged from v1 — `hand_feature_index` (`nnue.rs:77-87`) stays king-independent.
- **Dual-perspective normalization**: each perspective's board-feature indices depend only on *that* perspective's own king square, never the other's — this is what makes the existing `[values; 2]` accumulator shape sufficient (no cross-perspective coupling to introduce).

### Feature index formula
```
index_v2(king_sq, piece_sq, kind, owner_code) =
    king_sq.index() * (81 * 28) + piece_sq.index() * 28 + kind.index() * 2 + owner_code
```
(Kings themselves are excluded from the piece-feature list under standard HalfKP — a king is the index anchor, not a feature-contributing piece, on either perspective.)

### Dimensions
- Board part: `81 (king squares) * 81 (piece squares) * 28 (kind/owner) = 183,708`.
- `INPUT_v2 = 183,708 + 152 (hand, unchanged) ≈ 183,860` — **~76× v1's 2268 board input**, in the expected range for a HalfKP-style shogi net (comparable in order of magnitude to established shogi engines' HalfKP feature counts).
- **Active feature count per position is unchanged**: still ~1 active board feature per piece per perspective (same sparsity as v1). Only the index *space* grows — this is a memory-footprint change, not a per-eval compute-complexity change. Worth stating explicitly since it's easy to (wrongly) read "76× input dimension" as "76× slower."

### Estimated weight size
`ft` table dominates: `183,860 * 256 * 2 bytes (i16) ≈ 94 MB` (vs. v1's ≈1.24 MB total). `l2`/`out` stay negligible (`2*256*32*4 bytes ≈ 65 KB`). This is a **shared, static table** (one copy per process, behind whatever weight-storage mechanism `docs/design/evalfile_reload.md` settles on) — it is **not** per-`Board` state, so it does not change the YBW per-clone accumulator-copy cost (still ~1 KB per clone, unchanged). Call this out explicitly — it's the natural wrong-worry when INPUT grows 76×.

### Estimated eval cost
- Incremental path: unchanged asymptotically — one extra multiply/add per active feature to compute the (now king-dependent) index; negligible relative to the existing per-feature `add_col`/`sub_col` cost (`nnue.rs:466-484`, already a 256-wide loop).
- `refresh()` cost per call: unchanged in absolute terms (still O(pieces on board)), but now fires on every king move for the moving perspective instead of only at `Board` construction (§4) — the one thing that needs empirical profiling once implemented, not something this design can size precisely today. King moves are a modest fraction of moves in typical positions, so the expected overhead is a small, concentrated increase, not a blanket slowdown — but this is exactly the kind of claim that must be validated by a real (deferred) benchmark, not asserted here.

## 3. King-move handling — accumulator stack vs. refresh-on-king-move

A king move changes the index formula's `king_sq` parameter for *that perspective's entire board-feature set simultaneously* — the v1 invariant "deltas are their own inverse, no stack needed" (`board.rs:441`) breaks specifically for king moves.

**Recommendation: refresh-on-king-move, not an accumulator stack.**
- When the moved piece is a king: trigger a full `refresh()` of **only the moving side's own perspective-half**, on both `do_move` and `undo_move`. The *other* perspective is untouched by a king move (kings aren't feature-contributing pieces under HalfKP on either half) — no update needed there at all, not even incremental.
- When the moved piece is not a king (normal move, capture, promotion, non-promotion, drop): both perspectives stay O(1) symmetric incremental sub/add, exactly like v1, just using each perspective's own (unchanged) king square in the index formula.
- This is the lazier of the two options: an accumulator stack (push/pop per ply, sized to max search depth × YBW clone factor) would avoid repeated refreshes but adds a new data structure and a depth-dependent memory cost that scales differently from today's clone-per-split model. Refresh-on-king-move reuses the existing `refresh()` machinery unchanged and only changes *when* it's called (on king moves, not just at `Board` setup) — recommended as the default; keep the stack as a documented fallback if profiling later shows refresh cost actually dominates search time.

## 4. Weight format

- New magic: `SEKIRW02`. Own fixed dimensions baked in at compile time — same convention as v1 (`INPUT`/`L1`/`L2` as compile-time `const`s), not stored per-file.
- **The loader must be able to identify both `SEKIRW01` and `SEKIRW02`**, even though a given running binary only ever *uses* one of them (see the next bullet). "Identify" here means: read the 8-byte magic and recognize which known format it names, so a mismatch produces "this file is SEKIRW01, this binary is built for SEKIRW02" rather than a generic "bad magic" error indistinguishable from a truly corrupt file. This also matters for tooling that isn't tied to one compiled architecture — e.g. `scripts/verify_weights_registry.py` or a future registry-audit script that needs to report which format each file on disk is, across a mixed v1/v2 fleet, without loading either into a live evaluator.
- **One `Engine` instance uses exactly one architecture at a time** (restating the no-coexistence decision explicitly, since it's the load-bearing assumption behind every other bullet here): do not attempt v1/v2 coexistence in one running binary. Supporting both simultaneously would require making `INPUT`/`L1`/`L2` runtime values instead of `const`s (so the same process could hold either shape) — a materially bigger change than this feature-set redesign needs, and not requested. One engine build supports exactly one architecture; the magic string (once identified, per the bullet above) exists to fail fast with a clear "wrong format" error if the wrong file is loaded, not to enable runtime dispatch between two shapes.
- **v1-misread prevention**: distinct magic string, plus keep the existing strict byte-length check (`nnue.rs:233-241`'s pattern) even though the ~76× size difference alone would likely already catch a v1/v2 mismatch — same defensive redundancy the current code already uses (magic check *and* length check, not either alone).
- **Estimated weight size, memory, eval cost**: see §2/§3 for the full-81-king-square estimate (≈94 MB) — **not finalized**; §8 below adds a smaller candidate to compare against before committing to a specific scheme.

## 5. API changes required (none applied today)

- New `feature_index_v2(king_sq, piece_sq, kind, owner_code) -> usize`.
- A v2-shaped `NnueWeights`/`NnueAcc` type (or the existing types made generic over dimension — rejected per §4's no-coexistence decision) gated behind a Cargo feature flag on `sekirei-core` (e.g. `nnue_v2`) — simplest option given no-coexistence is already the decision, avoids a separate crate.
- `sekirei-train`'s exporter/trainer need matching v2-shaped feature-building code behind the same flag, since training and inference share `feature_index` today.

## 6. Migration steps

0. **Do the §8 comparison first** (full 81-king-square vs. king-bucket) — the steps below assume a scheme has already been chosen; don't start step 1 with the 94 MB full scheme locked in by default.
1. Implement `feature_index_v2` + a v2 accumulator/weights type behind the feature flag, as dead code (no wiring into the live evaluator yet).
2. Port the trainer/exporter to emit v2-shaped training features and weight files behind the same flag.
3. Generate one v2 weight file (small-scale) for a smoke comparison against v1 on the same dataset.
4. Build the bit-exact refresh-vs-incremental test suite (§7) — this must pass before any v2 weight is trusted.
5. A/B gate v2 vs. v1 strength (heavy — training + match play — explicitly deferred to when compute is free, not part of today's scope).
6. Promote only after that gate passes.

## 7. Test plan

- **Bit-exact refresh-vs-incremental equivalence** (the standard NNUE correctness invariant): for an arbitrary move sequence, the accumulator produced by `refresh()` from scratch must match the incrementally-updated accumulator exactly (`i16`-for-`i16`) at every ply, for both perspectives.
- King move triggers a refresh of exactly the moving perspective's half, on both `do_move` and `undo_move`; the non-moving perspective's half is provably untouched (same array contents before/after).
- Non-king moves (normal, capture, promotion, non-promotion, drop) never trigger a spurious refresh — purely incremental, same code path as v1.
- Magic/format-rejection tests mirroring v1's existing provenance tests: `SEKIRW02` loads; `SEKIRW01`/`JANOSW03` are rejected by a v2-only-built binary with a clear error, not misread as v2 data.

## 8. Open comparison to resolve before implementation (added after review, 2026-07-25)

§2's full-81-king-square scheme (≈94 MB, §4) is **one candidate, not a settled decision**. Before writing any code (migration step 0, §6), compare it against a **king-bucket** scheme: group the 81 king squares into a smaller number of buckets (e.g. by symmetry — files mirror around the center file, so a left/right-mirrored bucketing alone roughly halves the king dimension — or by coarser region, common in other engines' smaller HalfKP-family nets) and use `bucket(king_sq)` in place of `king_sq` in the index formula from §2. This is a small change to the *formula* (one lookup added before the existing multiply), not to the surrounding architecture — everything else in this document (dual-perspective halves, refresh-on-king-move, hand features unchanged, `SEKIRW02` magic, no v1/v2 coexistence) applies identically regardless of which bucketing (if any) is chosen.

Compare on three axes before choosing:
- **Weight size**: full 81-square gives `INPUT_v2 ≈ 183,860` (≈94 MB, §2/§4). A bucketed scheme with `B` buckets gives `B * 81 * 28 + 152` — e.g. `B=9` (one bucket per king *file*, ignoring rank) ≈ `9*81*28+152 ≈ 20,564` input dims ≈ **10.5 MB**, roughly a 9× reduction; `B=41` (mirror-symmetric: 81 squares collapse to 41 by left-right symmetry) ≈ `41*81*28+152 ≈ 93,140` ≈ 47.7 MB, roughly half. These are illustrative, not a recommendation — the actual bucket count/shape is exactly what needs comparing, not assumed.
- **Speed**: bucketing adds one lookup/branch per active feature (negligible per §2's "index space grows, active-feature count doesn't" point) but shrinks the `ft` table, which may matter more for cache behavior (a 10 MB table has a very different cache footprint than a 94 MB one) than for raw arithmetic cost — this needs a real measurement, not an estimate, once a candidate is implemented.
- **Expressiveness**: full 81-square lets the net learn a fully distinct weight per exact king square (maximum representational power, closest to "true" HalfKP); bucketing trades some of that away for a smaller/faster net, on the (unverified) assumption that positions with similar-enough king placement share enough structure that per-bucket weights don't lose much. This is the crux tradeoff and can only be settled by training small probes of each candidate on the same dataset and comparing validation loss, not by reasoning about it in the abstract.

**Do not finalize the 94 MB full scheme as the only option carried into migration step 1** — at minimum, size up one bucketed candidate (e.g. the file-only or mirror-symmetric bucketing above) alongside the full scheme, and let §6's step 0 comparison (not this document) decide between them before any training or gating begins.
