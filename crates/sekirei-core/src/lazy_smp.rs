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
use crate::search::{
    SearchConfig, SearchDiagnostics, SearchDiagnosticsSnapshot, SearchInfo, Searcher,
    TtReuseSnapshot, TtReuseStats, WorkOverlapSnapshot, WorkOverlapStats,
};
use crate::sfen::PositionHistory;
use crate::tt::{Tt, TtWriteSnapshot, TtWriteStats};

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
    /// Shared-TT write topology for this search when opt-in diagnostics are
    /// enabled. Isolated-TT controls leave this as `None`.
    pub tt_writes: Option<TtWriteSnapshot>,
    /// Sampled node overlap across workers when opt-in diagnostics are enabled.
    pub work_overlap: Option<LazySmpWorkOverlap>,
}

/// Diagnostic-only estimate of repeated position work across Lazy SMP workers.
/// Hashes are sampled at 1/256 and stored in a bounded fingerprint table;
/// `dropped` reports samples that could not be placed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LazySmpWorkOverlap {
    /// Sampled main-search and quiescence visits across every worker.
    pub sampled_visits: u64,
    /// Distinct sampled position fingerprints retained by the bounded table.
    pub unique_fingerprints: u64,
    /// Revisits by the worker that first observed the fingerprint.
    pub within_worker_revisits: u64,
    /// Visits by workers other than the fingerprint's first observer.
    pub cross_worker_visits: u64,
    /// Distinct fingerprints observed by at least two workers.
    pub shared_fingerprints: u64,
    /// Samples dropped after all bounded open-addressing probes were occupied.
    pub dropped: u64,
    /// Sampled TT stores that committed and were attributed to a worker.
    pub tt_sampled_stores: u64,
    /// Sampled TT hits observed by the search workers.
    pub tt_sampled_hits: u64,
    /// Sampled hits on entries last committed by the probing worker.
    pub tt_local_hits: u64,
    /// Sampled hits on entries last committed by another worker. This is the
    /// direct estimate of shared-TT results actually reused across workers.
    pub tt_foreign_hits: u64,
    /// Sampled hits whose committing worker was not retained by the bounded
    /// registry (for example, an entry predating this invocation).
    pub tt_unknown_hits: u64,
    /// Sampled committed stores that could not be placed in the registry.
    pub tt_dropped_stores: u64,
}

fn work_overlap_snapshot(
    value: WorkOverlapSnapshot,
    tt_reuse: TtReuseSnapshot,
) -> LazySmpWorkOverlap {
    LazySmpWorkOverlap {
        sampled_visits: value.sampled_visits,
        unique_fingerprints: value.unique_fingerprints,
        within_worker_revisits: value.within_worker_revisits,
        cross_worker_visits: value.cross_worker_visits,
        shared_fingerprints: value.shared_fingerprints,
        dropped: value.dropped,
        tt_sampled_stores: tt_reuse.sampled_stores,
        tt_sampled_hits: tt_reuse.sampled_hits,
        tt_local_hits: tt_reuse.local_hits,
        tt_foreign_hits: tt_reuse.foreign_hits,
        tt_unknown_hits: tt_reuse.unknown_hits,
        tt_dropped_stores: tt_reuse.dropped_stores,
    }
}

/// Compact per-worker result for Lazy SMP diagnostics.
#[derive(Clone, Copy, Debug)]
pub struct LazySmpWorkerInfo {
    /// Whether this worker supplied the result returned to the caller.
    pub selected: bool,
    /// Worker-selected best move, if any.
    pub best_move: Option<crate::mv::Move>,
    /// Worker score in centipawns or a mate score.
    pub score: i32,
    /// Deepest completed iterative-deepening depth.
    pub depth: u32,
    /// Nodes visited by this worker.
    pub nodes: u64,
    /// Wall-clock duration observed by this worker.
    pub elapsed: Duration,
    /// Whether this worker stopped before completing its current iteration.
    pub aborted: bool,
    /// Stop source reported by the worker search.
    pub abort_reason: &'static str,
    /// Transposition-table probes performed by this worker. Zero when
    /// opt-in Lazy SMP diagnostics are disabled.
    pub tt_probes: u64,
    /// Transposition-table hits observed by this worker.
    pub tt_hits: u64,
    /// Transposition-table store attempts made by this worker.
    pub tt_stores: u64,
    /// Sampled TT hits on entries last written by this worker.
    pub tt_local_hits: u64,
    /// Sampled TT hits on entries last written by another worker. For the
    /// selected worker this proves that its search consumed helper-authored
    /// TT data, not that the hit caused the returned move.
    pub tt_foreign_hits: u64,
    /// Sampled TT hits whose writer was not retained by the bounded registry.
    pub tt_unknown_hits: u64,
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
    /// Per-worker counters used only by opt-in scaling diagnostics.
    diagnostics: Vec<Arc<SearchDiagnostics>>,
    /// Shared-TT write observer used only by opt-in scaling diagnostics.
    tt_write_stats: Option<Arc<TtWriteStats>>,
    /// Shared sampled-work observer used only by opt-in scaling diagnostics.
    work_overlap_stats: Option<Arc<WorkOverlapStats>>,
    /// Shared sampled TT-owner observer used only by opt-in scaling
    /// diagnostics.
    tt_reuse_stats: Option<Arc<TtReuseStats>>,
}

