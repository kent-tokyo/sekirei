//! Parallel Alpha-Beta search — Young Brothers Wait (YBW) variant.
//!
//! Algorithm:
//!   1. Search the first (highest-priority) child sequentially to establish alpha.
//!   2. Remaining siblings ("young brothers") are searched in parallel via rayon,
//!      each with a null window [-alpha-1, -alpha] (PVS probe).
//!   3. Any sibling that fails high gets a sequential re-search with the full window.
//!   4. A shared AtomicBool aborts all sibling tasks the moment a beta cutoff is found.
//!
//! Parallelism is only activated at depth >= MIN_SPLIT_DEPTH to avoid spawning
//! threads for trivial leaf subtrees where overhead would dominate.
//!
//! Additional techniques:
//!   - Killer Move Heuristic (2 killers per ply)
//!   - History Heuristic (indexed by color × piece_kind × to_square)
//!   - Late Move Reduction (LMR)
//!   - Null Move Pruning (NMP, R=3)
//!   - Reverse Futility Pruning (RFP) at depth ≤ 3
//!   - Futility Pruning at depth 1
//!   - Late Move Pruning (LMP) at depth ≤ 2
//!   - Check Extension (+1 depth when a move gives check)
//!   - Aspiration Window (iterative deepening)
//!   - Delta Pruning in Quiescence Search

use rayon::prelude::*;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, AtomicU64, Ordering, fence};
use std::time::Duration;
use web_time::Instant;

use crate::board::Board;
use crate::budget::{Budget, soft_limit_expired};
use crate::color::Color;
use crate::eval::{PIECE_VALUE, evaluate, evaluation_cache_key};
use crate::movegen::{MoveBuffer, is_in_check, mate_in_one};
#[cfg(test)]
use crate::movegen::{discovered_check_candidates, generate_legal_captures, generate_legal_moves};
use crate::mv::{Move, MoveToken};
use crate::nnue::weights_active;
use crate::piece::PieceKind;
use crate::sfen::{PositionHistory, PositionHistoryEntry, RepetitionOutcome};
use crate::speculative::{SpecGroup, SpecState};
#[cfg(test)]
use crate::square::Square;
use crate::tt::{Bound, Tt, TtEntry};

/// Score for the fastest forced mate; actual mate scores are offset by ply distance to mate.
pub const MATE_SCORE: i32 = 900_000;
/// Lower search score bound (effectively -infinity).
pub const NEG_INF: i32 = -1_000_000;
/// Upper search score bound (effectively +infinity).
pub const POS_INF: i32 = 1_000_000;

/// Minimum remaining depth to activate parallel young-brother search.
const MIN_SPLIT_DEPTH: u32 = 3;

/// Stack budget for workers that execute recursive alpha-beta searches.
///
/// A legal depth-50 search can combine alpha-beta, extensions, and
/// quiescence frames. The platform default can overflow before a `stop`
/// request is observed, so every dedicated speculative pool uses this budget.
pub const RECURSIVE_SEARCH_STACK_BYTES: usize = 8 * 1024 * 1024;

/// Run a recursive-search regression with the production stack contract.
#[cfg(test)]
pub(crate) fn on_search_stack(test: impl FnOnce() + Send + 'static) {
    std::thread::Builder::new()
        .name("sekirei-search-regression".to_owned())
        .stack_size(RECURSIVE_SEARCH_STACK_BYTES)
        .spawn(test)
        .expect("spawn search regression")
        .join()
        .expect("search regression panicked");
}

mod aspiration;
mod child_search;
mod helper_retirement;
mod heuristics;
mod late_move;
mod move_order;
mod node;
mod node_moves;
pub mod params;
mod path;
mod pruning;
mod root_hint_retry;
mod root_safety;
mod root_split;
mod singular;
mod static_pruning;
mod tt_probe;
mod v2;
pub use aspiration::AspirationSnapshot;
use child_search::pvs_child;
pub(crate) use helper_retirement::{HelperRetirement, RetirementEndpoint};
use heuristics::{
    CorrKeys, CountermoveTable, HistoryTable, KillerTable, history_bonus, history_malus,
    pack_killer, unpack_killer,
};
use late_move::{
    LateMoveNode, LmrContext, check_reduction_cap, lmr_adjust, lmr_base_reduction, lmr_reduce,
    promotion_counts_as_quiet,
};
#[cfg(test)]
use move_order::sort_by_cached_i32_key_small;
use move_order::{
    QSEARCH_TT_ORDER_OFFSET, order_moves_in_place, qsearch_order_key, sort_moves_by_cached_key,
};
use node::NodeKey;
use node_moves::{MoveListContext, MovePreparation, NodeMoveList};
use params as p;
use path::{
    ContMoves, current_move_at, cut_at, doubles_at, restore_stack_path, set_current_move, set_cut,
    set_doubles, stack_path,
};
use pruning::{nmp_eval_allows, null_move_pruning, probcut};
use root_safety::{RootMateSafetyCache, RootSafety, root_mate_safety_filter};
#[cfg(test)]
use root_safety::{may_give_check, safe_root_moves};
pub use root_split::{
    CooperativeRootHelperLmrStats, CooperativeRootLatencyStats, CooperativeRootOutcomeStats,
    CooperativeRootPrestartCapStats, CooperativeRootPrestartDiagnostics,
    CooperativeRootPrestartStats, CooperativeRootStats, CooperativeRootWaitStats,
};
use singular::{SingularContext, SingularOutcome};
use static_pruning::{prepare_node_evaluation, razoring_score, reverse_futility_score};
use tt_probe::TtProbe;
pub use v2::NmpSnapshot;

// ============================================================
// Public API
// ============================================================

/// Iterative-deepening search parameters.
#[derive(Clone, Copy, Debug)]
pub struct SearchConfig {
    /// Maximum depth to search via iterative deepening.
    pub max_depth: u32,
    /// Hard time budget; the search aborts as soon as this elapses.
    pub time_limit: Option<Duration>,
    /// Hard node budget; unlike a wall-clock limit, this is reproducible for
    /// deterministic single-thread searches.
    pub node_limit: Option<u64>,
    /// Soft limit: exit after completing a depth if elapsed >= soft_limit and bestmove is stable.
    pub soft_limit: Option<Duration>,
    /// Number of PV lines to return (1 = normal, >1 = MultiPV).
    pub multi_pv: u32,
}

impl Default for SearchConfig {
    fn default() -> Self {
        SearchConfig {
            max_depth: 6,
            time_limit: None,
            node_limit: None,
            soft_limit: None,
            multi_pv: 1,
        }
    }
}

/// Result of a completed (or aborted) search.
pub struct SearchInfo {
    /// Best move found, if any.
    pub best_move: Option<Move>,
    /// Score of `best_move` in centipawns (or a mate score).
    pub score: i32,
    /// Deepest iterative-deepening depth completed.
    pub depth: u32,
    /// Total nodes visited across all depths.
    pub nodes: u64,
    /// Wall-clock time spent searching.
    pub elapsed: Duration,
    /// Transposition table occupancy, in permille (0-1000).
    pub hashfull: u32,
    /// Proof status of the returned root score.
    pub bound: SearchBound,
    /// Proof status of the last fully completed iterative-deepening pass.
    ///
    /// This remains useful when a later, deeper pass hits a node or time
    /// budget and therefore makes [`Self::bound`] unknown. Consumers must
    /// still require `depth > 0` before using the accompanying score.
    pub completed_bound: SearchBound,
    /// Whether the search stopped before completing its current budget.
    pub aborted: bool,
    /// Observed stop source: `none`, `budget`, or `external_stop`.
    pub abort_reason: &'static str,
    /// Legal principal-variation prefix reconstructed from exact TT entries.
    pub pv: Vec<Move>,
}

/// One fully completed iterative-deepening pass captured for diagnostics.
///
/// Normal production searches do not allocate or populate this record.  The
/// trace is intended to distinguish root-order and iteration effects without
/// changing pruning, evaluation, or the search budget.
pub struct SearchIteration {
    /// Completed root depth.
    pub depth: u32,
    /// Principal root move after this pass, if the position has a legal move.
    pub best_move: Option<Move>,
    /// Root score after this pass.
    pub score: i32,
    /// Cumulative nodes consumed through this pass.
    pub nodes: u64,
    /// Proof status returned by the root pass.
    pub bound: SearchBound,
    /// Legal principal-variation prefix captured before a later iteration can
    /// replace transposition-table entries.
    pub pv: Vec<Move>,
    /// Cumulative nodes spent in root mate-in-one checks, if a diagnostic
    /// observer was attached.
    pub root_mate_in_one_nodes: Option<u64>,
    /// Cumulative nodes spent filtering root mate blunders, if observed.
    pub root_mate_blunder_nodes: Option<u64>,
}

/// One ranked root candidate from a fully completed MultiPV iteration.
#[derive(Clone, Debug)]
pub struct MultiPvCandidate {
    /// One-based rank within this iteration.
    pub rank: u32,
    /// Legal root move represented by this candidate.
    pub root_move: Move,
    /// Root score from the side-to-move perspective.
    pub score: i32,
    /// Proof status returned by the candidate's root search.
    pub bound: SearchBound,
    /// Legal PV prefix captured before another candidate can replace TT data.
    pub pv: Vec<Move>,
}

/// Ranked candidates from one fully completed iterative-deepening pass.
#[derive(Clone, Debug)]
pub struct MultiPvIteration {
    /// Completed root depth shared by every candidate.
    pub depth: u32,
    /// Cumulative nodes consumed by the whole request through this pass.
    pub nodes: u64,
    /// Unique candidates in rank order.
    pub candidates: Vec<MultiPvCandidate>,
}

#[derive(Default)]
struct SearchTraces<'a> {
    iterations: Option<&'a mut Vec<SearchIteration>>,
    multi_pv: Option<&'a mut Vec<MultiPvIteration>>,
}

/// Result for one explicitly requested root move in a diagnostic comparison.
///
/// This is separate from [`SearchInfo`]: normal search remains a
/// single-best-move API, while root-candidate evidence is opt-in.
pub struct RootCandidateInfo {
    /// The legal root move that was searched.
    pub root_move: Move,
    /// Search result restricted to `root_move`.
    pub info: SearchInfo,
}

/// Proof status for a completed root score.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SearchBound {
    /// The root score was obtained with a complete window.
    Exact,
    /// The score is known to be at least the reported value.
    Lower,
    /// The score is known to be at most the reported value.
    Upper,
    /// The search was interrupted or otherwise did not prove a bound.
    Unknown,
}

impl SearchBound {
    /// Return the stable schema spelling used by diagnostic records.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Exact => "exact",
            Self::Lower => "lower",
            Self::Upper => "upper",
            Self::Unknown => "unknown",
        }
    }
}

/// Optional counters for explaining move-ordering behavior.
///
/// The observer is detached from normal searches, so production callers do not
/// pay for these atomic increments unless they explicitly opt in.
pub struct SearchDiagnostics {
    static_evaluations: AtomicU64,
    eval_cache_probes: AtomicU64,
    eval_cache_hits: AtomicU64,
    tt_probes: AtomicU64,
    tt_hits: AtomicU64,
    tt_stores: AtomicU64,
    /// Sampled TT hits attributed to this Lazy SMP worker. These counters are
    /// populated only by [`SearchDiagnostics::parallel_worker`].
    tt_local_hits: AtomicU64,
    tt_foreign_hits: AtomicU64,
    tt_unknown_hits: AtomicU64,
    order_tt: AtomicU64,
    order_killer: AtomicU64,
    order_countermove: AtomicU64,
    order_history: AtomicU64,
    root_mate_in_one_nodes: AtomicU64,
    root_mate_blunder_nodes: AtomicU64,
    root_mate_in_one_cache_hits: AtomicU64,
    root_mate_blunder_cache_hits: AtomicU64,
    alpha_beta_calls: AtomicU64,
    quiescence_calls: AtomicU64,
    static_evaluation_ns: AtomicU64,
    tt_probe_ns: AtomicU64,
    tt_store_ns: AtomicU64,
    movegen_order_ns: AtomicU64,
    movegen_generate_ns: AtomicU64,
    move_order_ns: AtomicU64,
    move_order_score_ns: AtomicU64,
    move_order_sort_ns: AtomicU64,
    /// Whether the `*_ns` timers are recorded. Reading the clock around every
    /// scored move is expensive, so root-only observers leave it off.
    timing: bool,
    /// Whether per-node counters (evaluations, TT, ordering, node calls) are
    /// recorded. Root mate-safety counters are always recorded.
    per_node: bool,
    quiescence_inclusive_ns: AtomicU64,
    root_mate_safety_ns: AtomicU64,
    /// Beta cutoffs of the main search by the position of the cutoff move
    /// in the node's move order (see [`SearchDiagnostics::cut_histogram`]),
    /// for quiet cutoff moves (row 0) and the others (row 1).
    cut_index: [[AtomicU64; CUT_BUCKETS]; 4],
    /// Cutoffs at nodes whose TT move is quiet, by the entry's bound (3) and
    /// the node depth minus the entry depth (<= 0, 1, 2, 3, 4-5, 6+): row 0
    /// when the TT move cut, row 1 when a later move did.
    tt_quiet_cut: [[AtomicU64; 18]; 2],
    /// Main-search TT probes by node depth (capped at 16): probes, hits,
    /// hits at least as deep as the node, hits at most one ply shallower.
    tt_by_depth: [[AtomicU64; 4]; 17],
    /// SEARCH_V2 nodes by depth: all, PV-like (ttPv), with a static eval at
    /// or above beta, in check.
    v2_nodes: [[AtomicU64; 4]; 17],
    /// Moves searched by node depth (1..7) × node in check × kind: first,
    /// capture, promotion, checking drop, checking move, quiet drop, quiet
    /// move.
    v2_moves: [[AtomicU64; 8]; 16],
    /// Main-search nodes that entered the move loop, without and with a TT
    /// move.
    loop_nodes: [AtomicU64; 2],
    /// Main-search calls with a full window (PV nodes).
    pv_calls: AtomicU64,
    /// Loop nodes by window (null, full) and the moves they searched
    /// (pruned moves left out).
    loop_by_window: [AtomicU64; 2],
    searched_by_window: [AtomicU64; 2],
    /// Searched later moves that give check, by window.
    searched_checks: [AtomicU64; 2],
    /// Late-move searches: reduced probes, their full-depth null-window
    /// re-searches, and full-window re-searches.
    research: [AtomicU64; 3],
    /// How main-search calls end before the move loop: depth 0, TT cutoff,
    /// mate in one, reverse futility, razoring, ProbCut, null move cutoff,
    /// and null-move searches that did not cut.
    exits: [AtomicU64; 8],
    /// Main-search calls by remaining depth (1..=16, deeper in 16): calls,
    /// TT cutoffs, razoring, reverse futility, null-move cutoffs, ProbCut,
    /// mate in one, loop nodes, and moves searched at loop nodes.
    by_depth: [[AtomicU64; 9]; 17],
    /// Main-search nodes that searched moves and failed low.
    fail_low_nodes: AtomicU64,
    /// SEARCH_V2 move-loop stages: nodes entering the quiet stage, nodes that
    /// skipped it after late-move pruning, quiet moves scored, and nodes
    /// entering the losing-tactical stage.
    move_stages: [AtomicU64; 4],
    v2_nmp: v2::NmpDiagnostics,
    aspiration: aspiration::AspirationDiagnostics,
    /// Quiescence calls, nodes in check, TT cutoffs, stand-pat cutoffs,
    /// terminal nodes, searched moves, beta cutoffs, depth-cap exits,
    /// mate-in-one exits, delta-pruning exits, and top-level qsearch calls.
    qsearch: [AtomicU64; 11],
    /// Diagnostic-only sampled node registry shared by Lazy SMP workers.
    /// Normal searches and ordinary cost profiles leave this detached.
    work_overlap: Option<WorkOverlapObserver>,
    /// Diagnostic-only sampled ownership of committed TT entries. A hit can
    /// then be classified as local, foreign-worker, or untracked without
    /// changing the production TT layout.
    tt_reuse: Option<TtReuseObserver>,
}

const WORK_SAMPLE_LOG2: u32 = 8;
const WORK_SAMPLE_SLOTS: usize = 1 << 15;
const WORK_SAMPLE_PROBES: usize = 8;
// Sampled fingerprints always have low byte 1, so 2 can reserve a slot while
// the inserting worker publishes its worker id before the final fingerprint.
const WORK_SAMPLE_CLAIMED: u64 = 2;

struct WorkSampleSlot {
    fingerprint: AtomicU64,
    first_worker: AtomicU32,
    shared: AtomicBool,
}

struct TtReuseSampleSlot {
    fingerprint: AtomicU64,
    last_writer: AtomicU32,
}

/// Bounded, lock-free sampler for estimating work repeated across Lazy SMP
/// workers. Only one out of every 256 mixed position hashes is observed, and
/// each fingerprint gets at most eight open-addressing probes. Saturation is
/// reported rather than silently treated as overlap.
pub(crate) struct WorkOverlapStats {
    slots: Box<[WorkSampleSlot]>,
    sampled_visits: AtomicU64,
    unique_fingerprints: AtomicU64,
    within_worker_revisits: AtomicU64,
    cross_worker_visits: AtomicU64,
    shared_fingerprints: AtomicU64,
    dropped: AtomicU64,
}

