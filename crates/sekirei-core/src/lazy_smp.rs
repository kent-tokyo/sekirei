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
    AspirationSnapshot, CooperativeRootStats, CooperativeRootWaitStats, HelperRetirement,
    NmpSnapshot, RootHintChannel, SearchConfig, SearchDiagnostics, SearchDiagnosticsSnapshot,
    SearchInfo, Searcher, TtReuseSnapshot, TtReuseStats, WorkOverlapSnapshot, WorkOverlapStats,
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

/// Invocation-scoped evidence for helper-to-main root-order hints.
///
/// The counters exist only when [`LAZY_FOREIGN_ROOT_HINT`] is enabled with a
/// shared transposition table and at least two workers.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LazySmpRootHintStats {
    /// Helper publications accepted as lock-free hint-slot updates.
    pub publications: u64,
    /// Same-generation, same-or-deeper hints read by the main worker.
    pub eligible_reads: u64,
    /// Reads that actually moved a legal final root candidate to the front.
    pub applied: u64,
}

/// Additional invocation-bound effectiveness counters, separate from the stable
/// three-field [`LazySmpRootHintStats`] public literal contract.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LazySmpRootHintEffectiveness {
    /// Helper publications accepted by the hint slot.
    pub publications: u64,
    /// Eligible hint reads by worker zero.
    pub eligible_reads: u64,
    /// Reads that changed root ordering.
    pub applied: u64,
    /// Invocation-scoped root-hint effectiveness diagnostic.
    pub first_move_beta_cuts: u64,
    /// Invocation-scoped root-hint effectiveness diagnostic.
    pub researches: u64,
    /// Invocation-scoped root-hint effectiveness diagnostic.
    pub research_nodes: u64,
    /// Invocation-scoped root-hint effectiveness diagnostic.
    pub stale_research_nodes: u64,
    /// Invocation-scoped root-hint effectiveness diagnostic.
    pub stale_reads: u64,
    /// Invocation-scoped root-hint effectiveness diagnostic.
    pub depth_gap_sum: u64,
    /// Invocation-scoped root-hint effectiveness diagnostic.
    pub depth_gap_max: u64,
    /// Invocation-scoped root-hint effectiveness diagnostic.
    pub generation_gap_sum: u64,
    /// Invocation-scoped root-hint effectiveness diagnostic.
    pub generation_gap_max: u64,
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
}

/// Sampled TT ownership counters for one Lazy SMP worker.
///
/// This additive diagnostic type is intentionally separate from
/// [`LazySmpWorkerInfo`], whose public struct-literal layout is stable in 0.4.x.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LazySmpWorkerTtReuse {
    /// Sampled TT hits on entries last written by this worker.
    pub local_hits: u64,
    /// Sampled TT hits on entries last written by another worker.
    pub foreign_hits: u64,
    /// Sampled TT hits whose writer was not retained by the bounded registry.
    pub unknown_hits: u64,
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
    /// Invocation-scoped helper-to-main ordering hint. It is present only for
    /// shared-TT searches with at least two workers and the opt-in flag.
    root_hint: Option<Arc<RootHintChannel>>,
    retirement: Option<Arc<HelperRetirement>>,
    /// Search-level observers and persistent workers are invocation-scoped.
    /// Serialize overlapping calls instead of mixing their resettable state.
    search_active: std::sync::atomic::AtomicBool,
    cooperative_stats: std::sync::Mutex<Option<(CooperativeRootStats, CooperativeRootWaitStats)>>,
    /// Lifecycle mutations are outside the search loop, but must be serialized
    /// so one waiter cannot re-arm the abort flag after another waiter reset it.
    lifecycle_mutation: std::sync::Mutex<()>,
}

struct SearchInvocationGuard<'a> {
    active: &'a std::sync::atomic::AtomicBool,
}

impl<'a> SearchInvocationGuard<'a> {
    fn try_acquire(active: &'a std::sync::atomic::AtomicBool) -> Option<Self> {
        active
            .compare_exchange(
                false,
                true,
                std::sync::atomic::Ordering::AcqRel,
                std::sync::atomic::Ordering::Acquire,
            )
            .ok()
            .map(|_| Self { active })
    }

    fn acquire_blocking(active: &'a std::sync::atomic::AtomicBool) -> Self {
        loop {
            if let Some(guard) = Self::try_acquire(active) {
                return guard;
            }
            std::thread::yield_now();
        }
    }

    fn acquire_after_requesting_abort(
        active: &'a std::sync::atomic::AtomicBool,
        external_abort: &std::sync::atomic::AtomicBool,
    ) -> Self {
        loop {
            if let Some(guard) = Self::try_acquire(active) {
                return guard;
            }
            // A lifecycle mutation must not race an active worker group, but
            // merely waiting could deadlock behind `go infinite`. Request a
            // cooperative stop, then wait until the owner joins every worker.
            external_abort.store(true, std::sync::atomic::Ordering::Relaxed);
            std::thread::yield_now();
        }
    }
}

