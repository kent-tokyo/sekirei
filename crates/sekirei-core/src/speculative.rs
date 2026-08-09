//! Speculative / preemptive search infrastructure.
//!
//! A `SpecGroup` holds one `Arc<AtomicBool>` abort flag per speculative task.
//! Dropping a `SpecGroup` immediately sets every remaining flag to `true` —
//! the RAII cancellation guarantee described in AGENTS.md.
//!
//! Correctness invariant: speculative tasks NEVER write to the shared TT if their
//! abort flag has been set. This prevents partially-computed 0-scores from
//! poisoning entries that the main search later reads.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};

use crate::board::Board;
use crate::budget::Budget;
use crate::movegen::generate_legal_moves;
use crate::mv::Move;
use crate::piece::PieceKind;
use crate::policy;
use crate::tt::{Bound, Tt, TtEntry};

/// sentinel stored in `best_score` while the task is still running
const RUNNING: i32 = i32::MAX;

// ---- Shared context for speculative tasks ----

/// Shared context handed to every speculative search task.
pub struct SpecState {
    /// Transposition table shared with the main search.
    pub tt: Arc<Tt>,
    /// The *same* budget instance the main search uses, not an independent
    /// copy — a USI stop or the watchdog firing must be visible to spec
    /// tasks without a separately hand-synced flag (see `search.rs`'s
    /// `SpeculativeSearcher::search`).
    pub(crate) budget: Arc<Budget>,
    /// Dedicated pool for speculative tasks, entirely separate from rayon's
    /// global pool. `alpha_beta`'s own YBW parallel dispatch
    /// (`work.into_par_iter()...collect()`, search.rs) implicitly runs on
    /// the global pool; before this field existed, `SpecGroup::spawn` used
    /// the bare `rayon::spawn` function, which also targets the global pool.
    /// An unbounded-depth speculative task (no cap beyond the search-wide
    /// deadline) occupying a global-pool worker could then starve
    /// `alpha_beta`'s own dispatch of a worker for the rest of the search:
    /// a thread outside rayon's registry that calls `.into_par_iter()`
    /// cannot itself steal work, it can only block on a `LockLatch` until a
    /// worker frees up (confirmed via `sample` -- the dispatching thread
    /// spent 100% of a 1s window in `pthread_cond_wait` inside
    /// `Registry::in_worker_cold`). Isolating speculation onto its own pool
    /// makes that structurally impossible regardless of how long any one
    /// speculative task runs.
    pub(crate) pool: Arc<rayon::ThreadPool>,
}

// ---- Per-task handle ----

struct SpecTask {
    mv: Move,
    task_abort: Arc<AtomicBool>,
    result: Arc<AtomicI32>, // RUNNING while in-flight; real score when done
}

// ---- RAII group ----

/// A group of speculative tasks that are aborted when the group is dropped.
///
/// Call `promote(winner)` before dropping to keep the winner's task running
/// so it can finish writing deeper TT entries for the next depth iteration.
pub struct SpecGroup {
    tasks: Vec<SpecTask>,
}

