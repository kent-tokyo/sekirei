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
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use crate::board::Board;
use crate::budget::{Budget, soft_limit_expired};
use crate::color::Color;
use crate::eval::{PIECE_VALUE, evaluate, evaluation_cache_key};
use crate::movegen::{
    MoveBuffer, discovered_check_candidates, is_in_check, mate_in_one, move_gives_direct_check,
};
#[cfg(test)]
use crate::movegen::{generate_legal_captures, generate_legal_moves};
use crate::mv::{Move, MoveToken};
use crate::nnue::weights_active;
use crate::piece::PieceKind;
use crate::sfen::{PositionHistory, PositionHistoryEntry, RepetitionOutcome};
use crate::speculative::{SpecGroup, SpecState};
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

/// Whether quiescence tries quiet checking moves at its first ply. Finding
/// them requires generating and playing every legal move at each leaf; in
/// local self-play, disabling it was clearly stronger.
const QSEARCH_CHECKS: bool = false;

/// Exact cache of the existing floating-point LMR formula for all representable
/// TT depths and the practical maximum shogi move count. This removes two
/// transcendental `ln` calls from every late-move probe without changing the
/// reduction chosen by the search.
static LMR_REDUCTION_TABLE: OnceLock<Box<[[u8; 600]]>> = OnceLock::new();

mod heuristics;
pub mod params;
use heuristics::{
    CorrKeys, CountermoveTable, HistoryTable, KillerTable, history_bonus, history_malus,
};
use params as p;

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
    /// Cumulative nodes spent in root mate-in-one checks, if a diagnostic
    /// observer was attached.
    pub root_mate_in_one_nodes: Option<u64>,
    /// Cumulative nodes spent filtering root mate blunders, if observed.
    pub root_mate_blunder_nodes: Option<u64>,
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
}

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

/// Per-search cache for root mate safety facts that do not depend on depth.
///
/// The root board and legal move set stay unchanged across iterative-deepening
/// passes.  Store unsafe moves rather than an ordered safe list, so later
/// passes retain their newly computed move ordering while avoiding the same
/// immediate-mate enumeration.
#[derive(Default)]
struct RootMateSafetyCache {
    mate_in_one_checked: bool,
    unsafe_moves: Option<Vec<Move>>,
}

impl SearchDiagnostics {
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
        }
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
        let after = slot.sequence.load(Ordering::Acquire);
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
                    Ordering::Acquire,
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
    let entry = {
        let _timer = state.timer(|d| &d.tt_probe_ns);
        state.tt.probe(hash)
    };
    if let Some(diagnostics) = state.counters() {
        diagnostics.tt_probes.fetch_add(1, Ordering::Relaxed);
        if entry.is_some() {
            diagnostics.tt_hits.fetch_add(1, Ordering::Relaxed);
        }
    }
    entry
}

#[inline]
fn store_tt_for_search(state: &SearchState, hash: u64, entry: TtEntry) {
    let _timer = state.timer(|d| &d.tt_store_ns);
    state.tt.store(hash, entry);
    if let Some(diagnostics) = state.counters() {
        diagnostics.tt_stores.fetch_add(1, Ordering::Relaxed);
    }
}

/// Search pruning switches used by diagnostic ablations.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PruningConfig {
    /// Enable null-move pruning and its verification search.
    pub null_move: bool,
    /// Enable late-move reductions for quiet, late-ordered moves.
    pub late_move_reduction: bool,
}

impl Default for PruningConfig {
    fn default() -> Self {
        Self {
            null_move: true,
            late_move_reduction: true,
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
        if let Some(cache) = &self.eval_cache {
            cache.clear();
        }
        self.history.clear();
        self.countermoves.clear();
    }

    /// Run iterative-deepening search from the current position up to `config.max_depth`
    /// or until a time limit / abort signal fires, returning the best line found.
    ///
    /// Call [`Self::reset_abort_flag`] before reusing a searcher after an abort.
    pub fn search(&self, board: &mut Board, config: SearchConfig) -> SearchInfo {
        let history = PositionHistory::initial(board.hash());
        self.search_impl(board, config, None, true, &history, None)
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
        self.search_impl(board, config, None, true, history, None)
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
        let info = self.search_impl(board, config, None, true, history, Some(&mut trace));
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
        self.search_impl(board, config, None, false, history, None)
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
        let info = self.search_impl(board, config, None, false, history, Some(&mut trace));
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
        self.search_impl(board, config, None, false, &history, None)
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
        self.search_impl(board, config, Some(root_move), true, history, None)
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
        mut iteration_trace: Option<&mut Vec<SearchIteration>>,
    ) -> SearchInfo {
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
            diagnostics: self.diagnostics.clone(),
            eval_cache: self.eval_cache.clone(),
            pruning: self.pruning,
        });

        let mut best_move = None;
        let mut best_score = NEG_INF;
        let mut done_depth = 0;
        let mut prev_best: Option<Move> = None;
        let mut bound = SearchBound::Unknown;
        let mut root_mate_safety_cache = RootMateSafetyCache::default();
        let history = &SearchHistory::root(history);

        for depth in 1..=config.max_depth {
            let (m, score, root_bound) = root_search(
                &state,
                board,
                depth,
                best_score,
                &[],
                root_move,
                root_mate_safety,
                history,
                Some(&mut root_mate_safety_cache),
            );

            if state.budget.should_abort() {
                // The interrupted iteration only reports a move whose search
                // at this depth completed inside the window (the previous
                // best failed low or was beaten), so it is the better choice.
                if depth > 1 && m.is_some() {
                    best_move = m;
                }
                break;
            }

            best_move = m.or(best_move);
            best_score = score;
            done_depth = depth;
            bound = root_bound;

            if let Some(trace) = iteration_trace.as_deref_mut() {
                let diagnostics = state
                    .diagnostics
                    .as_ref()
                    .map(|observer| observer.snapshot());
                trace.push(SearchIteration {
                    depth,
                    best_move,
                    score,
                    nodes: state.budget.nodes(),
                    bound: root_bound,
                    root_mate_in_one_nodes: diagnostics
                        .map(|snapshot| snapshot.root_mate_in_one_nodes),
                    root_mate_blunder_nodes: diagnostics
                        .map(|snapshot| snapshot.root_mate_blunder_nodes),
                });
            }

            if score.abs() >= MATE_SCORE - 1000 {
                break;
            }

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
        let pv = extract_pv(&self.tt, board, best_move, done_depth);

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
    /// The move that produced this node; `None` for the root, whose position
    /// is the last game entry.
    entry: Option<PositionHistoryEntry>,
}

impl<'a> SearchHistory<'a> {
    fn root(game: &'a PositionHistory) -> Self {
        SearchHistory {
            game,
            parent: None,
            entry: None,
        }
    }

