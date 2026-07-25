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

## 8. Open comparison to resolve before implementation (added 2026-07-25, expanded 2026-07-26)

§2's full-81-king-square scheme (≈94 MB, §4) is **one candidate, not a settled decision**. Before writing any code (migration step 0, §6), compare it against smaller/alternative king-relative schemes. All schemes below are a small change to the *formula* (one lookup/transform added before the existing multiply), not to the surrounding architecture — everything else in this document (dual-perspective halves, refresh-on-king-move, hand features unchanged, `SEKIRW02` magic, no v1/v2 coexistence) applies identically regardless of which is chosen, **except** where a row below says otherwise.

**Correction to the 2026-07-25 draft of this section**: the mirror-symmetric bucket count was given as "B=41." Re-derived by hand for this expansion: 9 files (1–9) form mirror pairs (1,9)/(2,8)/(3,7)/(4,6) plus the unpaired center file 5 → **5** distinct file-classes, not 4. `5 file-classes × 9 ranks = 45` distinct king-square-classes, not 41. The corrected figure (45) is used throughout this section; the 47.7 MB weight-size estimate this produced was coincidentally close to the corrected value (52.3 MB, below) but the class count itself was wrong and is fixed here.

### Six-way desk comparison

A recurring point, true of every row: **the active-feature count per evaluation never changes** (still ≈1 active board feature per piece per perspective, bounded by pieces-on-board/in-hand, order ~40) — only the *index space* (and therefore the weight table size) grows. Per-node incremental-update cost (`add_col`/`sub_col`) is therefore unaffected by which scheme is chosen; what changes is weight-table size, refresh frequency/scope, and (for scheme 6) whether the scheme is trainable at all.

| Scheme | Input dim (board) | Weight size (FT table) | Active features/eval | King-move refresh scope | Representation power | Impl. complexity | Change vs. v1 |
|---|---|---|---|---|---|---|---|
| **1. Full 81 king squares** | `81×81×28 = 183,708` (+152 hand = 183,860) | `183,860×256×2 ≈ 94.1 MB` | unchanged (~1/piece/perspective) | Full refresh of the **moving side's own perspective only**, on every king move (do *and* undo) | Highest among practical single-king schemes — one fully distinct weight row per exact king square, closest to classical HalfKP | High — biggest table, but the simplest *transform* (direct index, no bucket/mirror lookup) | ~76× input growth; new index fn, new refresh-on-king-move trigger, new file format |
| **2. 9 king buckets** (e.g. by king file, ignoring rank) | `9×81×28 = 20,412` (+152 = 20,564) | `20,564×256×2 ≈ 10.5 MB` | unchanged | Refresh only on a **bucket-boundary crossing** — many king moves stay in-bucket and need *no* refresh at all; when a crossing does happen, cost = same as a full scheme's refresh (every other piece's index for that perspective changes at once) | Lowest of the king-relative options — coarse, loses per-square nuance, but still strictly more king-aware than v1 (which has none) | Medium — needs a `king_sq → bucket` lookup table plus boundary-crossing detection for the refresh trigger | ~8.5× input growth; same *kind* of code change as scheme 1, much smaller magnitude |
| **3. 16 king buckets** | `16×81×28 = 36,288` (+152 = 36,440) | `36,440×256×2 ≈ 18.7 MB` | unchanged | Same mechanism as scheme 2; smaller buckets ⇒ boundary crossings somewhat more frequent than 9-bucket, still far rarer than scheme 1's "every move" | Between scheme 2 and scheme 1 | Medium — same shape as scheme 2 | ~15× input growth |
| **4. Mirror-symmetric bucketing** (45 classes, full square granularity, corrected above) | `45×81×28 = 102,060` (+152 = 102,212) | `102,212×256×2 ≈ 52.3 MB` | unchanged | **No refresh-frequency benefit** — applied at *full* granularity, virtually every king move still changes square and therefore class (mirroring only *shares storage* between symmetric squares, it does not coarsen regions). Refresh scope = same as scheme 1. | In principle **equal to scheme 1** (no information loss — shogi's left-right symmetry is a real rule-level property, not an approximation) *if* training data/self-play don't introduce a real left-right asymmetry the shared weights can't represent (e.g. an asymmetric opening-book skew) — a caveat worth testing, not assuming | **Highest of the practical options** — combines scheme 1's full-granularity refresh cost with an added file-mirroring transform in the hot feature-index path | ~42× input growth, plus the mirror-transform complexity on top |
| **5. Own-king-relative only** (own perspective indexed by its own king; enemy perspective indexed by *its* own king — i.e. the scheme §2 already specifies) | Identical to scheme 1 | Identical to scheme 1 | unchanged | Identical to scheme 1 | Identical to scheme 1 | Identical to scheme 1 | Identical to scheme 1 |
| **6. Both-king-relative** (each perspective's index depends on *both* own **and** enemy king square) | `81(own)×81(enemy)×81(piece)×28(kind) = 14,880,348` (+152 = 14,880,500) | `14,880,500×256×2 ≈ 7.62 GB` | unchanged | **Worse than every other scheme**: since each perspective's index now depends on the *enemy* king too, a king move on **either** side forces a refresh of **both** perspectives (not just the mover's own) — the single scheme here where refresh scope isn't even confined to the moving side | Highest *theoretical* capacity (directly encodes king-king interaction) — but 531,441 distinct king-pair combinations means almost all cells would be trained on vanishingly few real games; capacity that can't be filled with data isn't representation power in practice | High in principle, moot in practice — the memory requirement alone rules this out before complexity is even the binding constraint | ~6,150× input growth — categorically impractical, included here for completeness per the requested comparison, not as a real candidate |

Row 5 is listed separately from row 1 only because it was explicitly requested as its own comparison point — they describe the same scheme (this document's existing §2 design already *is* "own-king-relative only, full 81 squares"); row 6 exists to show concretely why the "index by both kings" alternative is rejected — not on complexity grounds primarily, but on a **7.6 GB weight table**, three-plus orders of magnitude beyond every other option.

### Recommendation for the first Arm B: separate "does king-relativity help" from "does a bigger net help"

Jumping straight to scheme 1/5 (full 81, ≈94 MB, ~76× v1's total size) confounds two different questions in one experiment: (a) does making board features king-relative improve play, and (b) does a ~76× larger evaluator improve play *regardless of the specific architectural idea* (a well-known confound in ML — a bigger model can win purely on capacity, unrelated to whatever change was hypothesized to matter). A **9-bucket scheme (row 2)** isolates (a) far better: only ~8.5× v1's size — a real but far more modest capacity increase — so an observed improvement is much more plausibly attributable to king-relative *indexing itself* rather than sheer parameter growth. **Recommended first Arm B: scheme 2 (9 king buckets).** Only if it clearly beats v1 should a follow-up probe scheme 3 (16 buckets) or scheme 1/5 (full 81) as an explicitly separate, explicitly-labeled "does more king-granularity/capacity help further" experiment — not folded into the same comparison as the first.

**Do not finalize the 94 MB full scheme (row 1/5) as the only — or even the first — option carried into migration step 1.** Size up the 9-bucket scheme alongside it, and let §6's step 0 comparison (not this document) make the actual empirical call once real training probes exist — this section is a desk estimate, not a substitute for that measurement.
