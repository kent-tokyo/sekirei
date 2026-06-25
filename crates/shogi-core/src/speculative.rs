//! Speculative / preemptive search infrastructure.
//!
//! A `SpecGroup` holds one `Arc<AtomicBool>` abort flag per speculative task.
//! Dropping a `SpecGroup` immediately sets every remaining flag to `true` —
//! the RAII cancellation guarantee described in AGENTS.md.
//!
//! Correctness invariant: speculative tasks NEVER write to the shared TT if their
//! abort flag has been set. This prevents partially-computed 0-scores from
//! poisoning entries that the main search later reads.

use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::Arc;

use crate::board::Board;
use crate::movegen::generate_legal_moves;
use crate::mv::Move;
use crate::policy;
use crate::tt::{Bound, Tt, TtEntry};

/// sentinel stored in `best_score` while the task is still running
const RUNNING: i32 = i32::MAX;

// ---- Shared context for speculative tasks ----

pub struct SpecState {
    pub tt:    Arc<Tt>,
    /// Set when the whole search session ends; tasks must check this too.
    pub abort: Arc<AtomicBool>,
}

// ---- Per-task handle ----

struct SpecTask {
    mv:         Move,
    task_abort: Arc<AtomicBool>,
    result:     Arc<AtomicI32>, // RUNNING while in-flight; real score when done
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
                let result     = Arc::new(AtomicI32::new(RUNNING));

                let abort_c  = task_abort.clone();
                let result_c = result.clone();
                let state_c  = state.clone();
                let mut b    = board.clone();

                rayon::spawn(move || {
                    // Check abort flags before doing any work
                    if abort_c.load(Ordering::Relaxed)
                        || state_c.abort.load(Ordering::Relaxed)
                    {
                        result_c.store(0, Ordering::Relaxed);
                        return;
                    }

                    let tok   = b.do_move(m);
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
                    if !abort_c.load(Ordering::Relaxed)
                        && !state_c.abort.load(Ordering::Relaxed)
                    {
                        state_c.tt.store(
                            b.hash(),
                            TtEntry {
                                score: -score, // negate: score is opponent's, -score is ours
                                depth: depth as u8,
                                bound: Bound::Exact,
                                mv:    Some(m),
                            },
                        );
                        result_c.store(-score, Ordering::Release);
                    } else {
                        result_c.store(0, Ordering::Relaxed);
                    }
                });

                SpecTask { mv: m, task_abort, result }
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
                if v != RUNNING { return Some(v); }
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
    state:      &Arc<SpecState>,
    task_abort: &AtomicBool,
    board:      &mut Board,
    alpha:      i32,
    beta:       i32,
    depth:      u32,
    ply:        u32,
) -> i32 {
    // Abort check first — callers must not use the return value 0 as a real score
    if task_abort.load(Ordering::Relaxed) || state.abort.load(Ordering::Relaxed) {
        return 0;
    }

    if depth == 0 {
        return crate::eval::evaluate(board);
    }

    let hash       = board.hash();
    let orig_alpha = alpha;
    let mut alpha  = alpha;

    // TT probe — skip if entry was written by an aborted task (we can't tell,
    // but entries with depth=0 or unreasonable scores are naturally harmless)
    if let Some(e) = state.tt.probe(hash) {
        if e.depth >= depth as u8 {
            match e.bound {
                Bound::Exact => return e.score,
                Bound::Lower => {
                    if e.score >= beta  { return e.score; }
                    if e.score >  alpha { alpha = e.score; }
                }
                Bound::Upper => {
                    if e.score <= alpha { return e.score; }
                }
            }
        }
    }

    let moves = generate_legal_moves(board);
    if moves.is_empty() {
        return -900_000 - ply as i32;
    }

    let mut best      = -1_000_000i32;
    let mut best_move = None;

    for m in moves {
        // Re-check abort before each recursive call
        if task_abort.load(Ordering::Relaxed) || state.abort.load(Ordering::Relaxed) {
            return 0; // do NOT write to TT with this incomplete best
        }

        let tok   = board.do_move(m);
        let s     = -spec_alpha_beta(state, task_abort, board, -beta, -alpha, depth - 1, ply + 1);
        board.undo_move(tok);

        // If the recursive call aborted, s == 0 is meaningless — bail out
        if task_abort.load(Ordering::Relaxed) || state.abort.load(Ordering::Relaxed) {
            return 0;
        }

        if s > best { best = s; best_move = Some(m); }
        if s >= beta {
            state.tt.store(hash, TtEntry { score: s, depth: depth as u8, bound: Bound::Lower, mv: best_move });
            return s;
        }
        if s > alpha { alpha = s; }
    }

    let bound = if best > orig_alpha { Bound::Exact } else { Bound::Upper };
    state.tt.store(hash, TtEntry { score: best, depth: depth as u8, bound, mv: best_move });
    best
}
