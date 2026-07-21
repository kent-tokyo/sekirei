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
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU64, Ordering};

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
    /// Nodes visited by speculative search, counted independently of
    /// `budget`'s node count (which only `alpha_beta`/`quiescence` tick).
    /// Speculative work was previously invisible to node counting entirely;
    /// this is a plain counter, not a deadline throttle like `Budget::tick`,
    /// since the deadline is already covered by the shared `budget`.
    pub spec_nodes: AtomicU64,
    /// Speculative tasks dispatched via `SpecGroup::spawn`, across the whole
    /// search (every depth iteration that speculates). See `tasks_completed`/
    /// `tasks_cancelled` for how each one concluded.
    pub tasks_started: AtomicU64,
    /// Tasks that ran to completion and stored a real (non-aborted) result.
    pub tasks_completed: AtomicU64,
    /// Tasks that produced no usable result: aborted (group dropped/budget
    /// deadline) before or during their search, or skipped outright because
    /// `policy::top_n`'s pseudo-legal candidate turned out to be a king
    /// capture.
    pub tasks_cancelled: AtomicU64,
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

                state.tasks_started.fetch_add(1, Ordering::Relaxed);
                rayon::spawn(move || {
                    // Check abort flags before doing any work
                    if abort_c.load(Ordering::Relaxed) || state_c.budget.should_abort() {
                        result_c.store(0, Ordering::Relaxed);
                        state_c.tasks_cancelled.fetch_add(1, Ordering::Relaxed);
                        return;
                    }
                    // policy::top_n uses pseudo-legal generation; skip king captures
                    if b.piece_at(m.to).is_some_and(|p| p.kind == PieceKind::Ou) {
                        result_c.store(0, Ordering::Relaxed);
                        state_c.tasks_cancelled.fetch_add(1, Ordering::Relaxed);
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
                        // `b.undo_move(tok)` above already returned `b` to the
                        // root position this SpecGroup was spawned from (ply 0
                        // relative to that root) — the store below is keyed on
                        // `b.hash()`, i.e. that root position, not the post-move
                        // position `spec_alpha_beta` searched. `score_to_tt` at
                        // ply 0 is a no-op by construction, but we still route
                        // through it (rather than storing `-score` raw) so every
                        // write into this shared `Arc<Tt>` — which the main
                        // search later probes with `score_from_tt` — goes
                        // through the same conversion, with no per-call-site
                        // judgment call about when it's safe to skip.
                        state_c.tt.store(
                            b.hash(),
                            TtEntry {
                                // negate: score is opponent's, -score is ours
                                score: crate::search::score_to_tt(-score, 0),
                                depth: depth as u8,
                                bound: Bound::Exact,
                                mv: Some(m),
                            },
                        );
                        result_c.store(-score, Ordering::Release);
                        state_c.tasks_completed.fetch_add(1, Ordering::Relaxed);
                    } else {
                        result_c.store(0, Ordering::Relaxed);
                        state_c.tasks_cancelled.fetch_add(1, Ordering::Relaxed);
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
    // Count this node before the abort check, matching `Budget::tick`'s own
    // convention of counting every node visited regardless of what happens
    // next. This is a plain counter, not a deadline throttle -- the deadline
    // is already covered by `state.budget`, shared with the main search.
    state.spec_nodes.fetch_add(1, Ordering::Relaxed);

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
    // but entries with depth=0 or unreasonable scores are naturally harmless).
    // `score_from_tt` matches the main search's probe convention (search.rs)
    // exactly, since this table is shared with it.
    if let Some(e) = state.tt.probe(hash)
        && e.depth >= depth as u8
    {
        let adj = crate::search::score_from_tt(e.score, ply);
        match e.bound {
            Bound::Exact => return adj,
            Bound::Lower => {
                if adj >= beta {
                    return adj;
                }
                if adj > alpha {
                    alpha = adj;
                }
            }
            Bound::Upper => {
                if adj <= alpha {
                    return adj;
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

    // Hand-verified mate-in-1 for black: white king cornered at (file9,rank1);
    // black king at (file7,rank2) covers both diagonal escapes; black rook
    // slides to (file9,rank5) delivering unstoppable check down the file. See
    // search.rs::regression_tests for the full derivation (same position is
    // reused there for the sibling `alpha_beta` regression test).
    const MATE_IN_1_SFEN: &str = "k8/2K6/9/9/4R4/9/9/9/9 b - 1";

    // Black rook on file9 with a clear path to the white king. Before
    // `policy::top_n` was switched to legal-move generation, this position's
    // pseudo-legal candidates included the rook-takes-king move.
    const KING_CAPTURE_CANDIDATE_SFEN: &str = "k8/9/9/9/R8/9/9/9/9 b - 1";

    // Black king at file5/rank1, black silver at file5/rank2, white rook at
    // file5/rank9: the silver is pinned (moving off file5 exposes the king to
    // the rook down the file). Pseudo-legal generation doesn't know about
    // pins; `generate_legal_moves` (which `top_n` now filters through) does.
    const PINNED_SILVER_SFEN: &str = "4K4/4S4/9/9/9/9/9/9/4r3k b - 1";

    fn spec_state() -> Arc<SpecState> {
        Arc::new(SpecState {
            tt: Tt::new(1),
            budget: Arc::new(Budget::new(None, Arc::new(AtomicBool::new(false)))),
            spec_nodes: AtomicU64::new(0),
            tasks_started: AtomicU64::new(0),
            tasks_completed: AtomicU64::new(0),
            tasks_cancelled: AtomicU64::new(0),
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

    // Regression (bug a): `policy::top_n` used to generate pseudo-legally, so
    // its candidates could include a move landing on the enemy king's square
    // — impossible in legal shogi. Now that `top_n` filters through
    // `generate_legal_moves`, no candidate should ever target the king.
    #[test]
    fn top_n_never_returns_a_king_capture_move() {
        use crate::square::Square;

        let board = Board::from_sfen(KING_CAPTURE_CANDIDATE_SFEN).unwrap();
        let tt = Tt::new(1);
        let king_sq = Square::from_shogi(9, 1);
        let candidates = policy::top_n(&board, &tt, 50);
        assert!(
            candidates.iter().all(|m| m.to != king_sq),
            "top_n should never return a king-capture candidate now that it \
             generates legal moves only"
        );
    }

    // Regression (bug a): pseudo-legal generation has no concept of pins, so
    // a candidate could leave the mover's own king in check (e.g. a pinned
    // silver stepping off the file a rook pins it against). `top_n` filtering
    // through `generate_legal_moves` must reject every such candidate.
    #[test]
    fn top_n_never_returns_a_move_that_leaves_the_mover_in_check() {
        let mut board = Board::from_sfen(PINNED_SILVER_SFEN).unwrap();
        let tt = Tt::new(1);
        let mover = board.side_to_move;
        let candidates = policy::top_n(&board, &tt, 50);
        assert!(
            !candidates.is_empty(),
            "sanity check: the pinned-silver position should still have legal moves"
        );
        for m in candidates {
            let tok = board.do_move(m);
            assert!(
                !crate::movegen::is_in_check(&board, mover),
                "top_n returned a move that leaves the mover's own king in check: {m:?}"
            );
            board.undo_move(tok);
        }
    }

    // Regression (bug b, ply-conversion off-by-one): `SpecGroup::spawn`'s TT
    // store runs AFTER `undo_move`, so `b` is back at the group's own root
    // (ply 0) at store time, not the ply the recursive `spec_alpha_beta` call
    // searched from. A prior version passed ply=1 to `score_to_tt` there,
    // shifting every mate score written into the shared root entry by one ply
    // (e.g. reading back "mate in 0" for an actual mate-in-1). `score_to_tt`/
    // `score_from_tt` are no-ops at ply 0 by construction, so the only correct
    // ply for a post-undo store is 0 — this pins that invariant directly,
    // independent of which move any particular policy pick happens to be.
    #[test]
    fn score_to_tt_round_trips_a_mate_score_at_ply_zero() {
        use crate::search::{MATE_SCORE, score_from_tt, score_to_tt};

        for score in [MATE_SCORE - 1, -(MATE_SCORE - 1)] {
            assert_eq!(
                score_from_tt(score_to_tt(score, 0), 0),
                score,
                "ply-0 store/read must round-trip a mate score exactly"
            );
        }
    }
}