impl SpecGroup {
    /// Spawn `top_n` speculative tasks that explore `board` at `depth` plies.
    pub fn spawn(board: &Board, state: &Arc<SpecState>, depth: u32, top_n: usize) -> Self {
        let candidates = policy::top_n(board, &state.tt, top_n);

        let tasks = candidates
            .into_iter()
            .map(|m| {
                let task_abort = Arc::new(AtomicBool::new(false));
                let result = Arc::new(AtomicI32::new(RUNNING));

                let abort_c = task_abort.clone();
                let result_c = result.clone();
                let state_c = state.clone();
                let mut b = board.clone();

                // Dedicated pool (see SpecState::pool), not the bare rayon::spawn
                // global-pool function -- must never share a pool with alpha_beta's
                // own YBW dispatch.
                state.pool.spawn(move || {
                    // Check abort flags before doing any work
                    if abort_c.load(Ordering::Relaxed) || state_c.budget.should_abort() {
                        result_c.store(0, Ordering::Relaxed);
                        return;
                    }
                    // policy::top_n uses pseudo-legal generation; skip king captures
                    if b.piece_at(m.to).is_some_and(|p| p.kind == PieceKind::Ou) {
                        result_c.store(0, Ordering::Relaxed);
                        return;
                    }

                    let tok = b.do_move(m);
                    let score = spec_alpha_beta(
                        &state_c,
                        &abort_c,
                        &mut b,
                        -1_000_000,
                        1_000_000,
                        depth.saturating_sub(1),
                        1,
                    );
                    b.undo_move(tok);

                    // Only write to TT if the search completed without abort.
                    // An aborted search may have propagated score=0 up the tree,
                    // which would poison TT entries read by the main search.
                    if !abort_c.load(Ordering::Relaxed) && !state_c.budget.should_abort() {
                        state_c.tt.store(
                            b.hash(),
                            TtEntry {
                                score: -score, // negate: score is opponent's, -score is ours
                                depth: depth as u8,
                                bound: Bound::Exact,
                                mv: Some(m),
                            },
                        );
                        result_c.store(-score, Ordering::Release);
                    } else {
                        result_c.store(0, Ordering::Relaxed);
                    }
                });

                SpecTask {
                    mv: m,
                    task_abort,
                    result,
                }
            })
            .collect();

        SpecGroup { tasks }
    }

    /// Remove `winner` from the abort list so it is NOT cancelled on drop.
    pub fn promote(&mut self, winner: Move) {
        self.tasks.retain(|t| t.mv != winner);
    }

    /// Non-blocking poll: `Some(score)` if task for `mv` finished, else `None`.
    pub fn poll(&self, mv: Move) -> Option<i32> {
        for t in &self.tasks {
            if t.mv == mv {
                let v = t.result.load(Ordering::Acquire);
                if v != RUNNING {
                    return Some(v);
                }
                return None;
            }
        }
        None
    }
}

impl Drop for SpecGroup {
    /// RAII: signal every non-promoted task to stop at its next abort check.
    fn drop(&mut self) {
        for t in &self.tasks {
            t.task_abort.store(true, Ordering::Relaxed);
        }
    }
}

// ---- Speculative Alpha-Beta ----
//
// Sequential (no parallel young brothers) to avoid competing with the main
// search for rayon worker threads.