impl Drop for SearchInvocationGuard<'_> {
    fn drop(&mut self) {
        self.active
            .store(false, std::sync::atomic::Ordering::Release);
    }
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
/// Let helpers publish the move from a fully completed iteration. Worker 0
/// may try a same-or-deeper hint first on its next root iteration. Only move
/// ordering is shared; helper scores, bounds and PVs never become results.
pub const LAZY_FOREIGN_ROOT_HINT: u32 = 2048;
/// Remove the normal one-ply root-LMR relief for helper workers only. Worker
/// 0 keeps the established root search, while helpers probe late quiet moves
/// more cheaply. This remains opt-in until an independent strength gate.
pub const LAZY_HELPER_ROOT_LMR: u32 = 4096;
/// Retire dominated helpers only after an exact completed iteration.
pub const LAZY_DOMINATED_RETIREMENT: u32 = 8192;
/// Two-participant root-only cooperative split prototype. Other counts fall back.
pub const LAZY_ROOT_SPLIT: u32 = 16384;
/// Default behaviour switches.
pub const LAZY_DEFAULT_FLAGS: u32 =
    LAZY_PERSISTENT | LAZY_MAIN_STOPS | LAZY_DEPTH_SKEW | LAZY_NO_YBW;

impl LazySmpSearcher {
    /// Completed exact iterations, retirement status, and sampled foreign reads
    /// of each worker's committed entries. Available only for the retirement experiment.
    pub fn retirement_stats(&self) -> Option<Vec<(u64, bool, u64)>> {
        self.retirement.as_ref().map(|channel| channel.snapshot())
    }

    /// Last joined cooperative invocation (outside the node loop).
    pub fn cooperative_root_stats(&self) -> Option<CooperativeRootStats> {
        self.cooperative_stats
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .map(|(stats, _)| stats)
    }

    /// Wait-phase evidence published together with the last joined root split.
    pub fn cooperative_root_wait_stats(&self) -> Option<CooperativeRootWaitStats> {
        self.cooperative_stats
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .map(|(_, stats)| stats)
    }

    /// Per-worker root aspiration evidence; empty with ordinary workers.
    pub fn aspiration_stats(&self) -> Vec<AspirationSnapshot> {
        self.diagnostics
            .iter()
            .map(|d| d.aspiration_snapshot())
            .collect()
    }