#[derive(Default)]
struct LazyDiagnosticObservers {
    workers: Vec<Arc<SearchDiagnostics>>,
    tt_writes: Option<Arc<TtWriteStats>>,
    work_overlap: Option<Arc<WorkOverlapStats>>,
    tt_reuse: Option<Arc<TtReuseStats>>,
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
/// Choose the move by a depth- and score-weighted vote of the workers
/// instead of the deepest worker's result (see [`select_by_vote`]).
pub const LAZY_VOTE: u32 = 16;
/// Take worker 0's result; the helpers only fill the shared TT.
pub const LAZY_MAIN_RESULT: u32 = 32;
/// Among the deepest workers, take the lowest-numbered one instead of the
/// highest-scoring one (the highest of several scores of the same depth is
/// biased upward).
pub const LAZY_TIE_FIRST: u32 = 64;
/// Give each worker a private TT. This is a diagnostic control for measuring
/// the causal value of TT sharing and is not part of the default policy.
pub const LAZY_ISOLATED_TT: u32 = 128;
/// With [`LAZY_DEPTH_SKEW`], use a four-worker depth ladder (0, 1, 2, 3)
/// instead of two identical parity groups (0, 1, 0, 1). This is an
/// experimental diversity control and is not part of the default policy.
pub const LAZY_DEPTH_LADDER: u32 = 256;
/// Rotate each helper's initial root order by its worker index. The main
/// worker keeps the normal order, and iterative-deepening feedback replaces
/// the rotated seed after the first completed root search.
pub const LAZY_ROOT_ROTATE: u32 = 512;
/// Preserve the deepest completed iteration, then use every worker's reported
/// move as a consensus tie-break among moves found at that depth. This avoids
/// the depth loss possible with [`LAZY_VOTE`] while still using helper output
/// when equally deep workers disagree.
pub const LAZY_DEPTH_CONSENSUS: u32 = 1024;
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
        Self::with_flags_and_hash_mb(tt, workers, flags, 16)
    }

    /// Create a Lazy SMP searcher with explicit behaviour switches and a total
    /// TT memory budget. [`LAZY_ISOLATED_TT`] divides that budget among the
    /// workers so shared and isolated diagnostics reserve comparable memory.
    pub fn with_flags_and_hash_mb(
        tt: Arc<Tt>,
        workers: usize,
        flags: u32,
        total_hash_mb: usize,
    ) -> Self {
        Self::from_parts(
            tt,
            workers,
            flags,
            total_hash_mb,
            LazyDiagnosticObservers::default(),
        )
    }

    fn from_parts(
        tt: Arc<Tt>,
        workers: usize,
        flags: u32,
        total_hash_mb: usize,
        observers: LazyDiagnosticObservers,
    ) -> Self {
        let LazyDiagnosticObservers {
            workers: diagnostics,
            tt_writes: tt_write_stats,
            work_overlap: work_overlap_stats,
            tt_reuse: tt_reuse_stats,
        } = observers;
        let workers = workers.max(1);
        debug_assert!(diagnostics.is_empty() || diagnostics.len() == workers);
        let share_tt = flags & LAZY_ISOLATED_TT == 0;
        let worker_hash_mb = if share_tt {
            total_hash_mb
        } else {
            (total_hash_mb / workers).max(1)
        };
        let external_abort = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let persistent = if flags & LAZY_PERSISTENT != 0 {
            (0..workers)
                .map(|index| {
                    let worker_tt = if share_tt {
                        tt.clone()
                    } else {
                        Tt::new(worker_hash_mb)
                    };
                    Self::worker_searcher(
                        worker_tt,
                        external_abort.clone(),
                        diagnostics.get(index).cloned(),
                        flags,
                        index,
                    )
                })
                .collect()
        } else {
            Vec::new()
        };
        Self {
            tt,
            workers,
            share_tt,
            hash_mb: worker_hash_mb,
            external_abort,
            flags,
            persistent,
            diagnostics,
            tt_write_stats,
            work_overlap_stats,
            tt_reuse_stats,
        }
    }

    /// Create a Lazy SMP searcher with counters for worker-local TT probes and
    /// shared-TT write topology. This deliberately remains a separate
    /// constructor so normal searches do not execute per-node atomics.
    pub fn diagnostic(workers: usize, flags: u32, total_hash_mb: usize) -> Self {
        let workers = workers.max(1);
        let share_tt = flags & LAZY_ISOLATED_TT == 0;
        let tt_write_stats = share_tt.then(|| Arc::new(TtWriteStats::default()));
        let tt = Tt::new_with_stats(total_hash_mb, tt_write_stats.clone());
        let work_overlap_stats = Arc::new(WorkOverlapStats::new());
        let tt_reuse_stats = Arc::new(TtReuseStats::new());
        let diagnostics: Vec<_> = (0..workers)
            .map(|index| {
                Arc::new(SearchDiagnostics::parallel_worker(
                    work_overlap_stats.clone(),
                    tt_reuse_stats.clone(),
                    index,
                ))
            })
            .collect();
        Self::from_parts(
            tt,
            workers,
            flags,
            total_hash_mb,
            LazyDiagnosticObservers {
                workers: diagnostics,
                tt_writes: tt_write_stats,
                work_overlap: Some(work_overlap_stats),
                tt_reuse: Some(tt_reuse_stats),
            },
        )
    }

    fn skew(flags: u32, index: usize) -> u32 {
        if flags & LAZY_DEPTH_SKEW == 0 {
            0
        } else if flags & LAZY_DEPTH_LADDER != 0 {
            (index % 4) as u32
        } else {
            (index % 2) as u32
        }
    }

    fn root_order_offset(flags: u32, index: usize) -> usize {
        if flags & LAZY_ROOT_ROTATE != 0 {
            index
        } else {
            0
        }
    }

    fn worker_searcher(
        tt: Arc<Tt>,
        external_abort: Arc<std::sync::atomic::AtomicBool>,
        diagnostics: Option<Arc<SearchDiagnostics>>,
        flags: u32,
        index: usize,
    ) -> Searcher {
        let mut searcher = if let Some(diagnostics) = diagnostics {
            Searcher::with_abort_flag_and_diagnostics(tt, external_abort, diagnostics)
        } else {
            Searcher::with_abort_flag(tt, external_abort)
        };
        searcher.set_depth_skew(Self::skew(flags, index));
        searcher.set_root_order_offset(Self::root_order_offset(flags, index));
        searcher.set_ybw_split(flags & LAZY_NO_YBW == 0);
        searcher
    }

    /// Create a diagnostic searcher whose workers use isolated TT instances.
    /// This is a causal control for measuring the value of TT sharing.
    pub fn new_isolated(tt: Arc<Tt>, workers: usize) -> Self {
        Self::new_isolated_with_hash_mb(tt, workers, 16)
    }

    /// Isolated-TT diagnostic constructor with an explicit table size.
    pub fn new_isolated_with_hash_mb(tt: Arc<Tt>, workers: usize, hash_mb: usize) -> Self {
        Self::with_flags_and_hash_mb(
            tt,
            workers,
            LAZY_ISOLATED_TT,
            hash_mb.saturating_mul(workers.max(1)),
        )
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
        if self.share_tt {
            // Every persistent worker points at this table, so advance it
            // exactly once rather than once per worker.
            self.tt.new_search();
        } else {
            // Isolated non-persistent workers allocate a fresh table for every
            // search. Persistent isolated workers keep one table each and
            // therefore need their generations advanced individually.
            for searcher in &self.persistent {
                searcher.new_search();
            }
        }
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
        if let Some(stats) = &self.work_overlap_stats {
            stats.reset();
        }
        if let Some(stats) = &self.tt_reuse_stats {
            stats.reset();
        }
        let tt_before = self.tt_write_stats.as_ref().map(|stats| stats.snapshot());
        let results: Vec<(SearchInfo, Option<SearchDiagnosticsSnapshot>)> = (0..self.workers)
            .into_par_iter()
            .map(|index| {
                let mut worker_board = board.clone();
                let diagnostics = self.diagnostics.get(index);
                let before = diagnostics.map(|observer| observer.snapshot());
                let info = if let Some(searcher) = self.persistent.get(index) {
                    searcher.search_with_history(&mut worker_board, config, history)
                } else {
                    let worker_tt = if self.share_tt {
                        self.tt.clone()
                    } else {
                        Tt::new(self.hash_mb)
                    };
                    let searcher = Self::worker_searcher(
                        worker_tt,
                        self.external_abort.clone(),
                        diagnostics.cloned(),
                        self.flags,
                        index,
                    );
                    searcher.search_with_history(&mut worker_board, config, history)
                };
                if index == 0 && self.workers > 1 && self.flags & LAZY_MAIN_STOPS != 0 {
                    // The main worker owns time management; helpers stop with it.
                    self.external_abort
                        .store(true, std::sync::atomic::Ordering::Relaxed);
                }
                let counters = diagnostics
                    .zip(before)
                    .map(|(observer, before)| diagnostics_delta(observer.snapshot(), before));
                (info, counters)
            })
            .collect();

        let total_nodes = results.iter().map(|(info, _)| info.nodes).sum();
        let mut worker_results: Vec<LazySmpWorkerInfo> = results
            .iter()
            .map(|(info, diagnostics)| LazySmpWorkerInfo {
                selected: false,
                best_move: info.best_move,
                score: info.score,
                depth: info.depth,
                nodes: info.nodes,
                elapsed: info.elapsed,
                aborted: info.aborted,
                abort_reason: info.abort_reason,
                tt_probes: diagnostics.map_or(0, |snapshot| snapshot.tt_probes),
                tt_hits: diagnostics.map_or(0, |snapshot| snapshot.tt_hits),
                tt_stores: diagnostics.map_or(0, |snapshot| snapshot.tt_stores),
                tt_local_hits: diagnostics.map_or(0, |snapshot| snapshot.tt_local_hits),
                tt_foreign_hits: diagnostics.map_or(0, |snapshot| snapshot.tt_foreign_hits),
                tt_unknown_hits: diagnostics.map_or(0, |snapshot| snapshot.tt_unknown_hits),
            })
            .collect();
        let result = select_worker_result(
            results.into_iter().map(|(info, _)| info).collect(),
            self.flags,
        );
        let selected_worker = worker_results
            .iter()
            .position(|worker| {
                worker.best_move == result.best_move
                    && worker.score == result.score
                    && worker.depth == result.depth
                    && worker.nodes == result.nodes
            })
            .expect("selected Lazy SMP result must come from one worker");
        worker_results[selected_worker].selected = true;
        LazySmpInfo {
            result,
            total_nodes,
            worker_results,
            workers: self.workers,
            elapsed: started.elapsed(),
            tt_writes: self
                .tt_write_stats
                .as_ref()
                .zip(tt_before)
                .map(|(stats, before)| tt_write_delta(stats.snapshot(), before)),
            work_overlap: self
                .work_overlap_stats
                .as_ref()
                .zip(self.tt_reuse_stats.as_ref())
                .map(|(overlap, reuse)| {
                    work_overlap_snapshot(overlap.snapshot(), reuse.snapshot())
                }),
        }
    }
}

