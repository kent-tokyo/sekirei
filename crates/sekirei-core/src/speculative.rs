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

                rayon::spawn(move || {
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
            state.tt.store(
                hash,
                TtEntry {
                    score: s,
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
            score: best,
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
}