    /// Per-worker NMP diagnostic snapshots; empty with ordinary workers.
    pub fn nmp_stats(&self) -> Vec<NmpSnapshot> {
        self.diagnostics.iter().map(|d| d.nmp_snapshot()).collect()
    }

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
        let root_hint =
            (flags & LAZY_FOREIGN_ROOT_HINT != 0 && share_tt && workers > 1 && workers <= 512)
                .then(|| Arc::new(RootHintChannel::new()));
        let retirement = ((flags & LAZY_DOMINATED_RETIREMENT != 0 || !diagnostics.is_empty())
            && share_tt
            && (2..=512).contains(&workers))
        .then(|| {
            Arc::new(HelperRetirement::new(
                workers,
                flags & LAZY_DOMINATED_RETIREMENT != 0 && workers >= 3,
            ))
        });
        let mut persistent: Vec<Searcher> = if flags & LAZY_PERSISTENT != 0 {
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
                        root_hint.clone(),
                    )
                })
                .collect()
        } else {
            Vec::new()
        };
        if let Some(channel) = &retirement {
            for (worker, searcher) in persistent.iter_mut().enumerate() {
                searcher.set_retirement(channel.clone(), worker);
            }
        }
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
            root_hint,
            retirement,
            search_active: std::sync::atomic::AtomicBool::new(false),
            cooperative_stats: std::sync::Mutex::new(None),
            lifecycle_mutation: std::sync::Mutex::new(()),
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
        root_hint: Option<Arc<RootHintChannel>>,
    ) -> Searcher {
        let mut searcher = if let Some(diagnostics) = diagnostics {
            Searcher::with_abort_flag_and_diagnostics(tt, external_abort, diagnostics)
        } else {
            Searcher::with_abort_flag(tt, external_abort)
        };
        searcher.set_depth_skew(Self::skew(flags, index));
        searcher.set_root_order_offset(Self::root_order_offset(flags, index));
        searcher.set_full_root_lmr(flags & LAZY_HELPER_ROOT_LMR != 0 && index > 0);
        searcher.set_ybw_split(flags & LAZY_NO_YBW == 0);
        if let Some(root_hint) = root_hint {
            searcher.set_root_hint_channel(root_hint, index);
        }
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
    ///
    /// The invocation guard prevents a caller preparing the next search from
    /// erasing a stop request that still belongs to an active worker group.
    /// If a search is active, this call requests its cooperative stop and
    /// waits until that invocation has joined all workers.
    pub fn reset_abort_flag(&self) {
        let _lifecycle = self
            .lifecycle_mutation
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _invocation = SearchInvocationGuard::acquire_after_requesting_abort(
            &self.search_active,
            &self.external_abort,
        );
        self.external_abort
            .store(false, std::sync::atomic::Ordering::Relaxed);
    }

    /// Reset the shared TT between games, matching the regular searcher API.
    /// If a search is active, this call requests its cooperative stop and
    /// waits for that invocation to join all workers first.
    pub fn clear_tt(&self) {
        let _lifecycle = self
            .lifecycle_mutation
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _invocation = SearchInvocationGuard::acquire_after_requesting_abort(
            &self.search_active,
            &self.external_abort,
        );
        self.tt.clear();
        for searcher in &self.persistent {
            if self.share_tt {
                searcher.clear_worker_state();
            } else {
                searcher.clear_tt();
            }
        }
    }

    /// Age the shared TT entries of earlier searches (call once per `go`).
    /// If a search is active, this call requests its cooperative stop and
    /// waits for that invocation to join all workers first.
    pub fn new_search(&self) {
        let _lifecycle = self
            .lifecycle_mutation
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _invocation = SearchInvocationGuard::acquire_after_requesting_abort(
            &self.search_active,
            &self.external_abort,
        );
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
    ///
    /// Uses the current Rayon pool. Configure its worker stack with
    /// [`crate::search::RECURSIVE_SEARCH_STACK_BYTES`] for the supported recursive
    /// search budget, as the USI executable does.
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
        let _invocation = SearchInvocationGuard::acquire_blocking(&self.search_active);
        self.search_with_history_inner(board, config, history).0
    }

    /// Try to search and return invocation-bound opt-in diagnostics.
    ///
    /// The second tuple member is `None` when the experiment is unavailable.
    /// Its counters are captured before this invocation returns, so a later or
    /// overlapping diagnostic search cannot substitute another generation's
    /// values. The outer `Option` is `None` when any search invocation is
    /// already active on this searcher.
    pub fn try_search_with_history_and_root_hint_stats(
        &self,
        board: &Board,
        config: SearchConfig,
        history: &PositionHistory,
    ) -> Option<(
        LazySmpInfo,
        Option<LazySmpRootHintStats>,
        Vec<LazySmpWorkerTtReuse>,
    )> {
        let (info, effectiveness, tt_reuse) =
            self.try_search_with_history_and_root_hint_effectiveness(board, config, history)?;
        let stats = effectiveness.map(|value| LazySmpRootHintStats {
            publications: value.publications,
            eligible_reads: value.eligible_reads,
            applied: value.applied,
        });
        Some((info, stats, tt_reuse))
    }

    /// Try to search with invocation-bound extended root-hint evidence.
    ///
    /// The outer `None` means another invocation owns this searcher. Keeping
    /// this additive API separate preserves the original hint-stat literals
    /// and three-member result contract.
    pub fn try_search_with_history_and_root_hint_effectiveness(
        &self,
        board: &Board,
        config: SearchConfig,
        history: &PositionHistory,
    ) -> Option<(
        LazySmpInfo,
        Option<LazySmpRootHintEffectiveness>,
        Vec<LazySmpWorkerTtReuse>,
    )> {
        let _invocation = SearchInvocationGuard::try_acquire(&self.search_active)?;
        Some(self.search_with_history_inner(board, config, history))
    }

    fn search_with_history_inner(
        &self,
        board: &Board,
        config: SearchConfig,
        history: &PositionHistory,
    ) -> (
        LazySmpInfo,
        Option<LazySmpRootHintEffectiveness>,
        Vec<LazySmpWorkerTtReuse>,
    ) {
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
        if let Some(channel) = &self.retirement {
            channel.reset();
        }
        let root_hint_generation = self
            .root_hint
            .as_ref()
            .map(|root_hint| root_hint.begin_go());
        let tt_before = self.tt_write_stats.as_ref().map(|stats| stats.snapshot());
        let split_stats = std::sync::OnceLock::new();
        let cooperative = self.workers == 2 && self.flags & LAZY_ROOT_SPLIT != 0 && self.share_tt;
        let participants = if cooperative { 1 } else { self.workers };
        let results: Vec<(
            SearchInfo,
            Option<(SearchDiagnosticsSnapshot, TtReuseSnapshot)>,
        )> = (0..participants)
            .into_par_iter()
            .map(|index| {
                let mut worker_board = board.clone();
                let diagnostics = self.diagnostics.get(index);
                let before =
                    diagnostics.map(|observer| (observer.snapshot(), observer.tt_reuse_snapshot()));
                let info = if let Some(searcher) = self.persistent.get(index) {
                    if cooperative {
                        let (info, stats, wait_stats) = searcher
                            .search_cooperative_root_with_history_and_wait_stats(
                                &mut worker_board,
                                config,
                                history,
                            );
                        let _ = split_stats.set((stats, wait_stats));
                        info
                    } else {
                        searcher.search_with_history(&mut worker_board, config, history)
                    }
                } else {
                    let worker_tt = if self.share_tt {
                        self.tt.clone()
                    } else {
                        Tt::new(self.hash_mb)
                    };
                    let mut searcher = Self::worker_searcher(
                        worker_tt,
                        self.external_abort.clone(),
                        diagnostics.cloned(),
                        self.flags,
                        index,
                        self.root_hint.clone(),
                    );
                    if let Some(channel) = &self.retirement {
                        searcher.set_retirement(channel.clone(), index);
                    }
                    if cooperative {
                        let (info, stats, wait_stats) = searcher
                            .search_cooperative_root_with_history_and_wait_stats(
                                &mut worker_board,
                                config,
                                history,
                            );
                        let _ = split_stats.set((stats, wait_stats));
                        info
                    } else {
                        searcher.search_with_history(&mut worker_board, config, history)
                    }
                };
                if index == 0 && self.workers > 1 && self.flags & LAZY_MAIN_STOPS != 0 {
                    // The main worker owns time management; helpers stop with it.
                    self.external_abort
                        .store(true, std::sync::atomic::Ordering::Relaxed);
                }
                let counters = diagnostics.zip(before).map(|(observer, before)| {
                    (
                        diagnostics_delta(observer.snapshot(), before.0),
                        tt_reuse_delta(observer.tt_reuse_snapshot(), before.1),
                    )
                });
                (info, counters)
            })
            .collect();

        *self
            .cooperative_stats
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = split_stats.get().copied();
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
                tt_probes: diagnostics.map_or(0, |snapshot| snapshot.0.tt_probes),
                tt_hits: diagnostics.map_or(0, |snapshot| snapshot.0.tt_hits),
                tt_stores: diagnostics.map_or(0, |snapshot| snapshot.0.tt_stores),
            })
            .collect();
        let worker_tt_reuse = results
            .iter()
            .map(|(_, diagnostics)| {
                diagnostics.map_or_else(LazySmpWorkerTtReuse::default, |snapshot| {
                    LazySmpWorkerTtReuse {
                        local_hits: snapshot.1.local_hits,
                        foreign_hits: snapshot.1.foreign_hits,
                        unknown_hits: snapshot.1.unknown_hits,
                    }
                })
            })
            .collect();
        let mut search_results: Vec<SearchInfo> =
            results.into_iter().map(|(info, _)| info).collect();
        let selected_worker = select_worker_index(&search_results, self.flags);
        let result = search_results.remove(selected_worker);
        worker_results[selected_worker].selected = true;
        let info = LazySmpInfo {
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
        };
        let root_hint_stats =
            self.root_hint
                .as_ref()
                .zip(root_hint_generation)
                .map(|(channel, generation)| {
                    let value = channel.snapshot(generation);
                    LazySmpRootHintEffectiveness {
                        publications: value.publications,
                        eligible_reads: value.eligible_reads,
                        applied: value.applied,
                        first_move_beta_cuts: value.first_move_beta_cuts,
                        researches: value.researches,
                        research_nodes: value.research_nodes,
                        stale_research_nodes: value.stale_research_nodes,
                        stale_reads: value.stale_reads,
                        depth_gap_sum: value.depth_gap_sum,
                        depth_gap_max: value.depth_gap_max,
                        generation_gap_sum: value.generation_gap_sum,
                        generation_gap_max: value.generation_gap_max,
                    }
                });
        (info, root_hint_stats, worker_tt_reuse)
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
        ..SearchDiagnosticsSnapshot::default()
    }
}