/// Bounded sampled registry for attributing TT hits to the worker that last
/// committed the entry. It is attached only to `LazyDiagnostics`; ordinary
/// searches retain the existing TT memory layout and hot path.
pub(crate) struct TtReuseStats {
    slots: Box<[TtReuseSampleSlot]>,
    sampled_stores: AtomicU64,
    sampled_hits: AtomicU64,
    local_hits: AtomicU64,
    foreign_hits: AtomicU64,
    unknown_hits: AtomicU64,
    dropped_stores: AtomicU64,
    contributions: [AtomicU64; 512],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TtReuseOrigin {
    Local,
    Foreign,
    Unknown,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct WorkOverlapSnapshot {
    pub sampled_visits: u64,
    pub unique_fingerprints: u64,
    pub within_worker_revisits: u64,
    pub cross_worker_visits: u64,
    pub shared_fingerprints: u64,
    pub dropped: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct TtReuseSnapshot {
    pub sampled_stores: u64,
    pub sampled_hits: u64,
    pub local_hits: u64,
    pub foreign_hits: u64,
    pub unknown_hits: u64,
    pub dropped_stores: u64,
}

#[derive(Clone)]
struct WorkOverlapObserver {
    stats: Arc<WorkOverlapStats>,
    worker: u32,
}

#[derive(Clone)]
struct TtReuseObserver {
    stats: Arc<TtReuseStats>,
    worker: u32,
}

impl WorkOverlapStats {
    pub(crate) fn new() -> Self {
        Self {
            slots: (0..WORK_SAMPLE_SLOTS)
                .map(|_| WorkSampleSlot {
                    fingerprint: AtomicU64::new(0),
                    first_worker: AtomicU32::new(0),
                    shared: AtomicBool::new(false),
                })
                .collect(),
            sampled_visits: AtomicU64::new(0),
            unique_fingerprints: AtomicU64::new(0),
            within_worker_revisits: AtomicU64::new(0),
            cross_worker_visits: AtomicU64::new(0),
            shared_fingerprints: AtomicU64::new(0),
            dropped: AtomicU64::new(0),
        }
    }

    /// Reset between searches. The caller guarantees that no worker is still
    /// using the sampler, just as it does when resetting the shared abort flag.
    pub(crate) fn reset(&self) {
        for slot in &self.slots {
            slot.fingerprint.store(0, Ordering::Relaxed);
            slot.first_worker.store(0, Ordering::Relaxed);
            slot.shared.store(false, Ordering::Relaxed);
        }
        self.sampled_visits.store(0, Ordering::Relaxed);
        self.unique_fingerprints.store(0, Ordering::Relaxed);
        self.within_worker_revisits.store(0, Ordering::Relaxed);
        self.cross_worker_visits.store(0, Ordering::Relaxed);
        self.shared_fingerprints.store(0, Ordering::Relaxed);
        self.dropped.store(0, Ordering::Relaxed);
    }

    pub(crate) fn snapshot(&self) -> WorkOverlapSnapshot {
        WorkOverlapSnapshot {
            sampled_visits: self.sampled_visits.load(Ordering::Relaxed),
            unique_fingerprints: self.unique_fingerprints.load(Ordering::Relaxed),
            within_worker_revisits: self.within_worker_revisits.load(Ordering::Relaxed),
            cross_worker_visits: self.cross_worker_visits.load(Ordering::Relaxed),
            shared_fingerprints: self.shared_fingerprints.load(Ordering::Relaxed),
            dropped: self.dropped.load(Ordering::Relaxed),
        }
    }

    #[inline]
    fn record(&self, hash: u64, worker: u32) {
        let mixed = mix_work_hash(hash);
        if mixed & ((1 << WORK_SAMPLE_LOG2) - 1) != 0 {
            return;
        }
        self.sampled_visits.fetch_add(1, Ordering::Relaxed);
        // Sampled mixed hashes have zero low bits, so setting bit zero gives a
        // non-zero table key without merging two sampled fingerprints.
        let fingerprint = mixed | 1;
        let start = (mixed.rotate_right(WORK_SAMPLE_LOG2) as usize) & (WORK_SAMPLE_SLOTS - 1);
        let worker = worker.saturating_add(1);
        for probe in 0..WORK_SAMPLE_PROBES {
            let slot = &self.slots[(start + probe) & (WORK_SAMPLE_SLOTS - 1)];
            let mut current = slot.fingerprint.load(Ordering::Acquire);
            if current == 0 {
                match slot.fingerprint.compare_exchange(
                    0,
                    WORK_SAMPLE_CLAIMED,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                ) {
                    Ok(_) => {
                        slot.first_worker.store(worker, Ordering::Relaxed);
                        slot.fingerprint.store(fingerprint, Ordering::Release);
                        self.unique_fingerprints.fetch_add(1, Ordering::Relaxed);
                        return;
                    }
                    Err(actual) => current = actual,
                }
            }
            while current == WORK_SAMPLE_CLAIMED {
                std::hint::spin_loop();
                current = slot.fingerprint.load(Ordering::Acquire);
            }
            if current != fingerprint {
                continue;
            }
            let first = slot.first_worker.load(Ordering::Relaxed);
            if first == worker {
                self.within_worker_revisits.fetch_add(1, Ordering::Relaxed);
            } else {
                self.cross_worker_visits.fetch_add(1, Ordering::Relaxed);
                if !slot.shared.swap(true, Ordering::Relaxed) {
                    self.shared_fingerprints.fetch_add(1, Ordering::Relaxed);
                }
            }
            return;
        }
        self.dropped.fetch_add(1, Ordering::Relaxed);
    }
}

impl TtReuseStats {
    pub(crate) fn new() -> Self {
        Self {
            slots: (0..WORK_SAMPLE_SLOTS)
                .map(|_| TtReuseSampleSlot {
                    fingerprint: AtomicU64::new(0),
                    last_writer: AtomicU32::new(0),
                })
                .collect(),
            sampled_stores: AtomicU64::new(0),
            sampled_hits: AtomicU64::new(0),
            local_hits: AtomicU64::new(0),
            foreign_hits: AtomicU64::new(0),
            unknown_hits: AtomicU64::new(0),
            dropped_stores: AtomicU64::new(0),
            contributions: std::array::from_fn(|_| AtomicU64::new(0)),
        }
    }

    /// Reset between searches after every worker from the previous search has
    /// joined. The registry identifies the last sampled writer only within
    /// one Lazy SMP invocation.
    pub(crate) fn reset(&self) {
        for slot in &self.slots {
            slot.fingerprint.store(0, Ordering::Relaxed);
            slot.last_writer.store(0, Ordering::Relaxed);
        }
        self.sampled_stores.store(0, Ordering::Relaxed);
        self.sampled_hits.store(0, Ordering::Relaxed);
        self.local_hits.store(0, Ordering::Relaxed);
        self.foreign_hits.store(0, Ordering::Relaxed);
        self.unknown_hits.store(0, Ordering::Relaxed);
        self.dropped_stores.store(0, Ordering::Relaxed);
        for value in &self.contributions {
            value.store(0, Ordering::Relaxed);
        }
    }

    pub(crate) fn snapshot(&self) -> TtReuseSnapshot {
        TtReuseSnapshot {
            sampled_stores: self.sampled_stores.load(Ordering::Relaxed),
            sampled_hits: self.sampled_hits.load(Ordering::Relaxed),
            local_hits: self.local_hits.load(Ordering::Relaxed),
            foreign_hits: self.foreign_hits.load(Ordering::Relaxed),
            unknown_hits: self.unknown_hits.load(Ordering::Relaxed),
            dropped_stores: self.dropped_stores.load(Ordering::Relaxed),
        }
    }

    #[inline]
    fn sampled_fingerprint(hash: u64) -> Option<(u64, usize)> {
        let mixed = mix_work_hash(hash);
        if mixed & ((1 << WORK_SAMPLE_LOG2) - 1) != 0 {
            return None;
        }
        Some((
            mixed | 1,
            (mixed.rotate_right(WORK_SAMPLE_LOG2) as usize) & (WORK_SAMPLE_SLOTS - 1),
        ))
    }

    #[inline]
    fn record_store(&self, hash: u64, worker: u32) {
        let Some((fingerprint, start)) = Self::sampled_fingerprint(hash) else {
            return;
        };
        self.sampled_stores.fetch_add(1, Ordering::Relaxed);
        let worker = worker.saturating_add(1);
        for probe in 0..WORK_SAMPLE_PROBES {
            let slot = &self.slots[(start + probe) & (WORK_SAMPLE_SLOTS - 1)];
            let mut current = slot.fingerprint.load(Ordering::Acquire);
            if current == 0 {
                match slot.fingerprint.compare_exchange(
                    0,
                    WORK_SAMPLE_CLAIMED,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                ) {
                    Ok(_) => {
                        slot.last_writer.store(worker, Ordering::Relaxed);
                        slot.fingerprint.store(fingerprint, Ordering::Release);
                        return;
                    }
                    Err(actual) => current = actual,
                }
            }
            if current == WORK_SAMPLE_CLAIMED {
                // A preempted publisher must not stall a search worker.
                continue;
            }
            if current == fingerprint {
                slot.last_writer.store(worker, Ordering::Release);
                return;
            }
        }
        self.dropped_stores.fetch_add(1, Ordering::Relaxed);
    }

    #[inline]
    fn record_hit(&self, hash: u64, worker: u32) -> Option<TtReuseOrigin> {
        let (fingerprint, start) = Self::sampled_fingerprint(hash)?;
        self.sampled_hits.fetch_add(1, Ordering::Relaxed);
        let worker = worker.saturating_add(1);
        for probe in 0..WORK_SAMPLE_PROBES {
            let slot = &self.slots[(start + probe) & (WORK_SAMPLE_SLOTS - 1)];
            let current = slot.fingerprint.load(Ordering::Acquire);
            if current == WORK_SAMPLE_CLAIMED {
                // A preempted publisher must not stall a search worker.
                continue;
            }
            if current == 0 {
                break;
            }
            if current != fingerprint {
                continue;
            }
            let writer = slot.last_writer.load(Ordering::Acquire);
            if writer == worker {
                self.local_hits.fetch_add(1, Ordering::Relaxed);
                return Some(TtReuseOrigin::Local);
            } else if writer != 0 {
                self.foreign_hits.fetch_add(1, Ordering::Relaxed);
                if let Some(value) = self.contributions.get((writer - 1) as usize) {
                    value.fetch_add(1, Ordering::Relaxed);
                }
                return Some(TtReuseOrigin::Foreign);
            } else {
                self.unknown_hits.fetch_add(1, Ordering::Relaxed);
                return Some(TtReuseOrigin::Unknown);
            }
        }
        self.unknown_hits.fetch_add(1, Ordering::Relaxed);
        Some(TtReuseOrigin::Unknown)
    }
}

#[inline]
fn mix_work_hash(mut value: u64) -> u64 {
    value ^= value >> 30;
    value = value.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value ^= value >> 27;
    value = value.wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}

/// Buckets of [`SearchDiagnostics::cut_histogram`]: move 1, 2, 3, 4, 5-6,
/// 7-8, 9-12, 13-16, 17-32, 33 and later.
pub const CUT_BUCKETS: usize = 10;

/// A point-in-time copy of [`SearchDiagnostics`] counters.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SearchDiagnosticsSnapshot {
    /// Number of calls to the configured static evaluator.
    pub static_evaluations: u64,
    /// Number of exact NNUE evaluation-cache probes.
    pub eval_cache_probes: u64,
    /// Number of probes that reused a hash-identical NNUE evaluation.
    pub eval_cache_hits: u64,
    /// Number of transposition-table probes.
    pub tt_probes: u64,
    /// Number of probes that found an entry.
    pub tt_hits: u64,
    /// Number of transposition-table store attempts.
    pub tt_stores: u64,
    /// Number of moves selected as the TT move.
    pub order_tt: u64,
    /// Number of moves selected by the killer heuristic.
    pub order_killer: u64,
    /// Number of moves selected by the countermove heuristic.
    pub order_countermove: u64,
    /// Number of moves scored by the history heuristic.
    pub order_history: u64,
    /// Nodes consumed by root mate-in-one checks.
    pub root_mate_in_one_nodes: u64,
    /// Nodes consumed by root mate-blunder filtering.
    pub root_mate_blunder_nodes: u64,
    /// Iterative-deepening passes that reused the completed root mate-in-one scan.
    pub root_mate_in_one_cache_hits: u64,
    /// Iterative-deepening passes that reused the completed root mate-blunder scan.
    pub root_mate_blunder_cache_hits: u64,
    /// Number of alpha-beta calls, including calls that enter quiescence.
    pub alpha_beta_calls: u64,
    /// Number of quiescence calls.
    pub quiescence_calls: u64,
    /// Quiescence outcomes and work: calls, in-check nodes, TT cutoffs,
    /// stand-pat cutoffs, terminal nodes, searched moves, beta cutoffs,
    /// depth-cap exits, mate-in-one exits, delta-pruning exits, and top-level
    /// qsearch calls.
    pub qsearch: [u64; 11],
    /// Inclusive wall time spent in static-evaluation calls.
    pub static_evaluation_ns: u64,
    /// Inclusive wall time spent probing the transposition table.
    pub tt_probe_ns: u64,
    /// Inclusive wall time spent storing transposition-table entries.
    pub tt_store_ns: u64,
    /// Inclusive wall time spent generating and ordering moves.
    pub movegen_order_ns: u64,
    /// Inclusive wall time spent generating legal/capture move lists.
    pub movegen_generate_ns: u64,
    /// Inclusive wall time spent assigning move-order scores and sorting.
    pub move_order_ns: u64,
    /// Wall time spent calculating move-order keys. This is nested in
    /// [`Self::move_order_sort_ns`], because cached-key sorting invokes the
    /// scorer while it orders the move list.
    pub move_order_score_ns: u64,
    /// Inclusive wall time spent in move-list sorting, including key creation.
    pub move_order_sort_ns: u64,
    /// Inclusive wall time spent in quiescence search.
    ///
    /// This overlaps the leaf component timers above because quiescence calls
    /// them recursively; consumers must not add component durations together.
    pub quiescence_inclusive_ns: u64,
    /// Wall time spent in root mate-safety checks and their cache handling.
    pub root_mate_safety_ns: u64,
}

/// A diagnostic-only inclusive timer.
///
/// When diagnostics are disabled this owns no clock reading, keeping the
/// normal search path unchanged. Timers deliberately report inclusive spans:
/// recursive search and its leaf components overlap, so reports describe
/// hotspots rather than an additive CPU-time breakdown.
struct ProfileTimer<'a> {
    counter: Option<&'a AtomicU64>,
    started: Option<Instant>,
}

impl<'a> ProfileTimer<'a> {
    #[inline]
    fn new(counter: Option<&'a AtomicU64>) -> Self {
        Self {
            started: counter.map(|_| Instant::now()),
            counter,
        }
    }
}

impl Drop for ProfileTimer<'_> {
    fn drop(&mut self) {
        if let (Some(counter), Some(started)) = (self.counter, self.started) {
            let elapsed = started.elapsed().as_nanos().min(u64::MAX as u128) as u64;
            counter.fetch_add(elapsed, Ordering::Relaxed);
        }
    }
}

impl SearchDiagnostics {
    /// Root aspiration evidence by depth; collected only by per-node observers.
    pub fn aspiration_snapshot(&self) -> AspirationSnapshot {
        self.aspiration.snapshot()
    }

    /// Opt-in V2 null-move evidence, including remaining-depth buckets.
    pub fn nmp_snapshot(&self) -> v2::NmpSnapshot {
        self.v2_nmp.snapshot()
    }

    pub(crate) fn parallel_worker(
        work_overlap: Arc<WorkOverlapStats>,
        tt_reuse: Arc<TtReuseStats>,
        worker: usize,
    ) -> Self {
        let worker = worker.min(u32::MAX as usize) as u32;
        Self {
            timing: false,
            per_node: true,
            work_overlap: Some(WorkOverlapObserver {
                stats: work_overlap,
                worker,
            }),
            tt_reuse: Some(TtReuseObserver {
                stats: tt_reuse,
                worker,
            }),
            ..Self::new()
        }
    }

    /// Create an observer that records per-node counters without wall-clock
    /// timers. This is intended for parallel-scaling diagnostics: atomic
    /// counters still perturb throughput, but avoiding a clock read around
    /// every hot-path component keeps that perturbation bounded and explicit.
    pub fn counters_only() -> Self {
        Self {
            timing: false,
            per_node: true,
            ..Self::new()
        }
    }

    /// Create an observer that records only the root mate-safety counters.
    /// Production USI searches use this: per-node atomic counters and clock
    /// reads would otherwise cost a large share of search throughput.
    pub fn root_safety_only() -> Self {
        Self {
            timing: false,
            per_node: false,
            ..Self::new()
        }
    }