fn spec_alpha_beta(
    state: &Arc<SpecState>,
    task_abort: &AtomicBool,
    board: &mut Board,
    alpha: i32,
    beta: i32,
    depth: u32,
    ply: u32,
) -> i32 {
    // Abort check first — callers must not use the return value 0 as a real score.
    // No self-throttled deadline check here: `state.budget` is the *same*
    // instance the main search ticks on every alpha_beta/quiescence node, and
    // the OS watchdog thread (spawned whenever a time limit is set) guarantees
    // the deadline fires regardless of rayon pool contention — so relying on
    // the shared `should_abort()` alone is enough to avoid the pool-starvation
    // hang a separate per-task check used to guard against.
    if task_abort.load(Ordering::Relaxed) || state.budget.should_abort() {
        return 0;
    }

    if depth == 0 {
        return crate::eval::evaluate(board);
    }

    let hash = board.hash();
    let orig_alpha = alpha;
    let mut alpha = alpha;

    // TT probe — skip if entry was written by an aborted task (we can't tell,
    // but entries with depth=0 or unreasonable scores are naturally harmless)
    if let Some(e) = state.tt.probe(hash)
        && e.depth >= depth as u8
    {
        match e.bound {
            Bound::Exact => return e.score,
            Bound::Lower => {
                if e.score >= beta {
                    return e.score;
                }
                if e.score > alpha {
                    alpha = e.score;
                }
            }
            Bound::Upper => {
                if e.score <= alpha {
                    return e.score;
                }
            }
        }
    }

    let moves = generate_legal_moves(board);
    if moves.is_empty() {
        return -(crate::search::MATE_SCORE - ply as i32);
    }

    let mut best = -1_000_000i32;
    let mut best_move = None;

    for m in moves {
        // Re-check abort before each recursive call
        if task_abort.load(Ordering::Relaxed) || state.budget.should_abort() {
            return 0; // do NOT write to TT with this incomplete best
        }

        let tok = board.do_move(m);
        let s = -spec_alpha_beta(state, task_abort, board, -beta, -alpha, depth - 1, ply + 1);
        board.undo_move(tok);

        // If the recursive call aborted, s == 0 is meaningless — bail out
        if task_abort.load(Ordering::Relaxed) || state.budget.should_abort() {
            return 0;
        }

        if s > best {
            best = s;
            best_move = Some(m);
        }
        if s >= beta {
            // `hash` (top of this function) is the position at THIS call's own
            // `ply` -- that's the ply score_to_tt needs, matching alpha_beta's
            // store_tt in search.rs.
            state.tt.store(
                hash,
                TtEntry {
                    score: crate::search::score_to_tt(s, ply),
                    depth: depth as u8,
                    bound: Bound::Lower,
                    mv: best_move,
                },
            );
            return s;
        }
        if s > alpha {
            alpha = s;
        }
    }

    let bound = if best > orig_alpha {
        Bound::Exact
    } else {
        Bound::Upper
    };
    state.tt.store(
        hash,
        TtEntry {
            score: crate::search::score_to_tt(best, ply),
            depth: depth as u8,
            bound,
            mv: best_move,
        },
    );
    best
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    // Hand-verified mate-in-1 for black: white king cornered at (file9,rank1);
    // black king at (file7,rank2) covers both diagonal escapes; black rook
    // slides to (file9,rank5) delivering unstoppable check down the file. See
    // search.rs::regression_tests for the full derivation (same position is
    // reused there for the sibling `alpha_beta` regression test).
    const MATE_IN_1_SFEN: &str = "k8/2K6/9/9/4R4/9/9/9/9 b - 1";

    // Black rook on file9 with a clear path to the white king: `policy::top_n`
    // (pseudo-legal, per its doc comment) includes the rook-takes-king move
    // among its candidates for this position.
    const KING_CAPTURE_CANDIDATE_SFEN: &str = "k8/9/9/9/R8/9/9/9/9 b - 1";

    fn spec_state() -> Arc<SpecState> {
        Arc::new(SpecState {
            tt: Tt::new(1),
            budget: Arc::new(Budget::new(None, Arc::new(AtomicBool::new(false)))),
            pool: Arc::new(
                rayon::ThreadPoolBuilder::new()
                    .num_threads(1)
                    .build()
                    .unwrap(),
            ),
        })
    }

    // Regression: `spec_alpha_beta`'s terminal-mate return used to be written
    // as `-900_000 - ply`, an independent copy of the same formula bug fixed in
    // search.rs's `alpha_beta`. The flipped sign on the ply term made a mate
    // discovered at a deeper ply score higher in magnitude than the identical
    // mate discovered shallower. Rather than a second hand-built position (hard
    // to verify and slow to brute-force at this function's fixed search depth),
    // this calls `spec_alpha_beta` directly on the SAME verified mate-in-1
    // position with two different starting `ply` values, isolating the
    // formula's dependence on its ply argument. depth=4 is enough for the
    // recursion to reach the real movegen/terminal check one ply down (this
    // function has no depth==0 quiescence detour, unlike alpha_beta).
    #[test]
    fn shorter_ply_mate_scores_higher_in_spec_alpha_beta() {
        let task_abort = AtomicBool::new(false);

        let mut board_a = Board::from_sfen(MATE_IN_1_SFEN).unwrap();
        let score_shallow = spec_alpha_beta(
            &spec_state(),
            &task_abort,
            &mut board_a,
            -1_000_000,
            1_000_000,
            4,
            1,
        );

        let mut board_b = Board::from_sfen(MATE_IN_1_SFEN).unwrap();
        let score_deep = spec_alpha_beta(
            &spec_state(),
            &task_abort,
            &mut board_b,
            -1_000_000,
            1_000_000,
            4,
            3,
        );

        const MATE_SCORE: i32 = crate::search::MATE_SCORE;
        assert!(
            score_shallow >= MATE_SCORE - 1000 && score_deep >= MATE_SCORE - 1000,
            "both calls must report a forced mate: {score_shallow} / {score_deep}"
        );
        assert!(
            score_shallow > score_deep,
            "mate found at the shallower ply ({score_shallow}) must score higher than the \
             identical mate found 2 plies deeper ({score_deep})"
        );
    }

    // Regression: `spec_alpha_beta`'s two internal `state.tt.store` calls (beta-cutoff
    // and end-of-loop) used to store the raw score it computed internally, without the
    // ply-relative encoding (`score_to_tt`/`score_from_tt`, search.rs) every write/read
    // in the main search's shared TT uses. Since `SpecState.tt` is the *same* TT the
    // main search's `alpha_beta` probes via `score_from_tt(entry.score, ply)`
    // unconditionally, an un-adjusted mate-adjacent score written here would be
    // corrupted when later read from a different ply than it was computed at.
    //
    // NOTE: `SpecGroup::spawn`'s own closure store was audited too, but does NOT need
    // this fix and is unchanged -- `b.undo_move(tok)` runs before that store, so it
    // stores at the pre-move (root) position's hash, ply=0 for this search instance,
    // and score_to_tt/score_from_tt are identity operations at ply=0 (`score + 0` /
    // `score - 0`). Mixing that closure into this fix added no real behavior change.
    // A separate, real bug was found auditing that closure while looking for a test
    // shape for it -- concurrent candidate tasks race to store unrelated Bound::Exact
    // results at that same parent hash, and completion order rather than move quality
    // decides the final entry. That's a same-parent-hash last-writer-wins race, not a
    // mate-score encoding problem (score_to_tt/score_from_tt aren't involved at ply=0
    // regardless of which task wins), and is filed as its own issue rather than folded
    // in here.
    //
    // The four tests below are distinct from `shorter_ply_mate_scores_higher_in_spec_
    // alpha_beta` above, which only checks the *returned* score's internal ordering
    // property -- unaffected by this fix, since only the TT *store* path changed here,
    // not any return value.

    // A: end-of-loop store (Bound::Exact/Upper), wide window, no cutoff.
    #[test]
    fn spec_alpha_beta_end_of_loop_store_is_ply_relative() {
        let tt = Tt::new(1);
        let state = Arc::new(SpecState {
            tt: tt.clone(),
            budget: Arc::new(Budget::new(None, Arc::new(AtomicBool::new(false)))),
            pool: Arc::new(
                rayon::ThreadPoolBuilder::new()
                    .num_threads(1)
                    .build()
                    .unwrap(),
            ),
        });
        let task_abort = AtomicBool::new(false);

        // Wide window (-1_000_000, 1_000_000): no move in this sparse position can
        // reach beta, so the loop always completes normally and falls through to the
        // end-of-loop store, not the beta-cutoff one.
        //
        // Call as if this mate were discovered deep in a real search tree (ply=5), not
        // near the root -- a raw, un-adjusted score would then be visibly wrong when
        // decoded from a different ply below.
        let mut board = Board::from_sfen(MATE_IN_1_SFEN).unwrap();
        let found_at_ply = 5u32;
        let score = spec_alpha_beta(
            &state,
            &task_abort,
            &mut board,
            -1_000_000,
            1_000_000,
            4,
            found_at_ply,
        );
        const MATE_SCORE: i32 = crate::search::MATE_SCORE;
        assert!(
            score >= MATE_SCORE - 1000,
            "expected a forced-mate score, got {score}"
        );

        let entry = tt
            .probe(board.hash())
            .expect("spec_alpha_beta should have stored an entry for this position");
        assert_eq!(
            entry.bound,
            Bound::Exact,
            "wide window with a best move beating orig_alpha must store Exact, not a cutoff bound"
        );

        // Read back through the SAME score_from_tt the main search's alpha_beta calls
        // on every real TT probe, at a different ply than this was found at -- exactly
        // the scenario a transposition into this position from elsewhere in the tree
        // would hit.
        let read_at_ply = 2u32;
        let decoded = crate::search::score_from_tt(entry.score, read_at_ply);

        // Derivation, not a copy of score_to_tt's formula: the live search score at a
        // node encodes "MATE_SCORE minus the terminal position's ABSOLUTE ply from
        // root" -- so a mate found while searching from found_at_ply already has that
        // node's own ply baked in. score_to_tt strips it back out by ADDING found_at_ply
        // (recovering the ply-INDEPENDENT "mate in N from this position" value);
        // score_from_tt re-applies it for a NEW reader by SUBTRACTING that reader's own
        // ply. Net: decoded = (score + found_at_ply) - read_at_ply. Reading from a
        // SHALLOWER ply than where it was found (read_at_ply < found_at_ply, as here)
        // must therefore INCREASE the decoded score -- consistent with "a mate reachable
        // via a shorter path from root scores higher" (shorter_ply_mate_scores_higher_
        // in_spec_alpha_beta, above).
        let expected = score + found_at_ply as i32 - read_at_ply as i32;
        assert_eq!(
            decoded, expected,
            "TT entry stored by spec_alpha_beta's end-of-loop path must decode to the \
             same absolute mate distance when read from a different ply, not the raw \
             un-adjusted score (raw stored score was {}, decoded at ply {read_at_ply} \
             was {decoded}, expected {expected})",
            entry.score
        );
    }

    // B: beta-cutoff store (Bound::Lower), narrow-relative beta forcing a fail-high on
    // the mating move specifically.
    #[test]
    fn spec_alpha_beta_beta_cutoff_store_is_ply_relative() {
        let tt = Tt::new(1);
        let state = Arc::new(SpecState {
            tt: tt.clone(),
            budget: Arc::new(Budget::new(None, Arc::new(AtomicBool::new(false)))),
            pool: Arc::new(
                rayon::ThreadPoolBuilder::new()
                    .num_threads(1)
                    .build()
                    .unwrap(),
            ),
        });
        let task_abort = AtomicBool::new(false);

        const MATE_SCORE: i32 = crate::search::MATE_SCORE;
        // beta is comfortably below the mate score this position's move must produce
        // (MATE_SCORE - small ply offset) but far above any plausible ordinary
        // positional/material evaluation on this sparse (2 kings + 1 rook) board --
        // only the actual mating move can trigger `s >= beta` here, regardless of the
        // order moves are tried in, so the cutoff is deterministic.
        let beta = MATE_SCORE - 500;
        let mut board = Board::from_sfen(MATE_IN_1_SFEN).unwrap();
        let found_at_ply = 5u32;
        let score = spec_alpha_beta(
            &state,
            &task_abort,
            &mut board,
            -1_000_000,
            beta,
            4,
            found_at_ply,
        );
        assert!(
            score >= beta,
            "expected the forced mate to trigger a beta cutoff (score >= beta), got \
             score={score}, beta={beta}"
        );

        let entry = tt
            .probe(board.hash())
            .expect("spec_alpha_beta should have stored an entry for this position");
        assert_eq!(
            entry.bound,
            Bound::Lower,
            "a beta cutoff (s >= beta) must store Bound::Lower, not the end-of-loop bound"
        );

        let read_at_ply = 2u32;
        let decoded = crate::search::score_from_tt(entry.score, read_at_ply);
        // Same derivation as test A: decoded = score + found_at_ply - read_at_ply.
        let expected = score + found_at_ply as i32 - read_at_ply as i32;
        assert_eq!(
            decoded, expected,
            "TT entry stored by spec_alpha_beta's beta-cutoff path must decode to the \
             same absolute mate distance when read from a different ply, not the raw \
             un-adjusted score (raw stored score was {}, decoded at ply {read_at_ply} \
             was {decoded}, expected {expected})",
            entry.score
        );
    }

    // C: ordinary (non-mate-range) scores must be completely unaffected -- score_to_tt/
    // score_from_tt only exist to normalize the distance-to-mate a score encodes; an
    // evaluation-scale score doesn't represent "plies to mate" at all, so ply has
    // nothing to adjust.
    #[test]
    fn score_to_tt_and_score_from_tt_are_no_ops_outside_the_mate_range() {
        const MATE_SCORE: i32 = crate::search::MATE_SCORE;
        for ordinary in [0, 1, -1, 137, -892, MATE_SCORE - 1001, -(MATE_SCORE - 1001)] {
            for ply in [0u32, 1, 5, 40] {
                assert_eq!(
                    crate::search::score_to_tt(ordinary, ply),
                    ordinary,
                    "score_to_tt must not adjust an ordinary score {ordinary} at ply {ply}"
                );
                assert_eq!(
                    crate::search::score_from_tt(ordinary, ply),
                    ordinary,
                    "score_from_tt must not adjust an ordinary score {ordinary} at ply {ply}"
                );
            }
        }
    }

    // D: pure round-trip test (no board/search machinery) covering BOTH mate
    // directions, per the request to add one where possible.
    #[test]
    fn score_to_tt_score_from_tt_round_trip_both_mate_directions() {
        const MATE_SCORE: i32 = crate::search::MATE_SCORE;
        let found_at_ply = 7u32;
        let read_at_ply = 3u32;

        // Winning mate (positive, "mate in N for us"): the storing node's own ply is
        // baked into the live search score (see test A's derivation comment); stripped
        // out by ADDING found_at_ply, re-applied for a new reader by SUBTRACTING
        // read_at_ply. Net: score + found_at_ply - read_at_ply.
        let winning = MATE_SCORE - 40; // "mate in 40" from wherever this is found
        let winning_stored = crate::search::score_to_tt(winning, found_at_ply);
        let winning_decoded = crate::search::score_from_tt(winning_stored, read_at_ply);
        assert_eq!(
            winning_decoded,
            winning + found_at_ply as i32 - read_at_ply as i32
        );

        // Losing mate (negative, "we get mated in N"): the sign flips throughout --
        // score_to_tt SUBTRACTS the storing ply, score_from_tt ADDS the reading ply.
        // Net: score - found_at_ply + read_at_ply -- the mirror image of the winning
        // case, matching that getting mated via a SHORTER path from root (a smaller
        // read_at_ply than found_at_ply) makes the position look WORSE (more negative),
        // the opposite direction from the winning case's "shorter path looks better".
        let losing = -(MATE_SCORE - 40);
        let losing_stored = crate::search::score_to_tt(losing, found_at_ply);
        let losing_decoded = crate::search::score_from_tt(losing_stored, read_at_ply);
        assert_eq!(
            losing_decoded,
            losing - found_at_ply as i32 + read_at_ply as i32
        );

        // Sanity: the two directions must move in opposite directions for the same
        // ply shift, confirming the sign truly flips rather than both branches
        // accidentally computing the same thing.
        assert_eq!(
            winning_decoded - winning,
            -(losing_decoded - losing),
            "winning-mate and losing-mate ply shifts must be exact mirror images"
        );
    }

    // Regression: `policy::top_n` generates pseudo-legally (per its own doc
    // comment), so its candidates can include a move landing on the enemy
    // king's square. Before the fix, `SpecGroup::spawn`'s spawned closure
    // called `do_move` on such a candidate unconditionally, panicking inside
    // `hand.add_captured(Ou)`. Since the closure runs on a background rayon
    // thread (fire-and-forget, not joined), a panic there would not fail this
    // test directly — so this polls `SpecGroup::poll` for the guarded result
    // (0, per the guard at the top of the spawned closure) instead of relying
    // on the panic to propagate.
    #[test]
    fn spec_group_spawn_skips_king_capture_without_panicking() {
        use crate::square::Square;

        let board = Board::from_sfen(KING_CAPTURE_CANDIDATE_SFEN).unwrap();
        let tt = Tt::new(1);
        let king_sq = Square::from_shogi(9, 1);
        let candidates = policy::top_n(&board, &tt, 50);
        let king_capture_move = candidates
            .iter()
            .copied()
            .find(|m| m.to == king_sq)
            .expect("expected a pseudo-legal move targeting the enemy king in this position");

        let state = Arc::new(SpecState {
            tt: tt.clone(),
            budget: Arc::new(Budget::new(None, Arc::new(AtomicBool::new(false)))),
            pool: Arc::new(
                rayon::ThreadPoolBuilder::new()
                    .num_threads(1)
                    .build()
                    .unwrap(),
            ),
        });
        let group = SpecGroup::spawn(&board, &state, 2, 50);

        let deadline = Instant::now() + Duration::from_secs(3);
        let mut result = None;
        while Instant::now() < deadline {
            if let Some(r) = group.poll(king_capture_move) {
                result = Some(r);
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(
            result,
            Some(0),
            "king-capture speculative task should short-circuit to 0 without panicking"
        );
    }

    // Regression: `SpecGroup::spawn` used to submit tasks via the bare
    // `rayon::spawn` function, which targets rayon's *global* pool -- the
    // SAME pool `alpha_beta`'s own YBW parallel dispatch
    // (`work.into_par_iter()...collect()`, search.rs) implicitly depends on.
    // A thread outside rayon's worker registry that calls `.into_par_iter()`
    // cannot steal work itself; it can only block on a `LockLatch` until a
    // worker frees up (confirmed via `sample`: the dispatching thread spent
    // 100% of a sampled second in `pthread_cond_wait` inside
    // `Registry::in_worker_cold`). An unbounded-depth speculative task could
    // then occupy every global-pool worker for the rest of the search,
    // starving `alpha_beta`'s own dispatch of a thread and freezing search
    // depth regardless of remaining time budget.
    //
    // This test fully saturates the *global* pool with long-lived occupying
    // tasks (matching however large it already is, so it's robust to test
    // order / prior initialization by other tests in this binary), then
    // verifies a `SpecGroup` task still completes promptly -- only possible
    // if it runs on a genuinely separate pool via `SpecState::pool`.
    #[test]
    fn spec_group_uses_isolated_pool_not_global() {
        use std::sync::atomic::AtomicUsize;

        let global_n = rayon::current_num_threads();
        let occupied = Arc::new(AtomicUsize::new(0));
        let release = Arc::new(AtomicBool::new(true));
        for _ in 0..global_n {
            let occ = occupied.clone();
            let rel = release.clone();
            rayon::spawn(move || {
                occ.fetch_add(1, Ordering::SeqCst);
                while rel.load(Ordering::Relaxed) {
                    std::thread::sleep(Duration::from_millis(1));
                }
            });
        }
        let saturate_deadline = Instant::now() + Duration::from_secs(5);
        while occupied.load(Ordering::SeqCst) < global_n && Instant::now() < saturate_deadline {
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(
            occupied.load(Ordering::SeqCst),
            global_n,
            "failed to fully saturate the global pool ({global_n} threads) -- test setup invalid"
        );

        let board = Board::from_sfen(MATE_IN_1_SFEN).unwrap();
        let tt = Tt::new(1);
        let state = Arc::new(SpecState {
            tt: tt.clone(),
            budget: Arc::new(Budget::new(None, Arc::new(AtomicBool::new(false)))),
            pool: Arc::new(
                rayon::ThreadPoolBuilder::new()
                    .num_threads(1)
                    .build()
                    .unwrap(),
            ),
        });
        let candidates = policy::top_n(&board, &tt, 1);
        let group = SpecGroup::spawn(&board, &state, 2, 1);

        let deadline = Instant::now() + Duration::from_secs(2);
        let mut done = false;
        while Instant::now() < deadline {
            if group.poll(candidates[0]).is_some() {
                done = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        release.store(false, Ordering::Relaxed); // free the global-pool hogs
        assert!(
            done,
            "SpecGroup task never completed while the global pool was fully saturated -- \
             it may be sharing the global pool instead of its own dedicated pool"
        );
    }
}