fn diagnostics_delta(
    after: SearchDiagnosticsSnapshot,
    before: SearchDiagnosticsSnapshot,
) -> SearchDiagnosticsSnapshot {
    SearchDiagnosticsSnapshot {
        tt_probes: after.tt_probes.saturating_sub(before.tt_probes),
        tt_hits: after.tt_hits.saturating_sub(before.tt_hits),
        tt_stores: after.tt_stores.saturating_sub(before.tt_stores),
        tt_local_hits: after.tt_local_hits.saturating_sub(before.tt_local_hits),
        tt_foreign_hits: after.tt_foreign_hits.saturating_sub(before.tt_foreign_hits),
        tt_unknown_hits: after.tt_unknown_hits.saturating_sub(before.tt_unknown_hits),
        ..SearchDiagnosticsSnapshot::default()
    }
}

fn tt_write_delta(after: TtWriteSnapshot, before: TtWriteSnapshot) -> TtWriteSnapshot {
    TtWriteSnapshot {
        attempted: after.attempted.saturating_sub(before.attempted),
        committed: after.committed.saturating_sub(before.committed),
        same_hash: after.same_hash.saturating_sub(before.same_hash),
        equal_depth_overwrites: after
            .equal_depth_overwrites
            .saturating_sub(before.equal_depth_overwrites),
        shallower_rejections: after
            .shallower_rejections
            .saturating_sub(before.shallower_rejections),
        collision_overwrites: after
            .collision_overwrites
            .saturating_sub(before.collision_overwrites),
    }
}