    #[inline]
    fn after_move(&'a self, hash: u64, mover: Color, gave_check: bool) -> SearchHistory<'a> {
        SearchHistory {
            game: self.game,
            parent: Some(self),
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
        let mut frame = Some(self);
        while let Some(f) = frame {
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

/// What a main-search node takes from its transposition-table entry.
struct TtProbe {
    /// A score the node can return at once.
    cutoff: Option<i32>,
    /// Alpha after a lower bound that did not cut.
    alpha: i32,
    /// The entry's move, for ordering.
    mv: Option<Move>,
    /// The entry's score when it is a lower or exact bound (singular extension).
    se_score: Option<i32>,
    /// The entry's depth (singular extension eligibility).
    se_depth: u8,
    /// Whether the table held an entry for this position at all.
    hit: bool,
}

impl TtProbe {
    /// Probe the entry of `hash` for a node at `depth` with window
    /// `(alpha, beta)`. In a singular verification search (`excluded`) the
    /// entry only supplies the move and the singular data: the node's move set
    /// lacks the candidate that produced the entry, so its bounds do not apply.
    #[inline]
    fn at(
        state: &SearchState,
        hash: u64,
        depth: u32,
        ply: u32,
        mut alpha: i32,
        beta: i32,
        excluded: bool,
    ) -> Self {
        let mut probe = TtProbe {
            cutoff: None,
            alpha,
            mv: None,
            se_score: None,
            se_depth: 0,
            hit: false,
        };
        let Some(entry) = probe_tt_for_search(state, hash) else {
            return probe;
        };
        probe.hit = true;
        let adj = score_from_tt(entry.score, ply);
        probe.mv = entry.mv;
        probe.se_depth = entry.depth;
        if !matches!(entry.bound, Bound::Upper) {
            probe.se_score = Some(adj);
        }
        if entry.depth >= depth as u8 && !excluded {
            match entry.bound {
                Bound::Exact => probe.cutoff = Some(adj),
                Bound::Lower => {
                    if adj >= beta {
                        probe.cutoff = Some(adj);
                    } else if adj > alpha {
                        alpha = adj;
                    }
                }
                Bound::Upper => {
                    if adj <= alpha {
                        probe.cutoff = Some(adj);
                    }
                }
            }
        }
        probe.alpha = alpha;
        probe
    }
}

/// ProbCut: when a shallow search of a clearly winning capture (SEE at least
/// `PC_MARGIN`) beats `beta + PC_MARGIN`, the node is assumed to fail high.
/// Returns that score.
fn probcut(
    state: &Arc<SearchState>,
    board: &mut Board,
    beta: i32,
    depth: u32,
    ply: u32,
    history: &SearchHistory<'_>,
) -> Option<i32> {
    let pc_beta = beta + p::PC_MARGIN();
    let mut caps = MoveBuffer::captures(board);
    let stm = board.side_to_move;
    caps.as_mut_list().retain(|m| {
        crate::movegen::see_swap(board, *m) >= p::PC_MARGIN()
            && (p::SKIP_NONPROMO() == 0 || !useless_non_promotion(*m, stm))
    });
    let cap_list = caps.as_mut_list().as_mut_slice();
    let mut cap_key = |m: &Move| -crate::movegen::see_swap(board, *m);
    sort_moves_by_cached_key(cap_list, &mut cap_key);
    let pc_depth = (depth - 4).min(3); // cap at 3 to keep the probe cheap
    for &cap in caps.as_slice() {
        if state.budget.should_abort() {
            break;
        }
        set_current_move(ply, Some(cap));
        let (tok, child_in_check, child_history) = play(board, cap, history);
        let pc_score = -alpha_beta(
            state,
            board,
            -pc_beta,
            -pc_beta + 1,
            pc_depth,
            ply + 1,
            false,
            Some(cap),
            None,
            Some(child_in_check),
            &child_history,
        );
        board.undo_move_for_search(tok);
        if pc_score >= pc_beta {
            return Some(pc_score);
        }
    }
    None
}

/// Null move pruning: a reduced search after passing the move. A fail-high
/// at depth 6 or more is confirmed by a real shallow search first, against
/// zugzwang-like horizon effects common in shogi. Returns the null-move score
/// when the node is cut.
fn null_move_pruning(
    state: &Arc<SearchState>,
    board: &mut Board,
    beta: i32,
    depth: u32,
    ply: u32,
    prev_mv: Option<Move>,
    history: &SearchHistory<'_>,
) -> Option<i32> {
    set_current_move(ply, None);
    let null_tok = board.do_null_move();
    let null_score = -alpha_beta(
        state,
        board,
        -beta,
        -beta + 1,
        depth.saturating_sub(1 + nmp_reduction(depth)),
        ply + 1,
        false,
        None,
        None,
        None,
        history,
    );
    board.undo_null_move(null_tok);
    if null_score < beta {
        return None;
    }
    if depth < 6 {
        return Some(null_score);
    }
    let verify = alpha_beta(
        state,
        board,
        beta - 1,
        beta,
        depth.saturating_sub(1 + nmp_reduction(depth)),
        ply,
        false,
        prev_mv,
        None,
        Some(false),
        history,
    );
    (verify >= beta).then_some(null_score)
}

/// Whether the static eval permits a null move against `beta`.
#[inline]
fn nmp_eval_allows(static_eval: Option<i32>, beta: i32) -> bool {
    let margin = p::NMP_EVAL_MARGIN();
    margin >= 5000 || static_eval.is_none_or(|se| se >= beta - margin)
}

/// Null move depth reduction at `depth`.
#[inline]
fn nmp_reduction(depth: u32) -> u32 {
    p::NMP_R() as u32 + depth * p::NMP_R_PER_DEPTH() as u32 / 64
}

/// Principal variation search of a child already played on `board`: a
/// null-window probe reduced by `reduce`, a full-depth null-window search
/// when a reduced probe fails high, and an exact full-window search when the
/// score lands inside the window. Returns the child's score for the parent.
#[allow(clippy::too_many_arguments)]
#[inline]
fn pvs_child(
    state: &Arc<SearchState>,
    board: &mut Board,
    alpha: i32,
    beta: i32,
    depth: u32,
    reduce: u32,
    ply: u32,
    m: Move,
    child_in_check: bool,
    child_history: &SearchHistory<'_>,
) -> i32 {
    let full_depth = depth - 1;
    let mut search = |lo: i32, hi: i32, d: u32| {
        -alpha_beta(
            state,
            board,
            -hi,
            -lo,
            d,
            ply + 1,
            true,
            Some(m),
            None,
            Some(child_in_check),
            child_history,
        )
    };
    let floor = (p::LMR_MIN_CHILD_DEPTH() as u32).min(full_depth);
    let mut s = search(
        alpha,
        alpha + 1,
        depth.saturating_sub(1 + reduce).max(floor),
    );
    if reduce > 0 && s > alpha {
        s = search(alpha, alpha + 1, full_depth);
    }
    if s > alpha && s < beta {
        s = search(alpha, beta, full_depth);
    }
    s
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

/// Outcome of the root mate-safety filters.
enum RootSafety {
    /// A move mates at once; the search is over.
    Mate(Move),
    /// The budget ran out inside the filters.
    Aborted,
    /// The root moves to search: `None` keeps the ordered list as it is.
    Filtered(Option<Vec<Move>>),
}

/// Root mate filters: play a mate in one when there is one, and at shallow
/// depths drop moves that allow an immediate mate. They are production
/// move-selection guards, not search nodes, but they count against the
/// caller's hard budget so a bounded search cannot spend unbounded time here.
fn root_mate_safety_filter(
    state: &SearchState,
    board: &mut Board,
    ordered: &[Move],
    depth: u32,
    mut root_mate_safety_cache: Option<&mut RootMateSafetyCache>,
) -> RootSafety {
    let _timer = ProfileTimer::new(
        state
            .diagnostics
            .as_deref()
            .map(|diagnostics| &diagnostics.root_mate_safety_ns),
    );
    // Root mate filters are production move-selection guards, not search
    // nodes. They must nevertheless count against the caller's hard
    // budget so a bounded search cannot spend unbounded time here.
    let mate_in_one_checked = root_mate_safety_cache
        .as_ref()
        .is_some_and(|cache| cache.mate_in_one_checked);
    if !mate_in_one_checked {
        let before_mate_in_one = state.budget.nodes();
        let mate_in_one = root_mate_in_one(state, board, ordered);
        if let Some(diagnostics) = state.diagnostics.as_deref() {
            diagnostics.root_mate_in_one_nodes.fetch_add(
                state.budget.nodes().saturating_sub(before_mate_in_one),
                Ordering::Relaxed,
            );
        }
        if let Some(m) = mate_in_one {
            return RootSafety::Mate(m);
        }
        if !state.budget.should_abort()
            && let Some(cache) = root_mate_safety_cache.as_deref_mut()
        {
            cache.mate_in_one_checked = true;
        }
    } else if let Some(diagnostics) = state.diagnostics.as_deref() {
        diagnostics
            .root_mate_in_one_cache_hits
            .fetch_add(1, Ordering::Relaxed);
    }
    if state.budget.should_abort() {
        return RootSafety::Aborted;
    }
    RootSafety::Filtered(if depth <= 2 {
        let unsafe_moves = root_mate_safety_cache
            .as_ref()
            .and_then(|cache| cache.unsafe_moves.as_deref());
        let unsafe_moves = if let Some(unsafe_moves) = unsafe_moves {
            if let Some(diagnostics) = state.diagnostics.as_deref() {
                diagnostics
                    .root_mate_blunder_cache_hits
                    .fetch_add(1, Ordering::Relaxed);
            }
            unsafe_moves.to_vec()
        } else {
            let before_blunder_filter = state.budget.nodes();
            let unsafe_moves = root_mate_blunders(state, board, ordered);
            if let Some(diagnostics) = state.diagnostics.as_deref() {
                diagnostics.root_mate_blunder_nodes.fetch_add(
                    state.budget.nodes().saturating_sub(before_blunder_filter),
                    Ordering::Relaxed,
                );
            }
            let Some(unsafe_moves) = unsafe_moves else {
                return RootSafety::Aborted;
            };
            if let Some(cache) = root_mate_safety_cache {
                cache.unsafe_moves = Some(unsafe_moves.clone());
            }
            unsafe_moves
        };
        safe_root_moves(ordered, &unsafe_moves)
    } else {
        None
    })
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
    root_mate_safety_cache: Option<&mut RootMateSafetyCache>,
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

    // Aspiration window: start tight around prev_score; widen on fail
    let use_asp = depth >= 2 && prev_score.abs() < MATE_SCORE - 1000;
    let (mut lo, mut hi) = if use_asp {
        (prev_score - p::ASP_DELTA(), prev_score + p::ASP_DELTA())
    } else {
        (NEG_INF, POS_INF)
    };

    // The window grows geometrically around the score that fell outside it,
    // so a large swing costs a few re-searches instead of one per step.
    let mut delta = p::ASP_DELTA();
    loop {
        let (m, score) = root_search_inner(state, board, depth, ordered, lo, hi, history);

        if state.budget.should_abort() {
            return (m, score, SearchBound::Unknown);
        }

        delta = delta.saturating_mul(2);
        if score <= lo {
            lo = if score.abs() >= MATE_SCORE - 1000 {
                NEG_INF
            } else {
                (score - delta).max(NEG_INF)
            };
        } else if score >= hi {
            hi = if score.abs() >= MATE_SCORE - 1000 {
                POS_INF
            } else {
                (score + delta).min(POS_INF)
            };
        } else {
            return (m, score, SearchBound::Exact);
        }

        // Full window fallback
        if lo <= NEG_INF && hi >= POS_INF {
            return (m, score, SearchBound::Exact);
        }
    }
}

/// Whether `m` can give check: a direct check, or a move of a piece that
/// may uncover a slider. Only such moves can mate, so the root mate filters
/// skip the rest without playing them.
fn may_give_check(board: &Board, m: Move, discoverers: crate::bitboard::Bitboard) -> bool {
    move_gives_direct_check(board, m) || m.from.is_some_and(|from| discoverers.contains(from))
}

/// Return an immediate mating move without changing the caller's board.
fn root_mate_in_one(state: &SearchState, board: &mut Board, ordered: &[Move]) -> Option<Move> {
    let discoverers = discovered_check_candidates(board);
    for &m in ordered {
        if state.budget.tick() {
            return None;
        }
        if !may_give_check(board, m, discoverers) {
            continue;
        }
        let tok = board.do_move(m);
        let mated = MoveBuffer::legal(board).is_empty() && is_in_check(board, board.side_to_move);
        board.undo_move(tok);
        if mated {
            return Some(m);
        }
    }
    None
}

/// At shallow root depths, discard moves that allow an immediate opponent mate.
/// Returning `None` means either no blunder was found or filtering would remove
/// every move; in both cases the original ordered list remains authoritative.
fn root_mate_blunders(
    state: &SearchState,
    board: &mut Board,
    ordered: &[Move],
) -> Option<Vec<Move>> {
    let mut unsafe_moves = Vec::new();
    for &m in ordered {
        if state.budget.tick() {
            return None;
        }
        let tok = board.do_move(m);
        let mut opponent_can_mate = false;
        let discoverers = discovered_check_candidates(board);
        for &opp_m in MoveBuffer::legal(board).as_slice() {
            if state.budget.tick() {
                board.undo_move(tok);
                return None;
            }
            if !may_give_check(board, opp_m, discoverers) {
                continue;
            }
            let tok2 = board.do_move(opp_m);
            let is_mate =
                MoveBuffer::legal(board).is_empty() && is_in_check(board, board.side_to_move);
            board.undo_move(tok2);
            if is_mate {
                opponent_can_mate = true;
                break;
            }
        }
        board.undo_move(tok);
        if opponent_can_mate {
            unsafe_moves.push(m);
        }
    }
    Some(unsafe_moves)
}

/// Preserve the current ordered list unless filtering leaves at least one move.
fn safe_root_moves(ordered: &[Move], unsafe_moves: &[Move]) -> Option<Vec<Move>> {
    if unsafe_moves.is_empty() {
        return None;
    }
    let safe_moves = ordered
        .iter()
        .copied()
        .filter(|candidate| !unsafe_moves.contains(candidate))
        .collect::<Vec<_>>();
    (!safe_moves.is_empty()).then_some(safe_moves)
}

fn root_search_inner(
    state: &Arc<SearchState>,
    board: &mut Board,
    depth: u32,
    ordered: &[Move],
    lo: i32,
    hi: i32,
    history: &SearchHistory<'_>,
) -> (Option<Move>, i32) {
    let mut best_move = None;
    let mut alpha = lo;

    for (i, &m) in ordered.iter().enumerate() {
        let mover = board.side_to_move;
        // Late quiet root moves get a reduced null-window probe first.
        let quiet = board.piece_at(m.to).is_none() && !m.promote;
        let reduce = if depth >= 3 && i >= 3 && quiet {
            lmr_base_reduction(depth, i + 1)
                .saturating_sub(1)
                .min(depth - 2)
        } else {
            0
        };
        set_current_move(0, Some(m));
        let tok = board.do_move(m);
        let child_in_check = is_in_check(board, board.side_to_move);
        let child_history = history.after_move(board.hash(), mover, child_in_check);
        let search = |board: &mut Board, a: i32, b: i32, d: u32| {
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
            let reduce = if child_in_check { 0 } else { reduce };
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

        if score > alpha {
            alpha = score;
            best_move = Some(m);
        }
        if alpha >= hi {
            break;
        }
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
                depth: depth as u8,
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

/// What identifies a main-search node in the transposition table: its hash,
/// the (possibly reduced) depth it is searched to, the ply, and the move a
/// singular verification search excludes.
#[derive(Clone, Copy)]
struct NodeKey {
    hash: u64,
    depth: u32,
    ply: u32,
    skip_move: Option<Move>,
    /// The node's correction keys and corrected static evaluation, when it
    /// has one (not in check, shallow enough to evaluate).
    corr: Option<(CorrKeys, i32)>,
    /// Opponent pieces at the node, to tell captures apart.
    enemy: crate::bitboard::Bitboard,
}

impl NodeKey {
    #[inline]
    fn store(&self, state: &SearchState, score: i32, bound: Bound, mv: Option<Move>) {
        store_tt(
            state,
            self.hash,
            score,
            self.depth,
            bound,
            mv,
            self.ply,
            self.skip_move,
        );
        self.learn_correction(state, score, bound, mv);
    }

    /// Teach the static-eval correction the result of this node when it
    /// says something about the evaluation: not for captures or promotions
    /// (the evaluation of the position does not see them), mate scores,
    /// singular verification searches, or bounds on the wrong side of the
    /// corrected evaluation.
    #[inline]
    fn learn_correction(&self, state: &SearchState, score: i32, bound: Bound, mv: Option<Move>) {
        let Some((keys, eval)) = self.corr else {
            return;
        };
        if self.skip_move.is_some()
            || state.budget.should_abort()
            || score.abs() >= MATE_SCORE - 1000
            || mv.is_some_and(|m| m.promote || self.enemy.contains(m.to))
            || (bound == Bound::Lower && score <= eval)
            || (bound == Bound::Upper && score >= eval)
        {
            return;
        }
        state
            .history
            .learn_correction(keys, score - eval, self.depth);
    }

    /// A beta cutoff on `cutoff_move`: penalise the quiet moves tried before
    /// it, reward it, store the lower bound and return the score.
    #[allow(clippy::too_many_arguments)]
    fn beta_cutoff(
        &self,
        state: &SearchState,
        best_score: i32,
        best_move: Option<Move>,
        tried_quiet: &[Move],
        tried_captures: &[Move],
        cutoff_move: Move,
        stm: Color,
        board: &Board,
        prev_mv: Option<Move>,
    ) -> i32 {
        let depth = self.depth;
        let cont = ContMoves::at(self.ply, prev_mv);
        for &qm in tried_quiet {
            state.history.malus(stm, qm, depth);
            cont.update(&state.history, stm, qm, -history_malus(depth));
        }
        let capture_malus = -history_malus(depth) * p::CAPT_UPDATE() / 16;
        for &cm in tried_captures {
            if let Some(victim) = board.piece_at(cm.to) {
                state
                    .history
                    .capture_add(stm, cm, victim.kind, capture_malus);
            }
        }
        reward_capture(&state.history, board, stm, cutoff_move, depth);
        update_quiet_heuristics(
            &state.killers,
            &state.history,
            &state.countermoves,
            cutoff_move,
            stm,
            self.ply,
            depth,
            board,
            prev_mv,
        );
        self.store(state, best_score, Bound::Lower, best_move);
        best_score
    }
}

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

/// One ply of the current search path.
#[derive(Clone, Copy)]
struct Frame {
    /// Static evaluation of the node at this ply; `i32::MIN` when it was not
    /// computed (in check, or deeper than the pruning that uses it).
    static_eval: i32,
    /// Move played from this ply into the child being searched; `None` for a
    /// null move.
    current_move: Option<Move>,
    /// Double singular extensions on the path from the root to this ply.
    doubles: u8,
}

impl Frame {
    const EMPTY: Frame = Frame {
        static_eval: i32::MIN,
        current_move: None,
        doubles: 0,
    };
}

/// Most double singular extensions allowed on one path, so that they cannot
/// feed each other without bound.
const MAX_DOUBLE_EXTENSIONS: u8 = 4;

/// Double extensions on the path to `ply`.
#[inline]
fn doubles_at(ply: u32) -> u8 {
    STACK.with(|stack| stack.borrow()[stack_index(ply)].doubles)
}

/// Set the double-extension count the children of the node at `ply - 1`
/// start from.
#[inline]
fn set_doubles(ply: u32, n: u8) {
    STACK.with(|stack| stack.borrow_mut()[stack_index(ply)].doubles = n);
}

/// Per-thread search stack indexed by ply. A young-brothers worker copies
/// the frames up to the split ply from the thread that split (see
/// `stack_path`/`restore_stack_path`), so the frames above it describe the
/// same path.
const STACK_LEN: usize = 256;
thread_local! {
    static STACK: std::cell::RefCell<[Frame; STACK_LEN]> =
        const { std::cell::RefCell::new([Frame::EMPTY; STACK_LEN]) };
}

#[inline]
fn stack_index(ply: u32) -> usize {
    (ply as usize).min(STACK_LEN - 1)
}

/// Record the move searched next from `ply` (`None`: a null move).
#[inline]
fn set_current_move(ply: u32, m: Option<Move>) {
    STACK.with(|stack| stack.borrow_mut()[stack_index(ply)].current_move = m);
}

/// Record the static evaluation at `ply` and report whether it improves on
/// the one two plies earlier (same side to move). Unknown evaluations (in
/// check, or not computed) count as improving.
#[inline]
fn record_static_eval(ply: u32, static_eval: Option<i32>) -> bool {
    STACK.with(|stack| {
        let mut stack = stack.borrow_mut();
        let p = stack_index(ply);
        stack[p].static_eval = static_eval.unwrap_or(i32::MIN);
        match static_eval {
            Some(se) if p >= 2 && stack[p - 2].static_eval != i32::MIN => {
                se > stack[p - 2].static_eval
            }
            _ => true,
        }
    })
}

/// The frames from the root to `ply`, for a worker thread that continues
/// this path.
fn stack_path(ply: u32) -> Vec<Frame> {
    STACK.with(|stack| stack.borrow()[..=stack_index(ply)].to_vec())
}

/// Install a path taken with `stack_path` on this thread.
fn restore_stack_path(path: &[Frame]) {
    STACK.with(|stack| stack.borrow_mut()[..path.len()].copy_from_slice(path));
}

/// The earlier moves a node's history lookups are keyed on: the opponent's
/// last move and the side's own moves two and four plies back.
#[derive(Clone, Copy, Default)]
struct ContMoves {
    prev: Option<Move>,
    own2: Option<Move>,
    own4: Option<Move>,
}

impl ContMoves {
    /// The moves before the node at `ply`; `prev` is its parent's move.
    #[inline]
    fn at(ply: u32, prev: Option<Move>) -> Self {
        let back = |n: u32| {
            ply.checked_sub(n)
                .and_then(|q| STACK.with(|stack| stack.borrow()[stack_index(q)].current_move))
        };
        ContMoves {
            prev,
            own2: back(2),
            own4: back(4),
        }
    }

    /// Continuation part of a quiet move's ordering score.
    #[inline]
    fn score(&self, history: &HistoryTable, stm: Color, m: Move) -> i32 {
        history.cont_get(stm, self.prev, m)
            + history.follow_get(stm, self.own2, m) * p::CONT2_WEIGHT() / 16
            + history.follow_get(stm, self.own4, m) * p::CONT4_WEIGHT() / 16
    }

    /// Add `delta` to the continuation entries of `m`.
    #[inline]
    fn update(&self, history: &HistoryTable, stm: Color, m: Move, delta: i32) {
        history.cont_add(stm, self.prev, m, delta);
        history.follow_add(stm, self.own2, m, delta * p::CONT2_UPDATE() / 16);
        history.follow_add(stm, self.own4, m, delta * p::CONT4_UPDATE() / 16);
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
    if let Some(diagnostics) = state.counters() {
        diagnostics.alpha_beta_calls.fetch_add(1, Ordering::Relaxed);
    }
    if let Some(outcome) = history.outcome_at_current_position() {
        return repetition_score(outcome, board.side_to_move, ply);
    }
    if state.budget.tick() {
        return 0;
    }

    // Mate distance pruning: tighten window — we can't improve beyond the nearest mate
    alpha = alpha.max(-(MATE_SCORE - ply as i32));
    let beta = beta.min(MATE_SCORE - ply as i32);
    if alpha >= beta {
        return alpha;
    }

    if depth == 0 {
        return quiescence(state, board, alpha, beta, ply, 0, known_in_check, history);
    }

    // TT probe
    let hash = board.hash();
    let orig_alpha = alpha;
    let tt = TtProbe::at(state, hash, depth, ply, alpha, beta, skip_move.is_some());
    if let Some(score) = tt.cutoff {
        return score;
    }
    alpha = tt.alpha;
    let tt_mv = tt.mv;
    let tt_se_score = tt.se_score;
    let tt_se_depth = tt.se_depth;

    // Internal Iterative Reduction: no TT move → move ordering is poor, search shallower
    let depth = if tt_mv.is_none() && depth >= 4 {
        depth - 1
    } else {
        depth
    };
    let stm = board.side_to_move;
    let mut node = NodeKey {
        hash,
        depth,
        ply,
        skip_move,
        corr: None,
        enemy: board.occ_for(stm.flip()),
    };

    // Countermove: best quiet response to the opponent's previous move
    let countermove = prev_mv.and_then(|pm| state.countermoves.get(stm.flip(), pm));

    // Static eval — computed once per node for RFP, razoring and futility.
    // Skipped when in check (position is not "quiet") or deeper than
    // STATIC_EVAL_MAX_DEPTH (7 by default; unused there).
    let in_check = known_in_check.unwrap_or_else(|| is_in_check(board, stm));
    // A mate in one for the side to move ends the node. Checking it here also
    // makes null-move and reduced searches see the opponent's mating threats.
    // A position with a TT entry was scanned when that entry was stored (a
    // mate in one returns before storing), so the scan is skipped there.
    if !in_check && skip_move.is_none() && !tt.hit && mate_in_one(board).is_some() {
        return MATE_SCORE - (ply as i32 + 1);
    }
    let raw_eval: Option<i32> = if !in_check && depth <= p::STATIC_EVAL_MAX_DEPTH() as u32 {
        Some(evaluate_for_search(state, board))
    } else {
        None
    };
    // The static evaluation corrected by what earlier searches found in
    // positions with the same pawns, hands or king squares.
    let static_eval = raw_eval.map(|se| {
        let keys = CorrKeys::of(board, stm);
        let corrected =
            (se + state.history.correction(keys)).clamp(-(MATE_SCORE - 2000), MATE_SCORE - 2000);
        node.corr = Some((keys, corrected));
        corrected
    });
    // Improving: the static eval beats the one two plies earlier (same side
    // to move). Unknown evals (in check, or not computed) count as improving.
    let improving = record_static_eval(ply, static_eval);
    let doubles = doubles_at(ply);
    set_doubles(ply + 1, doubles);

    // Reverse Futility Pruning: if a rough lower bound already beats beta, return early.
    if let Some(se) = static_eval
        && depth <= p::RFP_MAX_DEPTH() as u32
        && beta.abs() < MATE_SCORE - 1000
        && se
            - (p::RFP_MARGIN()
                - if improving {
                    p::RFP_IMPROVING_BONUS()
                } else {
                    0
                })
                * depth as i32
            >= beta
    {
        return se;
    }

    // Razoring: far below alpha at shallow non-PV depth, trust quiescence.
    if let Some(se) = static_eval
        && depth <= 2
        && beta - alpha == 1
        && alpha.abs() < MATE_SCORE - 1000
        && se + p::RAZOR_MARGIN_BASE() + p::RAZOR_MARGIN_PER_DEPTH() * depth as i32 <= alpha
    {
        let v = quiescence(
            state,
            board,
            alpha,
            alpha + 1,
            ply,
            0,
            Some(in_check),
            history,
        );
        if v <= alpha {
            return v;
        }
    }

    // ProbCut (not inside a singular search).
    if depth >= p::PC_MIN_DEPTH() as u32
        && !in_check
        && beta.abs() < MATE_SCORE - 1000
        && skip_move.is_none()
        && let Some(score) = probcut(state, board, beta, depth, ply, history)
    {
        return score;
    }

    // Null move pruning.
    if state.pruning.null_move
        && can_null
        && depth > p::NMP_R() as u32
        && beta.abs() < MATE_SCORE - 1000
        && !in_check
        && nmp_eval_allows(static_eval, beta)
        && let Some(score) = null_move_pruning(state, board, beta, depth, ply, prev_mv, history)
    {
        return score;
    }

    let killers = state.killers.get(ply as usize);
    // With a TT move, only that move is put first; the rest is ordered after
    // it fails to cut, since most cutoffs come from the first move.
    let mut deferred_order = false;
    let mut move_buffer = {
        let _timer = state.timer(|d| &d.movegen_order_ns);
        let mut move_buffer = {
            let _generate_timer = state.timer(|d| &d.movegen_generate_ns);
            MoveBuffer::legal_with_in_check(board, in_check)
        };
        if move_buffer.is_empty() {
            return -(MATE_SCORE - ply as i32); // shorter mate = higher score for the mating side
        }
        // The promoting twin of a skipped move is always in the list, so the
        // list cannot become empty here.
        if p::SKIP_NONPROMO() != 0 {
            move_buffer
                .as_mut_list()
                .retain(|m| !useless_non_promotion(*m, stm));
        }

        {
            let _order_timer = state.timer(|d| &d.move_order_ns);
            let list = move_buffer.as_mut_list().as_mut_slice();
            let tt_pos = tt_mv
                .filter(|_| skip_move.is_none())
                .and_then(|t| list.iter().position(|&m| m == t));
            if let Some(k) = tt_pos {
                list[..=k].rotate_right(1);
                deferred_order = true;
            } else {
                order_moves_in_place(
                    board,
                    list,
                    tt_mv,
                    killers,
                    countermove,
                    ContMoves::at(ply, prev_mv),
                    &state.history,
                    stm,
                    state.counters(),
                );
            }
        }
        move_buffer
    };

    // For singular search: filter out the excluded move (rare, only at depth >= SE_MIN_DEPTH / 2)
    if let Some(skip) = skip_move {
        move_buffer.as_mut_list().retain(|m| *m != skip);
    }
    let ordered = move_buffer.as_slice();
    if ordered.is_empty() {
        return alpha;
    } // all moves excluded (shouldn't happen in practice)

    let is_pv = beta - alpha > 1;

    // Singular Extension: check whether the TT move is clearly the best in this position.
    // If all other moves fail below (tt_score - SE_MARGIN), the TT move is "singular" and
    // we extend its search by one ply.
    let sing_ext = if let Some(se_score) = tt_se_score.filter(|_| {
        skip_move.is_none()
            && depth >= p::SE_MIN_DEPTH() as u32
            && !in_check
            && tt_mv.is_some()
            && tt_se_depth >= (depth as u8).saturating_sub(3)
    }) {
        let se_beta =
            (se_score - p::SE_MARGIN() - p::SE_MARGIN_PER_DEPTH() * depth as i32).max(alpha);
        let sval = alpha_beta(
            state,
            board,
            se_beta - 1,
            se_beta,
            depth / 2,
            ply,
            false,
            prev_mv,
            tt_mv,
            Some(in_check),
            history,
        );
        // 1 if the TT move is singular; 2 when clearly so at a non-PV node.
        if !is_pv && p::MULTICUT() != 0 && sval >= se_beta && se_beta >= beta {
            return se_beta;
        }
        if sval < se_beta {
            let double = p::SE_DOUBLE_MARGIN();
            1 + u32::from(
                double > 0 && !is_pv && doubles < MAX_DOUBLE_EXTENSIONS && sval < se_beta - double,
            )
        } else {
            0
        }
    } else {
        0
    };

    // Quiet moves tried so far — used to apply history malus on beta cutoff.
    let enemy = board.occ_for(stm.flip());
    let mut tried_quiet: Vec<Move> = Vec::new();
    let mut tried_captures: Vec<Move> = Vec::new();
    let lmr_ctx = LmrContext {
        pv: is_pv,
        improving,
        cont: ContMoves::at(ply, prev_mv),
    };

    // ---------- First child: always sequential ----------
    let first_move = ordered[0];
    set_current_move(ply, Some(first_move));
    let (tok, child_in_check, first_history) = play(board, first_move, history);
    // Apply singular extension to the TT move (ordered[0] when tt_mv is set)
    let first_ext = if tt_mv.is_some_and(|t| t == first_move) {
        sing_ext
    } else {
        0
    };
    set_doubles(ply + 1, doubles + u8::from(first_ext == 2));
    let score0 = -alpha_beta(
        state,
        board,
        -beta,
        -alpha,
        (depth - 1) + first_ext,
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
    // Track first_move for malus if it didn't cut off
    if !enemy.contains(first_move.to) && !first_move.promote {
        tried_quiet.push(first_move);
    } else if enemy.contains(first_move.to) {
        tried_captures.push(first_move);
    }

    if deferred_order {
        order_moves_in_place(
            board,
            &mut move_buffer.as_mut_list().as_mut_slice()[1..],
            None,
            killers,
            countermove,
            ContMoves::at(ply, prev_mv),
            &state.history,
            stm,
            state.counters(),
        );
    }
    let ordered = move_buffer.as_slice();
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
    let seq_start = if depth >= MIN_SPLIT_DEPTH && rayon::current_num_threads() > 1 {
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
        for (j, &m) in rest[seq_start..].iter().enumerate() {
            let i = seq_start + j;
            if state.budget.should_abort() {
                break;
            }

            let is_capture = m.from.is_some() && enemy.contains(m.to);
            // Drops are quiet too: they are the majority of shogi moves, and
            // excluding them exempted most late moves from futility pruning.
            let is_quiet = !is_capture && !m.promote;

            let late = LateMoveNode {
                depth,
                alpha,
                beta,
                in_check,
                static_eval,
                best_score,
                improving,
                history: if is_quiet && p::HP_MAX_DEPTH() > 0 {
                    lmr_ctx.history_of(&state.history, stm, m)
                } else {
                    0
                },
            };
            if late.prunes(board, m, is_quiet, i + 1) {
                continue;
            }

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
            let check_cap = check_reduction_cap(board, m, reduce);
            set_current_move(ply, Some(m));
            let (tok, child_in_check, child_history) = play(board, m, history);
            // Checks are not extended (every check extension variant lost
            // depth for nothing in shogi's check-rich trees); they are only
            // protected from reductions beyond `CHECK_R_MAX` plies.
            let reduce = if child_in_check {
                reduce.min(check_cap)
            } else {
                reduce
            };
            let s = pvs_child(
                state,
                board,
                alpha,
                beta,
                depth,
                reduce,
                ply,
                m,
                child_in_check,
                &child_history,
            );
            board.undo_move_for_search(tok);

            if s > best_score {
                best_score = s;
                best_move = Some(m);
            }
            if s >= beta {
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
    node.store(state, best_score, bound, best_move);
    best_score
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
    mut alpha: i32,
    beta: i32,
    ply: u32,
    qply: u32,
    known_in_check: Option<bool>,
    history: &SearchHistory<'_>,
) -> i32 {
    let _quiescence_timer = state.timer(|d| &d.quiescence_inclusive_ns);
    if let Some(diagnostics) = state.counters() {
        diagnostics.quiescence_calls.fetch_add(1, Ordering::Relaxed);
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
    if qply == 0
        && let Some(entry) = probe_tt_for_search(state, hash)
        && entry.depth == 0
    {
        let adj = score_from_tt(entry.score, ply);
        tt_mv = entry.mv;
        match entry.bound {
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
    // Bounds below are relative to the post-probe window. A non-cutting lower
    // bound may have raised alpha and must not make the eventual result look
    // exact merely because the cached bound was present.
    let orig_alpha = alpha;

    let in_check = known_in_check.unwrap_or_else(|| is_in_check(board, board.side_to_move));

    // Stand-pat and delta pruning only apply when not in check.
    // In check the side to move has no quiet option, so stand-pat is invalid.
    if !in_check {
        // Standing pat must not hide a mate in one at the horizon.
        if qply == 0 && mate_in_one(board).is_some() {
            return MATE_SCORE - (ply as i32 + 1);
        }
        let stand_pat = evaluate_for_search(state, board);
        if stand_pat >= beta {
            if qply == 0 && !state.budget.should_abort() {
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
            return stand_pat;
        }
        if stand_pat > alpha {
            alpha = stand_pat;
        }
        // Delta Pruning: if even the best possible capture+promotion cannot improve alpha, skip.
        // Max gain = Ryu capture (1300) + Fu→Tokin promotion bonus (500) = 1800cp.
        const DELTA_MARGIN: i32 = 1_800;
        if stand_pat + DELTA_MARGIN < alpha {
            if qply == 0 && !state.budget.should_abort() {
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
        let score = if in_check {
            -MATE_SCORE + ply as i32 // checkmate
        } else {
            alpha
        };
        if qply == 0 && !state.budget.should_abort() {
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
    for &m in move_buffer.as_slice() {
        if p::SKIP_NONPROMO() != 0 && useless_non_promotion(m, stm) {
            continue;
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
        let (tok, child_in_check, child_history) = play(board, m, history);
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
        board.undo_move_for_search(tok);

        if state.budget.should_abort() {
            return 0;
        }
        if score >= beta {
            if qply == 0 && !state.budget.should_abort() {
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

    // Quiet checks: at the shallowest qsearch level, search a handful of
    // non-capture moves that give check and have non-negative SEE.
    // Drops that give check (e.g. 飛打ち王手) are included naturally.
    if QSEARCH_CHECKS && !in_check && qply == 0 {
        const MAX_QCHECKS: usize = 4;
        let mut qcheck_count = 0;
        let qchecks = {
            let _timer = state.timer(|d| &d.movegen_order_ns);
            let mut qchecks = {
                let _generate_timer = state.timer(|d| &d.movegen_generate_ns);
                MoveBuffer::legal_with_in_check(board, false)
            };
            {
                let _order_timer = state.timer(|d| &d.move_order_ns);
                let mut tt_first = |m: &Move| i32::from(Some(*m) != tt_mv);
                let list = qchecks.as_mut_list().as_mut_slice();
                sort_moves_by_cached_key(list, &mut tt_first);
            }
            qchecks
        };
        for &m in qchecks.as_slice() {
            // Skip captures — already handled above
            if m.from.is_some() && board.piece_at(m.to).is_some() {
                continue;
            }
            // Test if this move gives check, then apply safety filter — combined in one do/undo
            let tok = board.do_move_for_search(m);
            let gives_check = is_in_check(board, board.side_to_move);
            if !gives_check {
                board.undo_move_for_search(tok);
                continue;
            }
            // Safety: skip if the checking piece can be immediately recaptured at a loss.
            // Promoting moves are exempt (promotion value offsets the risk).
            if !m.promote {
                let mover_val = PIECE_VALUE[m.piece_kind.index()];
                let captures = MoveBuffer::captures(board);
                let unsafe_check = captures
                    .as_slice()
                    .iter()
                    .filter(|r| r.to == m.to)
                    .any(|r| PIECE_VALUE[r.piece_kind.index()] < mover_val);
                if unsafe_check {
                    board.undo_move_for_search(tok);
                    continue;
                }
            }
            let mover = board.side_to_move.flip();
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
            board.undo_move_for_search(tok);

            if state.budget.should_abort() {
                return 0;
            }
            if score >= beta {
                if !state.budget.should_abort() {
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
            qcheck_count += 1;
            if qcheck_count >= MAX_QCHECKS {
                break;
            }
        }
    }

    if qply == 0 && !state.budget.should_abort() {
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

        let pv = extract_pv(&self.tt, board, best_move, done_depth);
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
) {
    // A singular-extension verification search excludes one legal move from
    // the move set. Its result is therefore not a valid TT result for the
    // unrestricted position. PR #4 fixes the read-side self-hit; keep the
    // corresponding write-side invariant explicit rather than relying on
    // Tt's incidental depth-preferred replacement policy.
    if skip_move.is_some() {
        return;
    }
    store_tt_for_search(
        state,
        hash,
        TtEntry {
            score: score_to_tt(score, ply),
            depth: depth as u8,
            bound,
            mv,
        },
    );
}

/// The parent's state that decides whether a late move is skipped.
#[derive(Clone, Copy)]
struct LateMoveNode {
    depth: u32,
    alpha: i32,
    beta: i32,
    in_check: bool,
    static_eval: Option<i32>,
    best_score: i32,
    improving: bool,
    /// History of the move (butterfly + continuation + follow-up), computed
    /// only when history pruning is enabled.
    history: i32,
}

impl LateMoveNode {
    /// Whether `m`, the `move_number`-th move of the node (1-based), is
    /// pruned before it is searched.
    ///
    /// - Depth 1: quiet moves whose static eval plus a margin stays below
    ///   alpha (futility).
    /// - Shallow non-PV nodes, quiet moves that do not give direct check:
    ///   move-count pruning and static-eval futility. Checks (mostly drops)
    ///   stay: they are shogi's main tactical resource.
    /// - Shallow non-PV nodes, captures that do not give direct check and
    ///   clearly lose material by SEE.
    #[inline]
    fn prunes(&self, board: &Board, m: Move, is_quiet: bool, move_number: usize) -> bool {
        let depth = self.depth;
        if depth == 1
            && let Some(se) = self.static_eval
            && is_quiet
            && se + p::FUTILITY_MARGIN() < self.alpha
        {
            return true;
        }
        let shallow_non_pv = self.beta - self.alpha == 1
            && !self.in_check
            && depth <= p::SHALLOW_PRUNE_MAX_DEPTH() as u32
            && self.best_score > -(MATE_SCORE - 1000);
        if !shallow_non_pv {
            return false;
        }
        let exempt_check = || check_exempt_from_pruning(board, m);
        if is_quiet {
            (move_number as u32 >= lmp_limit(depth, self.improving)
                || (depth >= 2
                    && self.static_eval.is_some_and(|se| {
                        se + p::SHALLOW_FUTILITY_BASE()
                            + p::SHALLOW_FUTILITY_PER_DEPTH() * depth as i32
                            <= self.alpha
                    })))
                && !exempt_check()
                || (depth <= p::HP_MAX_DEPTH() as u32
                    && self.history < -p::HP_MARGIN() * depth as i32
                    && !exempt_check())
        } else {
            m.from.is_some()
                && board.piece_at(m.to).is_some()
                && crate::movegen::see_swap(board, m) < -120 * depth as i32
                && !exempt_check()
        }
    }
}

/// Whether shallow pruning must keep `m` because it gives check: never
/// (`CHECK_PRUNE` 2), only when the checking piece is not hung (1), or
/// always (0, the historical behaviour).
#[inline]
fn check_exempt_from_pruning(board: &Board, m: Move) -> bool {
    match p::CHECK_PRUNE() {
        0 => move_gives_direct_check(board, m),
        1 => move_gives_direct_check(board, m) && crate::movegen::see_exchange(board, m) >= 0,
        _ => false,
    }
}

/// The late move reduction cap of a move that turns out to give check,
/// decided on the board before the move: `CHECK_R_MAX`, or
/// `CHECK_R_MAX_BAD` when the move hangs the moved piece.
#[inline]
fn check_reduction_cap(board: &Board, m: Move, reduce: u32) -> u32 {
    let (good, bad) = (p::CHECK_R_MAX() as u32, p::CHECK_R_MAX_BAD() as u32);
    if reduce <= good.min(bad) || good == bad {
        return good;
    }
    if crate::movegen::see_exchange(board, m) < 0 {
        bad
    } else {
        good
    }
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
        ContMoves::at(ply, prev_mv).update(history, stm, m, history_bonus(depth));
        if let Some(pm) = prev_mv {
            countermoves.update(stm.flip(), pm, m);
        }
    }
}

/// Offset that places every non-TT move after the TT move in the qsearch
/// order key; `qsearch_order_key` stays far below it.
const QSEARCH_TT_ORDER_OFFSET: i32 = 1_000_000;

/// Cheap MVV-LVA-style ordering key for quiescence: victim (+ promotion gain)
/// minus the attacker value. No board mutation, no recursion — fast enough to
/// call on every move at every qsearch node. Non-captures score by promotion
/// gain alone (0 for plain quiet moves).
#[inline]
fn qsearch_order_key(board: &Board, m: Move) -> i32 {
    let victim = board
        .piece_at(m.to)
        .map_or(0, |c| PIECE_VALUE[c.kind.index()]);
    let promo = if m.promote {
        PIECE_VALUE[m.piece_kind.promoted().index()] - PIECE_VALUE[m.piece_kind.index()]
    } else {
        0
    };
    victim + promo - PIECE_VALUE[m.piece_kind.index()]
}

/// Compute Late Move Reduction amount for a move.
/// Returns 0 if the move should not be reduced.
#[inline]
#[allow(clippy::too_many_arguments)]
fn lmr_reduce(
    board: &Board,
    m: Move,
    move_idx: usize,
    depth: u32,
    killers: &[Option<Move>; 2],
    tt_mv: Option<Move>,
    history: &HistoryTable,
    stm: Color,
) -> u32 {
    if depth < 3 {
        return 0;
    }
    if move_idx < 2 {
        return 0;
    }
    // Don't reduce captures or promotions
    if m.from.is_some_and(|_| board.piece_at(m.to).is_some()) {
        return 0;
    }
    if m.promote {
        return 0;
    }
    // Don't reduce TT move or killers
    if tt_mv.is_some_and(|t| t == m) {
        return 0;
    }
    if killers[0].is_some_and(|k| k == m) {
        return 0;
    }
    if killers[1].is_some_and(|k| k == m) {
        return 0;
    }
    let mut r = lmr_base_reduction(depth, move_idx);
    // History adjustment: well-tried quiet moves get less reduction; poorly-tried get more.
    let hist = history.get(stm, m);
    if hist > p::LMR_HIST() {
        r = r.saturating_sub(1);
    } else if hist < -p::LMR_HIST() && depth >= 5 {
        r += 1;
    }
    r
}

/// Shallow move-count pruning: quiet moves from this number on are pruned.
#[inline]
fn lmp_limit(depth: u32, improving: bool) -> u32 {
    let base = p::LMP_BASE() as u32 + depth * depth;
    if improving {
        base * p::LMP_IMPROVING_MUL() as u32 / 16
    } else {
        base
    }
}

/// What late move reductions know about the node beyond the move itself.
#[derive(Clone, Copy)]
struct LmrContext {
    pv: bool,
    improving: bool,
    cont: ContMoves,
}

impl LmrContext {
    /// History of `m` at this node: butterfly + continuation + two-ply
    /// follow-up.
    #[inline]
    fn history_of(&self, history: &HistoryTable, stm: Color, m: Move) -> i32 {
        history.get(stm, m)
            + history.cont_get(stm, self.cont.prev, m)
            + history.follow_get(stm, self.cont.own2, m)
    }
}

/// Adjust the base reduction `r` of a reducible move (0 stays 0). The
/// adjustments are summed in sixteenths of a ply and rounded, so that the
/// tuner can move them in small steps.
#[inline]
fn lmr_adjust(r: u32, ctx: &LmrContext, history: &HistoryTable, stm: Color, m: Move) -> u32 {
    if r == 0 {
        return 0;
    }
    let mut r16 = r as i32 * 16;
    let scale = p::LMR_STAT_SCALE();
    if scale != 0 {
        r16 -= ctx.history_of(history, stm, m) * scale / (1 << 13);
    }
    if ctx.pv {
        r16 -= p::LMR_PV_LESS16();
    }
    if !ctx.improving {
        r16 += p::LMR_NOT_IMPROVING16();
    }
    ((r16 + 8) / 16).max(0) as u32
}

#[inline]
fn lmr_base_reduction(depth: u32, move_idx: usize) -> u32 {
    const DEPTHS: usize = 128;
    const MOVES: usize = 600;
    if (depth as usize) < DEPTHS && move_idx < MOVES {
        let table = LMR_REDUCTION_TABLE.get_or_init(|| {
            let move_lns: [f32; MOVES] = std::array::from_fn(|index| (index.max(1) as f32).ln());
            (0..DEPTHS)
                .map(|d| {
                    let depth_ln = (d.max(1) as f32).ln();
                    std::array::from_fn(|index| lmr_formula(depth_ln, move_lns[index]) as u8)
                })
                .collect::<Vec<_>>()
                .into_boxed_slice()
        });
        table[depth as usize][move_idx] as u32
    } else {
        lmr_formula((depth as f32).ln(), (move_idx as f32).ln()) as u32
    }
}

/// Base late move reduction from the logarithms of depth and move number:
/// `BASE + ln(depth) * ln(move) / DIV` (1 + ... / 2 by default). The table
/// above is built on first use, so tuning options must be set before the
/// first search.
#[inline]
fn lmr_formula(depth_ln: f32, move_ln: f32) -> f32 {
    p::LMR_BASE16() as f32 / 16.0 + depth_ln * move_ln / (p::LMR_DIV100() as f32 / 100.0)
}

#[allow(clippy::too_many_arguments)]
fn order_moves_in_place(
    board: &mut Board,
    moves: &mut [Move],
    tt_mv: Option<Move>,
    killers: [Option<Move>; 2],
    countermove: Option<Move>,
    cont: ContMoves,
    history: &HistoryTable,
    stm: Color,
    diagnostics: Option<&SearchDiagnostics>,
) {
    // sort_by_cached_key computes the key exactly once per element, preventing
    // races where AtomicI32 history values change between comparisons in rayon threads.
    let mut key = |m: &Move| {
        let _score_timer = ProfileTimer::new(
            diagnostics.and_then(|diagnostics| diagnostics.timed(&diagnostics.move_order_score_ns)),
        );
        let m = *m;
        if tt_mv.is_some_and(|t| t == m) {
            if let Some(d) = diagnostics {
                d.order_tt.fetch_add(1, Ordering::Relaxed);
            }
            return i32::MIN;
        } // 1. TT move first

        // 2. Captures ordered by SEE (2-ply Static Exchange Evaluation)
        //    Winning/equal (see >= 0): searched before killers
        //    Losing (see < 0): searched after quiet moves
        if m.from.is_some()
            && let Some(victim) = board.piece_at(m.to)
        {
            let see = crate::movegen::see_swap(board, m);
            return if see >= 0 {
                // Best captures first; the capture history reorders them
                // inside the band above the killers.
                let hist = history.capture_get(stm, m, victim.kind) * p::CAPT_ORDER_WEIGHT() / 128;
                -(10_000 + (see + hist).clamp(0, 1_999))
            } else {
                10_000 - see // range: 10_001 to 11_300 (losing captures last)
            };
        }

        if killers[0].is_some_and(|k| k == m) {
            if let Some(d) = diagnostics {
                d.order_killer.fetch_add(1, Ordering::Relaxed);
            }
            return -9_100;
        } // 3. Killer 0
        if killers[1].is_some_and(|k| k == m) {
            if let Some(d) = diagnostics {
                d.order_killer.fetch_add(1, Ordering::Relaxed);
            }
            return -9_050;
        } // 4. Killer 1
        if countermove.is_some_and(|cm| cm == m) {
            if let Some(d) = diagnostics {
                d.order_countermove.fetch_add(1, Ordering::Relaxed);
            }
            return -9_000;
        } // 5. Countermove

        // 6. Remaining quiet moves by history score
        if let Some(d) = diagnostics {
            d.order_history.fetch_add(1, Ordering::Relaxed);
        }
        let mut score = history.get(stm, m) + cont.score(history, stm, m);
        if p::SAFE_CHECK_BONUS() > 0
            && move_gives_direct_check(board, m)
            && crate::movegen::see_swap(board, m) >= -75
        {
            score += p::SAFE_CHECK_BONUS();
        }
        let score = score.clamp(-9_000, 9_000);
        -(-8_000 + score)
    };
    let _sort_timer = ProfileTimer::new(
        diagnostics.and_then(|diagnostics| diagnostics.timed(&diagnostics.move_order_sort_ns)),
    );
    sort_moves_by_cached_key(moves, &mut key);
}

thread_local! {
    /// Reused key and move scratch for `sort_moves_by_cached_key`.
    static ORDER_SCRATCH: std::cell::RefCell<(Vec<i64>, Vec<Move>)> =
        const { std::cell::RefCell::new((Vec::new(), Vec::new())) };
}

/// Stable ordering by an i32 key computed once per move, as
/// `slice::sort_by_cached_key` would order it, without its per-call heap
/// allocation. Short lists use an insertion sort; longer ones sort packed
/// `(key, index)` values in reused per-thread scratch, where the index keeps
/// equal keys in their original order.
#[inline]
fn sort_moves_by_cached_key<F>(moves: &mut [Move], key: &mut F)
where
    F: FnMut(&Move) -> i32,
{
    const INSERTION_MAX: usize = 16;
    if moves.len() < 2 {
        return;
    }
    if moves.len() <= INSERTION_MAX {
        sort_by_cached_i32_key_small(moves, key);
        return;
    }
    debug_assert!(moves.len() <= usize::from(u16::MAX));
    ORDER_SCRATCH.with(|scratch| {
        let mut scratch = scratch.borrow_mut();
        let (keys, copy) = &mut *scratch;
        keys.clear();
        keys.extend(
            moves
                .iter()
                .enumerate()
                .map(|(index, m)| (i64::from(key(m)) << 16) | index as i64),
        );
        keys.sort_unstable();
        copy.clear();
        copy.extend_from_slice(moves);
        for (slot, &packed) in moves.iter_mut().zip(keys.iter()) {
            *slot = copy[(packed & 0xFFFF) as usize];
        }
    });
}

/// Stable cached-key ordering without a temporary heap allocation for the
/// move counts normally seen at a search node.
#[inline]
fn sort_by_cached_i32_key_small<F>(moves: &mut [Move], key: &mut F)
where
    F: FnMut(&Move) -> i32,
{
    const CAPACITY: usize = 64;
    debug_assert!(moves.len() <= CAPACITY);
    let placeholder = Move::drop(Square::from_index(0), PieceKind::Fu);
    let mut sorted = [placeholder; CAPACITY];
    let mut keys = [0i32; CAPACITY];

    for (index, &mv) in moves.iter().enumerate() {
        let mv_key = key(&mv);
        let mut position = index;
        while position > 0 && mv_key < keys[position - 1] {
            sorted[position] = sorted[position - 1];
            keys[position] = keys[position - 1];
            position -= 1;
        }
        sorted[position] = mv;
        keys[position] = mv_key;
    }
    moves.copy_from_slice(&sorted[..moves.len()]);
}

#[cfg(test)]
mod see_tests {
    use super::*;
    use crate::board::Board;
    use std::thread;
    use std::time::Instant;

    #[test]
    fn eval_cache_round_trips_signed_scores_and_rejects_collisions() {
        let cache = EvalCache::new();
        let first = 0x1234_5678_0000_0042;
        let collision = first ^ (1 << 32);
        assert_eq!(cache.probe(first), None);
        cache.store(first, -12_345);
        assert_eq!(cache.probe(first), Some(-12_345));
        assert_eq!(cache.probe(collision), None);
        cache.store(collision, 67_890);
        assert_eq!(cache.probe(collision), Some(67_890));
        assert_eq!(cache.probe(first), None);
        cache.clear();
        assert_eq!(cache.probe(collision), None);
    }

    #[test]
    fn eval_cache_concurrent_collision_never_returns_another_hash_score() {
        let cache = Arc::new(EvalCache::new());
        thread::scope(|scope| {
            for worker in 0..4u64 {
                let cache = cache.clone();
                scope.spawn(move || {
                    let hash = 0xfeed_0000_0000_002au64 ^ (worker << 32);
                    let score = worker as i32 * 1_000 - 1_500;
                    for _ in 0..10_000 {
                        cache.store(hash, score);
                        if let Some(observed) = cache.probe(hash) {
                            assert_eq!(observed, score);
                        }
                    }
                });
            }
        });
    }

    #[test]
    fn eval_cache_stays_within_preregistered_memory_budget() {
        assert!(std::mem::size_of::<EvalCacheSlot>() * EVAL_CACHE_ENTRIES <= 2 * 1024 * 1024);
    }

    #[test]
    fn lmr_table_is_bit_exact_with_previous_formula() {
        for depth in 3..128u32 {
            for move_idx in 2..600usize {
                let expected = (p::LMR_BASE16() as f32 / 16.0
                    + (depth as f32).ln() * (move_idx as f32).ln()
                        / (p::LMR_DIV100() as f32 / 100.0)) as u32;
                assert_eq!(lmr_base_reduction(depth, move_idx), expected);
            }
        }
    }

    #[test]
    fn lmr_protects_early_special_and_tactical_moves() {
        let mut board = Board::startpos();
        let moves = generate_legal_moves(&mut board);
        let quiet = moves[0];
        let history = HistoryTable::new();
        let no_killers = [None, None];

        assert_eq!(
            lmr_reduce(
                &board,
                quiet,
                1,
                5,
                &no_killers,
                None,
                &history,
                Color::Black
            ),
            0
        );
        assert!(
            lmr_reduce(
                &board,
                quiet,
                2,
                5,
                &no_killers,
                None,
                &history,
                Color::Black
            ) > 0
        );

        let killers = KillerTable::new();
        killers.add(0, quiet);
        assert_eq!(
            lmr_reduce(
                &board,
                quiet,
                5,
                8,
                &killers.get(0),
                None,
                &history,
                Color::Black
            ),
            0
        );
        assert_eq!(
            lmr_reduce(
                &board,
                quiet,
                5,
                8,
                &no_killers,
                Some(quiet),
                &history,
                Color::Black
            ),
            0
        );

        let promoted = Move::normal(quiet.from.unwrap(), quiet.to, quiet.piece_kind, true);
        assert_eq!(
            lmr_reduce(
                &board,
                promoted,
                5,
                8,
                &no_killers,
                None,
                &history,
                Color::Black
            ),
            0
        );

        let mut capture_board = Board::from_sfen("k8/9/9/9/4p4/9/4R4/9/8K b - 1").unwrap();
        let capture = generate_legal_captures(&mut capture_board)
            .into_iter()
            .find(|m| m.to == Square::from_shogi(5, 5))
            .expect("the tactical fixture must contain a capture");
        assert_eq!(
            lmr_reduce(
                &capture_board,
                capture,
                5,
                8,
                &no_killers,
                None,
                &history,
                Color::Black,
            ),
            0
        );
    }

    #[test]
    fn futility_pruning_reduces_only_late_quiet_work_at_depth_one() {
        let fresh = |tt| {
            Arc::new(SearchState {
                tt,
                budget: Arc::new(Budget::new(None, None, Arc::new(AtomicBool::new(false)))),
                killers: KillerTable::new(),
                history: Arc::new(HistoryTable::new()),
                countermoves: Arc::new(CountermoveTable::new()),
                diagnostics: None,
                eval_cache: None,
                pruning: PruningConfig::default(),
            })
        };
        let mut pruned_board = Board::startpos();
        let pruned_history = PositionHistory::initial(pruned_board.hash());
        let pruned_hash = pruned_board.hash();
        let pruned_state = fresh(Tt::new(1));
        let _ = alpha_beta(
            &pruned_state,
            &mut pruned_board,
            100_000,
            100_001,
            1,
            0,
            true,
            None,
            None,
            Some(false),
            &SearchHistory::root(&pruned_history),
        );
        let pruned_nodes = pruned_state.budget.nodes();

        let mut full_board = Board::startpos();
        let full_history = PositionHistory::initial(full_board.hash());
        let full_state = fresh(Tt::new(1));
        let _ = alpha_beta(
            &full_state,
            &mut full_board,
            NEG_INF,
            POS_INF,
            1,
            0,
            true,
            None,
            None,
            Some(false),
            &SearchHistory::root(&full_history),
        );
        assert!(pruned_nodes < full_state.budget.nodes());
        assert_eq!(pruned_board.hash(), pruned_hash);
        assert_eq!(full_board.hash(), Board::startpos().hash());
    }

    #[test]
    fn move_ordering_keeps_priority_bands_in_order() {
        let mut board = Board::startpos();
        let moves = generate_legal_moves(&mut board);
        assert!(moves.len() >= 4);
        let tt_move = moves[0];
        let killer_move = moves[1];
        let countermove = moves[2];
        let history_move = moves[3];

        let killers = KillerTable::new();
        killers.add(0, killer_move);
        let history = HistoryTable::new();
        history.update(Color::Black, history_move, 20);
        let mut ordered = vec![history_move, countermove, killer_move, tt_move];
        order_moves_in_place(
            &mut board,
            &mut ordered,
            Some(tt_move),
            killers.get(0),
            Some(countermove),
            ContMoves::default(),
            &history,
            Color::Black,
            None,
        );

        assert_eq!(
            ordered,
            vec![tt_move, killer_move, countermove, history_move]
        );
    }

    #[test]
    fn small_cached_key_sort_is_stable_and_scores_each_move_once() {
        let mut moves = vec![
            Move::drop(Square::from_index(3), PieceKind::Fu),
            Move::drop(Square::from_index(1), PieceKind::Fu),
            Move::drop(Square::from_index(2), PieceKind::Fu),
            Move::drop(Square::from_index(0), PieceKind::Fu),
        ];
        let expected = vec![moves[1], moves[3], moves[0], moves[2]];
        let mut calls = 0;
        sort_by_cached_i32_key_small(&mut moves, &mut |mv| {
            calls += 1;
            if mv.to == Square::from_index(1) || mv.to == Square::from_index(0) {
                0
            } else {
                1
            }
        });

        assert_eq!(calls, 4);
        assert_eq!(moves, expected);
    }

    #[test]
    fn root_mate_filter_prefilter_keeps_every_checking_move() {
        // The root mate filters skip moves `may_give_check` rejects; a skipped
        // move must never give check, or a mate could be missed.
        let mut state = 0x1234_5678_9ABC_DEF1u64;
        let mut rand = move |n: usize| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state % n as u64) as usize
        };
        let mut checks = 0;
        for _ in 0..60 {
            let mut board = Board::startpos();
            for _ in 0..(20 + rand(150)) {
                let moves = generate_legal_moves(&mut board);
                if moves.is_empty() {
                    break;
                }
                let discoverers = discovered_check_candidates(&board);
                for &m in &moves {
                    let mut after = board.clone();
                    after.do_move(m);
                    if is_in_check(&after, after.side_to_move) {
                        checks += 1;
                        assert!(may_give_check(&board, m, discoverers), "{m:?}");
                    }
                }
                board.do_move(moves[rand(moves.len())]);
            }
        }
        assert!(checks > 500);
    }

    #[test]
    fn diagnostics_observer_records_search_path_without_changing_result() {
        let diagnostics = Arc::new(SearchDiagnostics::new());
        let searcher = Searcher::with_diagnostics(Tt::new(1), diagnostics.clone());
        let mut board = Board::startpos();
        let result = searcher.search(
            &mut board,
            SearchConfig {
                max_depth: 2,
                // Root mate-safety now shares the hard node budget; leave
                // enough room to reach the instrumented alpha-beta path.
                node_limit: Some(20_000),
                ..SearchConfig::default()
            },
        );
        assert!(result.best_move.is_some());

        let snapshot = diagnostics.snapshot();
        assert!(snapshot.tt_probes > 0);
        assert!(snapshot.tt_hits <= snapshot.tt_probes);
        assert!(snapshot.tt_stores > 0);
        assert!(snapshot.alpha_beta_calls > 0);
        assert!(snapshot.quiescence_calls > 0);
        assert!(snapshot.static_evaluation_ns > 0);
        assert!(snapshot.tt_probe_ns > 0);
        assert!(snapshot.tt_store_ns > 0);
        assert!(snapshot.movegen_order_ns > 0);
        assert!(snapshot.movegen_generate_ns > 0);
        assert!(snapshot.move_order_ns > 0);
        assert!(snapshot.quiescence_inclusive_ns > 0);
        assert!(snapshot.root_mate_safety_ns > 0);
        assert!(
            snapshot.order_tt
                + snapshot.order_killer
                + snapshot.order_countermove
                + snapshot.order_history
                > 0
        );
        assert!(snapshot.root_mate_blunder_nodes > 0);
        assert!(snapshot.root_mate_in_one_cache_hits > 0);
        assert!(snapshot.root_mate_blunder_cache_hits > 0);
    }

    #[test]
    fn iteration_trace_keeps_only_completed_passes_and_reports_root_safety_cost() {
        let diagnostics = Arc::new(SearchDiagnostics::new());
        let searcher = Searcher::with_pruning_and_diagnostics(
            Tt::new(1),
            PruningConfig::default(),
            diagnostics,
        );
        let mut board = Board::startpos();
        let history = PositionHistory::initial(board.hash());
        let (info, trace) = searcher.search_with_history_trace(
            &mut board,
            SearchConfig {
                max_depth: 3,
                node_limit: Some(20_000),
                ..SearchConfig::default()
            },
            &history,
        );
        assert!(!trace.is_empty());
        assert_eq!(
            trace.last().map(|iteration| iteration.depth),
            Some(info.depth)
        );
        assert_eq!(
            trace.last().and_then(|iteration| iteration.best_move),
            info.best_move
        );
        assert!(
            trace
                .windows(2)
                .all(|pair| pair[0].depth < pair[1].depth && pair[0].nodes <= pair[1].nodes)
        );
        assert!(
            trace
                .last()
                .and_then(|iteration| iteration.root_mate_blunder_nodes)
                .unwrap_or(0)
                > 0
        );
        if let [first, second, ..] = trace.as_slice() {
            assert_eq!(
                first.root_mate_blunder_nodes, second.root_mate_blunder_nodes,
                "depth two must reuse complete depth-one root mate safety facts"
            );
        }
    }

    #[test]
    fn cached_root_mate_safety_preserves_the_current_move_order() {
        let mut board = Board::startpos();
        let moves = generate_legal_moves(&mut board);
        let unsafe_move = moves[0];
        let reordered = vec![moves[2], unsafe_move, moves[1]];
        assert_eq!(
            safe_root_moves(&reordered, &[unsafe_move]),
            Some(vec![moves[2], moves[1]]),
            "a cached unsafe set must filter membership without restoring stale ordering"
        );
        assert_eq!(safe_root_moves(&reordered, &[]), None);
    }

    #[test]
    fn cached_root_safety_retains_an_immediate_mate() {
        let mut board =
            Board::from_sfen("k8/2K6/9/9/4R4/9/9/9/9 b - 1").expect("mate fixture must parse");
        let info = Searcher::new(Tt::new(1)).search(
            &mut board,
            SearchConfig {
                max_depth: 3,
                node_limit: Some(10_000),
                ..SearchConfig::default()
            },
        );
        assert!(info.score >= MATE_SCORE - 1);
        assert_eq!(info.depth, 1, "root mate-in-one must finish immediately");
    }

    #[test]
    fn root_move_diagnostic_consumes_budget_and_keeps_requested_move() {
        let mut board = Board::startpos();
        let root_move = crate::sfen::move_from_usi("7g7f", &board).unwrap();
        let acc = board.acc.clone();
        let searcher = Searcher::new(Tt::new(1));
        let info = searcher.search_root_move(
            &mut board,
            SearchConfig {
                max_depth: 4,
                node_limit: Some(64),
                ..SearchConfig::default()
            },
            root_move,
        );
        assert_eq!(info.best_move, Some(root_move));
        assert_eq!(info.nodes, 64);
        assert_eq!(board.acc, acc);
        assert!(generate_legal_moves(&mut board).contains(&root_move));
    }

    #[test]
    fn root_candidate_diagnostic_is_explicit_and_restores_board() {
        let mut board = Board::startpos();
        let hash = board.hash();
        let acc = board.acc.clone();
        let first = crate::sfen::move_from_usi("7g7f", &board).unwrap();
        let second = crate::sfen::move_from_usi("2g2f", &board).unwrap();
        let illegal = Move::normal(
            Square::from_shogi(7, 7),
            Square::from_shogi(7, 5),
            PieceKind::Fu,
            false,
        );
        let searcher = Searcher::new(Tt::new(1));
        let results = searcher.search_root_candidates(
            &mut board,
            SearchConfig {
                max_depth: 2,
                node_limit: Some(32),
                ..SearchConfig::default()
            },
            &[first, first, illegal, second],
        );
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].root_move, first);
        assert_eq!(results[1].root_move, second);
        assert!(
            results
                .iter()
                .all(|result| result.info.best_move == Some(result.root_move))
        );
        assert_eq!(board.hash(), hash);
        assert_eq!(board.acc, acc);
        assert!(generate_legal_moves(&mut board).contains(&first));
        assert!(generate_legal_moves(&mut board).contains(&second));
    }

    #[test]
    fn root_move_diagnostic_rejects_illegal_move_without_mutation() {
        let mut board = Board::startpos();
        let hash = board.hash();
        let acc = board.acc.clone();
        let illegal = Move::normal(
            Square::from_shogi(7, 7),
            Square::from_shogi(7, 5),
            PieceKind::Fu,
            false,
        );
        let searcher = Searcher::new(Tt::new(1));
        let info = searcher.search_root_move(
            &mut board,
            SearchConfig {
                max_depth: 2,
                node_limit: Some(32),
                ..SearchConfig::default()
            },
            illegal,
        );
        assert_eq!(info.best_move, None);
        assert_eq!(info.depth, 0);
        assert_eq!(info.bound, SearchBound::Unknown);
        assert_eq!(info.nodes, 0);
        assert_eq!(board.hash(), hash);
        assert_eq!(board.acc, acc);
    }

    #[test]
    fn root_move_diagnostic_restores_capture_position() {
        let mut board = Board::from_sfen("k8/9/9/9/4p4/9/4R4/9/8K b - 1").unwrap();
        let hash = board.hash();
        let acc = board.acc.clone();
        let capture = generate_legal_moves(&mut board)
            .into_iter()
            .find(|candidate| candidate.to == Square::from_shogi(5, 5))
            .expect("the rook capture must be legal");
        let searcher = Searcher::new(Tt::new(1));
        let info = searcher.search_root_move(
            &mut board,
            SearchConfig {
                max_depth: 2,
                node_limit: Some(32),
                ..SearchConfig::default()
            },
            capture,
        );
        assert_eq!(info.best_move, Some(capture));
        assert_eq!(board.hash(), hash);
        assert_eq!(board.acc, acc);
        assert!(generate_legal_moves(&mut board).contains(&capture));
    }

    #[test]
    fn root_move_diagnostic_restores_promotion_position() {
        let mut board = Board::from_sfen("4k4/9/9/4P4/9/9/9/9/4K4 b - 1").unwrap();
        let hash = board.hash();
        let acc = board.acc.clone();
        let promotion = crate::sfen::move_from_usi("5d5c+", &board).unwrap();
        let searcher = Searcher::new(Tt::new(1));
        let info = searcher.search_root_move(
            &mut board,
            SearchConfig {
                max_depth: 2,
                node_limit: Some(32),
                ..SearchConfig::default()
            },
            promotion,
        );
        assert_eq!(info.best_move, Some(promotion));
        assert_eq!(board.hash(), hash);
        assert_eq!(board.acc, acc);
        assert!(generate_legal_moves(&mut board).contains(&promotion));
    }

    #[test]
    fn root_move_diagnostic_restores_drop_position() {
        let mut board = Board::from_sfen("4k4/9/9/9/9/9/9/9/4K4 b R 1").unwrap();
        let hash = board.hash();
        let acc = board.acc.clone();
        let drop = crate::sfen::move_from_usi("R*5e", &board).unwrap();
        let searcher = Searcher::new(Tt::new(1));
        let info = searcher.search_root_move(
            &mut board,
            SearchConfig {
                max_depth: 2,
                node_limit: Some(32),
                ..SearchConfig::default()
            },
            drop,
        );
        assert_eq!(info.best_move, Some(drop));
        assert_eq!(board.hash(), hash);
        assert_eq!(board.acc, acc);
        assert!(generate_legal_moves(&mut board).contains(&drop));
    }

    // Regression: a search with a tiny hard time limit and a huge max_depth must
    // terminate near the limit, not run to depth 99. Before the watchdog fix the
    // speculative tasks could saturate the rayon pool and starve the time check,
    // hanging the move indefinitely — this test would then never return.
    #[test]
    fn search_respects_time_limit() {
        use crate::tt::Tt;
        let searcher = SpeculativeSearcher::new(Tt::new(8), 4);
        let mut board = Board::startpos();
        let config = SearchConfig {
            max_depth: 99,
            // Generous enough for depth 1 to complete in the slow debug build even
            // under parallel-test rayon contention, so there is always a move;
            // tiny next to a depth-99 search, which would never finish unbounded.
            time_limit: Some(Duration::from_millis(1000)),
            node_limit: None,
            soft_limit: None,
            multi_pv: 1,
        };
        let t0 = Instant::now();
        let info = searcher.search(&mut board, config);
        let elapsed = t0.elapsed();
        eprintln!("search_respects_time_limit: returned in {elapsed:?}");
        assert!(info.best_move.is_some(), "search returned no move");
        // Generous ceiling for the debug build: the point is that it RETURNS
        // (a regressed hang never would), well short of a depth-99 search.
        assert!(
            elapsed < Duration::from_secs(20),
            "search overran its time limit: {elapsed:?}"
        );
    }

    #[test]
    fn sequential_search_respects_node_limit() {
        use crate::tt::Tt;

        let searcher = Searcher::new(Tt::new(1));
        let mut board = Board::startpos();
        let info = searcher.search(
            &mut board,
            SearchConfig {
                max_depth: 99,
                time_limit: None,
                node_limit: Some(64),
                soft_limit: None,
                multi_pv: 1,
            },
        );
        assert_eq!(info.nodes, 64);
        assert!(
            info.aborted,
            "node-limited search must expose its interruption"
        );
        assert_eq!(info.bound, SearchBound::Unknown);
        let best = info
            .best_move
            .expect("node-limited search must fall back to a legal move");
        assert!(generate_legal_moves(&mut board).contains(&best));
    }

    #[test]
    fn immediate_deadline_still_returns_a_legal_move_for_both_searchers() {
        use crate::tt::Tt;

        let config = SearchConfig {
            max_depth: 1,
            time_limit: Some(Duration::ZERO),
            node_limit: None,
            soft_limit: None,
            multi_pv: 1,
        };

        let mut sequential_board = Board::startpos();
        let sequential = Searcher::new(Tt::new(1)).search(&mut sequential_board, config);
        let sequential_move = sequential
            .best_move
            .expect("sequential search must fall back to a move at an immediate deadline");
        assert!(
            generate_legal_moves(&mut sequential_board).contains(&sequential_move),
            "sequential fallback must be legal"
        );

        let mut speculative_board = Board::startpos();
        let config = SearchConfig {
            max_depth: 1,
            time_limit: Some(Duration::ZERO),
            node_limit: None,
            soft_limit: None,
            multi_pv: 1,
        };
        let speculative =
            SpeculativeSearcher::new(Tt::new(1), 1).search(&mut speculative_board, config);
        let speculative_move = speculative
            .best_move
            .expect("speculative search must fall back to a move at an immediate deadline");
        assert!(
            generate_legal_moves(&mut speculative_board).contains(&speculative_move),
            "speculative fallback must be legal"
        );
    }

    #[test]
    fn speculative_search_keeps_the_fg5c_check_evasion_bestmove_legal() {
        // FG5-C screening run 2026-09-14: this is the exact root position
        // immediately before an engine reported `3a3b`, even though only
        // `3a4b` and `5a4b` evade check.  Exercise the production
        // SpecTopN=0 path with a bounded deterministic node budget; a TT
        // ordering hint must never escape the root's legal-move list.
        const SFEN: &str =
            "lns1gks1l/5+N3/p2ppB+P2/1pp2Bp1p/9/2P2+R3/PP1PPPP1P/9/LNSGKGSNL w R2Pg 44";
        let mut board = Board::from_sfen(SFEN).expect("FG5-C fixture SFEN must parse");
        let hash = board.hash();
        let acc = board.acc.clone();
        let searcher = SpeculativeSearcher::new(Tt::new(4), 0);
        let info = searcher.search(
            &mut board,
            SearchConfig {
                max_depth: 16,
                time_limit: None,
                node_limit: Some(100_000),
                soft_limit: None,
                multi_pv: 1,
            },
        );
        let best = info.best_move.expect("check evasion must have a bestmove");

        assert_eq!(board.hash(), hash, "search must restore the root board");
        assert_eq!(board.acc, acc, "search must restore the NNUE accumulator");
        assert!(
            generate_legal_moves(&mut board).contains(&best),
            "FG5-C check-evasion search returned illegal bestmove {best:?}"
        );
    }

    #[test]
    fn regular_search_preserves_a_non_startpos_root_board() {
        const SFEN: &str =
            "lnsg1gsnl/5k3/p1pppp1pp/6p2/9/1P4P2/P1PPPP1PP/2G1KG1S1/L+rS4NL w Brbnp 22";
        let mut board = Board::from_sfen(SFEN).expect("fixture SFEN must parse");
        let hash = board.hash();
        let side = board.side_to_move;
        let ply = board.ply;
        let accumulator = board.acc.clone();
        let searcher = Searcher::new(Tt::new(4));

        let _ = searcher.search(
            &mut board,
            SearchConfig {
                max_depth: 1,
                time_limit: None,
                node_limit: Some(128),
                soft_limit: None,
                multi_pv: 1,
            },
        );

        assert_eq!(
            board.hash(),
            hash,
            "regular search must restore the root hash"
        );
        assert_eq!(
            board.side_to_move, side,
            "regular search must restore the root side"
        );
        assert_eq!(board.ply, ply, "regular search must restore the root ply");
        assert_eq!(
            board.acc, accumulator,
            "regular search must restore the NNUE accumulator"
        );
    }

    #[test]
    fn root_mate_safety_respects_the_hard_node_budget() {
        let mut board = Board::startpos();
        let hash = board.hash();
        let searcher = Searcher::new(Tt::new(4));
        let info = searcher.search(
            &mut board,
            SearchConfig {
                max_depth: 1,
                time_limit: None,
                node_limit: Some(1),
                soft_limit: None,
                multi_pv: 1,
            },
        );

        assert!(info.aborted, "a one-node limit must stop root safety work");
        assert_eq!(info.nodes, 1, "root safety must consume the shared budget");
        assert_eq!(
            board.hash(),
            hash,
            "an aborted root filter must restore the board"
        );
    }
}

#[cfg(test)]
mod regression_tests {
    use super::*;
    use crate::board::Board;

    fn fresh_state(tt: Arc<Tt>) -> Arc<SearchState> {
        Arc::new(SearchState {
            tt,
            budget: Arc::new(Budget::new(None, None, Arc::new(AtomicBool::new(false)))),
            killers: KillerTable::new(),
            history: Arc::new(HistoryTable::new()),
            countermoves: Arc::new(CountermoveTable::new()),
            diagnostics: None,
            eval_cache: None,
            pruning: PruningConfig::default(),
        })
    }

    // Regression: root_search_inner used to always store Bound::Exact, even when
    // the root search failed high (alpha reached hi without exhausting the move
    // list). A fail-high result is only a lower bound on the true score, so
    // storing it as Exact corrupted later TT probes that trusted an exact score.
    //
    // `hi` here is set far below any realistic evaluation so the very first move
    // fails high immediately, forcing the bug's exact trigger condition.
    #[test]
    fn root_fail_high_stores_lower_bound_not_exact() {
        let mut board = Board::startpos();
        let moves = generate_legal_moves(&mut board);
        let tt = Tt::new(1);
        let state = fresh_state(tt.clone());
        let hash = board.hash();
        let history = PositionHistory::initial(hash);

        root_search_inner(
            &state,
            &mut board,
            1,
            &moves,
            NEG_INF,
            -500_000,
            &SearchHistory::root(&history),
        );

        let entry = tt
            .probe(hash)
            .expect("root_search_inner should have stored a TT entry");
        assert_eq!(entry.bound, Bound::Lower);
    }

    #[test]
    fn fourfold_history_is_adjudicated_before_tt_probe_or_store() {
        let mut board = Board::startpos();
        let hash = board.hash();
        let mut history = PositionHistory::initial(hash);
        // The history container is intentionally independent of move legality;
        // this unit test isolates the search rule that a history-dependent
        // result cannot be read from or written to the position-only TT.
        for _ in 0..3 {
            history.push_after_move(hash, Color::Black, false);
        }
        let tt = Tt::new(1);
        let seeded = TtEntry {
            score: 777,
            depth: 8,
            bound: Bound::Exact,
            mv: None,
        };
        tt.store(hash, seeded);
        let state = fresh_state(tt.clone());

        let score = alpha_beta(
            &state,
            &mut board,
            NEG_INF,
            POS_INF,
            4,
            0,
            true,
            None,
            None,
            None,
            &SearchHistory::root(&history),
        );

        assert_eq!(score, 0);
        assert_eq!(state.budget.nodes(), 0, "repetition must precede node work");
        let retained = tt.probe(hash).expect("seeded TT entry must remain");
        assert_eq!(
            retained.score, seeded.score,
            "history result must not replace TT"
        );
        assert_eq!(retained.depth, seeded.depth);
        assert_eq!(retained.bound, seeded.bound);
    }

    #[test]
    fn root_child_inherits_history_and_isolates_repetition_from_tt() {
        let mut board = Board::startpos();
        let parent_hash = board.hash();
        let forced_root = MoveBuffer::legal(&mut board).as_slice()[0];
        let child_hash = {
            let token = board.do_move(forced_root);
            let hash = board.hash();
            board.undo_move(token);
            hash
        };

        // The current position is not repeated. Only the child selected by
        // `forced_root` is its fourth occurrence, so this exercises the
        // root-to-child history extension rather than root adjudication.
        let mut history = PositionHistory::initial(child_hash);
        history.push_after_move(child_hash, Color::White, false);
        history.push_after_move(child_hash, Color::Black, false);
        history.push_after_move(parent_hash, Color::White, false);

        let tt = Tt::new(1);
        let seeded = TtEntry {
            score: 555,
            depth: 7,
            bound: Bound::Exact,
            mv: None,
        };
        tt.store(child_hash, seeded);
        let state = fresh_state(tt.clone());

        let (best, score, bound) = root_search(
            &state,
            &mut board,
            1,
            NEG_INF,
            &[],
            Some(forced_root),
            false,
            &SearchHistory::root(&history),
            None,
        );

        assert_eq!(best, Some(forced_root));
        assert_eq!(score, 0);
        assert_eq!(bound, SearchBound::Exact);
        let retained = tt.probe(child_hash).expect("child seed must remain");
        assert_eq!(retained.score, seeded.score);
        assert_eq!(retained.depth, seeded.depth);
        assert_eq!(retained.bound, seeded.bound);
    }

    #[test]
    fn singular_verification_does_not_store_an_unrestricted_tt_entry() {
        let tt = Tt::new(1);
        let state = fresh_state(tt.clone());
        let hash = 0x5eed_u64;
        let original = TtEntry {
            score: 321,
            depth: 12,
            bound: Bound::Exact,
            mv: None,
        };
        tt.store(hash, original);

        // This is the write-side half of the PR #4/PR #45 composition: a
        // verification search has an excluded move and must not publish its
        // partial move-set result under the unrestricted position hash.
        store_tt(
            &state,
            hash,
            -999,
            20,
            Bound::Lower,
            None,
            0,
            Some(Move::drop(Square::from_index(0), PieceKind::Fu)),
        );

        assert_eq!(
            tt.probe(hash).expect("seed entry must remain").score,
            original.score
        );
        assert_eq!(
            tt.probe(hash).expect("seed entry must remain").depth,
            original.depth
        );
        assert_eq!(
            tt.probe(hash).expect("seed entry must remain").bound,
            original.bound
        );
    }

    #[test]
    fn singular_verification_does_not_short_circuit_on_its_own_tt_entry() {
        let mut board = Board::startpos();
        let hash = board.hash();
        let tt_move = MoveBuffer::legal(&mut board).as_slice()[0];
        let tt = Tt::new(1);
        let state = fresh_state(tt.clone());
        let history = PositionHistory::initial(hash);

        // This matches the entry that makes a singular-extension verification
        // eligible: deep enough, usable score, and the move to exclude.
        store_tt(&state, hash, 50, 4, Bound::Exact, Some(tt_move), 0, None);
        let _ = alpha_beta(
            &state,
            &mut board,
            -14,
            50,
            4,
            0,
            false,
            None,
            Some(tt_move),
            None,
            &SearchHistory::root(&history),
        );

        assert!(
            state.budget.nodes() > 1,
            "verification search must not return directly from its own TT entry"
        );
    }

    // Regression: `external_abort` (USI "stop") used to only be checked at
    // alpha_beta/quiescence's own node-entry, never at loop-level call sites
    // like root_search_inner's move loop. A search stopped mid-flight would
    // have every node return 0 immediately, but the loop itself didn't
    // recognize the stop (it only read the internal deadline flag) — so it
    // could pick the first ordered move at a spurious score of 0 and store a
    // corrupted `Bound::Exact` entry over a genuine earlier result, at a
    // *deeper* depth that a depth-preferred TT would then refuse to
    // overwrite with the real re-search. `should_abort()` now ORs both flags
    // at every such site, so a pre-set `external_abort` must make the loop
    // bail before ever touching the TT.
    #[test]
    fn external_abort_does_not_corrupt_existing_tt_entry() {
        let mut board = Board::startpos();
        let moves = generate_legal_moves(&mut board);
        let tt = Tt::new(1);
        let hash = board.hash();
        let history = PositionHistory::initial(hash);

        // A genuine, unaborted search populates a real TT entry.
        root_search_inner(
            &fresh_state(tt.clone()),
            &mut board,
            2,
            &moves,
            NEG_INF,
            POS_INF,
            &SearchHistory::root(&history),
        );
        let genuine = tt
            .probe(hash)
            .expect("first call should have stored a genuine TT entry");

        // A second call, at a deeper depth, with external_abort already set —
        // simulating a USI "stop" that arrived before this root search began.
        let aborted_state = Arc::new(SearchState {
            tt: tt.clone(),
            budget: Arc::new(Budget::new(None, None, Arc::new(AtomicBool::new(true)))),
            killers: KillerTable::new(),
            history: Arc::new(HistoryTable::new()),
            countermoves: Arc::new(CountermoveTable::new()),
            diagnostics: None,
            eval_cache: None,
            pruning: PruningConfig::default(),
        });
        root_search_inner(
            &aborted_state,
            &mut board,
            7,
            &moves,
            NEG_INF,
            POS_INF,
            &SearchHistory::root(&history),
        );

        let after = tt
            .probe(hash)
            .expect("TT entry must still be present after the aborted call");
        assert_eq!(
            after.depth, genuine.depth,
            "an aborted call must not overwrite the genuine entry with a fake deeper depth"
        );
        assert_eq!(
            after.score, genuine.score,
            "an aborted call must not overwrite the genuine entry's score"
        );
        assert_eq!(after.bound, genuine.bound);
    }

    #[test]
    fn qsearch_stores_and_reuses_only_a_top_level_entry() {
        let tt = Tt::new(1);
        let state = fresh_state(tt.clone());
        let mut board = Board::startpos();
        let hash = board.hash();
        let history = PositionHistory::initial(hash);
        let first = quiescence(
            &state,
            &mut board,
            NEG_INF,
            POS_INF,
            3,
            0,
            None,
            &SearchHistory::root(&history),
        );
        let entry = tt
            .probe(hash)
            .expect("top-level qsearch should store depth zero");
        assert_eq!(entry.depth, 0);
        assert_eq!(score_from_tt(entry.score, 3), first);

        // An exact depth-zero hit must avoid re-searching the same top-level
        // qsearch, while remaining valid at the original ply.
        let second = quiescence(
            &state,
            &mut board,
            first - 1,
            first + 1,
            3,
            0,
            None,
            &SearchHistory::root(&history),
        );
        assert_eq!(second, first);
    }

    #[test]
    fn qsearch_does_not_store_after_abort_or_overwrite_deeper_entry() {
        let mut board = Board::startpos();
        let hash = board.hash();
        let history = PositionHistory::initial(hash);
        let tt = Tt::new(1);
        let state = fresh_state(tt.clone());
        tt.store(
            hash,
            TtEntry {
                score: 777,
                depth: 4,
                bound: Bound::Exact,
                mv: None,
            },
        );
        let _ = quiescence(
            &state,
            &mut board,
            NEG_INF,
            POS_INF,
            0,
            0,
            None,
            &SearchHistory::root(&history),
        );
        assert_eq!(tt.probe(hash).expect("deeper entry must remain").depth, 4);

        let aborted_tt = Tt::new(1);
        let aborted_state = Arc::new(SearchState {
            tt: aborted_tt.clone(),
            budget: Arc::new(Budget::new(None, None, Arc::new(AtomicBool::new(true)))),
            killers: KillerTable::new(),
            history: Arc::new(HistoryTable::new()),
            countermoves: Arc::new(CountermoveTable::new()),
            diagnostics: None,
            eval_cache: None,
            pruning: PruningConfig::default(),
        });
        let _ = quiescence(
            &aborted_state,
            &mut board,
            NEG_INF,
            POS_INF,
            0,
            0,
            None,
            &SearchHistory::root(&history),
        );
        assert!(
            aborted_tt.probe(hash).is_none(),
            "aborted qsearch must not publish a TT entry"
        );
    }

    #[test]
    fn qsearch_records_a_capture_and_restores_the_position() {
        let mut board = Board::from_sfen("k8/9/9/9/4p4/9/4R4/9/8K b - 1").unwrap();
        let hash = board.hash();
        let history = PositionHistory::initial(hash);
        let capture = generate_legal_captures(&mut board)
            .into_iter()
            .find(|m| m.to == Square::from_shogi(5, 5))
            .expect("the rook must have a legal pawn capture");
        let tt = Tt::new(1);
        let state = fresh_state(tt.clone());

        let _ = quiescence(
            &state,
            &mut board,
            NEG_INF,
            POS_INF,
            0,
            0,
            None,
            &SearchHistory::root(&history),
        );

        assert_eq!(board.hash(), hash, "qsearch must undo every capture");
        assert_eq!(
            tt.probe(hash).and_then(|entry| entry.mv),
            Some(capture),
            "the qsearch TT entry should retain its best capture"
        );
    }

    #[test]
    fn qsearch_delta_pruning_returns_an_upper_bound_without_mutation() {
        let mut board = Board::startpos();
        let hash = board.hash();
        let tt = Tt::new(1);
        let state = fresh_state(tt.clone());
        let alpha = 100_000;
        let history = PositionHistory::initial(hash);

        let score = quiescence(
            &state,
            &mut board,
            alpha,
            alpha + 1,
            0,
            0,
            None,
            &SearchHistory::root(&history),
        );

        assert_eq!(score, alpha, "delta pruning should return the raised alpha");
        assert_eq!(
            board.hash(),
            hash,
            "delta pruning must not mutate the board"
        );
        let entry = tt
            .probe(hash)
            .expect("delta pruning should cache its bound");
        assert_eq!(entry.bound, Bound::Upper);
        assert_eq!(score_from_tt(entry.score, 0), alpha);
    }

    #[test]
    fn qsearch_quiet_check_fixture_preserves_board_state() {
        let mut board = Board::from_sfen("4k4/9/9/9/4R4/9/9/9/4K4 b - 1").unwrap();
        let hash = board.hash();
        let quiet_check = Move::normal(
            Square::from_shogi(5, 5),
            Square::from_shogi(5, 2),
            PieceKind::Hisha,
            false,
        );
        assert!(
            generate_legal_moves(&mut board).contains(&quiet_check),
            "the rook quiet check must be legal"
        );
        let token = board.do_move(quiet_check);
        assert!(is_in_check(&board, board.side_to_move));
        board.undo_move(token);
        assert_eq!(board.hash(), hash, "quiet-check probe must undo exactly");

        let state = fresh_state(Tt::new(1));
        let history = PositionHistory::initial(hash);
        let _ = quiescence(
            &state,
            &mut board,
            NEG_INF,
            POS_INF,
            0,
            0,
            None,
            &SearchHistory::root(&history),
        );
        assert_eq!(
            board.hash(),
            hash,
            "qsearch quiet-check fixture must not mutate"
        );
    }

    #[test]
    fn supplied_check_state_matches_recomputed_state() {
        for sfen in [crate::sfen::STARTPOS_SFEN, "4r3k/9/9/9/9/9/9/9/4K4 b - 1"] {
            let mut recomputed_board = Board::from_sfen(sfen).unwrap();
            let recomputed_state = fresh_state(Tt::new(1));
            let recomputed_history = PositionHistory::initial(recomputed_board.hash());
            let recomputed = alpha_beta(
                &recomputed_state,
                &mut recomputed_board,
                NEG_INF,
                POS_INF,
                2,
                0,
                true,
                None,
                None,
                None,
                &SearchHistory::root(&recomputed_history),
            );

            let mut supplied_board = Board::from_sfen(sfen).unwrap();
            let supplied_check = is_in_check(&supplied_board, supplied_board.side_to_move);
            let supplied_state = fresh_state(Tt::new(1));
            let supplied_history = PositionHistory::initial(supplied_board.hash());
            let supplied = alpha_beta(
                &supplied_state,
                &mut supplied_board,
                NEG_INF,
                POS_INF,
                2,
                0,
                true,
                None,
                None,
                Some(supplied_check),
                &SearchHistory::root(&supplied_history),
            );
            assert_eq!(
                supplied, recomputed,
                "known check state changed score for {sfen}"
            );
        }
    }

    // Two hand-built, hand-verified positions for the mate-direction regression
    // tests below.
    //
    // MATE_IN_1_SFEN: white king cornered at (file9,rank1); black king at
    // (file7,rank2) covers both diagonal escapes; black rook slides to
    // (file9,rank5) delivering unstoppable check down the file. Verified: engine
    // reports score == MATE_SCORE - 1 (899_999) at depth 1.
    const MATE_IN_1_SFEN: &str = "k8/2K6/9/9/4R4/9/9/9/9 b - 1";

    // Regression: the mate score formula `-(MATE_SCORE - ply)` was once written
    // with the ply term's sign flipped (`-900_000 - ply`), which made a mate
    // discovered at a *deeper* ply score higher in magnitude than the identical
    // mate discovered shallower — the engine would then prefer a search path
    // that "finds" the win later over one that finds it sooner. Rather than
    // hand-building a second, genuinely-slower mate position (hard to verify by
    // hand and slow to brute-force-verify), this calls `alpha_beta` directly on
    // the SAME verified mate-in-1 position with two different starting `ply`
    // values: the formula must be correct for the ply argument on its own,
    // independent of which position produced it. depth=2 is the minimum that
    // lets the recursive call one ply down reach the real movegen/terminal
    // check in alpha_beta instead of diverting to quiescence (depth=0).
    // This bug occurred independently in two call sites (alpha_beta in this
    // file, and spec_alpha_beta in speculative.rs); the sibling test in
    // speculative.rs covers the second site the same way.
    #[test]
    fn shorter_ply_mate_scores_higher_in_alpha_beta() {
        let mut board_a = Board::from_sfen(MATE_IN_1_SFEN).unwrap();
        let history_a = PositionHistory::initial(board_a.hash());
        let state_a = fresh_state(Tt::new(1));
        let score_shallow = alpha_beta(
            &state_a,
            &mut board_a,
            NEG_INF,
            POS_INF,
            2,
            1,
            true,
            None,
            None,
            None,
            &SearchHistory::root(&history_a),
        );

        let mut board_b = Board::from_sfen(MATE_IN_1_SFEN).unwrap();
        let history_b = PositionHistory::initial(board_b.hash());
        let state_b = fresh_state(Tt::new(1));
        let score_deep = alpha_beta(
            &state_b,
            &mut board_b,
            NEG_INF,
            POS_INF,
            2,
            3,
            true,
            None,
            None,
            None,
            &SearchHistory::root(&history_b),
        );

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

    // Note: SpeculativeSearcher.search()'s reported score comes from the same
    // shared root_search/alpha_beta path as Searcher (SpeculativeSearcher only
    // wraps it with preemptive background speculation), so it does not exercise
    // speculative.rs's own independent copy of the mate-score formula. That
    // second call site (`spec_alpha_beta`) is tested directly in
    // speculative.rs::tests::shorter_mate_scores_higher_in_spec_alpha_beta.
}
