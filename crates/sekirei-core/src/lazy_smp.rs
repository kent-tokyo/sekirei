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
use crate::sfen::PositionHistory;
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
    /// Behaviour switches, see [`LAZY_PERSISTENT`], [`LAZY_MAIN_STOPS`] and
    /// [`LAZY_DEPTH_SKEW`].
    flags: u32,
    /// Persistent shared-TT workers (with [`LAZY_PERSISTENT`]): move-ordering
    /// history carries over from one search to the next, as in the
    /// single-worker searcher.
    persistent: Vec<Searcher>,
}

/// Keep one searcher per worker across searches (history tables persist
/// within a game and are cleared with the TT).
pub const LAZY_PERSISTENT: u32 = 1;
/// When worker 0 finishes its search, stop the helper workers.
pub const LAZY_MAIN_STOPS: u32 = 2;
/// Odd-numbered helper workers search every iteration one ply deeper.
pub const LAZY_DEPTH_SKEW: u32 = 4;
/// Workers search sequentially instead of splitting young brothers on the
/// shared Rayon pool (which the workers themselves already occupy).
pub const LAZY_NO_YBW: u32 = 8;
/// Default behaviour switches.
pub const LAZY_DEFAULT_FLAGS: u32 =
    LAZY_PERSISTENT | LAZY_MAIN_STOPS | LAZY_DEPTH_SKEW | LAZY_NO_YBW;

impl LazySmpSearcher {
    /// Create a Lazy SMP searcher. `workers == 0` is normalized to one worker.
    pub fn new(tt: Arc<Tt>, workers: usize) -> Self {
        Self::with_flags(tt, workers, LAZY_DEFAULT_FLAGS)
    }

    /// Create a Lazy SMP searcher with explicit behaviour switches
    /// (`LAZY_*` constants; 0 is the original independent-worker scheme).
    pub fn with_flags(tt: Arc<Tt>, workers: usize, flags: u32) -> Self {
        let workers = workers.max(1);
        let external_abort = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let persistent = if flags & LAZY_PERSISTENT != 0 {
            (0..workers)
                .map(|index| {
                    let mut searcher =
                        Searcher::with_abort_flag(tt.clone(), external_abort.clone());
                    searcher.set_depth_skew(Self::skew(flags, index));
                    searcher.set_ybw_split(flags & LAZY_NO_YBW == 0);
                    searcher
                })
                .collect()
        } else {
            Vec::new()
        };
        Self {
            tt,
            workers,
            share_tt: true,
            hash_mb: 16,
            external_abort,
            flags,
            persistent,
        }
    }

    fn skew(flags: u32, index: usize) -> u32 {
        u32::from(flags & LAZY_DEPTH_SKEW != 0 && index % 2 == 1)
    }

    /// Create a diagnostic searcher whose workers use isolated TT instances.
    /// This is a causal control for measuring the value of TT sharing.
    pub fn new_isolated(tt: Arc<Tt>, workers: usize) -> Self {
        Self::new_isolated_with_hash_mb(tt, workers, 16)
    }

    /// Isolated-TT diagnostic constructor with an explicit table size.
    pub fn new_isolated_with_hash_mb(tt: Arc<Tt>, workers: usize, hash_mb: usize) -> Self {
        let mut searcher = Self::with_flags(tt, workers, 0);
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
        for searcher in &self.persistent {
            searcher.clear_tt();
        }
    }

    /// Age the shared TT entries of earlier searches (call once per `go`).
    pub fn new_search(&self) {
        self.tt.new_search();
    }

    /// Probe the shared TT for a ponder move after the selected move.
    pub fn probe_tt(&self, hash: u64) -> Option<crate::mv::Move> {
        self.tt.probe(hash).and_then(|entry| entry.mv)
    }

    /// Search independent copies of `board` concurrently.
    pub fn search(&self, board: &Board, config: SearchConfig) -> LazySmpInfo {
        let history = PositionHistory::initial(board.hash());
        self.search_with_history(board, config, &history)
    }

    /// Search independent copies with the caller's already-played game
    /// history. Each worker receives an immutable cloneable history and only
    /// its private descendants extend it.
    pub fn search_with_history(
        &self,
        board: &Board,
        config: SearchConfig,
        history: &PositionHistory,
    ) -> LazySmpInfo {
        debug_assert_eq!(
            history.entries().last().map(|entry| entry.hash),
            Some(board.hash())
        );
        let started = Instant::now();
        let results: Vec<SearchInfo> = (0..self.workers)
            .into_par_iter()
            .map(|index| {
                let mut worker_board = board.clone();
                let info = if let Some(searcher) = self.persistent.get(index) {
                    searcher.search_with_history(&mut worker_board, config, history)
                } else {
                    let worker_tt = if self.share_tt {
                        self.tt.clone()
                    } else {
                        Tt::new(self.hash_mb)
                    };
                    let mut searcher =
                        Searcher::with_abort_flag(worker_tt, self.external_abort.clone());
                    searcher.set_depth_skew(Self::skew(self.flags, index));
                    searcher.set_ybw_split(self.flags & LAZY_NO_YBW == 0);
                    searcher.search_with_history(&mut worker_board, config, history)
                };
                if index == 0 && self.workers > 1 && self.flags & LAZY_MAIN_STOPS != 0 {
                    // The main worker owns time management; helpers stop with it.
                    self.external_abort
                        .store(true, std::sync::atomic::Ordering::Relaxed);
                }
                info
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
