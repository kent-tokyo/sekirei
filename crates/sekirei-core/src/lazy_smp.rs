//! Lazy-SMP-style independent root searches.
//!
//! Each worker searches a private board with private move-ordering heuristics,
//! while workers share the lock-free TT and one abort flag. This is deliberately
//! separate from YBW and speculative search: it provides an isolation boundary
//! for correctness testing before any USI option or strength claim is made.

use rayon::prelude::*;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::board::Board;
use crate::search::{SearchConfig, SearchInfo, Searcher};
use crate::tt::Tt;

/// Result of an independent-worker Lazy SMP search.
pub struct LazySmpInfo {
    /// Selected result from the deepest completed worker, with deterministic
    /// score and move tie-breaks.
    pub result: SearchInfo,
    /// Sum of nodes visited by all workers; this is the useful-work cost of
    /// the Lazy SMP invocation, rather than only the selected worker's count.
    pub total_nodes: u64,
    /// Per-worker diagnostics retained for noise and TT-sharing analysis.
    pub worker_results: Vec<LazySmpWorkerInfo>,
    /// Number of workers that participated.
    pub workers: usize,
    /// Wall-clock duration of the whole worker group.
    pub elapsed: Duration,
}

/// Compact per-worker result for Lazy SMP diagnostics.
#[derive(Clone, Copy, Debug)]
pub struct LazySmpWorkerInfo {
    /// Worker-selected best move, if any.
    pub best_move: Option<crate::mv::Move>,
    /// Worker score in centipawns or a mate score.
    pub score: i32,
    /// Deepest completed iterative-deepening depth.
    pub depth: u32,
    /// Nodes visited by this worker.
    pub nodes: u64,
}

/// Independent root-search workers sharing a lock-free transposition table.
pub struct LazySmpSearcher {
    tt: Arc<Tt>,
    workers: usize,
    share_tt: bool,
    hash_mb: usize,
    external_abort: Arc<std::sync::atomic::AtomicBool>,
}

impl LazySmpSearcher {
    /// Create a Lazy SMP searcher. `workers == 0` is normalized to one worker.
    pub fn new(tt: Arc<Tt>, workers: usize) -> Self {
        Self {
            tt,
            workers: workers.max(1),
            share_tt: true,
            hash_mb: 16,
            external_abort: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }

    /// Create a diagnostic searcher whose workers use isolated TT instances.
    /// This is a causal control for measuring the value of TT sharing.
    pub fn new_isolated(tt: Arc<Tt>, workers: usize) -> Self {
        Self::new_isolated_with_hash_mb(tt, workers, 16)
    }

    /// Isolated-TT diagnostic constructor with an explicit table size.
    pub fn new_isolated_with_hash_mb(tt: Arc<Tt>, workers: usize, hash_mb: usize) -> Self {
        let mut searcher = Self::new(tt, workers);
        searcher.share_tt = false;
        searcher.hash_mb = hash_mb;
        searcher
    }

    /// Returns the shared stop flag used by every worker.
    pub fn abort_flag(&self) -> Arc<std::sync::atomic::AtomicBool> {
        self.external_abort.clone()
    }

    /// Clear a previous stop signal before starting another search.
    pub fn reset_abort_flag(&self) {
        self.external_abort
            .store(false, std::sync::atomic::Ordering::Relaxed);
    }

    /// Reset the shared TT between games, matching the regular searcher API.
    pub fn clear_tt(&self) {
        self.tt.clear();
    }

    /// Probe the shared TT for a ponder move after the selected move.
    pub fn probe_tt(&self, hash: u64) -> Option<crate::mv::Move> {
        self.tt.probe(hash).and_then(|entry| entry.mv)
    }

    /// Search independent copies of `board` concurrently.
    pub fn search(&self, board: &Board, config: SearchConfig) -> LazySmpInfo {
        let started = Instant::now();
        let results: Vec<SearchInfo> = (0..self.workers)
            .into_par_iter()
            .map(|_| {
                let mut worker_board = board.clone();
                let worker_tt = if self.share_tt {
                    self.tt.clone()
                } else {
                    Tt::new(self.hash_mb)
                };
                Searcher::with_abort_flag(worker_tt, self.external_abort.clone())
                    .search(&mut worker_board, config)
            })
            .collect();

        let total_nodes = results.iter().map(|info| info.nodes).sum();
        let worker_results = results
            .iter()
            .map(|info| LazySmpWorkerInfo {
                best_move: info.best_move,
                score: info.score,
                depth: info.depth,
                nodes: info.nodes,
            })
            .collect();
        let result = results
            .into_iter()
            .reduce(select_result)
            .expect("Lazy SMP always has at least one worker");
        LazySmpInfo {
            result,
            total_nodes,
            worker_results,
            workers: self.workers,
            elapsed: started.elapsed(),
        }
    }
}

fn select_result(left: SearchInfo, right: SearchInfo) -> SearchInfo {
    let left_key = (left.depth, left.score, move_key(left.best_move));
    let right_key = (right.depth, right.score, move_key(right.best_move));
    if right_key > left_key { right } else { left }
}

fn move_key(mv: Option<crate::mv::Move>) -> (u8, u8, bool, u8) {
    mv.map_or((0, 0, false, 0), |m| {
        (
            m.from.map_or(81, |sq| sq.index()),
            m.to.index(),
            m.promote,
            m.piece_kind.index() as u8,
        )
    })
}