    #[inline(always)]
    fn timed<'a>(&self, counter: &'a AtomicU64) -> Option<&'a AtomicU64> {
        self.timing.then_some(counter)
    }

    #[inline(always)]
    fn per_node(&self) -> Option<&Self> {
        self.per_node.then_some(self)
    }

    /// Create an empty observer that also records the `*_ns` cost timers.
    pub fn new() -> Self {
        Self {
            timing: true,
            per_node: true,
            static_evaluations: AtomicU64::new(0),
            eval_cache_probes: AtomicU64::new(0),
            eval_cache_hits: AtomicU64::new(0),
            tt_probes: AtomicU64::new(0),
            tt_hits: AtomicU64::new(0),
            tt_stores: AtomicU64::new(0),
            tt_local_hits: AtomicU64::new(0),
            tt_foreign_hits: AtomicU64::new(0),
            tt_unknown_hits: AtomicU64::new(0),
            order_tt: AtomicU64::new(0),
            order_killer: AtomicU64::new(0),
            order_countermove: AtomicU64::new(0),
            order_history: AtomicU64::new(0),
            root_mate_in_one_nodes: AtomicU64::new(0),
            root_mate_blunder_nodes: AtomicU64::new(0),
            root_mate_in_one_cache_hits: AtomicU64::new(0),
            root_mate_blunder_cache_hits: AtomicU64::new(0),
            alpha_beta_calls: AtomicU64::new(0),
            quiescence_calls: AtomicU64::new(0),
            static_evaluation_ns: AtomicU64::new(0),
            tt_probe_ns: AtomicU64::new(0),
            tt_store_ns: AtomicU64::new(0),
            movegen_order_ns: AtomicU64::new(0),
            movegen_generate_ns: AtomicU64::new(0),
            move_order_ns: AtomicU64::new(0),
            move_order_score_ns: AtomicU64::new(0),
            move_order_sort_ns: AtomicU64::new(0),
            quiescence_inclusive_ns: AtomicU64::new(0),
            root_mate_safety_ns: AtomicU64::new(0),
            cut_index: std::array::from_fn(|_| std::array::from_fn(|_| AtomicU64::new(0))),
            tt_quiet_cut: std::array::from_fn(|_| std::array::from_fn(|_| AtomicU64::new(0))),
            tt_by_depth: std::array::from_fn(|_| std::array::from_fn(|_| AtomicU64::new(0))),
            v2_nodes: std::array::from_fn(|_| std::array::from_fn(|_| AtomicU64::new(0))),
            v2_moves: std::array::from_fn(|_| std::array::from_fn(|_| AtomicU64::new(0))),
            loop_nodes: std::array::from_fn(|_| AtomicU64::new(0)),
            pv_calls: AtomicU64::new(0),
            loop_by_window: std::array::from_fn(|_| AtomicU64::new(0)),
            searched_by_window: std::array::from_fn(|_| AtomicU64::new(0)),
            searched_checks: std::array::from_fn(|_| AtomicU64::new(0)),
            research: std::array::from_fn(|_| AtomicU64::new(0)),
            exits: std::array::from_fn(|_| AtomicU64::new(0)),
            by_depth: std::array::from_fn(|_| std::array::from_fn(|_| AtomicU64::new(0))),
            fail_low_nodes: AtomicU64::new(0),
            move_stages: std::array::from_fn(|_| AtomicU64::new(0)),
            v2_nmp: v2::NmpDiagnostics::default(),
            aspiration: aspiration::AspirationDiagnostics::default(),
            qsearch: std::array::from_fn(|_| AtomicU64::new(0)),
            work_overlap: None,
            tt_reuse: None,
        }
    }

    #[inline]
    fn record_work_sample(&self, hash: u64) {
        if let Some(observer) = &self.work_overlap {
            observer.stats.record(hash, observer.worker);
        }
    }

    #[inline]
    fn record_tt_store_origin(&self, hash: u64) {
        if let Some(observer) = &self.tt_reuse {
            observer.stats.record_store(hash, observer.worker);
        }
    }

    #[inline]
    fn record_tt_hit_origin(&self, hash: u64) {
        if let Some(observer) = &self.tt_reuse
            && let Some(origin) = observer.stats.record_hit(hash, observer.worker)
        {
            match origin {
                TtReuseOrigin::Local => &self.tt_local_hits,
                TtReuseOrigin::Foreign => &self.tt_foreign_hits,
                TtReuseOrigin::Unknown => &self.tt_unknown_hits,
            }
            .fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Record a beta cutoff by the `number`-th move (1-based) of a node.
    #[inline]
    fn record_cut(&self, number: usize, quiet: bool, had_tt_move: bool) {
        let bucket = match number {
            0..=4 => number.saturating_sub(1),
            5..=6 => 4,
            7..=8 => 5,
            9..=12 => 6,
            13..=16 => 7,
            17..=32 => 8,
            _ => 9,
        };
        self.cut_index[usize::from(!quiet) + 2 * usize::from(had_tt_move)][bucket]
            .fetch_add(1, Ordering::Relaxed);
    }

    /// Main-search calls by early exit (see the `exits` field).
    pub fn exit_counts(&self) -> [u64; 8] {
        std::array::from_fn(|i| self.exits[i].load(Ordering::Relaxed))
    }

    #[inline]
    fn exit(&self, i: usize) {
        self.exits[i].fetch_add(1, Ordering::Relaxed);
    }

    #[inline]
    fn at_depth(&self, depth: u32, k: usize) {
        self.by_depth[(depth as usize).min(16)][k].fetch_add(1, Ordering::Relaxed);
    }

    /// Main-search outcomes by remaining depth (see the `by_depth` field).
    pub fn depth_table(&self) -> [[u64; 9]; 17] {
        std::array::from_fn(|d| {
            std::array::from_fn(|k| self.by_depth[d][k].load(Ordering::Relaxed))
        })
    }

    /// Reduced probes of late moves, their full-depth null-window
    /// re-searches, and full-window re-searches.
    pub fn research_counts(&self) -> [u64; 3] {
        std::array::from_fn(|i| self.research[i].load(Ordering::Relaxed))
    }

    /// PV calls, then loop nodes and moves searched at null-window and
    /// full-window nodes.
    pub fn window_counts(&self) -> (u64, [u64; 2], [u64; 2]) {
        (
            self.pv_calls.load(Ordering::Relaxed),
            std::array::from_fn(|i| self.loop_by_window[i].load(Ordering::Relaxed)),
            std::array::from_fn(|i| self.searched_by_window[i].load(Ordering::Relaxed)),
        )
    }

    /// Record a cutoff at a node whose TT move is quiet (`tt_quiet_cut`).
    #[inline]
    fn record_tt_quiet_cut(&self, tt_move_cut: bool, bound: Option<Bound>, gap: i32) {
        let b = match bound {
            Some(Bound::Exact) => 0,
            Some(Bound::Lower) => 1,
            _ => 2,
        };
        let g = match gap {
            ..=0 => 0,
            1..=3 => gap as usize,
            4..=5 => 4,
            _ => 5,
        };
        self.tt_quiet_cut[usize::from(!tt_move_cut)][b * 6 + g].fetch_add(1, Ordering::Relaxed);
    }

    /// Record a main-search TT probe (`tt_by_depth`).
    #[inline]
    fn record_tt_probe(&self, depth: u32, hit: bool, entry_depth: u8) {
        let row = &self.tt_by_depth[depth.min(16) as usize];
        row[0].fetch_add(1, Ordering::Relaxed);
        if hit {
            row[1].fetch_add(1, Ordering::Relaxed);
            if u32::from(entry_depth) >= depth {
                row[2].fetch_add(1, Ordering::Relaxed);
            }
            if u32::from(entry_depth) + 1 >= depth {
                row[3].fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    /// Record a SEARCH_V2 node (`v2_nodes`).
    #[inline]
    fn record_v2_node(&self, depth: u32, tt_pv: bool, eval_above_beta: bool, in_check: bool) {
        let row = &self.v2_nodes[depth.min(16) as usize];
        row[0].fetch_add(1, Ordering::Relaxed);
        for (i, f) in [tt_pv, eval_above_beta, in_check].into_iter().enumerate() {
            if f {
                row[i + 1].fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    /// Record a searched move (`v2_moves`).
    #[inline]
    fn record_move_kind(&self, depth: u32, in_check: bool, kind: usize) {
        let d = depth.clamp(1, 7) as usize;
        self.v2_moves[d * 2 + usize::from(in_check)][kind].fetch_add(1, Ordering::Relaxed);
    }

    /// Searched moves by kind (see `v2_moves`).
    pub fn move_kind_counts(&self) -> [[u64; 8]; 16] {
        std::array::from_fn(|r| {
            std::array::from_fn(|i| self.v2_moves[r][i].load(Ordering::Relaxed))
        })
    }

    /// SEARCH_V2 nodes by depth (see `v2_nodes`).
    pub fn v2_node_counts(&self) -> [[u64; 4]; 17] {
        std::array::from_fn(|r| {
            std::array::from_fn(|i| self.v2_nodes[r][i].load(Ordering::Relaxed))
        })
    }

    /// TT probes by node depth (see `tt_by_depth`).
    pub fn tt_probe_counts(&self) -> [[u64; 4]; 17] {
        std::array::from_fn(|r| {
            std::array::from_fn(|i| self.tt_by_depth[r][i].load(Ordering::Relaxed))
        })
    }

    /// Cutoffs at nodes with a quiet TT move (see `tt_quiet_cut`).
    pub fn tt_quiet_cut_counts(&self) -> [[u64; 18]; 2] {
        std::array::from_fn(|r| {
            std::array::from_fn(|i| self.tt_quiet_cut[r][i].load(Ordering::Relaxed))
        })
    }

    /// Searched later moves (after the first) that give check, by window.
    pub fn searched_check_counts(&self) -> [u64; 2] {
        std::array::from_fn(|i| self.searched_checks[i].load(Ordering::Relaxed))
    }

    /// Beta cutoffs by the position of the cutoff move in its node's order
    /// (rows: quiet and other cutoff moves at nodes without a TT move, then
    /// the same with one), the nodes that failed low, and the nodes that
    /// entered the move loop without and with a TT move.
    pub fn cut_histogram(&self) -> ([[u64; CUT_BUCKETS]; 4], u64, [u64; 2]) {
        (
            std::array::from_fn(|r| {
                std::array::from_fn(|b| self.cut_index[r][b].load(Ordering::Relaxed))
            }),
            self.fail_low_nodes.load(Ordering::Relaxed),
            std::array::from_fn(|i| self.loop_nodes[i].load(Ordering::Relaxed)),
        )
    }

    /// Quiescence outcomes and work (see the `qsearch` field).
    pub fn qsearch_counts(&self) -> [u64; 11] {
        std::array::from_fn(|i| self.qsearch[i].load(Ordering::Relaxed))
    }

    /// SEARCH_V2 move-loop stage usage (see the `move_stages` field).
    pub fn move_stage_counts(&self) -> [u64; 4] {
        std::array::from_fn(|i| self.move_stages[i].load(Ordering::Relaxed))
    }

    /// Return counters collected so far without resetting the observer.
    pub fn snapshot(&self) -> SearchDiagnosticsSnapshot {
        SearchDiagnosticsSnapshot {
            static_evaluations: self.static_evaluations.load(Ordering::Relaxed),
            eval_cache_probes: self.eval_cache_probes.load(Ordering::Relaxed),
            eval_cache_hits: self.eval_cache_hits.load(Ordering::Relaxed),
            tt_probes: self.tt_probes.load(Ordering::Relaxed),
            tt_hits: self.tt_hits.load(Ordering::Relaxed),
            tt_stores: self.tt_stores.load(Ordering::Relaxed),
            order_tt: self.order_tt.load(Ordering::Relaxed),
            order_killer: self.order_killer.load(Ordering::Relaxed),
            order_countermove: self.order_countermove.load(Ordering::Relaxed),
            order_history: self.order_history.load(Ordering::Relaxed),
            root_mate_in_one_nodes: self.root_mate_in_one_nodes.load(Ordering::Relaxed),
            root_mate_blunder_nodes: self.root_mate_blunder_nodes.load(Ordering::Relaxed),
            root_mate_in_one_cache_hits: self.root_mate_in_one_cache_hits.load(Ordering::Relaxed),
            root_mate_blunder_cache_hits: self.root_mate_blunder_cache_hits.load(Ordering::Relaxed),
            alpha_beta_calls: self.alpha_beta_calls.load(Ordering::Relaxed),
            quiescence_calls: self.quiescence_calls.load(Ordering::Relaxed),
            qsearch: self.qsearch_counts(),
            static_evaluation_ns: self.static_evaluation_ns.load(Ordering::Relaxed),
            tt_probe_ns: self.tt_probe_ns.load(Ordering::Relaxed),
            tt_store_ns: self.tt_store_ns.load(Ordering::Relaxed),
            movegen_order_ns: self.movegen_order_ns.load(Ordering::Relaxed),
            movegen_generate_ns: self.movegen_generate_ns.load(Ordering::Relaxed),
            move_order_ns: self.move_order_ns.load(Ordering::Relaxed),
            move_order_score_ns: self.move_order_score_ns.load(Ordering::Relaxed),
            move_order_sort_ns: self.move_order_sort_ns.load(Ordering::Relaxed),
            quiescence_inclusive_ns: self.quiescence_inclusive_ns.load(Ordering::Relaxed),
            root_mate_safety_ns: self.root_mate_safety_ns.load(Ordering::Relaxed),
        }
    }
    /// Return sampled TT ownership counters for Lazy SMP diagnostics.
    ///
    /// Keeping these counters out of [`SearchDiagnosticsSnapshot`] preserves
    /// that public struct's exhaustive-literal contract across 0.4.x patches.
    pub(crate) fn tt_reuse_snapshot(&self) -> TtReuseSnapshot {
        TtReuseSnapshot {
            local_hits: self.tt_local_hits.load(Ordering::Relaxed),
            foreign_hits: self.tt_foreign_hits.load(Ordering::Relaxed),
            unknown_hits: self.tt_unknown_hits.load(Ordering::Relaxed),
            ..TtReuseSnapshot::default()
        }
    }
}

const EVAL_CACHE_ENTRIES: usize = 1 << 16;

struct EvalCacheSlot {
    sequence: AtomicU64,
    hash: AtomicU64,
    score: AtomicI32,
}

/// Exact, nonblocking cache for repeated NNUE static evaluations.
///
/// The sequence guard makes a concurrent overwrite a miss rather than a
/// mixed hash/score hit. Writers that lose the per-slot CAS simply skip the
/// optional store, so search workers never wait for one another.
struct EvalCache {
    slots: Box<[EvalCacheSlot]>,
    mask: usize,
}

impl EvalCache {
    fn new() -> Self {
        let slots = (0..EVAL_CACHE_ENTRIES)
            .map(|_| EvalCacheSlot {
                sequence: AtomicU64::new(0),
                hash: AtomicU64::new(0),
                score: AtomicI32::new(0),
            })
            .collect::<Vec<_>>()
            .into_boxed_slice();
        Self {
            slots,
            mask: EVAL_CACHE_ENTRIES - 1,
        }
    }

    #[inline]
    fn slot(&self, hash: u64) -> &EvalCacheSlot {
        &self.slots[hash as usize & self.mask]
    }

    #[inline]
    fn probe(&self, hash: u64) -> Option<i32> {
        let slot = self.slot(hash);
        let before = slot.sequence.load(Ordering::Acquire);
        if before == 0 || before & 1 != 0 {
            return None;
        }
        let stored_hash = slot.hash.load(Ordering::Relaxed);
        let score = slot.score.load(Ordering::Relaxed);
        // An acquire load orders operations after itself, but it does not keep
        // the preceding payload loads from moving past the validation load.
        // The fence completes the seqlock read protocol before we re-read the
        // sequence, so a concurrent overwrite is always observed as a miss.
        fence(Ordering::Acquire);
        let after = slot.sequence.load(Ordering::Relaxed);
        (before == after && after & 1 == 0 && stored_hash == hash).then_some(score)
    }

    #[inline]
    fn store(&self, hash: u64, score: i32) {
        let slot = self.slot(hash);
        let sequence = slot.sequence.load(Ordering::Relaxed);
        if sequence & 1 != 0
            || slot
                .sequence
                .compare_exchange_weak(
                    sequence,
                    sequence.wrapping_add(1),
                    Ordering::AcqRel,
                    Ordering::Relaxed,
                )
                .is_err()
        {
            return;
        }
        slot.hash.store(hash, Ordering::Relaxed);
        slot.score.store(score, Ordering::Relaxed);
        slot.sequence
            .store(sequence.wrapping_add(2), Ordering::Release);
    }

    fn clear(&self) {
        for slot in &self.slots {
            slot.sequence.store(0, Ordering::Relaxed);
        }
    }
}

fn active_eval_cache() -> Option<Arc<EvalCache>> {
    (weights_active() || crate::halfkp::is_active()).then(|| Arc::new(EvalCache::new()))
}

impl Default for SearchDiagnostics {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================
// Internal search state (shared across threads via Arc)
// ============================================================

struct SearchState {
    tt: Arc<Tt>,
    budget: Arc<Budget>,
    killers: KillerTable,
    history: Arc<HistoryTable>,
    countermoves: Arc<CountermoveTable>,
    diagnostics: Option<Arc<SearchDiagnostics>>,
    eval_cache: Option<Arc<EvalCache>>,
    pruning: PruningConfig,
    retirement: Option<RetirementEndpoint>,
}

impl SearchState {
    /// Timer for one cost counter, live only when a timing observer is attached.
    #[inline(always)]
    fn timer(&self, counter: fn(&SearchDiagnostics) -> &AtomicU64) -> ProfileTimer<'_> {
        ProfileTimer::new(
            self.diagnostics
                .as_deref()
                .and_then(|diagnostics| diagnostics.timed(counter(diagnostics))),
        )
    }

    /// The per-node counters, when an observer records them.
    #[inline(always)]
    fn counters(&self) -> Option<&SearchDiagnostics> {
        self.diagnostics
            .as_deref()
            .and_then(SearchDiagnostics::per_node)
    }
}

/// Call the static evaluator while accounting for it in an opt-in diagnostic.
///
/// The observer is absent from production searches, so no atomic operation is
/// performed outside an explicitly requested cost profile.
#[inline]
fn evaluate_for_search(state: &SearchState, board: &Board) -> i32 {
    let _timer = state.timer(|d| &d.static_evaluation_ns);
    if let Some(diagnostics) = state.counters() {
        diagnostics
            .static_evaluations
            .fetch_add(1, Ordering::Relaxed);
    }
    if let Some(cache) = &state.eval_cache {
        if let Some(diagnostics) = state.counters() {
            diagnostics
                .eval_cache_probes
                .fetch_add(1, Ordering::Relaxed);
        }
        let key = evaluation_cache_key(board.hash());
        if let Some(score) = cache.probe(key) {
            if let Some(diagnostics) = state.counters() {
                diagnostics.eval_cache_hits.fetch_add(1, Ordering::Relaxed);
            }
            return score;
        }
        let score = evaluate(board);
        cache.store(key, score);
        score
    } else {
        evaluate(board)
    }
}

#[inline]
fn probe_tt_for_search(state: &SearchState, hash: u64) -> Option<TtEntry> {
    probe_tt_for_search_pv(state, hash).map(|(entry, _)| entry)
}

/// [`probe_tt_for_search`] with the entry's PV flag.
#[inline]
fn probe_tt_for_search_pv(state: &SearchState, hash: u64) -> Option<(TtEntry, bool)> {
    let entry = {
        let _timer = state.timer(|d| &d.tt_probe_ns);
        state.tt.probe_pv(hash)
    };
    if let Some(diagnostics) = state.counters() {
        diagnostics.tt_probes.fetch_add(1, Ordering::Relaxed);
        if entry.is_some() {
            diagnostics.tt_hits.fetch_add(1, Ordering::Relaxed);
            diagnostics.record_tt_hit_origin(hash);
        }
    }
    if entry.is_some()
        && let Some(endpoint) = &state.retirement
    {
        endpoint
            .channel
            .reuse
            .record_hit(hash, endpoint.worker as u32);
    }
    entry
}

#[inline]
fn store_tt_for_search(state: &SearchState, hash: u64, entry: TtEntry) {
    store_tt_for_search_pv(state, hash, entry, false);
}

/// [`store_tt_for_search`] with the node's PV flag.
#[inline]
fn store_tt_for_search_pv(state: &SearchState, hash: u64, entry: TtEntry, pv: bool) {
    let _timer = state.timer(|d| &d.tt_store_ns);
    let committed = state.tt.store_pv_with_result(hash, entry, pv);
    if committed && let Some(endpoint) = &state.retirement {
        endpoint
            .channel
            .reuse
            .record_store(hash, endpoint.worker as u32);
    }
    if let Some(diagnostics) = state.counters() {
        diagnostics.tt_stores.fetch_add(1, Ordering::Relaxed);
        if committed {
            diagnostics.record_tt_store_origin(hash);
        }
    }
}

/// TM_MODE 1: whether to stop after a completed iteration. The soft limit
/// is scaled by the best move's stability (`TM_STAB_MAX` percent after a
/// change, `TM_STAB_STEP` less per further unchanged iteration, at least
/// `TM_STAB_MIN`) and raised by a falling score (`drop / TM_DROP_DIV`, at
/// most half again).
fn tm_scaled_soft_expired(
    elapsed: std::time::Duration,
    soft_limit: Option<std::time::Duration>,
    depth: u32,
    stable_iters: u32,
    drop: i32,
) -> bool {
    let Some(soft) = soft_limit else {
        return false;
    };
    if depth < 4 {
        return false;
    }
    let stab =
        (p::TM_STAB_MAX() - p::TM_STAB_STEP() * stable_iters.min(64) as i32).max(p::TM_STAB_MIN());
    let drop_pct = 100 + (drop.max(0) * 100 / p::TM_DROP_DIV().max(1)).min(50);
    let limit_ms = soft.as_millis() as u64 * stab.max(1) as u64 / 100 * drop_pct as u64 / 100;
    elapsed.as_millis() as u64 >= limit_ms
}

/// Search pruning switches used by diagnostic ablations.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PruningConfig {
    /// Enable null-move pruning and its verification search.
    pub null_move: bool,
    /// Enable late-move reductions for quiet, late-ordered moves.
    pub late_move_reduction: bool,
    /// Search young brothers in parallel on the Rayon pool (YBW). Lazy SMP
    /// workers already occupy the pool and search sequentially.
    pub ybw_split: bool,
}

impl Default for PruningConfig {
    fn default() -> Self {
        Self {
            null_move: true,
            late_move_reduction: true,
            ybw_split: true,
        }
    }
}

// ============================================================
// Searcher
// ============================================================

/// Sequential (with YBW-parallel helper threads) iterative-deepening alpha-beta searcher.
pub struct Searcher {
    tt: Arc<Tt>,
    /// Exposed for USI "stop" command — set to true to abort an in-progress search
    external_abort: Arc<AtomicBool>,
    diagnostics: Option<Arc<SearchDiagnostics>>,
    eval_cache: Option<Arc<EvalCache>>,
    pruning: PruningConfig,
    /// Move-ordering history, countermoves and eval correction, kept from
    /// one search to the next within a game and cleared with the TT
    /// (`clear_tt`, on a new game). Killers stay per search.
    history: Arc<HistoryTable>,
    countermoves: Arc<CountermoveTable>,
    /// Plies added to every iterative-deepening iteration (Lazy SMP helper
    /// workers use 1 so that they search one ply ahead of the main worker).
    depth_skew: u32,
    /// Deterministic rotation applied to the first root iteration only.
    /// Lazy SMP can use this to make helpers examine different root moves
    /// before the shared TT has enough information to diversify them.
    root_order_offset: usize,
    /// Use the full root LMR reduction instead of the usual one-ply relief.
    /// Lazy SMP may enable this only for helper workers as an opt-in
    /// experiment; ordinary searches and worker 0 keep the established path.
    full_root_lmr: bool,
    /// Optional Lazy SMP channel. Helpers publish only fully completed root
    /// iterations; worker 0 may use a same-or-deeper move as ordering advice
    /// on its next iteration. Scores and bounds are deliberately not shared.
    root_hint: Option<RootHintEndpoint>,
    retirement: Option<RetirementEndpoint>,
}

/// One lock-free, invocation-scoped root-order hint shared by Lazy SMP
/// workers. The slot is reset before every `go`, so a persistent worker can
/// never consume a move from an earlier position.
pub(crate) struct RootHintChannel {
    generation: AtomicU64,
    packed: AtomicU64,
    publications: GenerationCounter,
    eligible_reads: GenerationCounter,
    applied: GenerationCounter,
    generation_gap_max: GenerationCounter,
    generation_gap_sum: GenerationCounter,
    depth_gap_max: GenerationCounter,
    depth_gap_sum: GenerationCounter,
    stale_reads: GenerationCounter,
    stale_research_nodes: GenerationCounter,
    research_nodes: GenerationCounter,
    researches: GenerationCounter,
    first_move_beta_cuts: GenerationCounter,
}

#[derive(Clone)]
struct RootHintEndpoint {
    channel: Arc<RootHintChannel>,
    worker: usize,
}

#[derive(Clone)]
struct RootHintOrder {
    mv: Move,
    generation: u64,
    channel: Arc<RootHintChannel>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct RootHintSnapshot {
    pub publications: u64,
    pub eligible_reads: u64,
    pub applied: u64,
    pub generation_gap_max: u64,
    pub generation_gap_sum: u64,
    pub depth_gap_max: u64,
    pub depth_gap_sum: u64,
    pub stale_reads: u64,
    pub stale_research_nodes: u64,
    pub research_nodes: u64,
    pub researches: u64,
    pub first_move_beta_cuts: u64,
}

struct GenerationCounter {
    packed: AtomicU64,
}

impl GenerationCounter {
    const COUNT_BITS: u32 = RootHintChannel::GENERATION_SHIFT;
    const COUNT_MASK: u64 = (1 << Self::COUNT_BITS) - 1;

    const fn new() -> Self {
        Self {
            packed: AtomicU64::new(0),
        }
    }

    fn reset(&self, generation: u64) {
        self.packed
            .store(generation << Self::COUNT_BITS, Ordering::Relaxed);
    }

    fn increment(&self, generation: u64) {
        self.add(generation, 1);
    }

    fn add(&self, generation: u64, amount: u64) {
        self.update(generation, amount, false);
    }

    fn maximum(&self, generation: u64, value: u64) {
        self.update(generation, value, true);
    }

    fn update(&self, generation: u64, amount: u64, maximum: bool) {
        let mut current = self.packed.load(Ordering::Relaxed);
        loop {
            if current >> Self::COUNT_BITS != generation {
                return;
            }
            let count = current & Self::COUNT_MASK;
            if count == Self::COUNT_MASK {
                return;
            }
            let next_count = if maximum {
                count.max(amount).min(Self::COUNT_MASK)
            } else {
                count.saturating_add(amount).min(Self::COUNT_MASK)
            };
            let next = (generation << Self::COUNT_BITS) | next_count;
            match self.packed.compare_exchange_weak(
                current,
                next,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => return,
                Err(observed) => current = observed,
            }
        }
    }

    fn get(&self, generation: u64) -> u64 {
        let value = self.packed.load(Ordering::Relaxed);
        if value >> Self::COUNT_BITS == generation {
            value & Self::COUNT_MASK
        } else {
            0
        }
    }
}

impl RootHintChannel {
    const MOVE_BITS: u32 = 19;
    const DEPTH_BITS: u32 = 8;
    const WORKER_BITS: u32 = 9;
    const MOVE_MASK: u64 = (1 << Self::MOVE_BITS) - 1;
    const DEPTH_MASK: u64 = (1 << Self::DEPTH_BITS) - 1;
    const WORKER_MASK: u64 = (1 << Self::WORKER_BITS) - 1;
    const DEPTH_SHIFT: u32 = Self::MOVE_BITS;
    const WORKER_SHIFT: u32 = Self::DEPTH_SHIFT + Self::DEPTH_BITS;
    const GENERATION_SHIFT: u32 = Self::WORKER_SHIFT + Self::WORKER_BITS;
    const GENERATION_MASK: u64 = (1 << (64 - Self::GENERATION_SHIFT)) - 1;

    pub(crate) fn new() -> Self {
        Self {
            generation: AtomicU64::new(0),
            packed: AtomicU64::new(0),
            publications: GenerationCounter::new(),
            eligible_reads: GenerationCounter::new(),
            applied: GenerationCounter::new(),
            generation_gap_max: GenerationCounter::new(),
            generation_gap_sum: GenerationCounter::new(),
            depth_gap_max: GenerationCounter::new(),
            depth_gap_sum: GenerationCounter::new(),
            stale_reads: GenerationCounter::new(),
            stale_research_nodes: GenerationCounter::new(),
            research_nodes: GenerationCounter::new(),
            researches: GenerationCounter::new(),
            first_move_beta_cuts: GenerationCounter::new(),
        }
    }

    /// Start a new invocation and return the token every worker must retain.
    /// The generation is embedded in the same atomic word as the hint, so a
    /// delayed compare-exchange from an earlier invocation cannot repopulate
    /// the slot after this reset.
    pub(crate) fn begin_go(&self) -> u64 {
        // The packed representation retains 28 generation bits. LazySmpSearcher
        // serializes invocations and joins every helper before the next call,
        // so wrapping cannot make a live hint look current.
        let sequence = self.generation.fetch_add(1, Ordering::AcqRel);
        let generation = sequence % Self::GENERATION_MASK + 1;
        self.publications.reset(generation);
        self.eligible_reads.reset(generation);
        self.applied.reset(generation);
        self.generation_gap_max.reset(generation);
        self.generation_gap_sum.reset(generation);
        self.depth_gap_max.reset(generation);
        self.depth_gap_sum.reset(generation);
        self.stale_reads.reset(generation);
        self.stale_research_nodes.reset(generation);
        self.research_nodes.reset(generation);
        self.researches.reset(generation);
        self.first_move_beta_cuts.reset(generation);
        self.packed
            .store(generation << Self::GENERATION_SHIFT, Ordering::Release);
        generation
    }

    fn current_generation(&self) -> u64 {
        (self.packed.load(Ordering::Acquire) >> Self::GENERATION_SHIFT) & Self::GENERATION_MASK
    }

    fn encode(generation: u64, depth: u32, worker: usize, mv: Move) -> u64 {
        (u64::from(pack_killer(mv)) & Self::MOVE_MASK)
            | (u64::from(depth.min(Self::DEPTH_MASK as u32)) << Self::DEPTH_SHIFT)
            | ((worker.min(Self::WORKER_MASK as usize) as u64) << Self::WORKER_SHIFT)
            | ((generation & Self::GENERATION_MASK) << Self::GENERATION_SHIFT)
    }

    fn decode(value: u64) -> Option<(u64, u32, usize, Move)> {
        let raw_move = (value & Self::MOVE_MASK) as u32;
        if raw_move == 0 {
            return None;
        }
        let mv = unpack_killer(raw_move)?;
        let depth = ((value >> Self::DEPTH_SHIFT) & Self::DEPTH_MASK) as u32;
        let worker = ((value >> Self::WORKER_SHIFT) & Self::WORKER_MASK) as usize;
        let generation = (value >> Self::GENERATION_SHIFT) & Self::GENERATION_MASK;
        Some((generation, depth, worker, mv))
    }

    /// Publish a completed helper iteration. Deeper hints win; equal-depth
    /// races are resolved by worker index and compact move encoding so the
    /// retained value does not depend on scheduling order.
    fn publish(&self, generation: u64, depth: u32, worker: usize, mv: Move) {
        debug_assert!(worker > 0, "worker 0 consumes root hints");
        let candidate = Self::encode(generation, depth, worker, mv);
        let (_, stored_depth, stored_worker, stored_move) =
            Self::decode(candidate).expect("encoded root hint must contain a move");
        let candidate_key = (
            stored_depth,
            std::cmp::Reverse(stored_worker),
            std::cmp::Reverse(stored_move.raw()),
        );
        let mut current = self.packed.load(Ordering::Acquire);
        loop {
            let current_generation = (current >> Self::GENERATION_SHIFT) & Self::GENERATION_MASK;
            if current_generation != generation {
                return;
            }
            if let Some((_, old_depth, old_worker, old_move)) = Self::decode(current) {
                let old_key = (
                    old_depth,
                    std::cmp::Reverse(old_worker),
                    std::cmp::Reverse(old_move.raw()),
                );
                if old_key >= candidate_key {
                    return;
                }
            }
            match self.packed.compare_exchange_weak(
                current,
                candidate,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => {
                    self.publications.increment(generation);
                    return;
                }
                Err(observed) => current = observed,
            }
        }
    }

    fn move_at_least(&self, generation: u64, depth: u32) -> Option<Move> {
        let (hint_generation, hint_depth, _, mv) =
            Self::decode(self.packed.load(Ordering::Acquire))?;
        if hint_generation != generation || hint_depth < depth {
            self.stale_reads.increment(generation);
            return None;
        }
        let gap = u64::from(hint_depth - depth);
        self.depth_gap_sum.add(generation, gap);
        self.depth_gap_max.maximum(generation, gap);
        self.eligible_reads.increment(generation);
        Some(mv)
    }

    fn mark_applied(&self, generation: u64) {
        self.applied.increment(generation);
    }

    pub(crate) fn snapshot(&self, generation: u64) -> RootHintSnapshot {
        RootHintSnapshot {
            publications: self.publications.get(generation),
            eligible_reads: self.eligible_reads.get(generation),
            applied: self.applied.get(generation),
            generation_gap_max: self.generation_gap_max.get(generation),
            generation_gap_sum: self.generation_gap_sum.get(generation),
            depth_gap_max: self.depth_gap_max.get(generation),
            depth_gap_sum: self.depth_gap_sum.get(generation),
            stale_reads: self.stale_reads.get(generation),
            stale_research_nodes: self.stale_research_nodes.get(generation),
            research_nodes: self.research_nodes.get(generation),
            researches: self.researches.get(generation),
            first_move_beta_cuts: self.first_move_beta_cuts.get(generation),
        }
    }
}

impl Searcher {
    /// Create a searcher backed by the given shared transposition table.
    pub fn new(tt: Arc<Tt>) -> Self {
        Self::with_abort_flag(tt, Arc::new(AtomicBool::new(false)))
    }

    /// Create a searcher using a caller-owned abort flag. This lets independent
    /// Lazy SMP workers stop as one group without sharing mutable search state.
    pub fn with_abort_flag(tt: Arc<Tt>, external_abort: Arc<AtomicBool>) -> Self {
        Searcher {
            tt,
            external_abort,
            diagnostics: None,
            eval_cache: active_eval_cache(),
            pruning: PruningConfig::default(),
            history: Arc::new(HistoryTable::new()),
            countermoves: Arc::new(CountermoveTable::new()),
            depth_skew: 0,
            root_order_offset: 0,
            full_root_lmr: false,
            root_hint: None,
            retirement: None,
        }
    }

    /// Create a searcher using a caller-owned abort flag and diagnostics.
    ///
    /// Lazy SMP uses this only for explicitly enabled scaling measurements;
    /// production workers keep [`Self::with_abort_flag`] and therefore pay no
    /// per-node diagnostic-counter cost.
    pub fn with_abort_flag_and_diagnostics(
        tt: Arc<Tt>,
        external_abort: Arc<AtomicBool>,
        diagnostics: Arc<SearchDiagnostics>,
    ) -> Self {
        Searcher {
            tt,
            external_abort,
            diagnostics: Some(diagnostics),
            eval_cache: active_eval_cache(),
            pruning: PruningConfig::default(),
            history: Arc::new(HistoryTable::new()),
            countermoves: Arc::new(CountermoveTable::new()),
            depth_skew: 0,
            root_order_offset: 0,
            full_root_lmr: false,
            root_hint: None,
            retirement: None,
        }
    }

    /// Create a searcher with explicit pruning switches for controlled diagnostics.
    /// The default [`Self::new`] path keeps all production pruning enabled.
    pub fn with_pruning(tt: Arc<Tt>, pruning: PruningConfig) -> Self {
        Self {
            tt,
            external_abort: Arc::new(AtomicBool::new(false)),
            diagnostics: None,
            eval_cache: active_eval_cache(),
            pruning,
            history: Arc::new(HistoryTable::new()),
            countermoves: Arc::new(CountermoveTable::new()),
            depth_skew: 0,
            root_order_offset: 0,
            full_root_lmr: false,
            root_hint: None,
            retirement: None,
        }
    }

    /// Create a searcher with controlled pruning and opt-in diagnostics.
    pub fn with_pruning_and_diagnostics(
        tt: Arc<Tt>,
        pruning: PruningConfig,
        diagnostics: Arc<SearchDiagnostics>,
    ) -> Self {
        Searcher {
            tt,
            external_abort: Arc::new(AtomicBool::new(false)),
            diagnostics: Some(diagnostics),
            eval_cache: active_eval_cache(),
            pruning,
            history: Arc::new(HistoryTable::new()),
            countermoves: Arc::new(CountermoveTable::new()),
            depth_skew: 0,
            root_order_offset: 0,
            full_root_lmr: false,
            root_hint: None,
            retirement: None,
        }
    }

    /// Create a searcher that records optional move-ordering diagnostics.
    pub fn with_diagnostics(tt: Arc<Tt>, diagnostics: Arc<SearchDiagnostics>) -> Self {
        Self {
            tt,
            external_abort: Arc::new(AtomicBool::new(false)),
            diagnostics: Some(diagnostics),
            eval_cache: active_eval_cache(),
            pruning: PruningConfig::default(),
            history: Arc::new(HistoryTable::new()),
            countermoves: Arc::new(CountermoveTable::new()),
            depth_skew: 0,
            root_order_offset: 0,
            full_root_lmr: false,
            root_hint: None,
            retirement: None,
        }
    }

    /// Returns an `Arc` to the abort flag; store `true` to stop the search early.
    pub fn abort_flag(&self) -> Arc<AtomicBool> {
        self.external_abort.clone()
    }

    /// Clear a previous stop signal before starting a new search.
    pub fn reset_abort_flag(&self) {
        self.external_abort.store(false, Ordering::Relaxed);
    }

    /// Probe the TT for a legal ponder hint in the current position.
    pub fn probe_tt(&self, hash: u64) -> Option<Move> {
        self.tt.probe(hash).and_then(|entry| entry.mv)
    }

    /// Clear all cached positions before a new unrelated game.
    pub fn clear_tt(&self) {
        self.tt.clear();
        self.clear_worker_state();
    }

    /// Reset private evaluation and move-ordering state without clearing a
    /// transposition table shared with other Lazy SMP workers.
    pub(crate) fn clear_worker_state(&self) {
        if let Some(cache) = &self.eval_cache {
            cache.clear();
        }
        self.history.clear();
        self.countermoves.clear();
    }

    /// Age the TT entries of earlier searches (call once per `go`).
    pub fn new_search(&self) {
        self.tt.new_search();
    }

    #[cfg(test)]
    pub(crate) fn tt_search_count(&self) -> u64 {
        self.tt.search_count()
    }

    /// Search every iteration `skew` plies deeper (Lazy SMP helper workers).
    pub fn set_depth_skew(&mut self, skew: u32) {
        self.depth_skew = skew;
    }

    /// Rotate the initial root ordering by `offset` moves. Iterative-deepening
    /// feedback replaces this seed after the first completed root search.
    pub fn set_root_order_offset(&mut self, offset: usize) {
        self.root_order_offset = offset;
    }

    /// Remove the usual one-ply root-LMR relief. This crate-private control is
    /// used only by an opt-in Lazy SMP helper experiment.
    pub(crate) fn set_full_root_lmr(&mut self, enabled: bool) {
        self.full_root_lmr = enabled;
    }

    #[cfg(test)]
    pub(crate) fn full_root_lmr_enabled(&self) -> bool {
        self.full_root_lmr
    }

    pub(crate) fn set_retirement(&mut self, channel: Arc<HelperRetirement>, worker: usize) {
        self.retirement = Some(RetirementEndpoint { channel, worker });
    }

    /// Connect this searcher to an invocation-scoped Lazy SMP root-hint slot.
    pub(crate) fn set_root_hint_channel(&mut self, channel: Arc<RootHintChannel>, worker: usize) {
        self.root_hint = Some(RootHintEndpoint { channel, worker });
    }

    /// Enable or disable parallel young-brothers splitting (Lazy SMP workers
    /// search sequentially).
    pub fn set_ybw_split(&mut self, enabled: bool) {
        self.pruning.ybw_split = enabled;
    }

    /// Run iterative-deepening search from the current position up to `config.max_depth`
    /// or until a time limit / abort signal fires, returning the best line found.
    ///
    /// Call [`Self::reset_abort_flag`] before reusing a searcher after an abort.
    pub fn search(&self, board: &mut Board, config: SearchConfig) -> SearchInfo {
        let history = PositionHistory::initial(board.hash());
        self.search_impl(board, config, None, true, &history, SearchTraces::default())
    }

    /// Search a position together with its already-played game history.
    ///
    /// Repetition outcomes are history dependent and therefore are detected
    /// before transposition-table probing or storage.  The supplied history
    /// must end at `board`'s current hash.
    pub fn search_with_history(
        &self,
        board: &mut Board,
        config: SearchConfig,
        history: &PositionHistory,
    ) -> SearchInfo {
        debug_assert_eq!(
            history.entries().last().map(|entry| entry.hash),
            Some(board.hash())
        );
        self.search_impl(board, config, None, true, history, SearchTraces::default())
    }

    /// Run a normal history-aware search and retain each completed iteration.
    ///
    /// This is opt-in diagnostic instrumentation. It records only passes that
    /// finished before a hard stop, so an interrupted deeper pass cannot be
    /// mistaken for a stable root result.
    pub fn search_with_history_trace(
        &self,
        board: &mut Board,
        config: SearchConfig,
        history: &PositionHistory,
    ) -> (SearchInfo, Vec<SearchIteration>) {
        debug_assert_eq!(
            history.entries().last().map(|entry| entry.hash),
            Some(board.hash())
        );
        let mut trace = Vec::new();
        let info = self.search_impl(
            board,
            config,
            None,
            true,
            history,
            SearchTraces {
                iterations: Some(&mut trace),
                multi_pv: None,
            },
        );
        (info, trace)
    }

    /// Run bounded MultiPV search and retain only fully completed candidate
    /// sets. All candidates share one budget and one completed root depth.
    pub fn search_multi_pv_with_history_trace(
        &self,
        board: &mut Board,
        config: SearchConfig,
        history: &PositionHistory,
    ) -> (SearchInfo, Vec<MultiPvIteration>) {
        debug_assert_eq!(
            history.entries().last().map(|entry| entry.hash),
            Some(board.hash())
        );
        let mut trace = Vec::new();
        let info = self.search_impl(
            board,
            config,
            None,
            true,
            history,
            SearchTraces {
                iterations: None,
                multi_pv: Some(&mut trace),
            },
        );
        (info, trace)
    }

    /// Run a history-aware search without the production-only root mate safety
    /// filter. This is exclusively for controlled diagnostics: callers must
    /// not use it to choose an engine move.
    pub fn search_with_history_without_root_mate_safety(
        &self,
        board: &mut Board,
        config: SearchConfig,
        history: &PositionHistory,
    ) -> SearchInfo {
        debug_assert_eq!(
            history.entries().last().map(|entry| entry.hash),
            Some(board.hash())
        );
        self.search_impl(board, config, None, false, history, SearchTraces::default())
    }

    /// Diagnostic counterpart to [`Self::search_with_history_trace`] with the
    /// root mate safety filter disabled.
    pub fn search_with_history_trace_without_root_mate_safety(
        &self,
        board: &mut Board,
        config: SearchConfig,
        history: &PositionHistory,
    ) -> (SearchInfo, Vec<SearchIteration>) {
        debug_assert_eq!(
            history.entries().last().map(|entry| entry.hash),
            Some(board.hash())
        );
        let mut trace = Vec::new();
        let info = self.search_impl(
            board,
            config,
            None,
            false,
            history,
            SearchTraces {
                iterations: Some(&mut trace),
                multi_pv: None,
            },
        );
        (info, trace)
    }

    /// Search a bounded position label without the expensive shallow
    /// root-mate safety filter used for interactive move selection.
    ///
    /// The label search still uses the same legal move generator, alpha-beta,
    /// evaluator, and explicit node/time budget. Omitting the root-only
    /// filter keeps a finite label budget meaningful on positions with many
    /// drops; it must not be used to choose a production USI move.
    pub fn search_for_teacher(&self, board: &mut Board, config: SearchConfig) -> SearchInfo {
        let history = PositionHistory::initial(board.hash());
        self.search_impl(
            board,
            config,
            None,
            false,
            &history,
            SearchTraces::default(),
        )
    }

    /// Search while fixing the root move to `root_move`.
    ///
    /// This is intentionally a diagnostic API: it uses the same iterative
    /// deepening, evaluator, TT, and node budget as [`Self::search`], but
    /// restricts the root to one already-legal move.  It enables actual-move
    /// versus alternative-move comparisons without treating the played move as
    /// a correctness label or changing normal engine behavior.
    pub fn search_root_move(
        &self,
        board: &mut Board,
        config: SearchConfig,
        root_move: Move,
    ) -> SearchInfo {
        let history = PositionHistory::initial(board.hash());
        self.search_root_move_with_history(board, config, root_move, &history)
    }

    /// Search while fixing the root move and preserving an already-played
    /// history for repetition adjudication.
    pub fn search_root_move_with_history(
        &self,
        board: &mut Board,
        config: SearchConfig,
        root_move: Move,
        history: &PositionHistory,
    ) -> SearchInfo {
        debug_assert_eq!(
            history.entries().last().map(|entry| entry.hash),
            Some(board.hash())
        );
        if !MoveBuffer::legal(board).as_slice().contains(&root_move) {
            return SearchInfo {
                best_move: None,
                score: 0,
                depth: 0,
                nodes: 0,
                elapsed: Duration::ZERO,
                hashfull: self.tt.hashfull(),
                bound: SearchBound::Unknown,
                completed_bound: SearchBound::Unknown,
                aborted: false,
                abort_reason: "none",
                pv: Vec::new(),
            };
        }
        self.search_impl(
            board,
            config,
            Some(root_move),
            true,
            history,
            SearchTraces::default(),
        )
    }

    /// Search an explicit set of legal root candidates independently.
    ///
    /// Illegal candidates are ignored. Each returned result is isolated to
    /// its root move and the caller's board is restored after every search.
    /// The supplied order is not treated as an engine ranking or correctness
    /// label.
    pub fn search_root_candidates(
        &self,
        board: &mut Board,
        config: SearchConfig,
        candidates: &[Move],
    ) -> Vec<RootCandidateInfo> {
        let history = PositionHistory::initial(board.hash());
        self.search_root_candidates_with_history(board, config, candidates, &history)
    }

    /// Search explicit root candidates while preserving the supplied game
    /// history. Candidate searches remain individually isolated.
    pub fn search_root_candidates_with_history(
        &self,
        board: &mut Board,
        config: SearchConfig,
        candidates: &[Move],
        history: &PositionHistory,
    ) -> Vec<RootCandidateInfo> {
        debug_assert_eq!(
            history.entries().last().map(|entry| entry.hash),
            Some(board.hash())
        );
        let legal = MoveBuffer::legal(board);
        let mut selected = Vec::new();
        for candidate in candidates.iter().copied() {
            if legal.as_slice().contains(&candidate) && !selected.contains(&candidate) {
                selected.push(candidate);
            }
        }
        selected
            .iter()
            .copied()
            .map(|root_move| {
                self.reset_abort_flag();
                let info = self.search_root_move_with_history(board, config, root_move, history);
                RootCandidateInfo { root_move, info }
            })
            .collect()
    }

    fn search_impl(
        &self,
        board: &mut Board,
        config: SearchConfig,
        root_move: Option<Move>,
        root_mate_safety: bool,
        history: &PositionHistory,
        mut traces: SearchTraces<'_>,
    ) -> SearchInfo {
        // Root ordering belongs to one iterative-deepening invocation. The
        // state is thread-local for a cheap hot path, but Rayon may schedule a
        // different logical Lazy SMP worker on the same thread on the next
        // `go`. Clear it here so ordering cannot leak across searches, engine
        // instances, or logical workers.
        reset_root_order_state();
        if p::HIST_KEEP() == 0 {
            self.history.clear();
            self.countermoves.clear();
        } else if p::HIST_AGE() < 16 {
            self.history.age(p::HIST_AGE());
        }
        let state = Arc::new(SearchState {
            tt: self.tt.clone(),
            budget: Arc::new(Budget::new(
                config.time_limit,
                config.node_limit,
                self.external_abort.clone(),
            )),
            killers: KillerTable::new(),
            history: self.history.clone(),
            countermoves: self.countermoves.clone(),
            diagnostics: self.diagnostics.clone(),
            eval_cache: self.eval_cache.clone(),
            pruning: self.pruning,
            retirement: self.retirement.clone(),
        });

        let mut best_move = None;
        let mut best_score = NEG_INF;
        let mut done_depth = 0;
        let mut prev_best: Option<Move> = None;
        let mut bound = SearchBound::Unknown;
        let mut best_pv = Vec::new();
        let mut root_mate_safety_cache = RootMateSafetyCache::default();
        let history = &SearchHistory::root(history);
        // TM_MODE 1: completed iterations with an unchanged best move, and
        // the previous iteration's score.
        let mut stable_iters = 0u32;
        let mut prev_score: Option<i32> = None;
        // Capture the invocation token once. A helper from an earlier `go`
        // retains its old token and therefore cannot publish into this one.
        let root_hint_generation = self
            .root_hint
            .as_ref()
            .map(|endpoint| endpoint.channel.current_generation());

        // This history belongs to one go and one worker. MultiPV and fixed-root
        // diagnostics retain their existing aspiration policy.
        let adaptive_aspiration =
            p::ASP_VOLATILITY() != 0 && config.multi_pv.max(1) == 1 && root_move.is_none();
        let mut score_changes = aspiration::ScoreChanges::default();
        let mut retirement_previous = None;
        let mut contribution_previous = [0u64; 2];
        for iteration in 1..=config.max_depth {
            let depth = iteration + self.depth_skew;
            if depth > config.max_depth {
                break;
            }
            let requested_lines = if root_move.is_some() || traces.multi_pv.is_none() {
                1
            } else {
                config.multi_pv.max(1)
            };
            let mut excluded = Vec::new();
            let mut candidates = Vec::new();
            let mut primary = None;
            for rank in 1..=requested_lines {
                let root_hint = self.root_hint.as_ref().and_then(|endpoint| {
                    (endpoint.worker == 0
                        && rank == 1
                        && config.multi_pv.max(1) == 1
                        && root_move.is_none())
                    .then_some(())
                    .and_then(|()| {
                        endpoint
                            .channel
                            .move_at_least(root_hint_generation.unwrap_or(0), depth)
                            .map(|mv| RootHintOrder {
                                mv,
                                generation: root_hint_generation.unwrap_or(0),
                                channel: endpoint.channel.clone(),
                            })
                    })
                });
                // Keep eligibility even if the first helper publication arrives on retry.
                let root_hint_context = if p::ROOT_HINT_RETRY() != 0 {
                    root_hint_retry::Context::eligible(
                        self.root_hint.as_ref(),
                        root_hint_generation.unwrap_or(0),
                        rank,
                        config.multi_pv.max(1),
                        root_move,
                    )
                } else {
                    None
                };
                let candidate = root_search_with_window(
                    &state,
                    board,
                    depth,
                    best_score,
                    &excluded,
                    root_move,
                    root_mate_safety,
                    history,
                    self.root_order_offset,
                    self.full_root_lmr,
                    Some(&mut root_mate_safety_cache),
                    root_hint,
                    adaptive_aspiration.then(|| score_changes.initial_delta(p::ASP_DELTA())),
                    root_hint_context,
                );
                if rank == 1 {
                    primary = Some(candidate);
                }
                if state.budget.should_abort() {
                    break;
                }
                let Some(root_move) = candidate.0 else {
                    break;
                };
                let pv = extract_pv(&self.tt, board, Some(root_move), depth);
                candidates.push(MultiPvCandidate {
                    rank,
                    root_move,
                    score: candidate.1,
                    bound: candidate.2,
                    pv,
                });
                excluded.push(root_move);
            }

            if state.budget.should_abort() {
                // The interrupted iteration only reports a move whose search
                // at this depth completed inside the window (the previous
                // best failed low or was beaten), so it is the better choice.
                let interrupted_move = primary.and_then(|candidate| candidate.0);
                if depth > 1 && interrupted_move.is_some() {
                    best_move = interrupted_move;
                }
                break;
            }

            let Some((m, score, root_bound)) = primary else {
                break;
            };
            if let Some(trace) = traces.multi_pv.as_deref_mut()
                && !candidates.is_empty()
            {
                trace.push(MultiPvIteration {
                    depth,
                    nodes: state.budget.nodes(),
                    candidates: candidates.clone(),
                });
            }

            if adaptive_aspiration {
                score_changes.completed(score, root_bound);
            }
            best_move = m.or(best_move);
            best_score = score;
            done_depth = depth;
            bound = root_bound;
            if let Some(candidate) = candidates.first() {
                best_pv.clone_from(&candidate.pv);
            }
            if root_bound == SearchBound::Exact
                && config.multi_pv.max(1) == 1
                && root_move.is_none()
                && let (Some(endpoint), Some(completed_move), Some(generation)) =
                    (&self.root_hint, m, root_hint_generation)
                && endpoint.worker > 0
            {
                endpoint
                    .channel
                    .publish(generation, depth, endpoint.worker, completed_move);
            }

            let retire = if root_bound == SearchBound::Exact {
                if let (Some(endpoint), Some(mv)) = (&self.retirement, best_move) {
                    let stable = retirement_previous == Some(mv);
                    retirement_previous = Some(mv);
                    let current = endpoint.channel.contribution(endpoint.worker);
                    let recent = current.saturating_sub(contribution_previous[0]);
                    contribution_previous = [contribution_previous[1], current];
                    endpoint
                        .channel
                        .completed(endpoint.worker, depth, mv, stable, recent)
                } else {
                    false
                }
            } else {
                false
            };

            if let Some(trace) = traces.iterations.as_deref_mut() {
                let diagnostics = state
                    .diagnostics
                    .as_ref()
                    .map(|observer| observer.snapshot());
                let pv = candidates
                    .first()
                    .map(|candidate| candidate.pv.clone())
                    .unwrap_or_else(|| extract_pv(&self.tt, board, best_move, depth));
                trace.push(SearchIteration {
                    depth,
                    best_move,
                    score,
                    nodes: state.budget.nodes(),
                    bound: root_bound,
                    pv,
                    root_mate_in_one_nodes: diagnostics
                        .map(|snapshot| snapshot.root_mate_in_one_nodes),
                    root_mate_blunder_nodes: diagnostics
                        .map(|snapshot| snapshot.root_mate_blunder_nodes),
                });
            }

            if score.abs() >= MATE_SCORE - 1000 {
                break;
            }

            if retire {
                break;
            }
            if best_move == prev_best {
                stable_iters += 1;
            } else {
                stable_iters = 0;
            }
            let stop = if p::TM_MODE() == 0 {
                soft_limit_expired(
                    &state.budget,
                    config.soft_limit,
                    depth,
                    best_move == prev_best,
                )
            } else {
                tm_scaled_soft_expired(
                    state.budget.elapsed(),
                    config.soft_limit,
                    depth,
                    stable_iters,
                    prev_score.map_or(0, |ps| ps - score),
                )
            };
            if stop {
                break;
            }
            prev_best = best_move;
            prev_score = Some(score);
        }

        // Even if the hard deadline fires before depth 1 completes, return a
        // legal move whenever the position has one. This keeps a slow or
        // heavily contended environment from producing an invalid bestmove.
        if best_move.is_none() {
            let legal = MoveBuffer::legal(board);
            best_move = root_move
                .filter(|candidate| legal.as_slice().contains(candidate))
                .or_else(|| legal.as_slice().first().copied());
            if best_move.is_some() {
                best_score = evaluate_for_search(&state, board);
            }
        }
        let pv = if best_pv.is_empty() {
            extract_pv(&self.tt, board, best_move, done_depth)
        } else {
            best_pv
        };

        let aborted = state.budget.should_abort();
        let completed_bound = bound;
        let bound = if aborted { SearchBound::Unknown } else { bound };
        SearchInfo {
            best_move,
            score: best_score,
            depth: done_depth,
            nodes: state.budget.nodes(),
            elapsed: state.budget.elapsed(),
            hashfull: self.tt.hashfull(),
            bound,
            completed_bound,
            pv,
            aborted,
            abort_reason: state.budget.abort_reason(),
        }
    }
}

/// Reconstruct a conservative PV without treating non-exact TT bounds as a line.
fn extract_pv(tt: &Tt, board: &mut Board, first: Option<Move>, depth: u32) -> Vec<Move> {
    let mut line = Vec::new();
    let mut tokens = Vec::new();
    let mut next = first;
    let max_len = depth as usize;
    while line.len() < max_len {
        let mv = match next {
            Some(mv) => mv,
            None => break,
        };
        let legal = MoveBuffer::legal(board);
        if !legal.as_slice().contains(&mv) {
            break;
        }
        tokens.push(board.do_move(mv));
        line.push(mv);
        next = tt
            .probe(board.hash())
            .filter(|entry| entry.bound == Bound::Exact)
            .and_then(|entry| entry.mv);
    }
    for token in tokens.into_iter().rev() {
        board.undo_move(token);
    }
    line
}

// ============================================================
// Root search with Aspiration Window
// ============================================================

// Test-only mechanism observations; absent from every benchmark executable.
#[cfg(test)]
#[derive(Clone, Copy, Debug, Default)]
struct RepetitionFilterStats {
    queries: u64,
    fast_misses: u64,
    exact_scans: u64,
    visited_frames: u64,
}

#[cfg(test)]
thread_local! {
    static REPETITION_FILTER_STATS: std::cell::Cell<RepetitionFilterStats> =
        const { std::cell::Cell::new(RepetitionFilterStats {
            queries: 0, fast_misses: 0, exact_scans: 0, visited_frames: 0,
        }) };
}

#[cfg(test)]
fn record_repetition_filter(update: impl FnOnce(&mut RepetitionFilterStats)) {
    REPETITION_FILTER_STATS.with(|cell| {
        let mut stats = cell.get();
        update(&mut stats);
        cell.set(stats);
    });
}

/// Repetition history as seen from one search node: the game history
/// supplied with the position, followed by the moves of the current line.
///
/// Each frame lives on the stack of the node that made the move and points to
/// its parent, so a move neither allocates nor copies the game history (the
/// owned `PositionHistory::after_move` did both at every node, and its cost
/// grew with the game length). The entry sequence, and so every repetition
/// verdict, is the same as with the owned history.
#[derive(Clone, Copy)]
struct SearchHistory<'a> {
    game: &'a PositionHistory,
    parent: Option<&'a SearchHistory<'a>>,
    /// Bloom filter of hashes in the search line before this frame. A miss is
    /// exact and avoids walking the parent chain; a collision falls back to
    /// the full comparison below.
    prior_hashes: u64,
    /// The move that produced this node; `None` for the root, whose position
    /// is the last game entry.
    entry: Option<PositionHistoryEntry>,
}

impl<'a> SearchHistory<'a> {
    fn root(game: &'a PositionHistory) -> Self {
        SearchHistory {
            game,
            parent: None,
            prior_hashes: 0,
            entry: None,
        }
    }

    #[inline(always)]
    fn hash_bits(hash: u64) -> u64 {
        let first = 1u64 << (hash >> 58);
        // Both indices are fixed for the executable. A runtime switch could
        // require a bit that an already-created ancestor never inserted.
        #[cfg(feature = "repetition_filter2")]
        {
            first | (1u64 << ((hash >> 52) & 63))
        }
        #[cfg(not(feature = "repetition_filter2"))]
        {
            first
        }
    }

    #[inline]
    fn after_move(&'a self, hash: u64, mover: Color, gave_check: bool) -> SearchHistory<'a> {
        SearchHistory {
            game: self.game,
            parent: Some(self),
            prior_hashes: self.prior_hashes
                | self.entry.map_or(0, |entry| Self::hash_bits(entry.hash)),
            entry: Some(PositionHistoryEntry {
                hash,
                mover: Some(mover),
                gave_check,
            }),
        }
    }

    /// Same verdict as `PositionHistory::outcome_at_current_position` on the
    /// game history extended by the line.
    fn outcome_at_current_position(&self) -> Option<RepetitionOutcome> {
        let Some(current) = self.entry else {
            return self.game.outcome_at_current_position();
        };
        let mut count = self.game.count_hash(current.hash);
        // The current frame contributes one occurrence. With fewer than
        // three occurrences in game history and no possible prior line match,
        // fourfold repetition is impossible.
        #[cfg(test)]
        record_repetition_filter(|stats| stats.queries += 1);
        let bits = Self::hash_bits(current.hash);
        if count < 3 && self.prior_hashes & bits != bits {
            #[cfg(test)]
            record_repetition_filter(|stats| stats.fast_misses += 1);
            return None;
        }
        #[cfg(test)]
        record_repetition_filter(|stats| stats.exact_scans += 1);
        let mut frame = Some(self);
        while let Some(f) = frame {
            #[cfg(test)]
            record_repetition_filter(|stats| stats.visited_frames += 1);
            if f.entry.is_some_and(|e| e.hash == current.hash) {
                count += 1;
            }
            frame = f.parent;
        }
        if count < 4 {
            return None;
        }
        let mut line = Vec::new();
        let mut frame = Some(self);
        while let Some(f) = frame {
            line.extend(f.entry);
            frame = f.parent;
        }
        let mut full = self.game.clone();
        for e in line.into_iter().rev() {
            full.push_after_move(
                e.hash,
                e.mover.expect("search moves record their mover"),
                e.gave_check,
            );
        }
        full.outcome_at_current_position()
    }
}

/// Play `m` for the search: the undo token, whether the child is in check,
/// and the child's history frame.
#[inline(always)]
fn play<'a>(
    board: &mut Board,
    m: Move,
    history: &'a SearchHistory<'a>,
) -> (MoveToken, bool, SearchHistory<'a>) {
    let mover = board.side_to_move;
    let tok = board.do_move_for_search(m);
    let in_check = is_in_check(board, board.side_to_move);
    let child = history.after_move(board.hash(), mover, in_check);
    (tok, in_check, child)
}

#[allow(clippy::too_many_arguments)]
fn root_search(
    state: &Arc<SearchState>,
    board: &mut Board,
    depth: u32,
    prev_score: i32,
    excluded: &[Move],
    root_move: Option<Move>,
    root_mate_safety: bool,
    history: &SearchHistory<'_>,
    root_order_offset: usize,
    full_root_lmr: bool,
    root_mate_safety_cache: Option<&mut RootMateSafetyCache>,
    root_hint: Option<RootHintOrder>,
) -> (Option<Move>, i32, SearchBound) {
    let root_hint_context = if p::ROOT_HINT_RETRY() != 0 && root_move.is_none() {
        root_hint.as_ref().map(root_hint_retry::Context::from_hint)
    } else {
        None
    };
    root_search_with_window(
        state,
        board,
        depth,
        prev_score,
        excluded,
        root_move,
        root_mate_safety,
        history,
        root_order_offset,
        full_root_lmr,
        root_mate_safety_cache,
        root_hint,
        None,
        root_hint_context,
    )
}

#[allow(clippy::too_many_arguments)]
fn root_search_with_window(
    state: &Arc<SearchState>,
    board: &mut Board,
    depth: u32,
    prev_score: i32,
    excluded: &[Move],
    root_move: Option<Move>,
    root_mate_safety: bool,
    history: &SearchHistory<'_>,
    root_order_offset: usize,
    full_root_lmr: bool,
    root_mate_safety_cache: Option<&mut RootMateSafetyCache>,
    root_hint: Option<RootHintOrder>,
    initial_delta: Option<i32>,
    root_hint_context: Option<root_hint_retry::Context>,
) -> (Option<Move>, i32, SearchBound) {
    if let Some(outcome) = history.outcome_at_current_position() {
        return (
            None,
            repetition_score(outcome, board.side_to_move, 0),
            SearchBound::Exact,
        );
    }
    let mut move_buffer = {
        let _timer = state.timer(|d| &d.movegen_order_ns);
        let mut move_buffer = {
            let _generate_timer = state.timer(|d| &d.movegen_generate_ns);
            MoveBuffer::legal(board)
        };
        let moves = move_buffer.as_mut_list();
        if !excluded.is_empty() {
            moves.retain(|m| !excluded.contains(m));
        }
        if let Some(root_move) = root_move {
            moves.retain(|m| *m == root_move);
        }
        if moves.is_empty() {
            let score = if is_in_check(board, board.side_to_move) {
                -MATE_SCORE
            } else {
                0
            };
            return (None, score, SearchBound::Exact);
        }
        move_buffer
    };

    // A forced root move still needs a real child search: the child may be a
    // history-dependent fourth occurrence, which a static evaluation cannot
    // adjudicate correctly.
    if move_buffer.len() == 1 && root_move.is_none() {
        let only_move = move_buffer.as_slice()[0];
        let mover = board.side_to_move;
        set_current_move(0, Some(only_move));
        let tok = board.do_move(only_move);
        let child_in_check = is_in_check(board, board.side_to_move);
        let child_history = history.after_move(board.hash(), mover, child_in_check);
        set_cut(1, false);
        let score = -alpha_beta(
            state,
            board,
            NEG_INF,
            POS_INF,
            depth.saturating_sub(1),
            1,
            true,
            Some(only_move),
            None,
            Some(child_in_check),
            &child_history,
        );
        board.undo_move(tok);
        return (Some(only_move), score, SearchBound::Exact);
    }

    let tt_mv = probe_tt_for_search(state, board.hash()).and_then(|e| e.mv);
    let killers = state.killers.get(0);
    {
        let _timer = state.timer(|d| &d.movegen_order_ns);
        let _order_timer = state.timer(|d| &d.move_order_ns);
        order_moves_in_place(
            board,
            move_buffer.as_mut_list().as_mut_slice(),
            tt_mv,
            killers,
            None,
            ContMoves::default(),
            &state.history,
            board.side_to_move,
            state.diagnostics.as_deref(),
        );
    }
    let v2_root = p::SEARCH_V2() != 0 && p::V2_ROOT() != 0;
    if v2_root {
        v2::order_root(
            state,
            board,
            move_buffer.as_mut_list().as_mut_slice(),
            tt_mv,
        );
    }
    let ordered = move_buffer.as_slice();

    let safe_moves = if root_mate_safety {
        match root_mate_safety_filter(state, board, ordered, depth, root_mate_safety_cache) {
            RootSafety::Mate(m) => return (Some(m), MATE_SCORE - 1, SearchBound::Exact),
            RootSafety::Aborted => return (None, 0, SearchBound::Unknown),
            RootSafety::Filtered(moves) => moves,
        }
    } else {
        None
    };
    let ordered: &[Move] = safe_moves.as_deref().unwrap_or(ordered);
    // ROOT_ORDER: the order the previous iterations of this search left
    // (the moves that raised alpha first, best first; the rest as before).
    let reordered;
    let mut reused_iteration_order = false;
    let ordered: &[Move] = if p::ROOT_ORDER() != 0 || v2_root {
        reordered = ROOT_ORDER_STATE.with(|cell| {
            let st = cell.borrow();
            if st.0 == board.hash()
                && st.1.len() == ordered.len()
                && st.1.iter().all(|m| ordered.contains(m))
            {
                reused_iteration_order = true;
                st.1.clone()
            } else {
                ordered.to_vec()
            }
        });
        &reordered
    } else {
        ordered
    };
    let diversified = diversified_root_order(ordered, root_order_offset, reused_iteration_order);
    let ordered: &[Move] = diversified.as_deref().unwrap_or(ordered);
    let hinted = apply_root_hint(ordered, root_hint.clone());
    let ordered: &[Move] = hinted.as_deref().unwrap_or(ordered);
    let mut active_hint = root_hint.filter(|_| hinted.is_some());
    let mut retry_order: Option<Vec<Move>> = None;
    // Aspiration window: start tight around prev_score; widen on fail
    let use_asp = depth >= 2 && prev_score.abs() < MATE_SCORE - 1000;
    let initial_delta = initial_delta.unwrap_or_else(p::ASP_DELTA);
    if let Some(observer) = state.counters() {
        observer
            .aspiration
            .window(depth, use_asp, initial_delta, p::ASP_DELTA());
    }
    let (lo, hi) = if use_asp {
        (prev_score - initial_delta, prev_score + initial_delta)
    } else {
        (NEG_INF, POS_INF)
    };

    aspiration::search_window(lo, hi, initial_delta, |attempt, lo, hi| {
        #[cfg(test)]
        root_hint_retry::test_observer::boundary(depth, attempt);
        if attempt > 0 && p::ROOT_HINT_RETRY() != 0 && root_hint_context.is_some() {
            let current_order = retry_order.as_deref().unwrap_or(ordered);
            if let Some((promoted, hint)) = root_hint_retry::refresh(
                root_hint_context.as_ref(),
                depth,
                current_order,
                true,
                state.budget.should_abort(),
            ) {
                retry_order = Some(promoted);
                active_hint = Some(hint);
            }
        }
        let ordered = retry_order.as_deref().unwrap_or(ordered);
        let used_hint = active_hint.as_ref();
        #[cfg(test)]
        root_hint_retry::test_observer::pass(
            depth,
            attempt,
            ordered,
            lo,
            hi,
            used_hint.map(|hint| hint.mv),
        );
        let before = state.budget.nodes();
        let (m, score) = root_search_inner_tracked(
            state,
            board,
            depth,
            ordered,
            lo,
            hi,
            history,
            full_root_lmr,
            used_hint.filter(|_| attempt == 0),
        );
        if let Some(observer) = state.counters() {
            observer.aspiration.pass(
                depth,
                attempt,
                state.budget.nodes().saturating_sub(before),
                lo,
                hi,
                score,
                state.budget.should_abort(),
            );
        }
        if attempt > 0
            && let Some(hint) = used_hint
        {
            let nodes = state.budget.nodes().saturating_sub(before);
            hint.channel.researches.increment(hint.generation);
            hint.channel.research_nodes.add(hint.generation, nodes);
            let current = RootHintChannel::decode(hint.channel.packed.load(Ordering::Acquire));
            if current.is_some_and(|(generation, _, _, mv)| {
                generation != hint.generation || mv != hint.mv
            }) {
                hint.channel
                    .stale_research_nodes
                    .add(hint.generation, nodes);
            }
        }
        (m, score, state.budget.should_abort())
    })
}

/// Move a legal, non-excluded helper hint to the front without disturbing the
/// relative order of any other root move. A hint absent from the final root
/// candidate list is stale or inapplicable and is ignored.
fn promote_root_hint(ordered: &[Move], hint: Option<Move>) -> Option<Vec<Move>> {
    let hint = hint?;
    let index = ordered.iter().position(|mv| *mv == hint)?;
    if index == 0 {
        return None;
    }
    let mut promoted = ordered.to_vec();
    let hint = promoted.remove(index);
    promoted.insert(0, hint);
    Some(promoted)
}

fn apply_root_hint(ordered: &[Move], hint: Option<RootHintOrder>) -> Option<Vec<Move>> {
    let promoted = promote_root_hint(ordered, hint.as_ref().map(|value| value.mv));
    if promoted.is_some()
        && let Some(hint) = hint
    {
        hint.channel.mark_applied(hint.generation);
    }
    promoted
}

/// ROOT_ORDER: the root position and its move order after the latest root
/// search of this thread.
type RootOrder = (u64, Vec<Move>);
thread_local! {
    static ROOT_ORDER_STATE: std::cell::RefCell<RootOrder> =
        const { std::cell::RefCell::new((0, Vec::new())) };
}

fn reset_root_order_state() {
    ROOT_ORDER_STATE.with(|cell| {
        let mut state = cell.borrow_mut();
        state.0 = 0;
        state.1.clear();
    });
}

fn diversified_root_order(
    ordered: &[Move],
    offset: usize,
    reused_iteration_order: bool,
) -> Option<Vec<Move>> {
    if reused_iteration_order || offset == 0 || ordered.len() <= 1 {
        return None;
    }
    let mut diversified = ordered.to_vec();
    let shift = offset % diversified.len();
    diversified.rotate_left(shift);
    Some(diversified)
}

#[inline]
fn root_lmr_reduction(depth: u32, move_index: usize, quiet: bool, full_reduction: bool) -> u32 {
    if depth < 3 || move_index < 3 || !quiet {
        return 0;
    }
    let base = lmr_base_reduction(depth, move_index + 1);
    let reduction = if full_reduction {
        base
    } else {
        base.saturating_sub(1)
    };
    reduction.min(depth - 2)
}

#[cfg(test)]
#[allow(clippy::too_many_arguments)]
fn root_search_inner(
    state: &Arc<SearchState>,
    board: &mut Board,
    depth: u32,
    ordered: &[Move],
    lo: i32,
    hi: i32,
    history: &SearchHistory<'_>,
    full_root_lmr: bool,
) -> (Option<Move>, i32) {
    root_search_inner_tracked(
        state,
        board,
        depth,
        ordered,
        lo,
        hi,
        history,
        full_root_lmr,
        None,
    )
}

#[allow(clippy::too_many_arguments)]
fn root_search_inner_tracked(
    state: &Arc<SearchState>,
    board: &mut Board,
    depth: u32,
    ordered: &[Move],
    lo: i32,
    hi: i32,
    history: &SearchHistory<'_>,
    full_root_lmr: bool,
    hint: Option<&RootHintOrder>,
) -> (Option<Move>, i32) {
    if root_split::enabled() {
        return root_split::search(state, board, depth, ordered, lo, hi, history);
    }
    let mut best_move = None;
    let mut alpha = lo;
    // Moves that raised alpha, with their scores (ROOT_ORDER).
    let mut raised: Vec<(Move, i32)> = Vec::new();
    let v2_root = p::SEARCH_V2() != 0 && p::V2_ROOT() != 0;
    // Completed root moves are a prefix: the loop visits ordered moves
    // without skipping, and records completion only after the abort check.
    // Borrow that prefix for root learning instead of allocating a copy.
    let mut searched_len = 0;

    for (i, &m) in ordered.iter().enumerate() {
        let mover = board.side_to_move;
        // Late quiet root moves get a reduced null-window probe first
        // (ROOT_LMR 1: every late move).
        let quiet = p::ROOT_LMR() != 0 || (board.piece_at(m.to).is_none() && !m.promote);
        let reduce = root_lmr_reduction(depth, i, quiet, full_root_lmr);
        set_current_move(0, Some(m));
        let tok = board.do_move(m);
        let child_in_check = is_in_check(board, board.side_to_move);
        let child_history = history.after_move(board.hash(), mover, child_in_check);
        let search = |board: &mut Board, a: i32, b: i32, d: u32| {
            // Null-window root children are expected cut nodes.
            set_cut(1, b - a == 1);
            -alpha_beta(
                state,
                board,
                -b,
                -a,
                d,
                1,
                true,
                Some(m),
                None,
                Some(child_in_check),
                &child_history,
            )
        };
        // Principal variation search at the root: the first move gets the
        // full window; later moves a null-window probe, widened only when
        // they may raise alpha.
        let score = if i == 0 {
            search(board, alpha, hi, depth - 1)
        } else {
            let reduce = if child_in_check && p::ROOT_LMR() == 0 {
                0
            } else {
                reduce
            };
            let mut s = search(board, alpha, alpha + 1, depth - 1 - reduce);
            if reduce > 0 && s > alpha {
                s = search(board, alpha, alpha + 1, depth - 1);
            }
            if s > alpha && s < hi {
                s = search(board, alpha, hi, depth - 1);
            }
            s
        };
        board.undo_move(tok);

        if state.budget.should_abort() {
            break;
        }
        if v2_root {
            searched_len = i + 1;
        }

        if score > alpha {
            alpha = score;
            best_move = Some(m);
            raised.push((m, score));
        }
        if alpha >= hi {
            if i == 0
                && let Some(hint) = hint
            {
                hint.channel.first_move_beta_cuts.increment(hint.generation);
            }
            break;
        }
    }
    if v2_root
        && !state.budget.should_abort()
        && let Some(b) = best_move
    {
        v2::root_learn(state, board, b, &ordered[..searched_len], depth);
    }
    if (p::ROOT_ORDER() != 0 || v2_root) && !state.budget.should_abort() {
        raised.sort_by_key(|&(_, s)| std::cmp::Reverse(s));
        let mut order: Vec<Move> = raised.iter().map(|&(m, _)| m).collect();
        order.extend(
            ordered
                .iter()
                .copied()
                .filter(|m| !raised.iter().any(|(r, _)| r == m)),
        );
        let hash = board.hash();
        ROOT_ORDER_STATE.with(|cell| *cell.borrow_mut() = (hash, order));
    }

    // An abort means not all root moves were searched; do not publish a
    // partial result as an exact/lower-bound entry for the full root.
    if !state.budget.should_abort()
        && let Some(m) = best_move
    {
        let bound = if alpha >= hi {
            Bound::Lower // fail-high: true score ≥ alpha, exact unknown
        } else {
            Bound::Exact
        };
        store_tt_for_search(
            state,
            board.hash(),
            TtEntry {
                score: score_to_tt(alpha, 0), // ply=0 at root
                depth: crate::tt::depth_to_tt(depth),
                bound,
                mv: Some(m),
            },
        );
    }

    (best_move, alpha)
}

// ============================================================
// Core Alpha-Beta with YBW parallelism
// ============================================================

/// Convert a history-dependent fourfold repetition into a score from the
/// current side-to-move's perspective.  This deliberately runs before any TT
/// access: the same Zobrist position can be a draw, a perpetual-check loss, or
/// an ordinary position depending on the played history.
fn repetition_score(outcome: RepetitionOutcome, side_to_move: Color, ply: u32) -> i32 {
    match outcome {
        RepetitionOutcome::Draw => 0,
        RepetitionOutcome::PerpetualCheck(loser) if loser == side_to_move => {
            -(MATE_SCORE - ply as i32)
        }
        RepetitionOutcome::PerpetualCheck(_) => MATE_SCORE - ply as i32,
    }
}

#[allow(clippy::too_many_arguments)]
fn alpha_beta(
    state: &Arc<SearchState>,
    board: &mut Board,
    mut alpha: i32,
    beta: i32,
    depth: u32,
    ply: u32,
    can_null: bool,
    prev_mv: Option<Move>, // the move that led to this position (for countermove heuristic)
    skip_move: Option<Move>, // excluded move for singular extension search (None normally)
    known_in_check: Option<bool>, // supplied by a parent that already tested the moved position
    history: &SearchHistory<'_>,
) -> i32 {
    // SEARCH_V2: the rest of the tree is searched by `v2` (its own null
    // moves, singular searches and histories).
    if p::SEARCH_V2() != 0 && skip_move.is_none() && ply > 0 {
        return v2::entry(
            state,
            board,
            alpha,
            beta,
            depth,
            ply,
            known_in_check,
            history,
        );
    }
    if let Some(diagnostics) = state.counters() {
        diagnostics.alpha_beta_calls.fetch_add(1, Ordering::Relaxed);
        diagnostics.record_work_sample(board.hash());
        if beta - alpha > 1 {
            diagnostics.pv_calls.fetch_add(1, Ordering::Relaxed);
        }
    }
    if let Some(outcome) = history.outcome_at_current_position() {
        return repetition_score(outcome, board.side_to_move, ply);
    }
    // A leaf is counted once, by the quiescence search it turns into.
    if depth == 0 {
        if let Some(d) = state.counters() {
            d.exit(0);
        }
        if state.budget.should_abort() {
            return 0;
        }
        let alpha = alpha.max(-(MATE_SCORE - ply as i32));
        let beta = beta.min(MATE_SCORE - ply as i32);
        if alpha >= beta {
            return alpha;
        }
        return quiescence(state, board, alpha, beta, ply, 0, known_in_check, history);
    }
    if state.budget.tick() {
        return 0;
    }
    let entry_depth = depth;
    if let Some(d) = state.counters() {
        d.at_depth(entry_depth, 0);
    }
    // Expected cut node: a null-window node its parent expects to fail high.
    let cut_node = beta - alpha == 1 && cut_at(ply);

    // Mate distance pruning: tighten window — we can't improve beyond the nearest mate
    alpha = alpha.max(-(MATE_SCORE - ply as i32));
    let beta = beta.min(MATE_SCORE - ply as i32);
    if alpha >= beta {
        return alpha;
    }

    // TT probe
    let hash = board.hash();
    let orig_alpha = alpha;
    let pv_window = beta - alpha > 1;
    let tt = TtProbe::at(state, hash, depth, ply, alpha, beta, skip_move.is_some());
    if let Some(d) = state.counters() {
        d.record_tt_probe(depth, tt.hit, tt.se_depth);
    }
    if let Some(score) = tt.cutoff {
        if let Some(d) = state.counters() {
            d.exit(1);
            d.at_depth(entry_depth, 1);
        }
        return score;
    }
    // A PV node, or one whose entry was stored on a PV (TT_PV_MODE).
    let tt_pv = pv_window || (tt.hit && tt.pv);
    alpha = tt.alpha;
    let tt_mv = tt.mv;
    let tt_se_score = tt.se_score;
    let tt_se_depth = tt.se_depth;

    // Internal Iterative Reduction: no TT move → move ordering is poor, search shallower.
    // IIR_MODE limits it to PV and expected cut nodes (1), PV nodes (2) or
    // turns it off (3): a reduced node stores a shallower entry, which a
    // revisit at the full depth cannot cut on.
    let iir = tt_mv.is_none()
        && depth >= p::IIR_MIN_DEPTH() as u32
        && match p::IIR_MODE() {
            0 => true,
            1 => beta - alpha > 1 || cut_node,
            2 => beta - alpha > 1,
            _ => false,
        };
    let depth = if iir { depth - 1 } else { depth };
    let stm = board.side_to_move;
    let mut node = NodeKey::new(board, hash, depth, ply, skip_move, tt_pv);

    // Countermove: best quiet response to the opponent's previous move
    let countermove = prev_mv.and_then(|pm| state.countermoves.get(stm.flip(), pm));

    let in_check = known_in_check.unwrap_or_else(|| is_in_check(board, stm));
    // A mate in one for the side to move ends the node. Checking it here also
    // makes null-move and reduced searches see the opponent's mating threats.
    // A mate-in-one result is stored as an exact entry below, so a position
    // with a TT hit does not need to repeat this scan.
    if !in_check
        && skip_move.is_none()
        && !tt.hit
        && let Some(mate_move) = mate_in_one(board)
    {
        if let Some(d) = state.counters() {
            d.exit(2);
            d.at_depth(entry_depth, 6);
        }
        let score = MATE_SCORE - (ply as i32 + 1);
        node.store(state, score, Bound::Exact, Some(mate_move));
        return score;
    }
    let evaluation = prepare_node_evaluation(
        state,
        board,
        stm,
        depth,
        ply,
        in_check,
        tt.bound_score,
        &mut node,
    );
    let static_eval = evaluation.value;
    let improving = evaluation.improving;
    if let Some(c) = state.counters()
        && skip_move.is_none()
    {
        c.record_v2_node(
            depth,
            tt_pv,
            static_eval.is_some_and(|se| se >= beta),
            in_check,
        );
    }
    let doubles = doubles_at(ply);
    set_doubles(ply + 1, doubles);

    if let Some(score) = reverse_futility_score(
        board,
        static_eval,
        improving,
        depth,
        alpha,
        beta,
        tt_pv,
        tt_mv,
    ) {
        if let Some(d) = state.counters() {
            d.exit(3);
            d.at_depth(entry_depth, 3);
        }
        return score;
    }

    if let Some(score) = razoring_score(static_eval, depth, alpha, beta, |lo, hi| {
        quiescence(state, board, lo, hi, ply, 0, Some(in_check), history)
    }) {
        if let Some(d) = state.counters() {
            d.exit(4);
            d.at_depth(entry_depth, 2);
        }
        return score;
    }

    // ProbCut (not inside a singular search).
    if depth >= p::PC_MIN_DEPTH() as u32
        && !in_check
        && beta.abs() < MATE_SCORE - 1000
        && skip_move.is_none()
        && let Some(score) = probcut(state, board, beta, depth, ply, cut_node, history)
    {
        if let Some(d) = state.counters() {
            d.exit(5);
            d.at_depth(entry_depth, 5);
        }
        return score;
    }

    // Null move pruning.
    if state.pruning.null_move
        && can_null
        && depth > p::NMP_R() as u32
        && beta.abs() < MATE_SCORE - 1000
        && !in_check
        && nmp_eval_allows(static_eval, beta)
        && let Some(score) = null_move_pruning(
            state,
            board,
            beta,
            depth,
            ply,
            prev_mv,
            cut_node,
            history,
            static_eval,
        )
    {
        if let Some(d) = state.counters() {
            d.exit(6);
            d.at_depth(entry_depth, 4);
        }
        return score;
    }

    let killers = state.killers.get(ply as usize);
    let move_context = MoveListContext {
        depth,
        ply,
        previous_move: prev_mv,
        excluded_move: skip_move,
        in_check,
        tt_move: tt_mv,
        killers,
        countermove,
        side_to_move: stm,
    };
    let mut moves = match NodeMoveList::prepare(state, board, move_context) {
        MovePreparation::Mated => return -(MATE_SCORE - ply as i32),
        MovePreparation::ExcludedAll => return alpha,
        MovePreparation::Ready(moves) => moves,
    };
    let ordered = moves.as_slice();

    let is_pv = beta - alpha > 1;

    let singular = SingularContext {
        alpha,
        beta,
        depth,
        ply,
        prev_move: prev_mv,
        excluded_move: skip_move,
        in_check,
        tt_move: tt_mv,
        tt_score: tt_se_score,
        tt_depth: tt_se_depth,
        pv: is_pv,
        cut: cut_node,
        double_extensions: doubles,
    };
    let sing_ext = match singular.probe(state, board, history) {
        SingularOutcome::Extension(extension) => extension,
        SingularOutcome::Cutoff(score) => return score,
    };

    // Quiet moves tried so far — used to apply history malus on beta cutoff.
    let enemy = board.occ_for(stm.flip());
    let mut tried_quiet: Vec<Move> = Vec::new();
    let mut tried_captures: Vec<Move> = Vec::new();
    let lmr_ctx = LmrContext {
        pv: is_pv,
        cut: cut_node,
        improving,
        cont: ContMoves::at(ply, prev_mv),
        // TT_PV_MODE 1: less reduction at non-PV nodes stored on a PV;
        // 4: at PV nodes too.
        tt_pv: tt_pv
            && match p::TT_PV_MODE() & 5 {
                0 => false,
                1 => !is_pv,
                _ => true,
            },
        tt_capture: tt_mv.is_some_and(|t| t.from.is_some() && enemy.contains(t.to)),
        enemy,
    };

    // ---------- First child: always sequential ----------
    let first_move = ordered[0];
    if let Some(d) = state.counters() {
        d.loop_nodes[usize::from(tt_mv.is_some_and(|t| t == first_move))]
            .fetch_add(1, Ordering::Relaxed);
        d.loop_by_window[usize::from(is_pv)].fetch_add(1, Ordering::Relaxed);
        d.searched_by_window[usize::from(is_pv)].fetch_add(1, Ordering::Relaxed);
        d.at_depth(entry_depth, 7);
        d.at_depth(entry_depth, 8);
    }
    set_current_move(ply, Some(first_move));
    let (tok, child_in_check, first_history) = play(board, first_move, history);
    // Apply singular extension to the TT move (ordered[0] when tt_mv is set)
    let first_ext = if tt_mv.is_some_and(|t| t == first_move) {
        sing_ext
    } else {
        0
    };
    set_doubles(ply + 1, doubles + u8::from(first_ext >= 2));
    set_cut(ply + 1, !is_pv && !cut_node);
    let score0 = -alpha_beta(
        state,
        board,
        -beta,
        -alpha,
        (depth as i32 - 1 + first_ext).max(0) as u32,
        ply + 1,
        true,
        Some(first_move),
        None,
        Some(child_in_check),
        &first_history,
    );
    board.undo_move_for_search(tok);
    set_doubles(ply + 1, doubles);

    if state.budget.should_abort() {
        return 0;
    }

    let mut best_score = score0;
    let mut best_move = Some(first_move);

    if score0 >= beta {
        if let Some(d) = state.counters()
            && depth >= p::CUT_MIN_DEPTH() as u32
        {
            d.record_cut(
                1,
                !enemy.contains(first_move.to) && !first_move.promote,
                tt_mv.is_some_and(|t| t == first_move),
            );
            if let Some(t) = tt_mv
                && !enemy.contains(t.to)
                && !t.promote
            {
                d.record_tt_quiet_cut(
                    t == first_move,
                    tt.bound_score.map(|(b, _)| b),
                    depth as i32 - i32::from(tt.se_depth),
                );
            }
        }
        reward_capture(&state.history, board, stm, first_move, depth);
        update_quiet_heuristics(
            &state.killers,
            &state.history,
            &state.countermoves,
            first_move,
            stm,
            ply,
            depth,
            board,
            prev_mv,
        );
        node.store(state, score0, Bound::Lower, best_move);
        return score0;
    }
    if score0 > alpha {
        alpha = score0;
    }
    // The remaining moves of a node whose TT move is singular are searched
    // one ply deeper (SE_DEPTH_BUMP).
    let depth = if sing_ext > 0 && p::SE_DEPTH_BUMP() != 0 {
        depth + 1
    } else {
        depth
    };
    // Track first_move for malus if it didn't cut off
    if !enemy.contains(first_move.to) && !first_move.promote {
        tried_quiet.push(first_move);
    } else if enemy.contains(first_move.to) {
        tried_captures.push(first_move);
    }

    moves.order_after_first(state, board);
    let ordered = moves.as_slice();
    let rest = &ordered[1..];
    if rest.is_empty() {
        let bound = if best_score > orig_alpha {
            Bound::Exact
        } else {
            Bound::Upper
        };
        node.store(state, best_score, bound, best_move);
        return best_score;
    }

    // ---------- Young brothers ----------
    // Returns the index in `rest` where sequential processing should begin:
    // ybw_end after the parallel YBW pass, or 0 at shallow depths (no YBW).
    // With a single worker the young-brothers pass only delays cutoffs: it
    // probes every sibling before looking at any result. Search sequentially.
    let seq_start = if state.pruning.ybw_split
        && depth >= MIN_SPLIT_DEPTH
        && rayon::current_num_threads() > 1
    {
        let nw_abort = AtomicBool::new(false);
        let alpha_for_nw = alpha;

        // ponytail: limit parallel siblings; tail searched sequentially after YBW pass
        const YBW_MAX_SIBLINGS: usize = 6;
        let ybw_end = rest.len().min(YBW_MAX_SIBLINGS);

        // Null-window parallel probe (with LMR for quiet late moves)
        // Rayon joins before returning, so the closure can borrow state and the
        // abort flag directly. Clone only the worker's private Board; the old
        // staging Vec also cloned every Arc and allocated once per split.
        let path = stack_path(ply + 1);
        let nw_results: Vec<(Move, i32, usize)> = rest[..ybw_end]
            .par_iter()
            .enumerate()
            .filter_map(|(i, &m)| {
                if nw_abort.load(Ordering::Relaxed) || state.budget.should_abort() {
                    return None;
                }
                let idx = i + 1;
                let mut b = board.clone();
                let reduce = if state.pruning.late_move_reduction {
                    lmr_adjust(
                        lmr_reduce(&b, m, idx, depth, &killers, tt_mv, &state.history, stm),
                        &lmr_ctx,
                        &state.history,
                        stm,
                        m,
                    )
                } else {
                    0
                };
                let check_cap = check_reduction_cap(&b, m, reduce);
                restore_stack_path(&path);
                set_current_move(ply, Some(m));
                let (tok, child_in_check, child_history) = play(&mut b, m, history);
                let reduce = if child_in_check {
                    reduce.min(check_cap)
                } else {
                    reduce
                };
                set_cut(ply + 1, reduce > 0 || !cut_node);
                let probe_depth = depth.saturating_sub(1 + reduce);
                let s = -alpha_beta(
                    state,
                    &mut b,
                    -alpha_for_nw - 1,
                    -alpha_for_nw,
                    probe_depth,
                    ply + 1,
                    true,
                    Some(m),
                    None,
                    Some(child_in_check),
                    &child_history,
                );
                b.undo_move_for_search(tok);
                Some((m, s, idx))
            })
            .collect();

        // Sequential pass: handle fail-highs, update heuristics, apply history malus
        for (m, nw_score, _idx) in nw_results {
            if state.budget.should_abort() {
                break;
            }

            let is_quiet_ybw = !enemy.contains(m.to) && !m.promote;

            let s = if nw_score > alpha {
                // Fail-high: re-search at full depth with full window
                set_current_move(ply, Some(m));
                let (tok, child_in_check, child_history) = play(board, m, history);
                set_cut(ply + 1, false);
                let full = -alpha_beta(
                    state,
                    board,
                    -beta,
                    -alpha,
                    depth - 1,
                    ply + 1,
                    true,
                    Some(m),
                    None,
                    Some(child_in_check),
                    &child_history,
                );
                board.undo_move_for_search(tok);
                full
            } else {
                nw_score
            };

            if s > best_score {
                best_score = s;
                best_move = Some(m);
            }
            if s >= beta {
                nw_abort.store(true, Ordering::Relaxed);
                return node.beta_cutoff(
                    state,
                    best_score,
                    best_move,
                    &tried_quiet,
                    &tried_captures,
                    m,
                    stm,
                    board,
                    prev_mv,
                );
            }
            if s > alpha {
                alpha = s;
            }
            if is_quiet_ybw {
                tried_quiet.push(m);
            } else if enemy.contains(m.to) {
                tried_captures.push(m);
            }
        }
        ybw_end
    } else {
        0
    };

    // Sequential pass: remaining siblings (tail beyond YBW limit, or all at shallow depth).
    //
    // There is deliberately no late-move (move-count) pruning: shogi has many
    // quiet moves and drops, and in local self-play every move-count limit
    // lost strength while removing the last one (depth <= 2) gained.
    {
        let rest_len = rest.len();
        let mut next = seq_start;
        while next < rest_len {
            let i = next;
            next += 1;
            moves.order_lazy_tail_at(state, board, i + 1);
            let m = moves.as_slice()[i + 1];
            if state.budget.should_abort() {
                break;
            }

            let is_capture = m.from.is_some() && enemy.contains(m.to);
            // Drops are quiet too: they are the majority of shogi moves, and
            // excluding them exempted most late moves from futility pruning.
            let is_quiet = !is_capture && (!m.promote || promotion_counts_as_quiet(m));

            let reduce = if state.pruning.late_move_reduction {
                lmr_adjust(
                    lmr_reduce(board, m, i + 1, depth, &killers, tt_mv, &state.history, stm),
                    &lmr_ctx,
                    &state.history,
                    stm,
                    m,
                )
            } else {
                0
            };
            let late = LateMoveNode {
                depth,
                alpha,
                beta,
                in_check,
                static_eval,
                best_score,
                improving,
                history: if is_quiet && (p::HP_MAX_DEPTH() > 0 || p::PRUNE_HIST_DIV() > 0) {
                    lmr_ctx.history_of(&state.history, stm, m)
                } else {
                    0
                },
                lmr_depth: depth.saturating_sub(1 + reduce),
                cont_hist: if is_quiet && p::PRUNE_STYLE() == 1 {
                    lmr_ctx.history_of(&state.history, stm, m) - state.history.get(stm, m)
                } else {
                    0
                },
                main_hist: if p::PRUNE_STYLE() != 1 {
                    0
                } else if is_quiet {
                    state.history.get(stm, m)
                } else {
                    board
                        .piece_at(m.to)
                        .filter(|_| m.from.is_some())
                        .map_or(0, |v| state.history.capture_get(stm, m, v.kind))
                },
                alpha_raised: best_score > orig_alpha,
                pv: is_pv,
            };
            if late.prunes(board, m, is_quiet, i + 1) {
                // FUT_SOFT: a pruned quiet move still bounds the node from
                // above by its futility estimate, so the fail-low value
                // returned (and stored) is not lower than the pruned moves
                // could have scored.
                if p::FUT_SOFT() != 0
                    && is_quiet
                    && let Some(se) = static_eval
                    && best_score > -(MATE_SCORE - 1000)
                    && best_score < MATE_SCORE - 1000
                {
                    let est = if depth == 1 {
                        se + p::FUTILITY_MARGIN()
                    } else {
                        se + p::SHALLOW_FUTILITY_BASE()
                            + p::SHALLOW_FUTILITY_PER_DEPTH() * late.lmr_depth.max(1) as i32
                    };
                    best_score = best_score.max(est.min(alpha));
                }
                continue;
            }
            let check_cap = check_reduction_cap(board, m, reduce);
            if let Some(d) = state.counters() {
                d.searched_by_window[usize::from(is_pv)].fetch_add(1, Ordering::Relaxed);
                d.at_depth(entry_depth, 8);
            }
            set_current_move(ply, Some(m));
            let (tok, child_in_check, child_history) = play(board, m, history);
            if child_in_check && let Some(d) = state.counters() {
                d.searched_checks[usize::from(is_pv)].fetch_add(1, Ordering::Relaxed);
            }
            // Checks are not extended (every check extension variant lost
            // depth for nothing in shogi's check-rich trees); they are only
            // protected from reductions beyond `CHECK_R_MAX` plies.
            let reduce = if child_in_check {
                reduce.min(check_cap)
            } else {
                reduce
            };
            let ext = u32::from(
                p::LMR_EXT_HIST() > 0
                    && is_quiet
                    && depth >= 3
                    && lmr_ctx.history_of(&state.history, stm, m) > p::LMR_EXT_HIST(),
            );
            let s = pvs_child(
                state,
                board,
                (alpha, beta),
                depth,
                reduce,
                ext,
                ply,
                m,
                cut_node,
                child_in_check,
                &child_history,
                best_score,
            );
            board.undo_move_for_search(tok);

            if s > best_score {
                best_score = s;
                best_move = Some(m);
            }
            if s >= beta {
                if let Some(d) = state.counters()
                    && depth >= p::CUT_MIN_DEPTH() as u32
                {
                    d.record_cut(i + 2, is_quiet, moves.deferred_order());
                    if let Some(t) = tt_mv
                        && moves.deferred_order()
                        && !enemy.contains(t.to)
                        && !t.promote
                    {
                        d.record_tt_quiet_cut(
                            false,
                            tt.bound_score.map(|(b, _)| b),
                            depth as i32 - i32::from(tt.se_depth),
                        );
                    }
                }
                return node.beta_cutoff(
                    state,
                    best_score,
                    best_move,
                    &tried_quiet,
                    &tried_captures,
                    m,
                    stm,
                    board,
                    prev_mv,
                );
            }
            if s > alpha {
                alpha = s;
            }
            if is_quiet {
                tried_quiet.push(m);
            } else if is_capture {
                tried_captures.push(m);
            }
        }
    }

    let bound = if best_score > orig_alpha {
        Bound::Exact
    } else {
        Bound::Upper
    };
    if state.budget.should_abort() {
        return 0;
    }
    if bound == Bound::Upper
        && let Some(d) = state.counters()
    {
        d.fail_low_nodes.fetch_add(1, Ordering::Relaxed);
    }
    if skip_move.is_none() {
        learn_at_node_exit(
            state,
            board,
            bound,
            best_move,
            &tried_quiet,
            stm,
            ply,
            depth,
            prev_mv,
        );
    }
    // FL_MOVE 1: a node that failed low keeps the move already in the table
    // instead of storing the best of its failing moves.
    if bound == Bound::Upper && p::FL_MOVE() != 0 {
        node.store_without_move(state, best_score, bound, best_move);
    } else {
        node.store(state, best_score, bound, best_move);
    }
    best_score
}

/// History learning when a node ends without a cutoff (`HIST_EXACT`,
/// `HIST_PRIOR`).
#[allow(clippy::too_many_arguments)]
#[inline]
fn learn_at_node_exit(
    state: &SearchState,
    board: &Board,
    bound: Bound,
    best_move: Option<Move>,
    tried_quiet: &[Move],
    stm: Color,
    ply: u32,
    depth: u32,
    prev_mv: Option<Move>,
) {
    if bound == Bound::Exact
        && p::HIST_EXACT() != 0
        && let Some(bm) = best_move
        && board.piece_at(bm.to).is_none()
        && !bm.promote
    {
        let cont = ContMoves::at(ply, prev_mv);
        for &qm in tried_quiet.iter().filter(|&&qm| qm != bm) {
            state.history.malus(stm, qm, depth);
            cont.update(&state.history, stm, qm, -history_malus(depth));
        }
        update_quiet_heuristics(
            &state.killers,
            &state.history,
            &state.countermoves,
            bm,
            stm,
            ply,
            depth,
            board,
            prev_mv,
        );
    }
    let prior = p::HIST_PRIOR();
    if bound == Bound::Upper
        && prior > 0
        && depth >= 2
        && ply >= 1
        && let Some(pm) = prev_mv
        && !pm.promote
    {
        let bonus = history_bonus(depth) * prior / 16;
        state
            .history
            .apply(HistoryTable::idx(stm.flip(), pm), bonus);
        ContMoves::at(
            ply - 1,
            current_move_at(ply.saturating_sub(2)).filter(|_| ply >= 2),
        )
        .update(&state.history, stm.flip(), pm, bonus);
    }
}

// ============================================================
// Quiescence Search
// ============================================================

/// Resolve a position to quiescence before calling evaluate. Searches captures
/// always, all legal replies while in check, and (only at qply 0) a few safe
/// quiet checks. Bounded by QSEARCH_MAX_PLY so forcing-check lines can't recurse
/// without end.
#[allow(clippy::too_many_arguments)]
fn quiescence(
    state: &Arc<SearchState>,
    board: &mut Board,
    alpha: i32,
    beta: i32,
    ply: u32,
    qply: u32,
    known_in_check: Option<bool>,
    history: &SearchHistory<'_>,
) -> i32 {
    quiescence_with_pending(
        state,
        board,
        alpha,
        beta,
        ply,
        qply,
        known_in_check,
        history,
        None,
    )
}

#[cfg(test)]
thread_local! {
    static QUIET_HKP_QS_TT_SKIPS: std::cell::Cell<[u64; 2]> = const { std::cell::Cell::new([0; 2]) };
}

#[cfg(test)]
fn record_quiet_halfkp_qs_tt_skip(pending: Option<&crate::board::QuietHalfKpToken>, qs2: bool) {
    if pending.is_some_and(|token| !token.is_materialized_for_test()) {
        QUIET_HKP_QS_TT_SKIPS.with(|counter| {
            let mut counts = counter.get();
            counts[usize::from(qs2)] += 1;
            counter.set(counts);
        });
    }
}

#[cfg(all(test, feature = "tune"))]
fn reset_quiet_halfkp_qs_tt_skips() {
    QUIET_HKP_QS_TT_SKIPS.with(|counter| counter.set([0; 2]));
}

#[cfg(all(test, feature = "tune"))]
fn quiet_halfkp_qs_tt_skips() -> [u64; 2] {
    QUIET_HKP_QS_TT_SKIPS.with(std::cell::Cell::get)
}

/// Only the main move loop may lend a pending quiet edge. QS descendants
/// retain eager transitions and call the original wrapper with None.
#[allow(clippy::too_many_arguments)]
fn quiescence_with_pending(
    state: &Arc<SearchState>,
    board: &mut Board,
    mut alpha: i32,
    beta: i32,
    ply: u32,
    qply: u32,
    known_in_check: Option<bool>,
    history: &SearchHistory<'_>,
    pending: Option<&mut crate::board::QuietHalfKpToken>,
) -> i32 {
    let _quiescence_timer = state.timer(|d| &d.quiescence_inclusive_ns);
    if let Some(diagnostics) = state.counters() {
        diagnostics.quiescence_calls.fetch_add(1, Ordering::Relaxed);
        diagnostics.record_work_sample(board.hash());
        diagnostics.qsearch[0].fetch_add(1, Ordering::Relaxed);
        if qply == 0 {
            diagnostics.qsearch[10].fetch_add(1, Ordering::Relaxed);
        }
    }
    if let Some(outcome) = history.outcome_at_current_position() {
        return repetition_score(outcome, board.side_to_move, ply);
    }
    // Enforce the hard time limit here too: a heavy qsearch subtree (quiet checks
    // + recursive SEE) can run for many seconds without returning to alpha_beta,
    // which is the only other place that ticks the budget.
    if state.budget.tick() {
        return 0;
    }

    // Hard depth cap: terminate the quiescence even mid-check. Without this a
    // perpetual-check line recurses (in-check expands ALL legal replies below)
    // until the clock runs out — the move then blows past its byoyomi.
    const QSEARCH_MAX_PLY: u32 = 10;
    if qply >= QSEARCH_MAX_PLY {
        if let Some(diagnostics) = state.counters() {
            diagnostics.qsearch[7].fetch_add(1, Ordering::Relaxed);
        }
        // Legacy cap evaluates even in check; preserve that score rule.
        if let Some(token) = pending {
            board.materialize_quiet_halfkp_for_search(token);
        }
        return evaluate_for_search(state, board);
    }

    // A depth-zero TT entry represents only the top-level qsearch problem.
    // Recursive qsearch values depend on qply (quiet checks are expanded only
    // at qply 0), so they must not be reused as if they were interchangeable.
    // Main-search entries have depth >= 1 and are intentionally ignored here:
    // a shared speculative TT entry can have a different search window and
    // score semantics from this qsearch node.
    let hash = board.hash();
    let mut tt_mv = None;
    let qs_tt = p::QS_TT();
    let tt_here = qply == 0 || qs_tt & 1 != 0;
    let mut tt_bound: Option<(Bound, i32)> = None;
    let tt_entry = if tt_here || qs_tt & 4 != 0 {
        probe_tt_for_search(state, hash)
    } else {
        None
    };
    if qs_tt & 4 != 0
        && let Some(entry) = tt_entry
        && entry.depth > 0
    {
        tt_mv = entry.mv;
    }
    if tt_here
        && let Some(entry) = tt_entry
        && (entry.depth == 0 || qs_tt & 2 != 0)
    {
        let adj = score_from_tt(entry.score, ply);
        tt_mv = entry.mv;
        tt_bound = Some((entry.bound, adj));
        match entry.bound {
            Bound::Exact => {
                if let Some(diagnostics) = state.counters() {
                    diagnostics.qsearch[2].fetch_add(1, Ordering::Relaxed);
                }
                #[cfg(test)]
                record_quiet_halfkp_qs_tt_skip(pending.as_deref(), false);
                return adj;
            }
            Bound::Lower => {
                if adj >= beta {
                    if let Some(diagnostics) = state.counters() {
                        diagnostics.qsearch[2].fetch_add(1, Ordering::Relaxed);
                    }
                    #[cfg(test)]
                    record_quiet_halfkp_qs_tt_skip(pending.as_deref(), false);
                    return adj;
                }
                if adj > alpha {
                    alpha = adj;
                }
            }
            Bound::Upper => {
                if adj <= alpha {
                    if let Some(diagnostics) = state.counters() {
                        diagnostics.qsearch[2].fetch_add(1, Ordering::Relaxed);
                    }
                    #[cfg(test)]
                    record_quiet_halfkp_qs_tt_skip(pending.as_deref(), false);
                    return adj;
                }
            }
        }
    }
    // Bounds below are relative to the post-probe window. A non-cutting lower
    // bound may have raised alpha and must not make the eventual result look
    // exact merely because the cached bound was present.
    let orig_alpha = alpha;

    // In-check nodes skip stand-pat, but their eager evasions also require
    // a clean accumulator. Keep one barrier after all TT prefix returns.
    if let Some(token) = pending {
        board.materialize_quiet_halfkp_for_search(token);
    }

    let in_check = known_in_check.unwrap_or_else(|| is_in_check(board, board.side_to_move));
    if in_check && let Some(diagnostics) = state.counters() {
        diagnostics.qsearch[1].fetch_add(1, Ordering::Relaxed);
    }
    let mut stand_pat_value: Option<i32> = None;

    // Stand-pat and delta pruning only apply when not in check.
    // In check the side to move has no quiet option, so stand-pat is invalid.
    if !in_check {
        // Standing pat must not hide a mate in one at the horizon.
        let qstyle = p::QS_STYLE();
        if (qply == 0 || qstyle & 2 != 0)
            && let Some(mate_move) = mate_in_one(board)
        {
            if let Some(diagnostics) = state.counters() {
                diagnostics.qsearch[8].fetch_add(1, Ordering::Relaxed);
            }
            let score = MATE_SCORE - (ply as i32 + 1);
            store_tt_for_search(
                state,
                hash,
                TtEntry {
                    score: score_to_tt(score, ply),
                    depth: 0,
                    bound: Bound::Exact,
                    mv: Some(mate_move),
                },
            );
            return score;
        }
        let mut stand_pat = evaluate_for_search(state, board);
        if qstyle & 1 != 0 {
            let keys = CorrKeys::of(board, board.side_to_move);
            stand_pat = (stand_pat + state.history.correction(keys))
                .clamp(-(MATE_SCORE - 2000), MATE_SCORE - 2000);
        }
        if qs_tt & 8 != 0
            && let Some((bound, adj)) = tt_bound
            && adj.abs() < MATE_SCORE - 1000
            && match bound {
                Bound::Lower => adj > stand_pat,
                Bound::Upper => adj < stand_pat,
                Bound::Exact => true,
            }
        {
            stand_pat = adj;
        }
        stand_pat_value = Some(stand_pat);
        if stand_pat >= beta {
            if let Some(diagnostics) = state.counters() {
                diagnostics.qsearch[3].fetch_add(1, Ordering::Relaxed);
            }
            if tt_here && !state.budget.should_abort() {
                store_tt_for_search(
                    state,
                    hash,
                    TtEntry {
                        score: score_to_tt(stand_pat, ply),
                        depth: 0,
                        bound: Bound::Lower,
                        mv: None,
                    },
                );
            }
            if qstyle & 4 != 0 && stand_pat.abs() < MATE_SCORE - 2000 {
                return (stand_pat + beta) / 2;
            }
            return stand_pat;
        }
        if stand_pat > alpha {
            alpha = stand_pat;
        }
        // Delta Pruning: if even the best possible capture+promotion cannot improve alpha, skip.
        // Max gain = Ryu capture (1300) + Fu→Tokin promotion bonus (500) = 1800cp.
        if stand_pat + p::QS_DELTA() < alpha {
            if let Some(diagnostics) = state.counters() {
                diagnostics.qsearch[9].fetch_add(1, Ordering::Relaxed);
            }
            if tt_here && !state.budget.should_abort() {
                store_tt_for_search(
                    state,
                    hash,
                    TtEntry {
                        score: score_to_tt(alpha, ply),
                        depth: 0,
                        bound: Bound::Upper,
                        mv: None,
                    },
                );
            }
            return alpha;
        }
    }

    let move_buffer = {
        let _timer = state.timer(|d| &d.movegen_order_ns);
        let mut move_buffer = {
            let _generate_timer = state.timer(|d| &d.movegen_generate_ns);
            if in_check {
                MoveBuffer::legal_with_in_check(board, true) // must escape check; all legal moves required
            } else {
                MoveBuffer::captures_with_in_check(board, false)
            }
        };
        // Order by a cheap MVV-LVA-style key. A full SEE here is too costly
        // per node (qsearch is the hottest path); the coarse capture ordering is
        // plenty for quiescence and keeps each node fast enough to respect the clock.
        {
            let _order_timer = state.timer(|d| &d.move_order_ns);
            // TT move first, then by the MVV-LVA key. One i32 carries both
            // (the key is far below the TT offset), so the order equals that
            // of the (flag, key) pair, and short lists sort without a heap
            // allocation (the std cached-key sort allocates on every call).
            let mut qkey = |m: &Move| {
                let key = qsearch_order_key(board, *m);
                debug_assert!(key.abs() < QSEARCH_TT_ORDER_OFFSET);
                if Some(*m) == tt_mv {
                    -key
                } else {
                    QSEARCH_TT_ORDER_OFFSET - key
                }
            };
            let list = move_buffer.as_mut_list().as_mut_slice();
            sort_moves_by_cached_key(list, &mut qkey);
        }
        move_buffer
    };

    if move_buffer.is_empty() {
        if let Some(diagnostics) = state.counters() {
            diagnostics.qsearch[4].fetch_add(1, Ordering::Relaxed);
        }
        let score = if in_check {
            -MATE_SCORE + ply as i32 // checkmate
        } else {
            alpha
        };
        if tt_here && !state.budget.should_abort() {
            store_tt_for_search(
                state,
                hash,
                TtEntry {
                    score: score_to_tt(score, ply),
                    depth: 0,
                    bound: if in_check || score > orig_alpha {
                        Bound::Exact
                    } else {
                        Bound::Upper
                    },
                    mv: None,
                },
            );
        }
        return score;
    }

    let mut best_move = None;
    let stm = board.side_to_move;
    // In check: whether some evasion has already been found not to be mated.
    let mut escaped = false;
    // Capture pruning outside check (QS_MOVE_LIMIT, QS_FUT_MARGIN,
    // QS_SEE_MIN): recaptures on the square just moved to and captures that
    // give check are never pruned.
    let prev_to = ply.checked_sub(1).and_then(current_move_at).map(|pm| pm.to);
    let qs_limit = p::QS_MOVE_LIMIT() as u32;
    let qs_fut = p::QS_FUT_MARGIN();
    let qs_see = p::QS_SEE_MIN();
    let qs_on = qs_limit > 0 || qs_fut > 0 || qs_see > 0;
    let mut qs_count = 0u32;
    for &m in move_buffer.as_slice() {
        if p::SKIP_NONPROMO() != 0 && useless_non_promotion(m, stm) {
            continue;
        }
        if escaped {
            let capture = m.from.is_some() && board.piece_at(m.to).is_some();
            let skip = match p::QS_EVASION_PRUNE() {
                0 => false,
                1 => !capture,
                _ => !capture || crate::movegen::see_swap(board, m) < 0,
            };
            if skip {
                continue;
            }
        }
        // Skip captures that lose material in the exchange (bitboard SEE).
        if !in_check
            && board.piece_at(m.to).is_some_and(|victim| {
                PIECE_VALUE[victim.kind.index()] < PIECE_VALUE[m.piece_kind.index()]
            })
            && crate::movegen::see_swap(board, m) < 0
        {
            continue;
        }
        let prunable = qs_on && stand_pat_value.is_some() && prev_to != Some(m.to);
        let victim_value = board
            .piece_at(m.to)
            .map_or(0, |v| PIECE_VALUE[v.kind.index()]);
        let see_bad = prunable && qs_see > 0 && crate::movegen::see_swap(board, m) < -qs_see;
        if qs_on {
            set_current_move(ply, Some(m));
        }
        let (tok, child_in_check, child_history) = play(board, m, history);
        if prunable && !child_in_check {
            qs_count += 1;
            let futile =
                qs_fut > 0 && stand_pat_value.is_some_and(|sp| sp + qs_fut + victim_value <= alpha);
            if futile || see_bad || (qs_limit > 0 && qs_count > qs_limit) {
                board.undo_move_for_search(tok);
                continue;
            }
        }
        let score = -quiescence(
            state,
            board,
            -beta,
            -alpha,
            ply + 1,
            qply + 1,
            Some(child_in_check),
            &child_history,
        );
        if let Some(diagnostics) = state.counters() {
            diagnostics.qsearch[5].fetch_add(1, Ordering::Relaxed);
        }
        board.undo_move_for_search(tok);

        if state.budget.should_abort() {
            return 0;
        }
        if score >= beta {
            if let Some(diagnostics) = state.counters() {
                diagnostics.qsearch[6].fetch_add(1, Ordering::Relaxed);
            }
            if tt_here && !state.budget.should_abort() {
                store_tt_for_search(
                    state,
                    hash,
                    TtEntry {
                        score: score_to_tt(score, ply),
                        depth: 0,
                        bound: Bound::Lower,
                        mv: Some(m),
                    },
                );
            }
            return score;
        }
        if score > alpha {
            alpha = score;
            best_move = Some(m);
        }
        if in_check && score > -(MATE_SCORE - 1000) {
            escaped = true;
        }
    }

    // QS_PROMO: promotions that capture nothing (bit 1 pawns, 2 bishops and
    // rooks; bit 8 at every quiescence ply, otherwise at the first only).
    let qs_promo = p::QS_PROMO();
    if qs_promo & 3 != 0 && !in_check && (qply == 0 || qs_promo & 8 != 0) {
        let promos = MoveBuffer::quiet_promotions(board, qs_promo & 1 != 0, qs_promo & 2 != 0);
        for &m in promos.as_slice() {
            if qs_on {
                set_current_move(ply, Some(m));
            }
            let tok = board.do_move_for_search(m);
            let mover = stm;
            if is_in_check(board, mover) {
                board.undo_move_for_search(tok);
                continue;
            }
            let gives_check = is_in_check(board, board.side_to_move);
            let child_history = history.after_move(board.hash(), mover, gives_check);
            let score = -quiescence(
                state,
                board,
                -beta,
                -alpha,
                ply + 1,
                qply + 1,
                Some(gives_check),
                &child_history,
            );
            if let Some(diagnostics) = state.counters() {
                diagnostics.qsearch[5].fetch_add(1, Ordering::Relaxed);
            }
            board.undo_move_for_search(tok);
            if state.budget.should_abort() {
                return 0;
            }
            if score >= beta {
                if let Some(diagnostics) = state.counters() {
                    diagnostics.qsearch[6].fetch_add(1, Ordering::Relaxed);
                }
                if tt_here {
                    store_tt_for_search(
                        state,
                        hash,
                        TtEntry {
                            score: score_to_tt(score, ply),
                            depth: 0,
                            bound: Bound::Lower,
                            mv: Some(m),
                        },
                    );
                }
                return score;
            }
            if score > alpha {
                alpha = score;
                best_move = Some(m);
            }
        }
    }

    if tt_here && !state.budget.should_abort() {
        store_tt_for_search(
            state,
            hash,
            TtEntry {
                score: score_to_tt(alpha, ply),
                depth: 0,
                bound: if alpha > orig_alpha {
                    Bound::Exact
                } else {
                    Bound::Upper
                },
                mv: best_move,
            },
        );
    }
    alpha
}

// ============================================================
// Phase 3: Speculative / Preemptive Searcher
// ============================================================

/// Search statistics returned by `SpeculativeSearcher`
pub struct SpecSearchInfo {
    /// Best move found, if any.
    pub best_move: Option<Move>,
    /// Score of `best_move` in centipawns (or a mate score).
    pub score: i32,
    /// Deepest iterative-deepening depth completed.
    pub depth: u32,
    /// Total nodes visited across all depths.
    pub nodes: u64,
    /// Wall-clock time spent searching.
    pub elapsed: Duration,
    /// Transposition table occupancy, in permille (0-1000).
    pub hashfull: u32,
    /// Number of depth iterations where speculation correctly predicted
    /// the best move (policy hit).
    pub spec_hits: u32,
    /// Number of depth iterations where speculation was launched.
    pub spec_total: u32,
    /// MultiPV results: [(move, score)] ordered best-first. Index 0 == best_move.
    pub pv_list: Vec<(Move, i32)>,
    /// Legal principal-variation prefix reconstructed from exact TT entries.
    ///
    /// This is the continuation for the primary move only. MultiPV callers
    /// must not attribute it to a non-primary candidate.
    pub pv: Vec<Move>,
    /// Number of depths where bestmove changed (instability indicator).
    pub bestmove_changes: u32,
}

/// `SpeculativeSearcher` wraps iterative deepening with preemptive
/// parallel speculation driven by the policy function.
pub struct SpeculativeSearcher {
    tt: Arc<Tt>,
    eval_cache: Option<Arc<EvalCache>>,
    top_n: usize,
    external_abort: Arc<AtomicBool>,
    /// Move-ordering history, countermoves and eval correction kept across
    /// searches in a game (see `Searcher`).
    history: Arc<HistoryTable>,
    countermoves: Arc<CountermoveTable>,
    // Dedicated pool for SpecGroup's background tasks, isolated from rayon's
    // global pool so they can never starve alpha_beta's own YBW dispatch
    // (`work.into_par_iter()...collect()`) of a worker. See SpecState::pool.
    spec_pool: Arc<rayon::ThreadPool>,
}

impl SpeculativeSearcher {
    /// Create a speculative searcher that considers the top `top_n` candidate
    /// replies for preemptive background search, backed by the given shared TT.
    ///
    /// The dedicated pool includes one foreground worker in addition to the
    /// at-most-`top_n` background tasks. This gives every recursive search a
    /// controlled stack budget without reducing speculative capacity.
    pub fn new(tt: Arc<Tt>, top_n: usize) -> Self {
        let spec_pool = rayon::ThreadPoolBuilder::new()
            .num_threads(top_n.saturating_add(1).max(1))
            .stack_size(RECURSIVE_SEARCH_STACK_BYTES)
            .build()
            .expect("failed to build dedicated speculative-search thread pool");
        SpeculativeSearcher {
            tt,
            eval_cache: active_eval_cache(),
            top_n,
            external_abort: Arc::new(AtomicBool::new(false)),
            history: Arc::new(HistoryTable::new()),
            countermoves: Arc::new(CountermoveTable::new()),
            spec_pool: Arc::new(spec_pool),
        }
    }

    /// Returns a clone of the abort flag; set to `true` to stop an in-progress search.
    pub fn abort_flag(&self) -> Arc<AtomicBool> {
        self.external_abort.clone()
    }

    /// Clear a previous stop signal before starting a new search.
    pub fn reset_abort_flag(&self) {
        self.external_abort.store(false, Ordering::Relaxed);
    }

    /// Probe the TT for the best move stored at `hash` (used to extract ponder move).
    pub fn probe_tt(&self, hash: u64) -> Option<Move> {
        self.tt.probe(hash).and_then(|e| e.mv)
    }

    /// Reset the shared TT in place. Call on `usinewgame` so a new game never
    /// probes entries left behind by a previous, unrelated game.
    pub fn clear_tt(&self) {
        self.tt.clear();
        if let Some(cache) = &self.eval_cache {
            cache.clear();
        }
        self.history.clear();
        self.countermoves.clear();
    }

    /// Age transposition-table entries written by earlier searches.
    ///
    /// This mirrors [`Searcher::new_search`]. Without it, bucket replacement
    /// cannot prefer the current search over equally deep entries left by an
    /// earlier `go` command.
    pub fn new_search(&self) {
        self.tt.new_search();
    }

    /// Run iterative-deepening search with preemptive speculative parallelism on
    /// candidate replies, returning the best line plus speculation statistics.
    ///
    /// Call [`Self::reset_abort_flag`] before reusing a searcher after an abort.
    pub fn search(&self, board: &mut Board, config: SearchConfig) -> SpecSearchInfo {
        let history = PositionHistory::initial(board.hash());
        self.search_with_history(board, config, &history)
    }

    /// Run speculative search with the externally played history preserved.
    /// The foreground alpha-beta path uses it for fourfold-repetition
    /// adjudication; speculative background work remains a cache-only hint and
    /// never supplies a history-dependent score to the foreground result.
    pub fn search_with_history(
        &self,
        board: &mut Board,
        config: SearchConfig,
        history: &PositionHistory,
    ) -> SpecSearchInfo {
        self.spec_pool
            .install(|| self.search_with_history_on_worker(board, config, history))
    }

    /// Foreground implementation, always executed on the dedicated search
    /// pool so library callers receive the same stack guarantee as USI users.
    fn search_with_history_on_worker(
        &self,
        board: &mut Board,
        config: SearchConfig,
        history: &PositionHistory,
    ) -> SpecSearchInfo {
        debug_assert_eq!(
            history.entries().last().map(|entry| entry.hash),
            Some(board.hash())
        );
        if p::HIST_KEEP() == 0 {
            self.history.clear();
            self.countermoves.clear();
        }
        let state = Arc::new(SearchState {
            tt: self.tt.clone(),
            budget: Arc::new(Budget::new(
                config.time_limit,
                config.node_limit,
                self.external_abort.clone(),
            )),
            killers: KillerTable::new(),
            history: self.history.clone(),
            countermoves: self.countermoves.clone(),
            diagnostics: None,
            eval_cache: self.eval_cache.clone(),
            pruning: PruningConfig::default(),
            retirement: None,
        });

        // Spec tasks share the *same* Budget as the main search (not an
        // independent copy) so a USI stop or the watchdog firing is visible
        // to both without hand-syncing a separate flag between them.
        let spec_state = Arc::new(SpecState {
            tt: self.tt.clone(),
            budget: state.budget.clone(),
            pool: self.spec_pool.clone(),
        });

        // Watchdog: guarantee the search stops at the hard deadline regardless of
        // rayon scheduling. The per-node elapsed checks rely on a thread getting
        // scheduled to run them; when spec tasks + nested YBW saturate the pool,
        // that can be starved and the move blows past its byoyomi. An OS timer
        // thread is immune. It targets the per-search budget (recreated every
        // call), so a late fire after an early return is harmless.
        if let Some(lim) = config.time_limit {
            let budget = state.budget.clone();
            std::thread::spawn(move || {
                std::thread::sleep(lim);
                budget.abort_now();
            });
        }

        let mut best_move = None;
        let mut best_score = NEG_INF;
        let mut done_depth = 0u32;
        let mut spec_hits = 0u32;
        let mut spec_total = 0u32;
        let mut prev_best: Option<Move> = None;
        let mut pv_list: Vec<(Move, i32)> = Vec::new();
        let mut best_pv = Vec::new();
        let mut bestmove_changes = 0u32;
        let use_spec = config.multi_pv == 1;

        for depth in 1..=config.max_depth {
            // Speculative search only makes sense for single-PV (predicts opponent's reply to PV[0])
            let mut spec_group = if use_spec {
                spec_total += 1;
                Some(SpecGroup::spawn(board, &spec_state, depth + 1, self.top_n))
            } else {
                None
            };

            // MultiPV: run N root searches per depth, excluding previously found moves
            let mut depth_pv: Vec<(Move, i32)> = Vec::new();
            let mut excluded: Vec<Move> = Vec::new();
            for _ in 0..config.multi_pv {
                let (m, score, _) = root_search(
                    &state,
                    board,
                    depth,
                    best_score,
                    &excluded,
                    None,
                    true,
                    &SearchHistory::root(history),
                    0,
                    false,
                    None,
                    None,
                );
                if state.budget.should_abort() {
                    break;
                }
                match m {
                    Some(mv) => {
                        depth_pv.push((mv, score));
                        excluded.push(mv);
                    }
                    None => break,
                }
            }
            let m = depth_pv.first().map(|&(mv, _)| mv);
            let score = depth_pv.first().map(|&(_, s)| s).unwrap_or(NEG_INF);

            let timed_out = state.budget.should_abort();

            if let Some(ref mut sg) = spec_group
                && let Some(winner) = m
            {
                let hit = sg.poll(winner).is_some();
                if hit {
                    spec_hits += 1;
                }
                if !timed_out {
                    sg.promote(winner);
                }
            }
            drop(spec_group);

            if timed_out {
                break;
            }

            if !depth_pv.is_empty() {
                pv_list = depth_pv;
                best_move = m.or(best_move);
                best_score = score;
                done_depth = depth;
                // Capture the primary line while the TT still represents this
                // completed iteration. A later, interrupted iteration may
                // replace exact child entries, but must not detach the score,
                // depth, and best move above from the PV that explained them.
                best_pv = extract_pv(&self.tt, board, best_move, depth);
            }

            if score.abs() >= MATE_SCORE - 1000 {
                break;
            }

            if best_move != prev_best && depth >= 3 {
                bestmove_changes += 1;
            }

            // Soft limit: exit after a completed depth when bestmove is stable.
            if soft_limit_expired(
                &state.budget,
                config.soft_limit,
                depth,
                best_move == prev_best,
            ) {
                break;
            }
            prev_best = best_move;
        }

        // A deadline may arrive before any complete iterative-deepening
        // result, especially on a contended or slow host. Still return a
        // legal move instead of an empty bestmove response.
        if best_move.is_none() {
            best_move = MoveBuffer::legal(board).as_slice().first().copied();
            if best_move.is_some() {
                best_score = evaluate(board);
            }
        }

        state.budget.abort_now();

        let pv = if best_pv.is_empty() {
            extract_pv(&self.tt, board, best_move, done_depth)
        } else {
            best_pv
        };
        SpecSearchInfo {
            best_move,
            score: best_score,
            depth: done_depth,
            nodes: state.budget.nodes(),
            elapsed: state.budget.elapsed(),
            hashfull: self.tt.hashfull(),
            spec_hits,
            spec_total,
            pv_list,
            pv,
            bestmove_changes,
        }
    }
}

// ============================================================
// Helpers
// ============================================================

/// Convert a ply-relative score to position-relative for TT storage.
/// Mate scores encode the distance to mate; we strip the ply component so the stored
/// score is "mate in N from THIS position" independent of when we found it.
///
/// `pub(crate)`: also called from `speculative.rs`, which writes to this same shared
/// TT and must use the identical encoding, or `alpha_beta`'s `score_from_tt` on read
/// will misinterpret an un-adjusted mate score as already ply-relative.
#[inline]
pub(crate) fn score_to_tt(score: i32, ply: u32) -> i32 {
    let p = ply as i32;
    if score > MATE_SCORE - 1000 {
        score + p
    }
    // winning mate: add ply
    else if score < -MATE_SCORE + 1000 {
        score - p
    }
    // losing mate:  subtract ply
    else {
        score
    }
}

/// Convert a position-relative TT score back to a ply-relative search score.
///
/// `pub(crate)`: exercised directly from `speculative.rs`'s own tests, which assert
/// entries `speculative.rs` stores decode correctly through this same function (the
/// one `alpha_beta` actually calls on a real TT probe), not a reimplementation of its
/// logic in the test that could hide a matching bug in both places.
#[inline]
pub(crate) fn score_from_tt(stored: i32, ply: u32) -> i32 {
    let p = ply as i32;
    if stored > MATE_SCORE - 1000 {
        stored - p
    }
    // winning mate: subtract ply
    else if stored < -MATE_SCORE + 1000 {
        stored + p
    }
    // losing mate:  add ply
    else {
        stored
    }
}

#[inline]
#[allow(clippy::too_many_arguments)]
fn store_tt(
    state: &SearchState,
    hash: u64,
    score: i32,
    depth: u32,
    bound: Bound,
    mv: Option<Move>,
    ply: u32,
    skip_move: Option<Move>,
    pv: bool,
) {
    // A singular-extension verification search excludes one legal move from
    // the move set. Its result is therefore not a valid TT result for the
    // unrestricted position. PR #4 fixes the read-side self-hit; keep the
    // corresponding write-side invariant explicit rather than relying on
    // Tt's incidental depth-preferred replacement policy.
    if skip_move.is_some() {
        return;
    }
    store_tt_for_search_pv(
        state,
        hash,
        TtEntry {
            score: score_to_tt(score, ply),
            depth: crate::tt::depth_to_tt(depth),
            bound,
            mv,
        },
        pv,
    );
}

/// A non-promotion of a pawn, bishop or rook that could have promoted (and,
/// with `SKIP_NONPROMO` 2, of a lance to the second rank). The promoted
/// piece moves like the original and more (a lance on the second rank can
/// only step to the last rank, where it must promote), so these moves are
/// skipped below the root when `SKIP_NONPROMO` is set.
#[inline]
fn useless_non_promotion(m: Move, stm: Color) -> bool {
    if m.promote {
        return false;
    }
    let Some(from) = m.from else {
        return false;
    };
    match m.piece_kind {
        PieceKind::Fu | PieceKind::Kaku | PieceKind::Hisha => {
            let zone = match stm {
                Color::Black => crate::bitboard::Bitboard::PROMOTE_BLACK,
                Color::White => crate::bitboard::Bitboard::PROMOTE_WHITE,
            };
            zone.contains(from) || zone.contains(m.to)
        }
        PieceKind::Kyou => {
            p::SKIP_NONPROMO() >= 2
                && m.to.rank()
                    == match stm {
                        Color::Black => 2,
                        Color::White => 8,
                    }
        }
        _ => false,
    }
}

/// Update killer, history, and countermove tables when a quiet move causes a beta cutoff.
/// Must be called with `board` in the state BEFORE `do_move(m)` (so side_to_move is correct).
/// Capture-history bonus for a capture that caused a cutoff.
#[inline]
fn reward_capture(history: &HistoryTable, board: &Board, stm: Color, m: Move, depth: u32) {
    if m.from.is_some()
        && let Some(victim) = board.piece_at(m.to)
    {
        history.capture_add(
            stm,
            m,
            victim.kind,
            history_bonus(depth) * p::CAPT_UPDATE() / 16,
        );
    }
}

#[inline]
#[allow(clippy::too_many_arguments)]
fn update_quiet_heuristics(
    killers: &KillerTable,
    history: &HistoryTable,
    countermoves: &CountermoveTable,
    m: Move,
    stm: Color,
    ply: u32,
    depth: u32,
    board: &Board,
    prev_mv: Option<Move>,
) {
    // Quiet = neither capture nor promotion; drops included.
    if board.piece_at(m.to).is_none() && !m.promote {
        killers.add(ply as usize, m);
        history.update(stm, m, depth);
        if p::PAWN_HIST_WEIGHT() > 0 {
            history.pawnh_add(
                HistoryTable::pawn_bucket(board),
                stm,
                m,
                history_bonus(depth),
            );
        }
        ContMoves::at(ply, prev_mv).update(history, stm, m, history_bonus(depth));
        if let Some(pm) = prev_mv {
            countermoves.update(stm.flip(), pm, m);
        }
    }
}

#[cfg(test)]
mod see_tests;

#[cfg(test)]
mod regression_tests;