/// The result Lazy SMP reports from its workers' results (in worker order)
/// under the behaviour switches `flags`: worker 0's ([`LAZY_MAIN_RESULT`]),
/// a proven winning mate, the vote ([`LAZY_VOTE`]), depth-preserving consensus
/// ([`LAZY_DEPTH_CONSENSUS`]), the deepest with ties to the lowest-numbered
/// worker ([`LAZY_TIE_FIRST`]), or the default deepest result with score and
/// move tie-breaks.
pub fn select_worker_result(results: Vec<SearchInfo>, flags: u32) -> SearchInfo {
    if flags & LAZY_MAIN_RESULT != 0 {
        return results
            .into_iter()
            .next()
            .expect("Lazy SMP always has at least one worker");
    }

    if let Some(index) = proven_winning_mate_index(&results) {
        return results
            .into_iter()
            .nth(index)
            .expect("proven mate index comes from the worker results");
    }

    let results = results.into_iter();
    if flags & LAZY_VOTE != 0 {
        return select_by_vote(results.collect());
    } else if flags & LAZY_DEPTH_CONSENSUS != 0 {
        return select_by_depth_consensus(results.collect());
    } else if flags & LAZY_TIE_FIRST != 0 {
        results.reduce(|left, right| {
            if right.depth > left.depth {
                right
            } else {
                left
            }
        })
    } else {
        results.reduce(select_result)
    }
    .expect("Lazy SMP always has at least one worker")
}