fn tt_reuse_delta(after: TtReuseSnapshot, before: TtReuseSnapshot) -> TtReuseSnapshot {
    TtReuseSnapshot {
        local_hits: after.local_hits.saturating_sub(before.local_hits),
        foreign_hits: after.foreign_hits.saturating_sub(before.foreign_hits),
        unknown_hits: after.unknown_hits.saturating_sub(before.unknown_hits),
        ..TtReuseSnapshot::default()
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
    let mut results = results;
    let selected = select_worker_index(&results, flags);
    results.remove(selected)
}

fn select_worker_index(results: &[SearchInfo], flags: u32) -> usize {
    assert!(
        !results.is_empty(),
        "Lazy SMP always has at least one worker"
    );
    if flags & LAZY_MAIN_RESULT != 0 {
        return 0;
    }

    if let Some(index) = proven_winning_mate_index(results) {
        return index;
    }

    if flags & LAZY_VOTE != 0 {
        select_by_vote_index(results)
    } else if flags & LAZY_DEPTH_CONSENSUS != 0 {
        select_by_depth_consensus_index(results)
    } else if flags & LAZY_TIE_FIRST != 0 {
        results.iter().enumerate().fold(0, |best, (index, result)| {
            if result.depth > results[best].depth {
                index
            } else {
                best
            }
        })
    } else {
        results.iter().enumerate().fold(0, |best, (index, result)| {
            let best_key = (
                results[best].depth,
                results[best].score,
                move_key(results[best].best_move),
            );
            let key = (result.depth, result.score, move_key(result.best_move));
            if key > best_key { index } else { best }
        })
    }
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
    let index = proven_winning_mate_index(&results)
        .unwrap_or_else(|| select_by_depth_consensus_index(&results));
    results.remove(index)
}

fn select_by_depth_consensus_index(results: &[SearchInfo]) -> usize {
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
        .iter()
        .enumerate()
        .filter(|(_, result)| result.depth == max_depth)
        .max_by_key(|(_, result)| {
            let support = result.best_move.map_or(0, |mv| {
                candidates
                    .iter()
                    .find_map(|(candidate, support)| (*candidate == mv).then_some(*support))
                    .unwrap_or(0)
            });
            (support, result.score, move_key(result.best_move))
        })
        .map(|(index, _)| index)
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
    let index =
        proven_winning_mate_index(&results).unwrap_or_else(|| select_by_vote_index(&results));
    results.remove(index)
}

fn select_by_vote_index(results: &[SearchInfo]) -> usize {
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
    chosen
        .and_then(|(score, depth, key)| {
            results.iter().position(|result| {
                result.score == score && result.depth == depth && move_key(result.best_move) == key
            })
        })
        .unwrap_or_else(|| select_worker_index(results, 0))
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
    use crate::tt::{Bound, TtEntry};

    // Real recursive searches use the production pool-stack budget. Keep this
    // local to each fixture so ordinary cargo test needs no global pool/env mask.
    fn on_search_pool(workers: usize, test: impl FnOnce() + Send) {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .stack_size(crate::search::RECURSIVE_SEARCH_STACK_BYTES)
            .build()
            .expect("search regression pool");
        pool.install(test);
    }

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
    fn dominated_retirement_is_opt_in_and_invocation_scoped() {
        let plain = LazySmpSearcher::new(Tt::new(1), 4);
        assert!(plain.retirement_stats().is_none());
        let single = LazySmpSearcher::with_flags(
            Tt::new(1),
            1,
            LAZY_DEFAULT_FLAGS | LAZY_DOMINATED_RETIREMENT,
        );
        assert!(single.retirement_stats().is_none());
        let searcher = LazySmpSearcher::with_flags(
            Tt::new(1),
            4,
            LAZY_DEFAULT_FLAGS | LAZY_DOMINATED_RETIREMENT,
        );
        let board = Board::startpos();
        let history = PositionHistory::initial(board.hash());
        let config = SearchConfig {
            max_depth: 2,
            ..SearchConfig::default()
        };
        let first = searcher.search_with_history(&board, config, &history);
        let stats = searcher.retirement_stats().unwrap();
        assert_eq!(stats.len(), 4);
        assert!(stats.iter().all(|(_, retired, _)| !retired));
        assert!(first.result.best_move.is_some());
        searcher.reset_abort_flag();
        let terminal = Board::empty();
        let history = PositionHistory::initial(terminal.hash());
        searcher.search_with_history(
            &terminal,
            SearchConfig {
                max_depth: 1,
                ..config
            },
            &history,
        );
        assert!(
            searcher
                .retirement_stats()
                .unwrap()
                .iter()
                .all(|(_, retired, _)| !retired)
        );
    }

    #[test]
    fn cooperative_flag_is_exactly_two_worker_opt_in() {
        for workers in [1, 2, 4] {
            let searcher = LazySmpSearcher::with_flags(
                Tt::new(1),
                workers,
                LAZY_DEFAULT_FLAGS | LAZY_ROOT_SPLIT,
            );
            let board = Board::startpos();
            let history = PositionHistory::initial(board.hash());
            let info = searcher.search_with_history(
                &board,
                SearchConfig {
                    max_depth: 2,
                    ..SearchConfig::default()
                },
                &history,
            );
            assert_eq!(searcher.cooperative_root_stats().is_some(), workers == 2);
            assert_eq!(
                searcher.cooperative_root_wait_stats().is_some(),
                workers == 2
            );
            if let Some(wait) = searcher.cooperative_root_wait_stats() {
                let stats = searcher.cooperative_root_stats().unwrap();
                assert!(wait.waiting_confirmations <= stats.main_confirmations);
                assert!(wait.waiting_confirmation_nodes <= stats.confirmation_nodes);
                assert!(wait.waiting_alpha_raises <= stats.confirmed_alpha_raises);
            }
            assert_eq!(
                info.worker_results.len(),
                if workers == 2 { 1 } else { workers }
            );
            assert!(info.result.best_move.is_some());
        }
    }

    #[test]
    fn cooperative_snapshot_replaces_both_records_after_terminal_followup() {
        let searcher =
            LazySmpSearcher::with_flags(Tt::new(1), 2, LAZY_DEFAULT_FLAGS | LAZY_ROOT_SPLIT);
        assert!(searcher.cooperative_root_stats().is_none());
        assert!(searcher.cooperative_root_wait_stats().is_none());
        let board = Board::startpos();
        let config = SearchConfig {
            max_depth: 2,
            ..SearchConfig::default()
        };
        searcher.search_with_history(&board, config, &PositionHistory::initial(board.hash()));
        assert!(searcher.cooperative_root_stats().unwrap().generations > 0);
        assert!(searcher.cooperative_root_wait_stats().is_some());
        searcher.reset_abort_flag();
        let terminal = Board::empty();
        searcher.search_with_history(
            &terminal,
            config,
            &PositionHistory::initial(terminal.hash()),
        );
        let stats = searcher.cooperative_root_stats().unwrap();
        let wait = searcher.cooperative_root_wait_stats().unwrap();
        assert_eq!(stats.generations, 0);
        assert_eq!(stats.main_confirmations, 0);
        assert_eq!(wait.waiting_confirmations, 0);
        assert_eq!(wait.waiting_confirmation_nodes, 0);
        assert_eq!(wait.waiting_alpha_raises, 0);
        assert_eq!(wait.notification_wait_us, 0);
    }

    #[test]
    fn helper_root_lmr_flag_does_not_change_a_single_worker() {
        on_search_pool(1, || {
            let board = Board::startpos();
            let config = SearchConfig {
                max_depth: 4,
                time_limit: None,
                node_limit: None,
                soft_limit: None,
                multi_pv: 1,
            };
            let baseline = LazySmpSearcher::with_flags(Tt::new(4), 1, LAZY_DEFAULT_FLAGS)
                .search(&board, config)
                .result;
            let candidate = LazySmpSearcher::with_flags(
                Tt::new(4),
                1,
                LAZY_DEFAULT_FLAGS | LAZY_HELPER_ROOT_LMR,
            )
            .search(&board, config)
            .result;

            assert_eq!(candidate.best_move, baseline.best_move);
            assert_eq!(candidate.score, baseline.score);
            assert_eq!(candidate.depth, baseline.depth);
            assert_eq!(candidate.nodes, baseline.nodes);
            assert_eq!(candidate.pv, baseline.pv);
        });
    }

    #[test]
    fn helper_root_lmr_flag_only_configures_helpers() {
        let flags = LAZY_DEFAULT_FLAGS | LAZY_HELPER_ROOT_LMR;
        let main = LazySmpSearcher::worker_searcher(
            Tt::new(1),
            Arc::new(std::sync::atomic::AtomicBool::new(false)),
            None,
            flags,
            0,
            None,
        );
        let helper = LazySmpSearcher::worker_searcher(
            Tt::new(1),
            Arc::new(std::sync::atomic::AtomicBool::new(false)),
            None,
            flags,
            1,
            None,
        );
        let disabled_helper = LazySmpSearcher::worker_searcher(
            Tt::new(1),
            Arc::new(std::sync::atomic::AtomicBool::new(false)),
            None,
            LAZY_DEFAULT_FLAGS,
            1,
            None,
        );

        assert!(!main.full_root_lmr_enabled());
        assert!(helper.full_root_lmr_enabled());
        assert!(!disabled_helper.full_root_lmr_enabled());
    }

    #[test]
    fn foreign_root_hint_requires_multiple_workers_and_a_shared_tt() {
        let flagged = LAZY_DEFAULT_FLAGS | LAZY_FOREIGN_ROOT_HINT;
        let enabled = LazySmpSearcher::with_flags(Tt::new(1), 2, flagged);
        let default = LazySmpSearcher::with_flags(Tt::new(1), 2, LAZY_DEFAULT_FLAGS);
        let one_worker = LazySmpSearcher::with_flags(Tt::new(1), 1, flagged);
        let isolated = LazySmpSearcher::with_flags(Tt::new(1), 2, flagged | LAZY_ISOLATED_TT);

        assert!(enabled.root_hint.is_some());
        assert!(default.root_hint.is_none());
        assert!(one_worker.root_hint.is_none());
        assert!(isolated.root_hint.is_none());
    }

    #[test]
    fn invocation_guard_rejects_overlap_and_releases_on_drop() {
        let active = std::sync::atomic::AtomicBool::new(false);
        let first = SearchInvocationGuard::try_acquire(&active).expect("first invocation");
        assert!(SearchInvocationGuard::try_acquire(&active).is_none());
        drop(first);
        assert!(SearchInvocationGuard::try_acquire(&active).is_some());
    }

    #[test]
    fn blocking_invocation_guard_resumes_after_release() {
        let active = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let first = SearchInvocationGuard::try_acquire(&active).expect("first invocation");
        let worker_active = active.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            let _guard = SearchInvocationGuard::acquire_blocking(&worker_active);
            tx.send(()).expect("signal acquired invocation");
        });

        drop(first);
        rx.recv_timeout(Duration::from_secs(5))
            .expect("waiting invocation must resume after release");
        worker.join().expect("blocking invocation worker");
    }

    fn wait_for_lifecycle_abort(searcher: &LazySmpSearcher) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !searcher
            .external_abort
            .load(std::sync::atomic::Ordering::Relaxed)
        {
            assert!(
                Instant::now() < deadline,
                "lifecycle mutation must request cooperative abort"
            );
            std::thread::yield_now();
        }
    }

    #[test]
    fn reset_abort_waits_for_the_active_invocation() {
        let searcher = Arc::new(LazySmpSearcher::with_flags(
            Tt::new(1),
            2,
            LAZY_DEFAULT_FLAGS,
        ));
        let active = SearchInvocationGuard::try_acquire(&searcher.search_active)
            .expect("hold one active invocation");
        let resetting = Arc::clone(&searcher);
        let (tx, rx) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            resetting.reset_abort_flag();
            tx.send(()).expect("signal reset completion");
        });

        wait_for_lifecycle_abort(&searcher);
        assert!(rx.try_recv().is_err());
        drop(active);
        rx.recv_timeout(Duration::from_secs(5))
            .expect("reset must resume after the active invocation exits");
        worker.join().expect("reset worker");
        assert!(
            !searcher
                .external_abort
                .load(std::sync::atomic::Ordering::Relaxed)
        );
    }

    #[test]
    fn lifecycle_mutations_cannot_rearm_abort_after_reset() {
        let searcher = Arc::new(LazySmpSearcher::with_flags(
            Tt::new(1),
            2,
            LAZY_DEFAULT_FLAGS,
        ));
        let active = SearchInvocationGuard::try_acquire(&searcher.search_active)
            .expect("hold one active invocation");

        let resetting = Arc::clone(&searcher);
        let (reset_tx, reset_rx) = std::sync::mpsc::channel();
        let reset_worker = std::thread::spawn(move || {
            resetting.reset_abort_flag();
            reset_tx.send(()).expect("signal reset completion");
        });
        wait_for_lifecycle_abort(&searcher);

        let clearing = Arc::clone(&searcher);
        let (clear_tx, clear_rx) = std::sync::mpsc::channel();
        let clear_worker = std::thread::spawn(move || {
            clearing.clear_tt();
            clear_tx.send(()).expect("signal clear completion");
        });

        drop(active);
        reset_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("reset must finish first");
        clear_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("queued lifecycle mutation must finish");
        reset_worker.join().expect("reset worker");
        clear_worker.join().expect("clear worker");
        assert!(
            !searcher
                .external_abort
                .load(std::sync::atomic::Ordering::Relaxed),
            "a queued lifecycle mutation must not re-arm a stale stop request"
        );
    }

    #[test]
    fn tt_lifecycle_mutations_wait_for_the_active_invocation() {
        let tt = Tt::new(1);
        let searcher = Arc::new(LazySmpSearcher::with_flags(
            tt.clone(),
            2,
            LAZY_DEFAULT_FLAGS,
        ));
        let hash = 0x1234_5678;
        tt.store(
            hash,
            TtEntry {
                score: 42,
                depth: 3,
                bound: Bound::Exact,
                mv: None,
            },
        );

        let active = SearchInvocationGuard::try_acquire(&searcher.search_active)
            .expect("hold one active invocation");
        let clearing = Arc::clone(&searcher);
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            clearing.clear_tt();
            done_tx.send(()).expect("signal clear completion");
        });
        wait_for_lifecycle_abort(&searcher);
        assert!(done_rx.try_recv().is_err());
        assert!(tt.probe(hash).is_some(), "active search must retain its TT");
        drop(active);
        done_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("clear must resume after the active invocation exits");
        worker.join().expect("clear worker");
        assert!(tt.probe(hash).is_none());

        searcher
            .external_abort
            .store(false, std::sync::atomic::Ordering::Relaxed);
        let active = SearchInvocationGuard::try_acquire(&searcher.search_active)
            .expect("hold another active invocation");
        let aging = Arc::clone(&searcher);
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            aging.new_search();
            done_tx.send(()).expect("signal generation completion");
        });
        wait_for_lifecycle_abort(&searcher);
        let search_count = tt.search_count();
        assert!(done_rx.try_recv().is_err());
        assert_eq!(tt.search_count(), search_count);
        drop(active);
        done_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("generation update must resume after the invocation exits");
        worker.join().expect("generation worker");
        assert_eq!(tt.search_count(), search_count + 1);
    }

    #[test]
    fn lifecycle_abort_stops_and_joins_a_real_worker_group() {
        let searcher = Arc::new(LazySmpSearcher::diagnostic(2, LAZY_DEFAULT_FLAGS, 1));
        let searching = Arc::clone(&searcher);
        let (search_tx, search_rx) = std::sync::mpsc::channel();
        let search_worker = std::thread::spawn(move || {
            // Do not inherit the process-global Rayon width: coverage and
            // constrained CI jobs may intentionally run that pool with one
            // thread, which cannot start both logical Lazy SMP workers.
            let pool = rayon::ThreadPoolBuilder::new()
                .num_threads(2)
                .stack_size(crate::search::RECURSIVE_SEARCH_STACK_BYTES)
                .build()
                .expect("two-thread test pool");
            let info = pool.install(|| {
                let board = Board::startpos();
                let history = PositionHistory::initial(board.hash());
                searching.search_with_history(
                    &board,
                    SearchConfig {
                        max_depth: 100,
                        ..SearchConfig::default()
                    },
                    &history,
                )
            });
            search_tx
                .send(info.result.aborted)
                .expect("signal search completion");
        });

        let deadline = Instant::now() + Duration::from_secs(5);
        while !searcher
            .diagnostics
            .iter()
            .all(|observer| observer.snapshot().alpha_beta_calls > 0)
        {
            assert!(
                Instant::now() < deadline,
                "a real Lazy SMP worker must enter alpha-beta"
            );
            std::thread::yield_now();
        }

        let resetting = Arc::clone(&searcher);
        let (reset_tx, reset_rx) = std::sync::mpsc::channel();
        let reset_worker = std::thread::spawn(move || {
            resetting.reset_abort_flag();
            reset_tx.send(()).expect("signal reset completion");
        });

        assert!(
            search_rx
                .recv_timeout(Duration::from_secs(5))
                .expect("cooperative abort must stop the active worker group")
        );
        reset_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("reset must resume after every search worker joins");
        search_worker.join().expect("real search worker");
        reset_worker.join().expect("real reset worker");
        assert!(
            !searcher
                .external_abort
                .load(std::sync::atomic::Ordering::Relaxed)
        );
    }

    #[test]
    fn root_hint_stats_are_invocation_bound_and_reset_between_searches() {
        on_search_pool(2, || {
            let flags = LAZY_DEFAULT_FLAGS | LAZY_FOREIGN_ROOT_HINT;
            let searcher = LazySmpSearcher::with_flags(Tt::new(1), 2, flags);
            let board = Board::startpos();
            let history = PositionHistory::initial(board.hash());
            let (_, first, _) = searcher
                .try_search_with_history_and_root_hint_stats(
                    &board,
                    SearchConfig {
                        max_depth: 2,
                        time_limit: None,
                        node_limit: None,
                        soft_limit: None,
                        multi_pv: 1,
                    },
                    &history,
                )
                .expect("first diagnostic invocation");
            let first = first.expect("enabled experiment must return one snapshot");
            assert!(first.applied <= first.eligible_reads);

            let (_, second, _) = searcher
                .try_search_with_history_and_root_hint_stats(
                    &board,
                    SearchConfig {
                        max_depth: 0,
                        time_limit: None,
                        node_limit: None,
                        soft_limit: None,
                        multi_pv: 1,
                    },
                    &history,
                )
                .expect("second diagnostic invocation");
            assert_eq!(second, Some(LazySmpRootHintStats::default()));

            let disabled = LazySmpSearcher::with_flags(Tt::new(1), 2, LAZY_DEFAULT_FLAGS);
            let (_, stats, _) = disabled
                .try_search_with_history_and_root_hint_stats(
                    &board,
                    SearchConfig {
                        max_depth: 0,
                        time_limit: None,
                        node_limit: None,
                        soft_limit: None,
                        multi_pv: 1,
                    },
                    &history,
                )
                .expect("disabled diagnostic invocation");
            assert_eq!(stats, None);
        });
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
        let deepest = select_worker_result(vec![info(1, 120, 12), info(2, 100, 11)], 0);
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
    fn selector_reports_the_exact_worker_when_results_are_identical() {
        let results = vec![info(2, 100, 12), info(2, 100, 12)];

        assert_eq!(select_worker_index(&results, 0), 0);
        assert_eq!(select_worker_index(&results, LAZY_TIE_FIRST), 0);
        assert_eq!(select_worker_index(&results, LAZY_DEPTH_CONSENSUS), 1);
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
        on_search_pool(2, || {
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
        });
    }

    #[test]
    fn opt_in_diagnostics_report_worker_probes_and_shared_tt_writes() {
        on_search_pool(2, || {
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
        });
    }
}