/// Keep the deepest completed iteration and use shallower workers only as
/// supporting votes for the moves reported by deepest workers.
///
/// A move not produced at the maximum depth can never win. Among eligible
/// moves, the number of workers reporting that move is the primary key; the
/// selected result still comes from a maximum-depth worker, with score and
/// move order as deterministic tie-breaks. Thus this selector cannot reduce
/// the completed depth merely to follow a majority of shallower helpers.
pub fn select_by_depth_consensus(mut results: Vec<SearchInfo>) -> SearchInfo {
    if let Some(index) = proven_winning_mate_index(&results) {
        return results.remove(index);
    }

    let max_depth = results
        .iter()
        .map(|result| result.depth)
        .max()
        .expect("Lazy SMP always has at least one worker");

    let mut candidates: Vec<(crate::mv::Move, usize)> = Vec::new();
    for result in results.iter().filter(|result| result.depth == max_depth) {
        let Some(mv) = result.best_move else {
            continue;
        };
        if candidates.iter().all(|(candidate, _)| *candidate != mv) {
            let support = results
                .iter()
                .filter(|worker| worker.best_move == Some(mv))
                .count();
            candidates.push((mv, support));
        }
    }

    results
        .into_iter()
        .filter(|result| result.depth == max_depth)
        .max_by_key(|result| {
            let support = result.best_move.map_or(0, |mv| {
                candidates
                    .iter()
                    .find_map(|(candidate, support)| (*candidate == mv).then_some(*support))
                    .unwrap_or(0)
            });
            (support, result.score, move_key(result.best_move))
        })
        .expect("maximum Lazy SMP depth comes from one worker")
}

/// Return the worker that proved the shortest winning mate.
///
/// A completed exact mate is stronger evidence than a deeper heuristic score,
/// so ordinary Lazy SMP selectors must not discard it merely because another
/// worker completed one more iteration. [`LAZY_MAIN_RESULT`] remains an
/// explicit diagnostic exception: that mode promises to return worker 0.
fn proven_winning_mate_index(results: &[SearchInfo]) -> Option<usize> {
    let mate_bound = crate::search::MATE_SCORE - 1000;
    results
        .iter()
        .enumerate()
        .filter(|(_, result)| {
            result.depth > 0
                && result.best_move.is_some()
                && result.completed_bound == crate::search::SearchBound::Exact
                && result.score >= mate_bound
        })
        .max_by_key(|(_, result)| (result.score, result.depth, move_key(result.best_move)))
        .map(|(index, _)| index)
}

fn select_result(left: SearchInfo, right: SearchInfo) -> SearchInfo {
    let left_key = (left.depth, left.score, move_key(left.best_move));
    let right_key = (right.depth, right.score, move_key(right.best_move));
    if right_key > left_key { right } else { left }
}

/// Weight added to every vote above the lowest worker score, so that the
/// lowest-scoring worker still counts.
const VOTE_BASE: i64 = 24;

/// Pick the result whose move collects the most votes: each worker with a
/// move votes for it with `(score - lowest score + VOTE_BASE) * depth`.
/// Among the workers behind the winning move, the deepest (then highest
/// scoring) one supplies the result. A worker that found a mate for the side
/// to move overrides the vote (the shortest such mate wins), since a proven
/// win needs no majority.
pub fn select_by_vote(mut results: Vec<SearchInfo>) -> SearchInfo {
    if let Some(index) = proven_winning_mate_index(&results) {
        return results.remove(index);
    }
    let with_move: Vec<&SearchInfo> = results.iter().filter(|r| r.best_move.is_some()).collect();
    let chosen = if with_move.is_empty() {
        None
    } else {
        let lowest = with_move
            .iter()
            .map(|r| i64::from(r.score))
            .min()
            .unwrap_or(0);
        let mut votes: Vec<(crate::mv::Move, i64)> = Vec::new();
        for r in &with_move {
            let mv = r.best_move.expect("filtered");
            let w = (i64::from(r.score) - lowest + VOTE_BASE) * i64::from(r.depth.max(1));
            match votes.iter_mut().find(|(m, _)| *m == mv) {
                Some(entry) => entry.1 += w,
                None => votes.push((mv, w)),
            }
        }
        let (winner, _) = votes
            .iter()
            .copied()
            .max_by_key(|&(m, w)| (w, move_key(Some(m))))
            .expect("at least one vote");
        with_move
            .iter()
            .filter(|r| r.best_move == Some(winner))
            .map(|r| (r.score, r.depth, move_key(r.best_move)))
            .max_by_key(|&(score, depth, _)| (depth, score))
    };
    let mut fallback = None;
    let mut picked = None;
    for r in results {
        if picked.is_none()
            && chosen.is_some_and(|(score, depth, key)| {
                r.score == score && r.depth == depth && move_key(r.best_move) == key
            })
        {
            picked = Some(r);
        } else {
            fallback = Some(match fallback {
                None => r,
                Some(f) => select_result(f, r),
            });
        }
    }
    picked
        .or(fallback)
        .expect("Lazy SMP always has at least one worker")
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mv::Move;
    use crate::piece::PieceKind;
    use crate::search::SearchBound;
    use crate::square::Square;

    fn info(to: u8, score: i32, depth: u32) -> SearchInfo {
        SearchInfo {
            best_move: Some(Move::drop(Square::from_index(to), PieceKind::Kin)),
            score,
            depth,
            nodes: 0,
            elapsed: Duration::ZERO,
            hashfull: 0,
            bound: SearchBound::Exact,
            completed_bound: SearchBound::Exact,
            aborted: false,
            abort_reason: "none",
            pv: Vec::new(),
        }
    }

    #[test]
    fn depth_ladder_diversifies_four_workers_without_changing_default_skew() {
        let default = (0..4)
            .map(|worker| LazySmpSearcher::skew(LAZY_DEPTH_SKEW, worker))
            .collect::<Vec<_>>();
        let ladder = (0..8)
            .map(|worker| LazySmpSearcher::skew(LAZY_DEPTH_SKEW | LAZY_DEPTH_LADDER, worker))
            .collect::<Vec<_>>();

        assert_eq!(default, [0, 1, 0, 1]);
        assert_eq!(ladder, [0, 1, 2, 3, 0, 1, 2, 3]);
        assert_eq!(LazySmpSearcher::skew(LAZY_DEPTH_LADDER, 3), 0);
    }

    #[test]
    fn root_rotation_keeps_main_order_and_offsets_helpers() {
        let flags = LAZY_DEFAULT_FLAGS | LAZY_ROOT_ROTATE;
        let offsets = (0..4)
            .map(|worker| LazySmpSearcher::root_order_offset(flags, worker))
            .collect::<Vec<_>>();

        assert_eq!(offsets, [0, 1, 2, 3]);
        assert_eq!(LazySmpSearcher::root_order_offset(LAZY_DEFAULT_FLAGS, 3), 0);
    }

    #[test]
    fn vote_prefers_the_move_most_workers_agree_on() {
        // One deeper worker alone against three agreeing workers.
        let picked = select_by_vote(vec![
            info(1, 120, 12),
            info(2, 100, 11),
            info(2, 105, 11),
            info(2, 98, 11),
        ]);
        assert_eq!(
            picked.best_move,
            Some(Move::drop(Square::from_index(2), PieceKind::Kin))
        );
        assert_eq!(picked.score, 105);
        // The deepest-worker rule would have taken move 1.
        let deepest = vec![info(1, 120, 12), info(2, 100, 11)]
            .into_iter()
            .reduce(select_result)
            .expect("two results");
        assert_eq!(
            deepest.best_move,
            Some(Move::drop(Square::from_index(1), PieceKind::Kin))
        );
    }

    #[test]
    fn depth_consensus_uses_support_without_sacrificing_depth() {
        let picked = select_by_depth_consensus(vec![
            info(1, 120, 12),
            info(2, 100, 12),
            info(2, 90, 11),
            info(2, 80, 10),
        ]);

        assert_eq!(picked.depth, 12);
        assert_eq!(
            picked.best_move,
            Some(Move::drop(Square::from_index(2), PieceKind::Kin))
        );
        assert_eq!(picked.score, 100);
    }

    #[test]
    fn depth_consensus_cannot_promote_a_shallower_majority() {
        let picked = select_worker_result(
            vec![
                info(1, 120, 12),
                info(2, 100, 11),
                info(2, 105, 11),
                info(2, 98, 11),
            ],
            LAZY_DEPTH_CONSENSUS,
        );

        assert_eq!(picked.depth, 12);
        assert_eq!(
            picked.best_move,
            Some(Move::drop(Square::from_index(1), PieceKind::Kin))
        );
    }

    #[test]
    fn depth_consensus_falls_back_deterministically_without_moves() {
        let mut first = info(1, 10, 8);
        first.best_move = None;
        let mut second = info(2, 20, 8);
        second.best_move = None;

        let picked = select_by_depth_consensus(vec![first, second]);

        assert_eq!(picked.depth, 8);
        assert_eq!(picked.score, 20);
        assert_eq!(picked.best_move, None);
    }

    #[test]
    fn main_result_and_first_tie_avoid_the_highest_equal_depth_score() {
        let results = || vec![info(1, -100, 9), info(2, 60, 9), info(3, 10, 8)];
        let to = |r: SearchInfo| r.best_move.map(|m| m.to.index());
        // Default: the deepest, ties to the highest score.
        assert_eq!(to(select_worker_result(results(), 0)), Some(2));
        assert_eq!(to(select_worker_result(results(), LAZY_TIE_FIRST)), Some(1));
        assert_eq!(
            to(select_worker_result(results(), LAZY_MAIN_RESULT)),
            Some(1)
        );
        // A deeper later worker still wins with LAZY_TIE_FIRST.
        let deeper = vec![info(1, -100, 9), info(2, -150, 10)];
        assert_eq!(to(select_worker_result(deeper, LAZY_TIE_FIRST)), Some(2));
    }

    #[test]
    fn isolated_tt_flag_keeps_the_default_policy_unchanged() {
        let shared = LazySmpSearcher::with_flags_and_hash_mb(Tt::new(1), 2, LAZY_DEFAULT_FLAGS, 1);
        let isolated = LazySmpSearcher::with_flags_and_hash_mb(
            Tt::new(1),
            2,
            LAZY_DEFAULT_FLAGS | LAZY_ISOLATED_TT,
            8,
        );

        assert!(shared.share_tt);
        assert!(!isolated.share_tt);
        assert_eq!(isolated.hash_mb, 4);
        assert_eq!(shared.persistent.len(), 2);
        assert_eq!(isolated.persistent.len(), 2);
    }

    #[test]
    fn new_search_ages_shared_and_persistent_isolated_tables_once() {
        let shared_tt = Tt::new(1);
        let shared =
            LazySmpSearcher::with_flags_and_hash_mb(shared_tt.clone(), 2, LAZY_DEFAULT_FLAGS, 1);
        shared.new_search();
        assert_eq!(shared_tt.search_count(), 1);
        assert!(
            shared
                .persistent
                .iter()
                .all(|searcher| searcher.tt_search_count() == 1)
        );

        let unused_front_tt = Tt::new(1);
        let isolated = LazySmpSearcher::with_flags_and_hash_mb(
            unused_front_tt.clone(),
            2,
            LAZY_DEFAULT_FLAGS | LAZY_ISOLATED_TT,
            2,
        );
        isolated.new_search();
        assert_eq!(unused_front_tt.search_count(), 0);
        assert!(
            isolated
                .persistent
                .iter()
                .all(|searcher| searcher.tt_search_count() == 1)
        );
    }

    #[test]
    fn vote_yields_to_a_found_mate() {
        let mate = crate::search::MATE_SCORE - 7;
        let picked = select_by_vote(vec![info(2, 100, 12), info(2, 101, 12), info(3, mate, 9)]);
        assert_eq!(picked.score, mate);
    }

    #[test]
    fn proven_winning_mate_overrides_a_deeper_heuristic_result() {
        let mate = crate::search::MATE_SCORE - 7;
        let results = || vec![info(1, 120, 12), info(2, mate, 9), info(3, 80, 13)];
        let to = |result: SearchInfo| result.best_move.map(|mv| mv.to.index());

        assert_eq!(to(select_worker_result(results(), 0)), Some(2));
        assert_eq!(to(select_worker_result(results(), LAZY_TIE_FIRST)), Some(2));
        assert_eq!(to(select_worker_result(results(), LAZY_VOTE)), Some(2));
        assert_eq!(
            to(select_worker_result(results(), LAZY_DEPTH_CONSENSUS)),
            Some(2)
        );
        assert_eq!(to(select_by_depth_consensus(results())), Some(2));
        assert_eq!(
            to(select_worker_result(results(), LAZY_MAIN_RESULT)),
            Some(1)
        );
    }

    #[test]
    fn unproven_mate_like_score_does_not_override_depth() {
        let mut unproven = info(2, crate::search::MATE_SCORE - 7, 9);
        unproven.completed_bound = SearchBound::Unknown;

        let picked = select_worker_result(vec![info(1, 120, 12), unproven], 0);
        assert_eq!(picked.best_move.map(|mv| mv.to.index()), Some(1));
    }

    #[test]
    fn diagnostics_mark_exactly_one_selected_worker() {
        let searcher = LazySmpSearcher::new(Tt::new(1), 2);
        let board = Board::startpos();
        let result = searcher.search(
            &board,
            SearchConfig {
                max_depth: 4,
                node_limit: Some(500),
                ..SearchConfig::default()
            },
        );

        assert_eq!(result.worker_results.len(), 2);
        assert_eq!(
            result
                .worker_results
                .iter()
                .filter(|worker| worker.selected)
                .count(),
            1
        );
        let selected = result
            .worker_results
            .iter()
            .find(|worker| worker.selected)
            .expect("one worker must be selected");
        assert_eq!(selected.best_move, result.result.best_move);
        assert_eq!(selected.score, result.result.score);
        assert_eq!(selected.depth, result.result.depth);
        assert_eq!(selected.nodes, result.result.nodes);
        assert_eq!(selected.elapsed, result.result.elapsed);
        assert_eq!(selected.aborted, result.result.aborted);
        assert_eq!(selected.abort_reason, result.result.abort_reason);
        assert_eq!(
            result.total_nodes,
            result
                .worker_results
                .iter()
                .map(|worker| worker.nodes)
                .sum::<u64>()
        );
        assert!(result.tt_writes.is_none());
        assert!(
            result
                .worker_results
                .iter()
                .all(|worker| worker.tt_probes == 0 && worker.tt_stores == 0)
        );
    }

    #[test]
    fn opt_in_diagnostics_report_worker_probes_and_shared_tt_writes() {
        let searcher = LazySmpSearcher::diagnostic(2, LAZY_DEFAULT_FLAGS, 1);
        let result = searcher.search(
            &Board::startpos(),
            SearchConfig {
                max_depth: 5,
                node_limit: Some(2_000),
                ..SearchConfig::default()
            },
        );

        let writes = result
            .tt_writes
            .expect("shared diagnostic TT must report write topology");
        assert!(writes.attempted > 0);
        assert!(writes.committed > 0);
        assert!(
            result
                .worker_results
                .iter()
                .map(|worker| worker.tt_probes)
                .sum::<u64>()
                > 0
        );
        assert!(
            result
                .worker_results
                .iter()
                .map(|worker| worker.tt_stores)
                .sum::<u64>()
                > 0
        );
        let overlap = result
            .work_overlap
            .expect("opt-in diagnostics must report sampled work overlap");
        assert!(overlap.dropped <= overlap.sampled_visits);
    }
}
