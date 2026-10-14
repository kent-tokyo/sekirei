//! Sekirei — USI (Universal Shogi Interface) engine binary.
//!
//! Run: `cargo run --release -p sekirei`
//! Then paste USI commands on stdin.

use std::io::{self, BufRead, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use sekirei_core::{
    board::Board,
    color::Color,
    dfpn::{DfpnConfig, DfpnOutcome, DfpnResult, DfpnSolver},
    eval::{
        NnueOutputMode, set_nnue_output_mode, set_nnue_residual_scale_permille,
        validate_nnue_residual_scale_permille,
    },
    halfkp,
    lazy_smp::{
        LAZY_DEFAULT_FLAGS, LazySmpRootHintEffectiveness, LazySmpSearcher, LazySmpWorkOverlap,
        LazySmpWorkerInfo, LazySmpWorkerTtReuse,
    },
    mcts::{MaterialValue, SharedTreeMcts, SharedTreeMctsConfig},
    movegen::generate_legal_moves,
    nnue::load_evaluator,
    search::{
        AspirationSnapshot, CooperativeRootStats, CooperativeRootWaitStats, MATE_SCORE,
        NmpSnapshot, RECURSIVE_SEARCH_STACK_BYTES, SearchConfig, SearchDiagnostics,
        SearchDiagnosticsSnapshot, SearchInfo, Searcher, SpecSearchInfo, SpeculativeSearcher,
    },
    sfen::{PositionHistory, RepetitionOutcome, move_to_usi, parse_position_cmd_with_history},
    tt::{Tt, TtWriteSnapshot},
};

#[cfg(feature = "opening-book")]
mod book;
#[cfg(feature = "opening-book")]
mod book_log;
mod go;
mod invariant;
mod session;
#[cfg(feature = "opening-book")]
use book::{Book, BookProvenance};
#[cfg(feature = "opening-book")]
use book_log::{BookDecisionLogger, DecisionContext};
use invariant::DiagCtx;
#[cfg(feature = "opening-book")]
use sekirei_core::sfen::board_to_sfen;
use session::{
    abort_and_join as abort_and_join_inflight_search,
    invalidate_and_join as invalidate_and_join_inflight_search,
    mutate_after_join as mutate_evaluator_after_join,
};

// ---- Engine identity ----

const ENGINE_NAME: &str = "Sekirei";
const ENGINE_AUTHOR: &str = "Kentaro Tanabe";
const ENGINE_VERSION: &str = env!("CARGO_PKG_VERSION");
const DEFAULT_HASH_MB: usize = 64;
const HASH_MB_RANGE: SpinRange = SpinRange::new(1, 2048);
const THREADS_RANGE: SpinRange = SpinRange::new(0, 512);
const SPEC_TOP_N_RANGE: SpinRange = SpinRange::new(0, 512);
const LAZY_FLAGS_RANGE: SpinRange = SpinRange::new(0, 32767);
const MOVE_OVERHEAD_RANGE: SpinRange = SpinRange::new(0, 5000);
const INCREMENT_USE_PERCENT_RANGE: SpinRange = SpinRange::new(0, 100);
const MULTI_PV_RANGE: SpinRange = SpinRange::new(1, 256);
const BOOK_MAX_PLY_RANGE: SpinRange = SpinRange::new(0, 200);
#[cfg(feature = "opening-book")]
const DEFAULT_BOOK_FILE: &str = "data/opening_book.jsonl";
// Keep the USI controller and core speculative pool on one documented stack
// contract for recursive alpha-beta searches.
const SEARCH_STACK_BYTES: usize = RECURSIVE_SEARCH_STACK_BYTES;
// Dedicated speculative-search pool size. Was hardcoded in make_searcher()
// with no USI option (issue #9); this is that same value now exposed as
// the SpecTopN option's default, so not setting it changes nothing.
const DEFAULT_SPEC_TOP_N: usize = 0;
// Lazy SMP behaviour switches (USI option LazyFlags, `lazy_smp::LAZY_*`).
static LAZY_FLAGS: AtomicU32 = AtomicU32::new(LAZY_DEFAULT_FLAGS);
// Opt-in per-node counters for Lazy SMP scaling measurements. Production
// searches keep this false because even relaxed atomics perturb throughput.
static LAZY_DIAGNOSTICS: AtomicBool = AtomicBool::new(false);
static NMP_DIAGNOSTICS: AtomicBool = AtomicBool::new(false);
static ASPIRATION_DIAGNOSTICS: AtomicBool = AtomicBool::new(false);
// MultiPV as last set; `SearchMode=Auto` needs a root-candidate backend for
// MultiPV > 1 (only the speculative searcher reports several root lines).
static AUTO_MULTI_PV: AtomicU32 = AtomicU32::new(1);
// Share (percent) of the Fischer increment added to each move's base time
// (USI option IncrementUsePercent, default 75). 0 restores the historical
// rule, which spreads the increment over the remaining-moves estimate.
static INC_USE_PCT: AtomicU32 = AtomicU32::new(75);
// SpecTopN used by `Auto` for MultiPV analysis (the former default).
const AUTO_MULTI_PV_SPEC_TOP_N: usize = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct SpinRange {
    min: u64,
    max: u64,
}

impl SpinRange {
    const fn new(min: u64, max: u64) -> Self {
        Self { min, max }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SearchMode {
    /// One thread: the sequential searcher; more threads: Lazy SMP.
    Auto,
    Speculative,
    LazySmp,
    Dfpn,
    SharedMcts,
}

struct SearchResult {
    best_move: Option<sekirei_core::mv::Move>,
    score: i32,
    depth: u32,
    nodes: u64,
    elapsed: Duration,
    hashfull: u32,
    pv_list: Vec<(sekirei_core::mv::Move, i32)>,
    /// Primary legal continuation.  `pv_list` is the root-only MultiPV
    /// ranking; this is deliberately kept separate so a continuation is
    /// never attributed to another root candidate.
    pv: Vec<sekirei_core::mv::Move>,
    worker_stats: Vec<LazySmpWorkerInfo>,
    lazy_worker_tt_reuse: Vec<LazySmpWorkerTtReuse>,
    lazy_tt_writes: Option<TtWriteSnapshot>,
    lazy_work_overlap: Option<LazySmpWorkOverlap>,
    lazy_root_hints: Option<LazySmpRootHintEffectiveness>,
    lazy_retirement: Option<Vec<(u64, bool, u64)>>,
    cooperative_root: Option<CooperativeRootStats>,
    cooperative_root_wait: Option<CooperativeRootWaitStats>,
    nmp_stats: Vec<NmpSnapshot>,
    aspiration_stats: Vec<AspirationSnapshot>,
    shared_mcts_stats: Option<(u32, u32, u32)>,
    /// Per-search root mate-safety metrics, only available on the sequential
    /// backend used by `Speculative` with `SpecTopN=0`.
    root_safety: Option<SearchDiagnosticsSnapshot>,
    /// Bounded mate-proof status that must not be collapsed into a normal
    /// centipawn score or a fabricated mate distance.
    dfpn: Option<DfpnReport>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct DfpnReport {
    outcome: DfpnOutcome,
    aborted: bool,
    limit_depth: u32,
    best_move_from_proof: bool,
}

struct SequentialBackend {
    searcher: Arc<Searcher>,
    diagnostics: Arc<SearchDiagnostics>,
}

enum SearchBackend {
    /// The Speculative USI mode with `SpecTopN=0` has no speculative work.
    /// Route it to the regular searcher so all production root-safety logic
    /// (including the cached mate-safety pass) is actually active.
    Sequential(Arc<SequentialBackend>),
    Speculative(Arc<SpeculativeSearcher>),
    LazySmp(Arc<LazySmpSearcher>),
    Dfpn(Arc<DfpnBackend>),
    SharedMcts(Arc<SharedMctsBackend>),
}

struct DfpnBackend {
    solver: DfpnSolver,
    abort: Arc<AtomicBool>,
}

struct SharedMctsBackend {
    searcher: SharedTreeMcts,
    abort: Arc<AtomicBool>,
}

impl SearchBackend {
    fn speculative(hash_mb: usize, spec_top_n: usize) -> Self {
        if spec_top_n == 0 {
            // Only the root mate-safety counters are reported; per-node
            // counters and cost timers stay off in production searches.
            let diagnostics = Arc::new(
                if NMP_DIAGNOSTICS.load(Ordering::Relaxed)
                    || ASPIRATION_DIAGNOSTICS.load(Ordering::Relaxed)
                {
                    SearchDiagnostics::counters_only()
                } else {
                    SearchDiagnostics::root_safety_only()
                },
            );
            return Self::Sequential(Arc::new(SequentialBackend {
                searcher: Arc::new(Searcher::with_diagnostics(
                    Tt::new(hash_mb),
                    diagnostics.clone(),
                )),
                diagnostics,
            }));
        }
        Self::Speculative(Arc::new(SpeculativeSearcher::new(
            Tt::new(hash_mb),
            spec_top_n,
        )))
    }

    fn lazy_smp(hash_mb: usize, workers: usize) -> Self {
        let flags = LAZY_FLAGS.load(Ordering::Relaxed);
        let searcher = if LAZY_DIAGNOSTICS.load(Ordering::Relaxed)
            || NMP_DIAGNOSTICS.load(Ordering::Relaxed)
            || ASPIRATION_DIAGNOSTICS.load(Ordering::Relaxed)
        {
            LazySmpSearcher::diagnostic(workers, flags, hash_mb)
        } else {
            LazySmpSearcher::with_flags_and_hash_mb(Tt::new(hash_mb), workers, flags, hash_mb)
        };
        Self::LazySmp(Arc::new(searcher))
    }

    fn dfpn() -> Self {
        Self::Dfpn(Arc::new(DfpnBackend {
            solver: DfpnSolver,
            abort: Arc::new(AtomicBool::new(false)),
        }))
    }

    fn shared_mcts() -> Self {
        Self::SharedMcts(Arc::new(SharedMctsBackend {
            searcher: SharedTreeMcts::default(),
            abort: Arc::new(AtomicBool::new(false)),
        }))
    }

    fn abort_flag(&self) -> Arc<AtomicBool> {
        match self {
            Self::Sequential(s) => s.searcher.abort_flag(),
            Self::Speculative(s) => s.abort_flag(),
            Self::LazySmp(s) => s.abort_flag(),
            Self::Dfpn(s) => Arc::clone(&s.abort),
            Self::SharedMcts(s) => Arc::clone(&s.abort),
        }
    }

    fn reset_abort_flag(&self) {
        match self {
            Self::Sequential(s) => s.searcher.reset_abort_flag(),
            Self::Speculative(s) => s.reset_abort_flag(),
            Self::LazySmp(s) => s.reset_abort_flag(),
            Self::Dfpn(s) => s.abort.store(false, Ordering::Relaxed),
            Self::SharedMcts(s) => s.abort.store(false, Ordering::Relaxed),
        }
    }

    fn clear_tt(&self) {
        match self {
            Self::Sequential(s) => s.searcher.clear_tt(),
            Self::Speculative(s) => s.clear_tt(),
            Self::LazySmp(s) => s.clear_tt(),
            Self::Dfpn(_) => {}
            Self::SharedMcts(_) => {}
        }
    }

    fn probe_tt(&self, hash: u64) -> Option<sekirei_core::mv::Move> {
        match self {
            Self::Sequential(s) => s.searcher.probe_tt(hash),
            Self::Speculative(s) => s.probe_tt(hash),
            Self::LazySmp(s) => s.probe_tt(hash),
            Self::Dfpn(_) => None,
            Self::SharedMcts(_) => None,
        }
    }

    fn search(
        &self,
        board: &mut Board,
        config: SearchConfig,
        position_history: &PositionHistory,
    ) -> SearchResult {
        if let Some(outcome) = position_history.outcome_at_current_position() {
            let score = match outcome {
                RepetitionOutcome::Draw => 0,
                RepetitionOutcome::PerpetualCheck(loser) if loser == board.side_to_move => {
                    -MATE_SCORE + 1
                }
                RepetitionOutcome::PerpetualCheck(_) => MATE_SCORE - 1,
            };
            return SearchResult {
                best_move: fallback_legal_move(board),
                score,
                depth: 0,
                nodes: 0,
                elapsed: Duration::ZERO,
                hashfull: 0,
                pv_list: Vec::new(),
                pv: Vec::new(),
                worker_stats: Vec::new(),
                lazy_worker_tt_reuse: Vec::new(),
                lazy_tt_writes: None,
                lazy_work_overlap: None,
                lazy_root_hints: None,
                lazy_retirement: None,
                cooperative_root: None,
                cooperative_root_wait: None,
                nmp_stats: Vec::new(),
                aspiration_stats: Vec::new(),
                shared_mcts_stats: None,
                root_safety: None,
                dfpn: None,
            };
        }
        match self {
            Self::Sequential(s) => s.searcher.new_search(),
            Self::Speculative(s) => s.new_search(),
            Self::LazySmp(s) => s.new_search(),
            _ => {}
        }
        match self {
            Self::Sequential(s) => {
                let before = s.diagnostics.snapshot();
                let nmp_before = s.diagnostics.nmp_snapshot();
                let aspiration_before = ASPIRATION_DIAGNOSTICS
                    .load(Ordering::Relaxed)
                    .then(|| s.diagnostics.aspiration_snapshot());
                let info = s
                    .searcher
                    .search_with_history(board, config, position_history);
                let mut result = normalize_sequential_result(info);
                result.root_safety = Some(diagnostics_delta(s.diagnostics.snapshot(), before));
                if let Some(before) = aspiration_before {
                    result.aspiration_stats = vec![aspiration_delta(
                        s.diagnostics.aspiration_snapshot(),
                        before,
                    )];
                }
                if NMP_DIAGNOSTICS.load(Ordering::Relaxed) {
                    result.nmp_stats = vec![nmp_delta(s.diagnostics.nmp_snapshot(), nmp_before)];
                }
                result
            }
            Self::Speculative(s) => {
                normalize_spec_result(s.search_with_history(board, config, position_history))
            }
            Self::LazySmp(s) => {
                let nmp_before = s.nmp_stats();
                let aspiration_before = (ASPIRATION_DIAGNOSTICS.load(Ordering::Relaxed)
                    || LAZY_DIAGNOSTICS.load(Ordering::Relaxed))
                .then(|| s.aspiration_stats());
                let (info, lazy_root_hints, lazy_worker_tt_reuse) = s
                    .try_search_with_history_and_root_hint_effectiveness(
                        board,
                        config,
                        position_history,
                    )
                    .expect("USI generation manager must serialize Lazy SMP searches");
                let result = info.result;
                SearchResult {
                    best_move: result.best_move.or_else(|| fallback_legal_move(board)),
                    score: result.score,
                    depth: result.depth,
                    nodes: info.total_nodes,
                    // The selected worker's duration is not the duration of
                    // the Lazy SMP group; use wall time for truthful NPS and
                    // USI timing diagnostics.
                    elapsed: info.elapsed,
                    hashfull: result.hashfull,
                    pv_list: Vec::new(),
                    pv: result.pv,
                    worker_stats: info.worker_results,
                    lazy_worker_tt_reuse,
                    lazy_tt_writes: info.tt_writes,
                    lazy_work_overlap: info.work_overlap,
                    lazy_root_hints,
                    lazy_retirement: s.retirement_stats(),
                    cooperative_root: s.cooperative_root_stats(),
                    cooperative_root_wait: s.cooperative_root_wait_stats(),
                    nmp_stats: s
                        .nmp_stats()
                        .into_iter()
                        .zip(nmp_before)
                        .map(|(after, before)| nmp_delta(after, before))
                        .collect(),
                    aspiration_stats: aspiration_before
                        .map(|before| {
                            s.aspiration_stats()
                                .into_iter()
                                .zip(before)
                                .map(|(after, before)| aspiration_delta(after, before))
                                .collect()
                        })
                        .unwrap_or_default(),
                    shared_mcts_stats: None,
                    root_safety: None,
                    dfpn: None,
                }
            }
            Self::Dfpn(s) => {
                let started = Instant::now();
                let timer = config.time_limit.map(|limit| {
                    let abort = Arc::clone(&s.abort);
                    let (cancel_tx, cancel_rx) = mpsc::channel();
                    let handle = std::thread::spawn(move || {
                        if cancel_rx.recv_timeout(limit).is_err() {
                            abort.store(true, Ordering::Relaxed);
                        }
                    });
                    (cancel_tx, handle)
                });
                let result = s.solver.solve_with_abort(
                    board,
                    DfpnConfig {
                        max_depth: config.max_depth.min(u16::MAX as u32) as u16,
                        node_limit: config.node_limit.unwrap_or(100_000),
                        ..DfpnConfig::default()
                    },
                    &s.abort,
                );
                if let Some((cancel_tx, handle)) = timer {
                    let _ = cancel_tx.send(());
                    let _ = handle.join();
                }
                normalize_dfpn_result(board, config.max_depth, result, started.elapsed())
            }
            Self::SharedMcts(s) => {
                let started = Instant::now();
                let timer = config.time_limit.map(|limit| {
                    let abort = Arc::clone(&s.abort);
                    let (cancel_tx, cancel_rx) = mpsc::channel();
                    let handle = std::thread::spawn(move || {
                        if cancel_rx.recv_timeout(limit).is_err() {
                            abort.store(true, Ordering::Relaxed);
                        }
                    });
                    (cancel_tx, handle)
                });
                let info = s.searcher.search_with_abort(
                    board,
                    SharedTreeMctsConfig {
                        simulations: config.node_limit.unwrap_or(128).min(u32::MAX as u64) as u32,
                        max_depth: config.max_depth.min(u16::MAX as u32) as u16,
                        share_transpositions: true,
                    },
                    &sekirei_core::mcts::UniformPolicy,
                    &MaterialValue,
                    &s.abort,
                );
                if let Some((cancel_tx, handle)) = timer {
                    let _ = cancel_tx.send(());
                    let _ = handle.join();
                }
                SearchResult {
                    best_move: info.best_move.or_else(|| fallback_legal_move(board)),
                    score: info.score,
                    depth: config.max_depth,
                    nodes: info.nodes as u64,
                    elapsed: started.elapsed(),
                    hashfull: 0,
                    pv_list: Vec::new(),
                    pv: info.best_move.into_iter().collect(),
                    worker_stats: Vec::new(),
                    lazy_worker_tt_reuse: Vec::new(),
                    lazy_tt_writes: None,
                    lazy_work_overlap: None,
                    lazy_root_hints: None,
                    lazy_retirement: None,
                    cooperative_root: None,
                    cooperative_root_wait: None,
                    nmp_stats: Vec::new(),
                    aspiration_stats: Vec::new(),
                    shared_mcts_stats: Some((
                        info.simulations,
                        info.nodes,
                        info.transposition_hits,
                    )),
                    root_safety: None,
                    dfpn: None,
                }
            }
        }
    }
}

fn normalize_sequential_result(info: SearchInfo) -> SearchResult {
    SearchResult {
        best_move: info.best_move,
        score: info.score,
        depth: info.depth,
        nodes: info.nodes,
        elapsed: info.elapsed,
        hashfull: info.hashfull,
        pv_list: Vec::new(),
        pv: info.pv,
        worker_stats: Vec::new(),
        lazy_worker_tt_reuse: Vec::new(),
        lazy_tt_writes: None,
        lazy_work_overlap: None,
        lazy_root_hints: None,
        lazy_retirement: None,
        cooperative_root: None,
        cooperative_root_wait: None,
        nmp_stats: Vec::new(),
        aspiration_stats: Vec::new(),
        shared_mcts_stats: None,
        root_safety: None,
        dfpn: None,
    }
}

fn normalize_spec_result(info: SpecSearchInfo) -> SearchResult {
    SearchResult {
        best_move: info.best_move,
        score: info.score,
        depth: info.depth,
        nodes: info.nodes,
        elapsed: info.elapsed,
        hashfull: info.hashfull,
        pv_list: info.pv_list,
        pv: info.pv,
        worker_stats: Vec::new(),
        lazy_worker_tt_reuse: Vec::new(),
        lazy_tt_writes: None,
        lazy_work_overlap: None,
        lazy_root_hints: None,
        lazy_retirement: None,
        cooperative_root: None,
        cooperative_root_wait: None,
        nmp_stats: Vec::new(),
        aspiration_stats: Vec::new(),
        shared_mcts_stats: None,
        root_safety: None,
        dfpn: None,
    }
}

fn normalize_dfpn_result(
    board: &Board,
    limit_depth: u32,
    result: DfpnResult,
    elapsed: Duration,
) -> SearchResult {
    let proof_move = result.best_move;
    SearchResult {
        best_move: proof_move.or_else(|| fallback_legal_move(board)),
        // Only the dedicated DFPN renderer consumes this value. It emits
        // `mate +` for a proof with unknown distance and omits `score` for
        // bounded disproofs and incomplete searches.
        score: if result.outcome == DfpnOutcome::Proven {
            MATE_SCORE - 1
        } else {
            0
        },
        // DFPN is not iterative deepening. The requested boundary is kept in
        // `DfpnReport` instead of being advertised as a completed depth.
        depth: 0,
        nodes: result.nodes,
        elapsed,
        hashfull: 0,
        pv_list: Vec::new(),
        pv: proof_move.into_iter().collect(),
        worker_stats: Vec::new(),
        lazy_worker_tt_reuse: Vec::new(),
        lazy_tt_writes: None,
        lazy_work_overlap: None,
        lazy_root_hints: None,
        lazy_retirement: None,
        cooperative_root: None,
        cooperative_root_wait: None,
        nmp_stats: Vec::new(),
        aspiration_stats: Vec::new(),
        shared_mcts_stats: None,
        root_safety: None,
        dfpn: Some(DfpnReport {
            outcome: result.outcome,
            aborted: result.aborted,
            limit_depth,
            best_move_from_proof: proof_move.is_some(),
        }),
    }
}

fn diagnostics_delta(
    after: SearchDiagnosticsSnapshot,
    before: SearchDiagnosticsSnapshot,
) -> SearchDiagnosticsSnapshot {
    SearchDiagnosticsSnapshot {
        static_evaluations: after
            .static_evaluations
            .saturating_sub(before.static_evaluations),
        eval_cache_probes: after
            .eval_cache_probes
            .saturating_sub(before.eval_cache_probes),
        eval_cache_hits: after.eval_cache_hits.saturating_sub(before.eval_cache_hits),
        tt_probes: after.tt_probes.saturating_sub(before.tt_probes),
        tt_hits: after.tt_hits.saturating_sub(before.tt_hits),
        tt_stores: after.tt_stores.saturating_sub(before.tt_stores),
        order_tt: after.order_tt.saturating_sub(before.order_tt),
        order_killer: after.order_killer.saturating_sub(before.order_killer),
        order_countermove: after
            .order_countermove
            .saturating_sub(before.order_countermove),
        order_history: after.order_history.saturating_sub(before.order_history),
        root_mate_in_one_nodes: after
            .root_mate_in_one_nodes
            .saturating_sub(before.root_mate_in_one_nodes),
        root_mate_blunder_nodes: after
            .root_mate_blunder_nodes
            .saturating_sub(before.root_mate_blunder_nodes),
        root_mate_in_one_cache_hits: after
            .root_mate_in_one_cache_hits
            .saturating_sub(before.root_mate_in_one_cache_hits),
        root_mate_blunder_cache_hits: after
            .root_mate_blunder_cache_hits
            .saturating_sub(before.root_mate_blunder_cache_hits),
        alpha_beta_calls: after
            .alpha_beta_calls
            .saturating_sub(before.alpha_beta_calls),
        quiescence_calls: after
            .quiescence_calls
            .saturating_sub(before.quiescence_calls),
        qsearch: std::array::from_fn(|index| {
            after.qsearch[index].saturating_sub(before.qsearch[index])
        }),
        static_evaluation_ns: after
            .static_evaluation_ns
            .saturating_sub(before.static_evaluation_ns),
        tt_probe_ns: after.tt_probe_ns.saturating_sub(before.tt_probe_ns),
        tt_store_ns: after.tt_store_ns.saturating_sub(before.tt_store_ns),
        movegen_order_ns: after
            .movegen_order_ns
            .saturating_sub(before.movegen_order_ns),
        movegen_generate_ns: after
            .movegen_generate_ns
            .saturating_sub(before.movegen_generate_ns),
        move_order_ns: after.move_order_ns.saturating_sub(before.move_order_ns),
        move_order_score_ns: after
            .move_order_score_ns
            .saturating_sub(before.move_order_score_ns),
        move_order_sort_ns: after
            .move_order_sort_ns
            .saturating_sub(before.move_order_sort_ns),
        quiescence_inclusive_ns: after
            .quiescence_inclusive_ns
            .saturating_sub(before.quiescence_inclusive_ns),
        root_mate_safety_ns: after
            .root_mate_safety_ns
            .saturating_sub(before.root_mate_safety_ns),
    }
}

/// A bounded/aborted auxiliary search must still return a legal move whenever
/// the position has one. Returning `resign` for an `Unknown` DFPN/MCTS result
/// turns a time limit into a false game loss.
fn fallback_legal_move(board: &Board) -> Option<sekirei_core::mv::Move> {
    let mut probe = board.clone();
    generate_legal_moves(&mut probe).into_iter().next()
}

/// Render a USI PV without fabricating a continuation for a secondary
/// MultiPV root.  The primary line is reconstructed from exact TT entries;
/// the root move remains available as a valid one-ply PV if reconstruction is
/// shorter than the completed depth.
fn render_pv(
    primary: &[sekirei_core::mv::Move],
    root: sekirei_core::mv::Move,
    is_primary: bool,
) -> String {
    if is_primary && primary.first() == Some(&root) {
        primary
            .iter()
            .copied()
            .map(move_to_usi)
            .collect::<Vec<_>>()
            .join(" ")
    } else {
        move_to_usi(root)
    }
}

/// Look up a TT ponder reply without ever applying an unvalidated move to the
/// caller's board.  `bestmove` is checked before this helper is called; the
/// TT reply itself is still only a cache hint and must be legal in the child
/// position before it may be emitted to a GUI.
fn legal_ponder_move(
    searcher: &SearchBackend,
    board: &mut Board,
    best_move: sekirei_core::mv::Move,
) -> Option<sekirei_core::mv::Move> {
    let token = board.do_move(best_move);
    let ponder = searcher
        .probe_tt(board.hash())
        .filter(|candidate| generate_legal_moves(board).contains(candidate));
    board.undo_move(token);
    ponder
}

/// Render an engine score using the USI score grammar.
///
/// Search mate scores are encoded near `MATE_SCORE`; exposing those raw
/// centipawn sentinels makes a forced mate look like an absurd evaluation to
/// GUIs and analysis tools.  Ordinary scores remain unchanged.
fn score_to_usi(score: i32) -> String {
    if score >= MATE_SCORE - 1000 {
        format!("mate {}", MATE_SCORE - score)
    } else if score <= -MATE_SCORE + 1000 {
        format!("mate -{}", MATE_SCORE + score)
    } else {
        format!("cp {score}")
    }
}

fn render_dfpn_info(info: &SearchResult, nps: u64, elapsed_ms: u64) -> Option<(String, String)> {
    let report = info.dfpn?;
    let outcome = match report.outcome {
        DfpnOutcome::Proven => "proven",
        DfpnOutcome::Disproven => "disproven",
        DfpnOutcome::Unknown => "unknown",
    };
    let completed = report.outcome != DfpnOutcome::Unknown && !report.aborted;
    let distance = if report.outcome == DfpnOutcome::Proven {
        "unknown"
    } else {
        "none"
    };
    let bestmove_source = if report.best_move_from_proof {
        "proof"
    } else if info.best_move.is_some() {
        "fallback"
    } else {
        "none"
    };
    let status = format!(
        "info string dfpn outcome {outcome} scope bounded distance {distance} limit_depth {} completed {completed} aborted {} bestmove_source {bestmove_source}",
        report.limit_depth, report.aborted
    );
    let search_info = if report.outcome == DfpnOutcome::Proven {
        let pv = info
            .pv
            .iter()
            .copied()
            .map(move_to_usi)
            .collect::<Vec<_>>()
            .join(" ");
        let pv_suffix = if pv.is_empty() {
            String::new()
        } else {
            format!(" pv {pv}")
        };
        format!(
            "info depth 0 score mate + nodes {} nps {nps} time {elapsed_ms} hashfull {}{pv_suffix}",
            info.nodes, info.hashfull
        )
    } else {
        // USI has no standard score token for a bounded no-mate proof or an
        // incomplete proof search. Omitting it is less misleading than cp 0.
        format!(
            "info depth 0 nodes {} nps {nps} time {elapsed_ms} hashfull {}",
            info.nodes, info.hashfull
        )
    };
    Some((status, search_info))
}

fn emit_lazy_smp_diagnostics(info: &SearchResult) {
    if info.worker_stats.is_empty() {
        return;
    }
    let total_worker_nodes = info
        .worker_stats
        .iter()
        .map(|worker| worker.nodes)
        .sum::<u64>();
    let selected_index = info
        .worker_stats
        .iter()
        .position(|worker| worker.selected)
        .expect("Lazy SMP diagnostics must identify the selected worker");
    let selected = info.worker_stats[selected_index];
    let selected_reuse = info
        .lazy_worker_tt_reuse
        .get(selected_index)
        .copied()
        .unwrap_or_default();
    let selected_share_permille = selected
        .nodes
        .saturating_mul(1000)
        .checked_div(total_worker_nodes)
        .unwrap_or(0);
    let agreement = info
        .worker_stats
        .iter()
        .filter(|worker| worker.best_move == selected.best_move)
        .count();
    // Every non-selected root result is discarded by the final selector even
    // though its TT entries may still have helped another worker. Report both
    // the complete discarded-root cost and the stricter divergent subset.
    let discarded_root_nodes = total_worker_nodes.saturating_sub(selected.nodes);
    let divergent_discarded_nodes = info
        .worker_stats
        .iter()
        .filter(|worker| !worker.selected && worker.best_move != selected.best_move)
        .map(|worker| worker.nodes)
        .sum::<u64>();
    // This is an end-to-end worker-duration spread, not a timestamped
    // measurement from the instant the shared abort flag was raised.
    let main_elapsed_ms = info.worker_stats[0].elapsed.as_millis();
    let max_elapsed_ms = info
        .worker_stats
        .iter()
        .map(|worker| worker.elapsed.as_millis())
        .max()
        .unwrap_or(main_elapsed_ms);
    let stop_lag_ms = max_elapsed_ms.saturating_sub(main_elapsed_ms);
    let summary = info
        .worker_stats
        .iter()
        .enumerate()
        .map(|(i, worker)| {
            let marker = if worker.selected { "*" } else { "" };
            let reuse = info
                .lazy_worker_tt_reuse
                .get(i)
                .copied()
                .unwrap_or_default();
            format!(
                "w{i}{marker}:d{}:n{}:s{}:t{}:x{}:a{}:p{}:h{}:r{}:l{}:f{}:u{}",
                worker.depth,
                worker.nodes,
                worker.score,
                worker.elapsed.as_millis(),
                u8::from(worker.aborted),
                worker.abort_reason,
                worker.tt_probes,
                worker.tt_hits,
                worker.tt_stores,
                reuse.local_hits,
                reuse.foreign_hits,
                reuse.unknown_hits,
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    println!(
        "info string lazy_smp selected w{selected_index} node_share_permille {selected_share_permille} move_agreement {agreement}/{} discarded_root_nodes {discarded_root_nodes} divergent_discarded_nodes {divergent_discarded_nodes} stop_lag_ms {stop_lag_ms} selected_tt_local_hits {} selected_tt_foreign_hits {} selected_tt_unknown_hits {} {summary}",
        info.worker_stats.len(),
        selected_reuse.local_hits,
        selected_reuse.foreign_hits,
        selected_reuse.unknown_hits,
    );
}

fn emit_lazy_smp_tt_diagnostics(info: &SearchResult) {
    let Some(writes) = info.lazy_tt_writes else {
        return;
    };
    let probes = info
        .worker_stats
        .iter()
        .map(|worker| worker.tt_probes)
        .sum::<u64>();
    let hits = info
        .worker_stats
        .iter()
        .map(|worker| worker.tt_hits)
        .sum::<u64>();
    println!(
        "info string lazy_smp_tt probes {probes} hits {hits} attempted {} committed {} same_hash {} equal_depth {} shallower_rejected {} collision_overwrites {}",
        writes.attempted,
        writes.committed,
        writes.same_hash,
        writes.equal_depth_overwrites,
        writes.shallower_rejections,
        writes.collision_overwrites,
    );
}

fn emit_lazy_smp_overlap_diagnostics(info: &SearchResult) {
    let Some(overlap) = info.lazy_work_overlap else {
        return;
    };
    println!(
        "info string lazy_smp_overlap sample_rate 256 visits {} unique {} within_worker_revisits {} cross_worker_visits {} shared_fingerprints {} dropped {} tt_sampled_stores {} tt_sampled_hits {} tt_local_hits {} tt_foreign_hits {} tt_unknown_hits {} tt_dropped_stores {}",
        overlap.sampled_visits,
        overlap.unique_fingerprints,
        overlap.within_worker_revisits,
        overlap.cross_worker_visits,
        overlap.shared_fingerprints,
        overlap.dropped,
        overlap.tt_sampled_stores,
        overlap.tt_sampled_hits,
        overlap.tt_local_hits,
        overlap.tt_foreign_hits,
        overlap.tt_unknown_hits,
        overlap.tt_dropped_stores,
    );
}

fn aspiration_delta(after: AspirationSnapshot, before: AspirationSnapshot) -> AspirationSnapshot {
    AspirationSnapshot {
        by_depth: std::array::from_fn(|d| {
            std::array::from_fn(|e| after.by_depth[d][e].saturating_sub(before.by_depth[d][e]))
        }),
    }
}

fn nmp_delta(after: NmpSnapshot, before: NmpSnapshot) -> NmpSnapshot {
    NmpSnapshot {
        attempts: after.attempts.saturating_sub(before.attempts),
        fail_high: after.fail_high.saturating_sub(before.fail_high),
        verification_attempts: after
            .verification_attempts
            .saturating_sub(before.verification_attempts),
        verification_successes: after
            .verification_successes
            .saturating_sub(before.verification_successes),
        verification_overturns: after
            .verification_overturns
            .saturating_sub(before.verification_overturns),
        pv_returns: after.pv_returns.saturating_sub(before.pv_returns),
        tracking_overflow: after
            .tracking_overflow
            .saturating_sub(before.tracking_overflow),
        prior_reductions: after
            .prior_reductions
            .saturating_sub(before.prior_reductions),
        by_depth: std::array::from_fn(|d| {
            std::array::from_fn(|e| after.by_depth[d][e].saturating_sub(before.by_depth[d][e]))
        }),
    }
}

fn lazy_smp_root_hint_diagnostic_line(stats: LazySmpRootHintEffectiveness) -> String {
    format!(
        "info string lazy_smp_root_hint publications {} eligible_reads {} applied {} first_move_beta_cuts {} researches {} research_nodes {} stale_research_nodes {} stale_reads {} depth_gap_sum {} depth_gap_max {} generation_gap_sum {} generation_gap_max {}",
        stats.publications,
        stats.eligible_reads,
        stats.applied,
        stats.first_move_beta_cuts,
        stats.researches,
        stats.research_nodes,
        stats.stale_research_nodes,
        stats.stale_reads,
        stats.depth_gap_sum,
        stats.depth_gap_max,
        stats.generation_gap_sum,
        stats.generation_gap_max,
    )
}

fn emit_lazy_smp_root_hint_diagnostics(info: &SearchResult) {
    let Some(stats) = info.lazy_root_hints else {
        return;
    };
    println!("{}", lazy_smp_root_hint_diagnostic_line(stats));
}

/// Emit the complete response for one search generation.  Keeping this in one
/// place prevents `go`, `stop`, and `ponderhit` from rendering a completed
/// result with subtly different score/PV/ponder semantics.
fn emit_search_result(
    searcher: &SearchBackend,
    board: &mut Board,
    info: SearchResult,
    diag_ctx: &DiagCtx,
) {
    let elapsed_ms = info.elapsed.as_millis().max(1) as u64;
    let nps = info.nodes.saturating_mul(1000) / elapsed_ms;
    emit_lazy_smp_diagnostics(&info);
    emit_lazy_smp_tt_diagnostics(&info);
    emit_lazy_smp_overlap_diagnostics(&info);
    emit_lazy_smp_root_hint_diagnostics(&info);
    if let Some(workers) = &info.lazy_retirement {
        for (worker, (iterations, retired, contributions)) in workers.iter().enumerate() {
            println!(
                "info string lazy_smp_retirement worker {worker} completed_iterations {iterations} retired {} foreign_contributions {contributions}",
                u8::from(*retired)
            );
        }
    }
    if let Some(stats) = info.cooperative_root {
        println!(
            "info string cooperative_root generations {} helper_nodes {} helper_probes {} helper_fail_highs {} main_confirmations {} confirmed_alpha_raises {} main_nodes {} main_tasks {} confirmation_nodes {} helper_completed_nodes {} helper_aborted_nodes {} helper_aborted_probes {} spawn_failures {} helpers_started {} helper_start_delay_us {} join_wait_us {} alpha_gap_sum {} alpha_gap_max {} physical_helper_threads_started {} physical_helper_threads_joined {} executor_start_delay_us {} helper_dispatch_delay_us {}",
            stats.generations,
            stats.helper_nodes,
            stats.helper_probes,
            stats.helper_fail_highs,
            stats.main_confirmations,
            stats.confirmed_alpha_raises,
            stats.main_nodes,
            stats.main_tasks,
            stats.confirmation_nodes,
            stats.helper_completed_nodes,
            stats.helper_aborted_nodes,
            stats.helper_aborted_probes,
            stats.spawn_failures,
            stats.helpers_started,
            stats.helper_start_delay_us,
            stats.join_wait_us,
            stats.alpha_gap_sum,
            stats.alpha_gap_max,
            stats.physical_helper_threads_started,
            stats.physical_helper_threads_joined,
            stats.executor_start_delay_us,
            stats.helper_dispatch_delay_us,
        );
    }
    if let Some(stats) = info.cooperative_root_wait.filter(|stats| stats.enabled) {
        println!(
            "info string cooperative_root_wait enabled 1 waiting_confirmations {} waiting_confirmation_nodes {} waiting_alpha_raises {} notification_wait_us {}",
            stats.waiting_confirmations,
            stats.waiting_confirmation_nodes,
            stats.waiting_alpha_raises,
            stats.notification_wait_us,
        );
    }
    if info
        .cooperative_root_wait
        .is_some_and(|stats| stats.priority_enabled)
    {
        println!("info string cooperative_root_policy priority 1");
    }
    if let Some(stats) = info.cooperative_root_wait.and_then(|stats| stats.latency) {
        println!(
            "info string cooperative_root_latency enabled 1 publications {} scout_publications {} research_publications {} confirmation_publications {} idle_publications {} phase_transition_publications {} beta_scout_publications {} confirmations {} confirmation_latency_us_sum {} confirmation_latency_us_max {} confirmation_main_nodes_sum {} confirmation_main_nodes_max {} scout_confirmations {} scout_delay_nodes_sum {} beta_scout_confirmations {} beta_scout_delay_nodes_sum {} beta_scout_confirmed_cuts {} unconfirmed_publications {} rejected_publications {}",
            stats.publications,
            stats.scout_publications,
            stats.research_publications,
            stats.confirmation_publications,
            stats.idle_publications,
            stats.phase_transition_publications,
            stats.beta_scout_publications,
            stats.confirmations,
            stats.confirmation_latency_us_sum,
            stats.confirmation_latency_us_max,
            stats.confirmation_main_nodes_sum,
            stats.confirmation_main_nodes_max,
            stats.scout_confirmations,
            stats.scout_delay_nodes_sum,
            stats.beta_scout_confirmations,
            stats.beta_scout_delay_nodes_sum,
            stats.beta_scout_confirmed_cuts,
            stats.unconfirmed_publications,
            stats.rejected_publications,
        );
    }
    if let Some(stats) = info.cooperative_root_wait.and_then(|stats| stats.outcomes) {
        println!(
            "info string cooperative_root_outcomes enabled 1 scout_confirmations {} research_confirmations {} confirmation_confirmations {} idle_confirmations {} phase_transition_confirmations {} scout_beta_cuts {} research_beta_cuts {} confirmation_beta_cuts {} idle_beta_cuts {} phase_transition_beta_cuts {} priority_choices {} priority_reorders {} reordered_alpha_raises {} reordered_beta_cuts {} reordered_confirmation_nodes {}",
            stats.scout_confirmations,
            stats.research_confirmations,
            stats.confirmation_confirmations,
            stats.idle_confirmations,
            stats.phase_transition_confirmations,
            stats.scout_beta_cuts,
            stats.research_beta_cuts,
            stats.confirmation_beta_cuts,
            stats.idle_beta_cuts,
            stats.phase_transition_beta_cuts,
            stats.priority_choices,
            stats.priority_reorders,
            stats.reordered_alpha_raises,
            stats.reordered_beta_cuts,
            stats.reordered_confirmation_nodes,
        );
    }
    for (worker, stats) in info.aspiration_stats.iter().enumerate() {
        let totals: [u64; 10] =
            std::array::from_fn(|column| stats.by_depth.iter().map(|r| r[column]).sum());
        println!(
            "info string root_aspiration worker {worker} counts {}",
            totals
                .iter()
                .map(u64::to_string)
                .collect::<Vec<_>>()
                .join(",")
        );
        for (depth, counts) in stats
            .by_depth
            .iter()
            .enumerate()
            .filter(|(_, row)| row.iter().any(|&n| n != 0))
        {
            println!(
                "info string root_aspiration_depth worker {worker} depth {depth} counts {}",
                counts
                    .iter()
                    .map(u64::to_string)
                    .collect::<Vec<_>>()
                    .join(",")
            );
        }
    }
    for (worker, stats) in info.nmp_stats.iter().enumerate() {
        println!(
            "info string v2_nmp worker {worker} attempts {} fail_high {} verification_attempts {} verification_successes {} verification_overturns {} pv_returns {} tracking_overflow {} prior_reductions {}",
            stats.attempts,
            stats.fail_high,
            stats.verification_attempts,
            stats.verification_successes,
            stats.verification_overturns,
            stats.pv_returns,
            stats.tracking_overflow,
            stats.prior_reductions
        );
        for (depth, counts) in stats
            .by_depth
            .iter()
            .enumerate()
            .filter(|(_, counts)| counts.iter().any(|&c| c != 0))
        {
            println!(
                "info string v2_nmp_depth worker {worker} depth {depth} counts {}",
                counts
                    .iter()
                    .map(u64::to_string)
                    .collect::<Vec<_>>()
                    .join(",")
            );
        }
    }
    if let Some(root_safety) = info.root_safety {
        println!(
            "info string root_mate_safety mate1_cache_hits {} blunder_cache_hits {} mate1_nodes {} blunder_nodes {}",
            root_safety.root_mate_in_one_cache_hits,
            root_safety.root_mate_blunder_cache_hits,
            root_safety.root_mate_in_one_nodes,
            root_safety.root_mate_blunder_nodes,
        );
    }
    if let Some((simulations, arena_nodes, transposition_hits)) = info.shared_mcts_stats {
        println!(
            "info string shared_mcts simulations {simulations} arena_nodes {arena_nodes} transposition_hits {transposition_hits}"
        );
    }
    if let Some((status, search_info)) = render_dfpn_info(&info, nps, elapsed_ms) {
        println!("{status}");
        println!("{search_info}");
    } else if info.pv_list.len() > 1 {
        for (i, &(mv, score)) in info.pv_list.iter().enumerate() {
            println!(
                "info multipv {} depth {} score {} nodes {} nps {} time {} hashfull {} pv {}",
                i + 1,
                info.depth,
                score_to_usi(score),
                info.nodes,
                nps,
                elapsed_ms,
                info.hashfull,
                render_pv(&info.pv, mv, i == 0)
            );
        }
    } else if let Some(m) = info.best_move {
        println!(
            "info depth {} score {} nodes {} nps {} time {} hashfull {} pv {}",
            info.depth,
            score_to_usi(info.score),
            info.nodes,
            nps,
            elapsed_ms,
            info.hashfull,
            render_pv(&info.pv, m, true)
        );
    }

    let best = info
        .best_move
        .map(move_to_usi)
        .unwrap_or_else(|| "resign".to_string());
    if let Some(mv) = info.best_move {
        invariant::assert_legal_bestmove(board, mv, diag_ctx);
    }
    let ponder_token = info
        .best_move
        .and_then(|m| legal_ponder_move(searcher, board, m));
    if let Some(pm) = ponder_token {
        println!("bestmove {best} ponder {}", move_to_usi(pm));
    } else {
        println!("bestmove {best}");
    }
    io::stdout().lock().flush().ok();
}

// ---- Main loop ----

/// Initialize Rayon's process-global pool before the first search, with a
/// stack budget sufficient for the engine's supported recursive search depth.
/// Rayon permits this only once; later calls intentionally preserve the pool
/// selected by an earlier `Threads` option.
fn ensure_search_pool(threads: usize) {
    let builder = rayon::ThreadPoolBuilder::new().stack_size(SEARCH_STACK_BYTES);
    let builder = if threads == 0 {
        builder
    } else {
        builder.num_threads(threads)
    };
    let _ = builder.build_global();
}

/// Run a top-level USI search with the same stack guarantee as its Rayon
/// workers. A failed spawn is unrecoverable because no valid USI response can
/// be produced without a search controller.
fn spawn_search_thread(task: impl FnOnce() + Send + 'static) -> JoinHandle<()> {
    std::thread::Builder::new()
        .name("sekirei-search".to_owned())
        .stack_size(SEARCH_STACK_BYTES)
        .spawn(task)
        .expect("failed to spawn sekirei search thread")
}

fn main() {
    if let Some(arg) = std::env::args().nth(1)
        && matches!(arg.as_str(), "--version" | "-V")
    {
        println!("Sekirei {ENGINE_VERSION}");
        return;
    }
    if let Some(arg) = std::env::args().nth(1)
        && matches!(arg.as_str(), "--help" | "-h")
    {
        println!(
            "Sekirei {}\n\nUSI shogi engine\n\nUsage:\n  sekirei [NNUE_WEIGHTS]\n  sekirei --version\n  sekirei --help\n\nThe engine reads USI commands from stdin.",
            ENGINE_VERSION
        );
        return;
    }

    // Optional: load NNUE weights from first command-line argument
    // Usage: cargo run --release -p usi -- weights.bin
    let mut weight_path = String::new();
    let mut weight_hash: Option<u64> = None;
    if let Some(path) = std::env::args().nth(1) {
        match load_evaluator(Path::new(&path)) {
            Ok(format) => {
                eprintln!(
                    "info string NNUE weights loaded from {path} ({})",
                    format.as_str()
                );
                weight_hash = invariant::hash_file(&path);
                weight_path = path;
            }
            Err(e) => eprintln!("info string weight load failed ({path}): {e}"),
        }
    }
    let binary_hash = std::env::current_exe()
        .ok()
        .and_then(|p| invariant::hash_file(p.to_str()?));

    let stdin = io::stdin();
    let stdout = io::stdout();

    let mut hash_mb = DEFAULT_HASH_MB;
    let mut spec_top_n = DEFAULT_SPEC_TOP_N;
    // Mirrors the USI Threads option; zero means one Lazy SMP worker.
    let mut threads: u32 = 0;
    let mut search_mode = SearchMode::Auto;
    let mut searcher = make_searcher(hash_mb, spec_top_n, threads_for_lazy_smp(0), search_mode);
    let mut eval_file: Option<String> = None;
    // NNUE is intentionally a process-global OnceLock.  Remember the path
    // which filled it so every `isready` barrier (including match-runner's
    // per-game barrier) is quiet and does not attempt a second load.
    let mut loaded_eval_file = (!weight_path.is_empty()).then(|| weight_path.clone());
    let mut move_overhead_ms: u64 = 50;
    let mut multi_pv: u32 = 1;
    #[cfg(feature = "opening-book")]
    let mut use_book = false;
    #[cfg(feature = "opening-book")]
    let mut book_max_ply: usize = 30;
    #[cfg(feature = "opening-book")]
    let mut book_min_confidence: f64 = 0.20;
    #[cfg(feature = "opening-book")]
    let mut book_file = DEFAULT_BOOK_FILE.to_string();
    #[cfg(feature = "opening-book")]
    let mut book: Option<Book> = None;
    #[cfg(feature = "opening-book")]
    let mut book_loaded_path: Option<String> = None;
    #[cfg(feature = "opening-book")]
    let mut book_load_failed = false;
    #[cfg(feature = "opening-book")]
    let mut book_decision_logger = BookDecisionLogger::default();
    #[cfg(feature = "opening-book")]
    let mut book_decision_counter: u64 = 0;

    // Current board position (updated by "position" commands)
    let mut board = Board::startpos();
    let mut position_history = PositionHistory::initial(board.hash());
    // Ply reached by the last "position" command's move list (0 = startpos) --
    // used only to gate book lookups to the opening phase (BookMaxPly).
    #[cfg(feature = "opening-book")]
    let mut current_ply: usize = 0;

    // ---- invariant-check bookkeeping (crates/sekirei-usi/src/invariant.rs) ----
    // Incremented on every "usinewgame" -- carried into a bestmove-illegal
    // diagnostic dump so a failure can be tied back to a specific game in a
    // long-lived process, the way sprint_gate.sh's per-game logs are.
    let mut game_counter: u64 = 0;
    // Raw body of the last "position" command, for the same reason.
    let mut last_position_cmd = String::from("startpos");
    // Abort flag and handle for the currently running search (None if no search in flight)
    let mut search_abort: Option<Arc<AtomicBool>> = None;
    let mut search_handle: Option<JoinHandle<()>> = None;
    // A completed worker may be just about to publish `bestmove` when the
    // GUI replaces the position.  The abort flag alone is cooperative, so a
    // monotonically increasing generation gives publication an exact command
    // boundary: a worker may only publish or retain ponder output for the
    // `go` command that created it.
    let search_generation = Arc::new(AtomicU64::new(0));
    // Set true before aborting a ponder search so the dying thread skips bestmove output
    let suppress_bm: Arc<AtomicBool> = Arc::new(AtomicBool::new(false));
    // Saved args from `go ponder ...` so ponderhit can restart with real time limits
    let mut ponder_go_args: Option<String> = None;
    // A ponder search may finish before the GUI sends either `ponderhit` or
    // `stop` (a mate-in-one is the usual case).  USI must not send that
    // bestmove early: retain it and associate it with the command that ends
    // the ponder generation instead.
    let ponder_result: Arc<Mutex<Option<SearchResult>>> = Arc::new(Mutex::new(None));
    let mut active_ponder = false;

    for raw in stdin.lock().lines() {
        let Ok(line) = raw else { break };
        let line = line.trim().to_string();
        if line.is_empty() {
            continue;
        }

        let (cmd, rest) = line
            .split_once(' ')
            .map(|(c, r)| (c, r.trim()))
            .unwrap_or((&line, ""));

        match cmd {
            "usi" => {
                println!("id name {ENGINE_NAME}");
                println!("id author {ENGINE_AUTHOR}");
                println!("id version {ENGINE_VERSION}");
                print_spin_option("Hash", DEFAULT_HASH_MB as u64, HASH_MB_RANGE);
                print_spin_option("Threads", 0, THREADS_RANGE);
                println!(
                    "option name SearchMode type combo default Auto var Auto var Speculative var LazySMP var Dfpn var SharedMcts"
                );
                print_spin_option("SpecTopN", DEFAULT_SPEC_TOP_N as u64, SPEC_TOP_N_RANGE);
                print_spin_option("LazyFlags", u64::from(LAZY_DEFAULT_FLAGS), LAZY_FLAGS_RANGE);
                println!("option name LazyDiagnostics type check default false");
                println!("option name NmpDiagnostics type check default false");
                println!("option name AspirationDiagnostics type check default false");
                print_spin_option("MoveOverhead", 50, MOVE_OVERHEAD_RANGE);
                print_spin_option("IncrementUsePercent", 75, INCREMENT_USE_PERCENT_RANGE);
                println!("option name Ponder type check default false");
                print_spin_option("MultiPV", 1, MULTI_PV_RANGE);
                println!("option name EvalFile type string default ");
                println!(
                    "option name NnueOutput type combo default absolute var absolute var residual-material"
                );
                println!(
                    "option name NnueResidualScalePermille type spin default 1000 min 0 max 2000"
                );
                println!(
                    "option name FV_SCALE type spin default {} min 1 max 128",
                    halfkp::DEFAULT_FV_SCALE
                );
                #[cfg(feature = "opening-book")]
                {
                    println!("option name UseBook type check default false");
                    print_spin_option("BookMaxPly", 30, BOOK_MAX_PLY_RANGE);
                    println!("option name BookMinConfidence type string default 0.20");
                    println!("option name BookFile type string default {DEFAULT_BOOK_FILE}");
                    println!("option name BookDecisionLog type string default");
                    println!("option name BookExperimentId type string default default");
                }
                #[cfg(feature = "tune")]
                for spec in sekirei_core::search::params::ALL {
                    println!(
                        "option name T_{} type spin default {} min {} max {}",
                        spec.name, spec.default, spec.min, spec.max
                    );
                }
                println!("usiok");
                stdout.lock().flush().ok();
            }

            "isready" => {
                if let Some(ref path) = eval_file
                    && loaded_eval_file.as_deref() != Some(path.as_str())
                {
                    if let Some(loaded) = &loaded_eval_file {
                        println!(
                            "info string weight load failed: EvalFile switch rejected; already loaded {loaded}; restart the engine to use {path}"
                        );
                    } else {
                        let loaded = mutate_evaluator_after_join(
                            &mut search_abort,
                            &mut search_handle,
                            || match load_evaluator(Path::new(path)) {
                                Ok(format) => {
                                    println!("info string NNUE weights loaded from {path}");
                                    println!("info string evaluator format {}", format.as_str());
                                    true
                                }
                                Err(e) => {
                                    println!("info string weight load failed: {e}");
                                    false
                                }
                            },
                        );
                        if loaded {
                            // The board and searcher were constructed before
                            // the load. Refresh the accumulator and rebuild
                            // the backend so its optional NNUE cache observes
                            // the newly active global evaluator.
                            board.refresh_acc();
                            weight_hash = invariant::hash_file(path);
                            weight_path = path.clone();
                            searcher = make_searcher(
                                hash_mb,
                                spec_top_n,
                                threads_for_lazy_smp(threads),
                                search_mode,
                            );
                            loaded_eval_file = Some(path.clone());
                        }
                    }
                }
                #[cfg(feature = "opening-book")]
                if use_book && book_loaded_path.as_deref() != Some(book_file.as_str()) {
                    match Book::load(&book_file) {
                        Ok(b) => {
                            let provenance = match b.provenance() {
                                BookProvenance::Versioned {
                                    schema_version,
                                    producer_version,
                                    build_config,
                                    build_config_fingerprint,
                                } => format!(
                                    "schema={schema_version} producer={producer_version} build_config={build_config} build_config_fingerprint={build_config_fingerprint}"
                                ),
                                BookProvenance::LegacyFingerprint => {
                                    "legacy-fingerprint".to_string()
                                }
                                BookProvenance::Headerless => "headerless".to_string(),
                            };
                            println!(
                                "info string opening book loaded from {book_file} ({} positions; {provenance})",
                                b.len(),
                            );
                            book = Some(b);
                            book_loaded_path = Some(book_file.clone());
                            book_load_failed = false;
                        }
                        Err(e) => {
                            println!("info string opening book load failed ({book_file}): {e}");
                            book = None;
                            book_loaded_path = Some(book_file.clone()); // don't retry every isready
                            book_load_failed = true;
                        }
                    }
                }
                println!("readyok");
                stdout.lock().flush().ok();
            }

            "setoption" => {
                // "setoption name <Name> value <Value>"
                let parts: Vec<&str> = rest.split_whitespace().collect();
                let option_name = parts.get(1).copied();
                // GUIs send the standard `USI_Hash` (the USI protocol's
                // table size option) whether or not the engine lists it.
                if matches!(option_name, Some("Hash" | "USI_Hash"))
                    && let Some(mb) =
                        parse_spin_option_or_report(&parts, option_name.unwrap(), HASH_MB_RANGE)
                {
                    abort_and_join_inflight_search(&mut search_abort, &mut search_handle);
                    hash_mb = mb as usize;
                    searcher = make_searcher(
                        hash_mb,
                        spec_top_n,
                        threads_for_lazy_smp(threads),
                        search_mode,
                    );
                } else if let Some(name) = option_name.and_then(|n| n.strip_prefix("T_")) {
                    // Search constants for self-play tuning (builds with the
                    // `tune` feature only).
                    let value = parts.get(3).and_then(|v| v.parse::<i32>().ok());
                    if !value.is_some_and(|v| sekirei_core::search::params::set(name, v)) {
                        println!(
                            "info string cannot set T_{name}: unknown, bad value, or a build without the tune feature"
                        );
                    }
                } else if matches!(
                    option_name,
                    Some("LazyDiagnostics" | "NmpDiagnostics" | "AspirationDiagnostics")
                ) {
                    let Some(enabled) = parse_check_option_or_report(&parts, option_name.unwrap())
                    else {
                        continue;
                    };
                    abort_and_join_inflight_search(&mut search_abort, &mut search_handle);
                    if option_name == Some("LazyDiagnostics") {
                        LAZY_DIAGNOSTICS.store(enabled, Ordering::Relaxed);
                    } else if option_name == Some("NmpDiagnostics") {
                        NMP_DIAGNOSTICS.store(enabled, Ordering::Relaxed);
                    } else {
                        ASPIRATION_DIAGNOSTICS.store(enabled, Ordering::Relaxed);
                    }
                    searcher = make_searcher(
                        hash_mb,
                        spec_top_n,
                        threads_for_lazy_smp(threads),
                        search_mode,
                    );
                } else if option_name == Some("LazyFlags")
                    && let Some(flags) =
                        parse_spin_option_or_report(&parts, "LazyFlags", LAZY_FLAGS_RANGE)
                {
                    abort_and_join_inflight_search(&mut search_abort, &mut search_handle);
                    LAZY_FLAGS.store(flags as u32, Ordering::Relaxed);
                    searcher = make_searcher(
                        hash_mb,
                        spec_top_n,
                        threads_for_lazy_smp(threads),
                        search_mode,
                    );
                } else if option_name == Some("SpecTopN")
                    && let Some(top_n) =
                        parse_spin_option_or_report(&parts, "SpecTopN", SPEC_TOP_N_RANGE)
                {
                    // Same abort+join requirement as Hash just above, and for the
                    // identical reason: this rebuilds the searcher's dedicated
                    // speculative-search pool, which the in-flight search's own
                    // Arc<SpeculativeSearcher> clone is unaffected by, but nothing
                    // else here would otherwise block the main loop from moving on
                    // to the next command before that search's bestmove is printed.
                    abort_and_join_inflight_search(&mut search_abort, &mut search_handle);
                    spec_top_n = top_n as usize;
                    searcher = make_searcher(
                        hash_mb,
                        spec_top_n,
                        threads_for_lazy_smp(threads),
                        search_mode,
                    );
                } else if option_name == Some("Threads")
                    && let Some(worker_count) =
                        parse_spin_option_or_report(&parts, "Threads", THREADS_RANGE)
                {
                    threads = worker_count as u32;
                    ensure_search_pool(worker_count as usize);
                    if matches!(search_mode, SearchMode::LazySmp | SearchMode::Auto) {
                        abort_and_join_inflight_search(&mut search_abort, &mut search_handle);
                        searcher = make_searcher(
                            hash_mb,
                            spec_top_n,
                            threads_for_lazy_smp(threads),
                            search_mode,
                        );
                    }
                } else if option_name == Some("SearchMode")
                    && let Some(mode) = parts.get(3)
                {
                    let new_mode = match *mode {
                        "Auto" => SearchMode::Auto,
                        "LazySMP" => SearchMode::LazySmp,
                        "Speculative" => SearchMode::Speculative,
                        "Dfpn" => SearchMode::Dfpn,
                        "SharedMcts" => SearchMode::SharedMcts,
                        _ => continue,
                    };
                    abort_and_join_inflight_search(&mut search_abort, &mut search_handle);
                    search_mode = new_mode;
                    searcher = make_searcher(
                        hash_mb,
                        spec_top_n,
                        threads_for_lazy_smp(threads),
                        search_mode,
                    );
                } else if option_name == Some("IncrementUsePercent")
                    && let Some(percent) = parse_spin_option_or_report(
                        &parts,
                        "IncrementUsePercent",
                        INCREMENT_USE_PERCENT_RANGE,
                    )
                {
                    INC_USE_PCT.store(percent as u32, Ordering::Relaxed);
                } else if option_name == Some("MoveOverhead")
                    && let Some(overhead) =
                        parse_spin_option_or_report(&parts, "MoveOverhead", MOVE_OVERHEAD_RANGE)
                {
                    move_overhead_ms = overhead;
                } else if option_name == Some("MultiPV")
                    && let Some(value) =
                        parse_spin_option_or_report(&parts, "MultiPV", MULTI_PV_RANGE)
                {
                    multi_pv = value as u32;
                    let before = AUTO_MULTI_PV.swap(multi_pv, Ordering::Relaxed);
                    if search_mode == SearchMode::Auto && (before > 1) != (multi_pv > 1) {
                        abort_and_join_inflight_search(&mut search_abort, &mut search_handle);
                        searcher = make_searcher(
                            hash_mb,
                            spec_top_n,
                            threads_for_lazy_smp(threads),
                            search_mode,
                        );
                    }
                } else if parts.get(1) == Some(&"EvalFile") {
                    // value may contain spaces (e.g. paths with spaces)
                    if let Some(val) = rest.split_once("value ").map(|(_, v)| v.trim())
                        && !val.is_empty()
                    {
                        eval_file = Some(val.to_string());
                    }
                } else if parts.get(1) == Some(&"NnueOutput") {
                    let mode = match parts.get(3).copied() {
                        Some("absolute") => NnueOutputMode::Absolute,
                        Some("residual-material") => NnueOutputMode::ResidualMaterial,
                        Some(value) => {
                            println!(
                                "info string invalid NnueOutput {value}; expected absolute or residual-material"
                            );
                            continue;
                        }
                        None => continue,
                    };
                    // TT scores depend on the evaluator, so never retain entries
                    // across a semantic switch.  The default is absolute and
                    // residual files require their training sidecar to be checked
                    // by the caller before this option is selected.
                    mutate_evaluator_after_join(&mut search_abort, &mut search_handle, || {
                        set_nnue_output_mode(mode)
                    });
                    searcher = make_searcher(
                        hash_mb,
                        spec_top_n,
                        threads_for_lazy_smp(threads),
                        search_mode,
                    );
                    println!("info string NNUE output mode {}", mode.as_str());
                } else if parts.get(1) == Some(&"NnueResidualScalePermille") {
                    let Some(scale) = parts.get(3).and_then(|value| value.parse::<u16>().ok())
                    else {
                        println!(
                            "info string invalid NnueResidualScalePermille; expected 0..=2000"
                        );
                        continue;
                    };
                    if let Err(error) = validate_nnue_residual_scale_permille(scale) {
                        println!("info string invalid NnueResidualScalePermille: {error}");
                        continue;
                    }
                    // A residual-scale change alters every static evaluation,
                    // so cached bounds from the prior scale cannot be reused.
                    // Validation must remain side-effect-free: only update the
                    // global scale after the old search has fully joined.
                    mutate_evaluator_after_join(&mut search_abort, &mut search_handle, || {
                        set_nnue_residual_scale_permille(scale)
                            .expect("residual scale was validated before search shutdown")
                    });
                    searcher = make_searcher(
                        hash_mb,
                        spec_top_n,
                        threads_for_lazy_smp(threads),
                        search_mode,
                    );
                    println!("info string NNUE residual scale {scale} permille");
                } else if parts.get(1) == Some(&"FV_SCALE") {
                    // Output divisor for external HalfKP networks. Some
                    // published networks recommend a value other than 16.
                    let Some(scale) = parts.get(3).and_then(|value| value.parse::<i32>().ok())
                    else {
                        println!("info string invalid FV_SCALE; expected 1..=128");
                        continue;
                    };
                    if !(1..=128).contains(&scale) {
                        println!("info string invalid FV_SCALE; expected 1..=128");
                        continue;
                    }
                    mutate_evaluator_after_join(&mut search_abort, &mut search_handle, || {
                        halfkp::set_fv_scale(scale).expect("FV_SCALE was validated")
                    });
                    searcher = make_searcher(
                        hash_mb,
                        spec_top_n,
                        threads_for_lazy_smp(threads),
                        search_mode,
                    );
                    println!("info string FV_SCALE {scale}");
                } else if cfg!(feature = "opening-book") && parts.get(1) == Some(&"UseBook") {
                    #[cfg(feature = "opening-book")]
                    if let Some(v) = parts.get(3) {
                        use_book = *v == "true";
                    }
                } else if cfg!(feature = "opening-book")
                    && option_name == Some("BookMaxPly")
                    && let Some(_max_ply) =
                        parse_spin_option_or_report(&parts, "BookMaxPly", BOOK_MAX_PLY_RANGE)
                {
                    #[cfg(feature = "opening-book")]
                    {
                        book_max_ply = _max_ply as usize;
                    }
                } else if cfg!(feature = "opening-book")
                    && option_name == Some("BookMinConfidence")
                    && let Some(_confidence) =
                        parse_unit_interval_option_or_report(&parts, "BookMinConfidence")
                {
                    #[cfg(feature = "opening-book")]
                    {
                        book_min_confidence = _confidence;
                    }
                } else if cfg!(feature = "opening-book")
                    && parts.get(1) == Some(&"BookFile")
                    && let Some(val) = rest.split_once("value ").map(|(_, v)| v.trim())
                    && !val.is_empty()
                {
                    #[cfg(feature = "opening-book")]
                    {
                        book_file = val.to_string();
                        book = None;
                        book_loaded_path = None;
                        book_load_failed = false;
                    }
                } else if cfg!(feature = "opening-book") && parts.get(1) == Some(&"BookDecisionLog")
                {
                    #[cfg(feature = "opening-book")]
                    {
                        let value = rest
                            .split_once("value ")
                            .map(|(_, value)| value.trim())
                            .unwrap_or("");
                        let experiment = book_decision_logger.experiment_id().to_string();
                        book_decision_logger.configure(value, &experiment);
                    }
                } else if cfg!(feature = "opening-book")
                    && parts.get(1) == Some(&"BookExperimentId")
                {
                    #[cfg(feature = "opening-book")]
                    {
                        let value = rest
                            .split_once("value ")
                            .map(|(_, value)| value.trim())
                            .unwrap_or("default");
                        book_decision_logger.set_experiment_id(value);
                    }
                }
            }

            "usinewgame" => {
                invalidate_and_join_inflight_search(
                    &search_generation,
                    &mut search_abort,
                    &mut search_handle,
                );
                active_ponder = false;
                ponder_go_args = None;
                ponder_result
                    .lock()
                    .expect("ponder result lock poisoned")
                    .take();
                suppress_bm.store(false, Ordering::Relaxed);
                board = Board::startpos();
                position_history = PositionHistory::initial(board.hash());
                #[cfg(feature = "opening-book")]
                {
                    current_ply = 0;
                    book_decision_counter = 0;
                }
                last_position_cmd = String::from("startpos");
                searcher.clear_tt();
                game_counter += 1;
            }

            "position" => {
                // `position` supersedes the old question at command arrival,
                // not only after the replacement position parses.  Parsing a
                // long move list can take long enough for the old worker to
                // publish in the meantime, and malformed input must fail
                // closed instead of leaving the old search alive.
                invalidate_and_join_inflight_search(
                    &search_generation,
                    &mut search_abort,
                    &mut search_handle,
                );
                active_ponder = false;
                ponder_go_args = None;
                ponder_result
                    .lock()
                    .expect("ponder result lock poisoned")
                    .take();
                suppress_bm.store(false, Ordering::Relaxed);

                match parse_position_cmd_with_history(rest) {
                    Ok((b, history)) => {
                        board = b;
                        position_history = history;
                        // Only gates book lookups (BookMaxPly) -- doesn't need to
                        // handle every conceivable "position" form, just the
                        // "startpos moves ..." shape this project's own tooling
                        // always sends.
                        #[cfg(feature = "opening-book")]
                        {
                            current_ply = rest
                                .split_whitespace()
                                .skip_while(|&t| t != "moves")
                                .skip(1)
                                .count();
                        }
                        last_position_cmd = rest.to_string();
                        // Must run before any search on this position -- an
                        // already-desynced board must never be allowed to
                        // search at all, since its bestmove would be answering
                        // the wrong question. Panics (stderr diagnostics) on
                        // mismatch instead of returning, by design. Replays the
                        // move list independently against a shadow board (not
                        // just the base SFEN) -- see invariant.rs's
                        // verify_position_replay doc comment for why.
                        invariant::verify_position_replay(
                            rest,
                            &board,
                            &invariant::ReplayDiagCtx {
                                game_counter,
                                weight_path: weight_path.clone(),
                                weight_hash,
                                binary_hash,
                            },
                        );
                    }
                    Err(e) => eprintln!("position error: {e}"),
                }
            }

            "go" => {
                // Abort any in-flight search and join before starting a new one.
                // suppress_bm stays false so the dying thread still emits bestmove.
                abort_and_join_inflight_search(&mut search_abort, &mut search_handle);
                let pondering = rest.split_whitespace().any(|t| t == "ponder");
                active_ponder = pondering;
                ponder_result
                    .lock()
                    .expect("ponder result lock poisoned")
                    .take();
                if pondering {
                    ponder_go_args = Some(rest.to_string());
                } else {
                    ponder_go_args = None;
                }
                // Reset suppress flag now that the previous thread has joined.
                suppress_bm.store(false, Ordering::Relaxed);
                let generation = search_generation.fetch_add(1, Ordering::AcqRel) + 1;

                // Book lookup: skip search entirely for a known opening
                // position, within BookMaxPly. Not applied while pondering --
                // that has its own ponderhit/new-position protocol flow that
                // an instant book bestmove would short-circuit incorrectly.
                #[cfg(feature = "opening-book")]
                if !pondering && book_decision_logger.enabled() {
                    book_decision_counter += 1;
                    let sfen = board_to_sfen(&board);
                    let mut candidates = Vec::new();
                    let mut selected = None;
                    let fallback_reason = if !use_book {
                        Some("use_book_false")
                    } else if current_ply >= book_max_ply {
                        Some("past_book_max_ply")
                    } else if let Some(loaded_book) = &book {
                        let decision = loaded_book.decision(&sfen, &board, book_min_confidence);
                        candidates = decision.candidates;
                        selected = decision.selected;
                        decision.fallback_reason
                    } else if book_load_failed {
                        Some("load_failure")
                    } else {
                        Some("book_unavailable")
                    };
                    let selected_action = selected.map(move_to_usi);
                    let experiment_id = book_decision_logger.experiment_id();
                    if let Err(error) = book_decision_logger.decision(DecisionContext {
                        experiment_id,
                        game_counter,
                        decision_counter: book_decision_counter,
                        ply: current_ply,
                        sfen: &sfen,
                        use_book,
                        book_max_ply,
                        book_min_confidence,
                        book_sha256: book.as_ref().map(Book::sha256),
                        book_schema_version: book.as_ref().and_then(Book::schema_version),
                        book_producer_version: book.as_ref().and_then(Book::producer_version),
                        build_config_fingerprint: book
                            .as_ref()
                            .and_then(Book::build_config_fingerprint),
                        candidates: &candidates,
                        selected_action: selected_action.as_deref(),
                        action_source: if selected.is_some() { "book" } else { "search" },
                        fallback_reason,
                    }) {
                        println!("info string book decision log failed: {error}");
                    }
                    if let Some(mv) = selected {
                        println!("info string book move");
                        invariant::assert_legal_bestmove(
                            &board,
                            mv,
                            &DiagCtx {
                                game_counter,
                                last_position_cmd: last_position_cmd.clone(),
                                weight_path: weight_path.clone(),
                                weight_hash,
                                threads,
                                board_hash_at_search_start: board.hash(),
                                accumulator_hash_at_search_start: invariant::hash_accumulator(
                                    &board.acc,
                                ),
                            },
                        );
                        println!("bestmove {}", move_to_usi(mv));
                        stdout.lock().flush().ok();
                        continue;
                    }
                } else if !pondering
                    && use_book
                    && current_ply < book_max_ply
                    && let Some(loaded_book) = &book
                    && let Some(mv) =
                        loaded_book.lookup(&board_to_sfen(&board), &board, book_min_confidence)
                {
                    println!("info string book move");
                    invariant::assert_legal_bestmove(
                        &board,
                        mv,
                        &DiagCtx {
                            game_counter,
                            last_position_cmd: last_position_cmd.clone(),
                            weight_path: weight_path.clone(),
                            weight_hash,
                            threads,
                            board_hash_at_search_start: board.hash(),
                            accumulator_hash_at_search_start: invariant::hash_accumulator(
                                &board.acc,
                            ),
                        },
                    );
                    println!("bestmove {}", move_to_usi(mv));
                    stdout.lock().flush().ok();
                    continue;
                }

                let config = parse_go(
                    rest,
                    board.side_to_move,
                    move_overhead_ms,
                    pondering,
                    multi_pv,
                );
                ensure_search_pool(threads as usize);
                searcher.reset_abort_flag();
                let abort = searcher.abort_flag();
                search_abort = Some(abort);

                let searcher2 = Arc::clone(&searcher);
                let mut board2 = board.clone();
                let position_history2 = position_history.clone();
                let suppress2 = Arc::clone(&suppress_bm);
                let ponder_result2 = Arc::clone(&ponder_result);
                let search_generation2 = Arc::clone(&search_generation);
                let diag_ctx = DiagCtx {
                    game_counter,
                    last_position_cmd: last_position_cmd.clone(),
                    weight_path: weight_path.clone(),
                    weight_hash,
                    threads,
                    board_hash_at_search_start: board.hash(),
                    accumulator_hash_at_search_start: invariant::hash_accumulator(&board.acc),
                };

                search_handle = Some(spawn_search_thread(move || {
                    let info = searcher2.search(&mut board2, config, &position_history2);

                    if pondering {
                        if search_generation2.load(Ordering::Acquire) == generation {
                            *ponder_result2.lock().expect("ponder result lock poisoned") =
                                Some(info);
                        }
                        return;
                    }
                    if search_generation2.load(Ordering::Acquire) != generation
                        || suppress2.load(Ordering::Relaxed)
                    {
                        return; // ponderhit aborted this search; caller starts a new one
                    }
                    emit_search_result(&searcher2, &mut board2, info, &diag_ctx);
                }));
            }

            "stop" => {
                let was_pondering = active_ponder;
                abort_and_join_inflight_search(&mut search_abort, &mut search_handle);
                active_ponder = false;
                ponder_go_args = None;
                if was_pondering
                    && let Some(info) = ponder_result
                        .lock()
                        .expect("ponder result lock poisoned")
                        .take()
                {
                    let diag_ctx = DiagCtx {
                        game_counter,
                        last_position_cmd: last_position_cmd.clone(),
                        weight_path: weight_path.clone(),
                        weight_hash,
                        threads,
                        board_hash_at_search_start: board.hash(),
                        accumulator_hash_at_search_start: invariant::hash_accumulator(&board.acc),
                    };
                    emit_search_result(&searcher, &mut board, info, &diag_ctx);
                }
            }

            "ponderhit" => {
                if !active_ponder {
                    continue;
                }
                // If ponder already completed (notably a forced mate), return
                // that single retained response.  Otherwise discard the
                // aborted partial search and restart with real time limits.
                let completed = search_handle.as_ref().is_some_and(JoinHandle::is_finished);
                abort_and_join_inflight_search(&mut search_abort, &mut search_handle);
                active_ponder = false;
                if completed
                    && let Some(info) = ponder_result
                        .lock()
                        .expect("ponder result lock poisoned")
                        .take()
                {
                    ponder_go_args = None;
                    let diag_ctx = DiagCtx {
                        game_counter,
                        last_position_cmd: last_position_cmd.clone(),
                        weight_path: weight_path.clone(),
                        weight_hash,
                        threads,
                        board_hash_at_search_start: board.hash(),
                        accumulator_hash_at_search_start: invariant::hash_accumulator(&board.acc),
                    };
                    emit_search_result(&searcher, &mut board, info, &diag_ctx);
                    continue;
                }
                ponder_result
                    .lock()
                    .expect("ponder result lock poisoned")
                    .take();
                suppress_bm.store(false, Ordering::Relaxed);
                if let Some(ref args) = ponder_go_args.take() {
                    let config =
                        parse_go(args, board.side_to_move, move_overhead_ms, false, multi_pv);
                    ensure_search_pool(threads as usize);
                    searcher.reset_abort_flag();
                    let abort = searcher.abort_flag();
                    search_abort = Some(abort);
                    let searcher2 = Arc::clone(&searcher);
                    let mut board2 = board.clone();
                    let position_history2 = position_history.clone();
                    let suppress2 = Arc::clone(&suppress_bm);
                    let generation = search_generation.fetch_add(1, Ordering::AcqRel) + 1;
                    let search_generation2 = Arc::clone(&search_generation);
                    let diag_ctx = DiagCtx {
                        game_counter,
                        last_position_cmd: last_position_cmd.clone(),
                        weight_path: weight_path.clone(),
                        weight_hash,
                        threads,
                        board_hash_at_search_start: board.hash(),
                        accumulator_hash_at_search_start: invariant::hash_accumulator(&board.acc),
                    };
                    search_handle = Some(spawn_search_thread(move || {
                        let info = searcher2.search(&mut board2, config, &position_history2);
                        if search_generation2.load(Ordering::Acquire) != generation
                            || suppress2.load(Ordering::Relaxed)
                        {
                            return;
                        }
                        emit_search_result(&searcher2, &mut board2, info, &diag_ctx);
                    }));
                }
            }

            "gameover" =>
            {
                #[cfg(feature = "opening-book")]
                if let Err(error) = book_decision_logger.terminal(game_counter, rest) {
                    println!("info string book decision log failed: {error}");
                }
            }

            "quit" => {
                invalidate_and_join_inflight_search(
                    &search_generation,
                    &mut search_abort,
                    &mut search_handle,
                );
                break;
            }

            _ => {
                eprintln!("unknown command: '{cmd}'");
            }
        }
    }

    // A GUI or CSA adapter may terminate by closing stdin instead of sending
    // `quit`.  Dropping a JoinHandle would detach the search worker, so apply
    // the same abort-and-join barrier on every input-loop exit.
    invalidate_and_join_inflight_search(&search_generation, &mut search_abort, &mut search_handle);
}

// ---- Helpers ----

fn print_spin_option(option_name: &str, default: u64, range: SpinRange) {
    println!(
        "option name {option_name} type spin default {default} min {} max {}",
        range.min, range.max
    );
}

/// Parse a USI spin option and enforce the same inclusive range advertised by
/// the `usi` response.  Keeping this check at the protocol boundary prevents a
/// malformed GUI command from turning into an unbounded allocation or worker
/// count deeper in the engine.
fn parse_spin_option(parts: &[&str], option_name: &str, range: SpinRange) -> Result<u64, String> {
    let raw = parts.get(3).copied().unwrap_or("<missing>");
    if parts.get(2) != Some(&"value") {
        return Err(format!(
            "info string invalid {option_name} value {raw}; expected setoption name {option_name} value <{}..={}>",
            range.min, range.max
        ));
    }
    let value = raw.parse::<u64>().map_err(|_| {
        format!(
            "info string invalid {option_name} value {raw}; expected {}..={}",
            range.min, range.max
        )
    })?;
    if value < range.min || value > range.max {
        return Err(format!(
            "info string invalid {option_name} value {value}; expected {}..={}",
            range.min, range.max
        ));
    }
    Ok(value)
}

fn parse_spin_option_or_report(parts: &[&str], option_name: &str, range: SpinRange) -> Option<u64> {
    match parse_spin_option(parts, option_name, range) {
        Ok(value) => Some(value),
        Err(error) => {
            println!("{error}");
            None
        }
    }
}

fn parse_check_option(parts: &[&str], option_name: &str) -> Result<bool, String> {
    let raw = parts.get(3).copied().unwrap_or("<missing>");
    if parts.get(2) != Some(&"value") {
        return Err(format!(
            "info string invalid {option_name} value {raw}; expected setoption name {option_name} value <true|false>"
        ));
    }
    match raw {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(format!(
            "info string invalid {option_name} value {raw}; expected true or false"
        )),
    }
}

fn parse_check_option_or_report(parts: &[&str], option_name: &str) -> Option<bool> {
    match parse_check_option(parts, option_name) {
        Ok(value) => Some(value),
        Err(error) => {
            println!("{error}");
            None
        }
    }
}

fn parse_unit_interval_option(parts: &[&str], option_name: &str) -> Result<f64, String> {
    let raw = parts.get(3).copied().unwrap_or("<missing>");
    if parts.get(2) != Some(&"value") {
        return Err(format!(
            "info string invalid {option_name} value {raw}; expected setoption name {option_name} value <0.0..=1.0>"
        ));
    }
    let value = raw.parse::<f64>().map_err(|_| {
        format!("info string invalid {option_name} value {raw}; expected 0.0..=1.0")
    })?;
    if !value.is_finite() || !(0.0..=1.0).contains(&value) {
        return Err(format!(
            "info string invalid {option_name} value {raw}; expected 0.0..=1.0"
        ));
    }
    Ok(value)
}

fn parse_unit_interval_option_or_report(parts: &[&str], option_name: &str) -> Option<f64> {
    match parse_unit_interval_option(parts, option_name) {
        Ok(value) => Some(value),
        Err(error) => {
            println!("{error}");
            None
        }
    }
}

fn threads_for_lazy_smp(threads: u32) -> usize {
    threads.max(1) as usize
}

fn make_searcher(
    hash_mb: usize,
    spec_top_n: usize,
    threads: usize,
    mode: SearchMode,
) -> Arc<SearchBackend> {
    Arc::new(match mode {
        SearchMode::Auto if AUTO_MULTI_PV.load(Ordering::Relaxed) > 1 => {
            SearchBackend::speculative(hash_mb, AUTO_MULTI_PV_SPEC_TOP_N)
        }
        SearchMode::Auto if threads <= 1 => SearchBackend::speculative(hash_mb, 0),
        SearchMode::Auto => SearchBackend::lazy_smp(hash_mb, threads),
        SearchMode::Speculative => SearchBackend::speculative(hash_mb, spec_top_n),
        SearchMode::LazySmp => SearchBackend::lazy_smp(hash_mb, threads),
        SearchMode::Dfpn => SearchBackend::dfpn(),
        SearchMode::SharedMcts => SearchBackend::shared_mcts(),
    })
}

// ---- Go command time-control parsing ----

fn parse_go(
    args: &str,
    side: Color,
    overhead_ms: u64,
    pondering: bool,
    multi_pv: u32,
) -> SearchConfig {
    go::parse(
        args,
        side,
        overhead_ms,
        pondering,
        multi_pv,
        u64::from(INC_USE_PCT.load(Ordering::Relaxed)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use sekirei_core::color::Color;
    use std::sync::mpsc;

    #[test]
    fn evaluator_mutation_happens_after_inflight_search_has_joined() {
        let abort = Arc::new(AtomicBool::new(false));
        let worker_abort = Arc::clone(&abort);
        let (event_tx, event_rx) = mpsc::channel();
        let worker_tx = event_tx.clone();
        let handle = std::thread::spawn(move || {
            while !worker_abort.load(Ordering::Relaxed) {
                std::thread::yield_now();
            }
            worker_tx.send("search-finished").unwrap();
        });
        let mut search_abort = Some(abort);
        let mut search_handle = Some(handle);

        mutate_evaluator_after_join(&mut search_abort, &mut search_handle, || {
            event_tx.send("evaluator-mutated").unwrap();
        });

        assert_eq!(event_rx.recv().unwrap(), "search-finished");
        assert_eq!(event_rx.recv().unwrap(), "evaluator-mutated");
        assert!(search_abort.is_none());
        assert!(search_handle.is_none());
    }

    #[test]
    fn score_to_usi_preserves_cp_and_converts_mates() {
        assert_eq!(score_to_usi(137), "cp 137");
        assert_eq!(score_to_usi(-892), "cp -892");
        assert_eq!(score_to_usi(MATE_SCORE - 1), "mate 1");
        assert_eq!(score_to_usi(-(MATE_SCORE - 3)), "mate -3");
    }

    #[test]
    fn score_to_usi_keeps_mate_threshold_consistent() {
        assert_eq!(score_to_usi(MATE_SCORE - 1000), "mate 1000");
        assert_eq!(score_to_usi(-(MATE_SCORE - 1000)), "mate -1000");
        assert_eq!(score_to_usi(MATE_SCORE - 1001), "cp 898999");
    }

    #[test]
    fn dfpn_proof_does_not_turn_the_requested_limit_into_a_mate_distance() {
        let search = |max_depth| {
            let backend = SearchBackend::dfpn();
            let mut board = Board::from_sfen("k8/2K6/9/9/4R4/9/9/9/9 b - 1").unwrap();
            let history = PositionHistory::initial(board.hash());
            backend.search(
                &mut board,
                SearchConfig {
                    max_depth,
                    node_limit: Some(1_000),
                    ..SearchConfig::default()
                },
                &history,
            )
        };

        let depth_one = search(1);
        let depth_two = search(2);
        for (limit, result) in [(1, depth_one), (2, depth_two)] {
            let report = result.dfpn.expect("DFPN status must survive the adapter");
            assert_eq!(report.outcome, DfpnOutcome::Proven);
            assert_eq!(report.limit_depth, limit);
            assert_eq!(result.depth, 0);
            assert!(report.best_move_from_proof);
            let (status, info) = render_dfpn_info(&result, 1, 1).unwrap();
            assert!(status.contains("outcome proven"));
            assert!(status.contains("distance unknown"));
            assert!(status.contains(&format!("limit_depth {limit}")));
            assert!(status.contains("bestmove_source proof"));
            assert!(info.contains("score mate +"));
            assert!(!info.contains(&format!("score mate {limit}")));
        }
    }

    #[test]
    fn dfpn_unknown_and_disproven_are_not_rendered_as_cp_zero() {
        let backend = SearchBackend::dfpn();
        for node_limit in [0, 1] {
            backend.reset_abort_flag();
            let mut board = Board::from_sfen("k8/2K6/9/9/4R4/9/9/9/9 b - 1").unwrap();
            let history = PositionHistory::initial(board.hash());
            let result = backend.search(
                &mut board,
                SearchConfig {
                    max_depth: 50,
                    node_limit: Some(node_limit),
                    ..SearchConfig::default()
                },
                &history,
            );
            let report = result.dfpn.unwrap();
            assert_eq!(report.outcome, DfpnOutcome::Unknown);
            assert!(report.aborted);
            assert!(!report.best_move_from_proof);
            assert!(
                result.best_move.is_some(),
                "USI still needs a legal fallback"
            );
            assert!(
                result.pv.is_empty(),
                "fallback must not be presented as proof PV"
            );
            let (status, info) = render_dfpn_info(&result, 1, 1).unwrap();
            assert!(status.contains("outcome unknown"));
            assert!(status.contains("completed false"));
            assert!(status.contains("bestmove_source fallback"));
            assert!(!info.contains("score cp 0"));
            assert!(!info.contains(" pv "));
        }

        let mut board = Board::from_sfen("9/9/9/9/9/9/9/9/9 b - 1").unwrap();
        let history = PositionHistory::initial(board.hash());
        let result = backend.search(
            &mut board,
            SearchConfig {
                max_depth: 7,
                node_limit: Some(1_000),
                ..SearchConfig::default()
            },
            &history,
        );
        let (status, info) = render_dfpn_info(&result, 1, 1).unwrap();
        assert!(status.contains("outcome disproven"));
        assert!(status.contains("completed true"));
        assert!(status.contains("bestmove_source none"));
        assert!(!info.contains("score cp 0"));
    }

    #[test]
    fn parse_go_binc_winc() {
        // Black has 60s + 1s increment, moves_left = 30, IncrementUsePercent 75:
        // base = 60000/30 + 1000*75/100 = 2750, hard = 2750*3/2 = 4125,
        // soft = 2750*4/5 = 2200
        let cfg = parse_go(
            "btime 60000 wtime 60000 binc 1000 winc 1000",
            Color::Black,
            0,
            false,
            1,
        );
        assert!(cfg.time_limit.is_some(), "hard limit should be set");
        assert!(cfg.soft_limit.is_some(), "soft limit should be set");
        let hard = cfg.time_limit.unwrap().as_millis();
        let soft = cfg.soft_limit.unwrap().as_millis();
        assert!(soft < hard, "soft_limit must be less than hard time_limit");
    }

    #[test]
    fn parse_go_movestogo() {
        // 60s, movestogo=20 → from_main = 60000/20 = 3000
        let cfg = parse_go(
            "btime 60000 wtime 60000 movestogo 20",
            Color::Black,
            0,
            false,
            1,
        );
        let hard = cfg.time_limit.unwrap().as_millis();
        // base = 3000, hard = 4500
        assert!((hard as i64 - 4500).abs() < 100, "hard={hard}");
    }

    #[test]
    fn parse_go_byoyomi_only() {
        // byoyomi 5000, no main time → panic mode, no soft limit
        // byo_safe = 5000 and base = 5000: unused byoyomi is lost, so all of
        // it (minus the zero overhead) is available.
        let cfg = parse_go("byoyomi 5000", Color::Black, 0, false, 1);
        assert!(cfg.time_limit.is_some());
        assert!(cfg.soft_limit.is_none(), "panic mode: no soft limit");
        let hard = cfg.time_limit.unwrap().as_millis();
        assert_eq!(
            hard, 5000,
            "byoyomi-only search should use the full byoyomi"
        );
    }

    #[test]
    fn parse_go_soft_less_than_hard() {
        // Normal case: ample time, no panic
        let cfg = parse_go("btime 120000 wtime 120000", Color::Black, 0, false, 1);
        let hard = cfg.time_limit.unwrap().as_millis();
        let soft = cfg.soft_limit.unwrap().as_millis();
        assert!(soft < hard, "soft={soft} hard={hard}");
    }

    #[test]
    fn byoyomi_hard_within_overhead() {
        // byoyomi 5000, overhead 300 → hard must be <= byo - overhead = 4700
        let cfg = parse_go("byoyomi 5000", Color::Black, 300, false, 1);
        let hard = cfg.time_limit.unwrap().as_millis();
        assert!(hard <= 4700, "hard={hard} exceeds byoyomi - overhead");
    }

    #[test]
    fn pondering_no_limits() {
        let cfg = parse_go("btime 60000 wtime 60000 ponder", Color::Black, 50, true, 1);
        assert!(cfg.time_limit.is_none());
        assert!(cfg.soft_limit.is_none());
    }

    #[test]
    fn infinite_go_has_no_limits() {
        let cfg = parse_go("infinite", Color::White, 50, false, 1);
        assert!(cfg.time_limit.is_none());
        assert!(cfg.soft_limit.is_none());
    }

    #[test]
    fn multipv_value_reaches_search_config() {
        let cfg = parse_go("depth 2", Color::Black, 0, false, 3);
        assert_eq!(cfg.multi_pv, 3);
        assert_eq!(cfg.max_depth, 2);
    }

    #[test]
    fn nodes_value_reaches_search_config_without_a_time_limit() {
        let cfg = parse_go("nodes 4096", Color::Black, 0, false, 1);
        assert_eq!(cfg.node_limit, Some(4096));
        assert!(cfg.time_limit.is_none());
        assert!(cfg.soft_limit.is_none());
    }

    #[test]
    fn malformed_nodes_value_is_ignored_without_panicking() {
        let cfg = parse_go("nodes not-a-number depth 1", Color::Black, 0, false, 1);
        assert_eq!(cfg.node_limit, None);
        assert_eq!(cfg.max_depth, 1);
    }

    #[test]
    fn malformed_clock_value_is_ignored_without_panicking() {
        let cfg = parse_go("btime not-a-number depth 1", Color::Black, 0, false, 1);
        assert!(cfg.time_limit.is_none());
        assert_eq!(cfg.max_depth, 1);
    }

    #[test]
    fn spin_option_parser_enforces_advertised_bounds() {
        let runtime_range = SpinRange::new(4, 9);
        assert_eq!(runtime_range.min, 4);
        assert_eq!(runtime_range.max, 9);
        assert_eq!(
            parse_spin_option(&["name", "Threads", "value", "0"], "Threads", THREADS_RANGE,),
            Ok(0)
        );
        assert_eq!(
            parse_spin_option(
                &["name", "LazyFlags", "value", "8191"],
                "LazyFlags",
                LAZY_FLAGS_RANGE,
            ),
            Ok(8191)
        );
        assert_eq!(
            parse_spin_option(
                &["name", "LazyFlags", "value", "4111"],
                "LazyFlags",
                LAZY_FLAGS_RANGE,
            ),
            Ok(4111)
        );
        assert!(
            parse_spin_option(
                &["name", "LazyFlags", "value", "32768"],
                "LazyFlags",
                LAZY_FLAGS_RANGE,
            )
            .is_err()
        );
        assert!(
            parse_spin_option(
                &["name", "Threads", "value", "513"],
                "Threads",
                THREADS_RANGE,
            )
            .is_err()
        );
        assert!(
            parse_spin_option(
                &["name", "Hash", "value", "18446744073709551616"],
                "Hash",
                HASH_MB_RANGE,
            )
            .is_err()
        );
        assert!(parse_spin_option(&["name", "MultiPV"], "MultiPV", MULTI_PV_RANGE).is_err());
        assert!(
            parse_spin_option(&["name", "Hash", "not-value", "64"], "Hash", HASH_MB_RANGE,)
                .is_err()
        );
    }

    #[test]
    fn check_option_parser_accepts_only_explicit_boolean_values() {
        assert_eq!(
            parse_check_option(
                &["name", "LazyDiagnostics", "value", "true"],
                "LazyDiagnostics",
            ),
            Ok(true)
        );
        assert_eq!(
            parse_check_option(
                &["name", "LazyDiagnostics", "value", "false"],
                "LazyDiagnostics",
            ),
            Ok(false)
        );
        for parts in [
            &["name", "LazyDiagnostics"][..],
            &["name", "LazyDiagnostics", "not-value", "true"][..],
            &["name", "LazyDiagnostics", "value", "yes"][..],
        ] {
            assert!(parse_check_option(parts, "LazyDiagnostics").is_err());
        }
    }

    #[test]
    fn unit_interval_parser_rejects_malformed_non_finite_and_out_of_range_values() {
        assert!(
            parse_unit_interval_option(
                &["name", "BookMinConfidence", "not-value", "0.5"],
                "BookMinConfidence",
            )
            .is_err()
        );
        assert!(
            parse_unit_interval_option(
                &["name", "BookMinConfidence", "value"],
                "BookMinConfidence",
            )
            .is_err()
        );
        for invalid in ["garbage", "NaN", "inf", "-0.1", "1.1"] {
            assert!(
                parse_unit_interval_option(
                    &["name", "BookMinConfidence", "value", invalid],
                    "BookMinConfidence",
                )
                .is_err(),
                "accepted {invalid}"
            );
        }
        assert_eq!(
            parse_unit_interval_option(
                &["name", "BookMinConfidence", "value", "0.25"],
                "BookMinConfidence",
            ),
            Ok(0.25)
        );
    }

    #[test]
    fn unit_interval_reporting_wrapper_preserves_valid_values_and_rejects_errors() {
        assert_eq!(
            parse_unit_interval_option_or_report(&["name", "x", "value", "0.25"], "x"),
            Some(0.25)
        );
        assert_eq!(
            parse_unit_interval_option_or_report(&["name", "x", "value", "NaN"], "x"),
            None
        );
    }

    #[test]
    fn every_backend_supports_common_lifecycle_operations() {
        let backends = [
            SearchBackend::speculative(1, 0),
            SearchBackend::speculative(1, 1),
            SearchBackend::lazy_smp(1, 1),
            SearchBackend::dfpn(),
            SearchBackend::shared_mcts(),
        ];
        for backend in backends {
            backend.abort_flag().store(true, Ordering::Relaxed);
            backend.reset_abort_flag();
            assert!(!backend.abort_flag().load(Ordering::Relaxed));
            backend.clear_tt();
            assert_eq!(backend.probe_tt(0x1234), None);
        }
        assert_eq!(threads_for_lazy_smp(0), 1);
        assert_eq!(threads_for_lazy_smp(3), 3);
    }

    #[test]
    fn lazy_root_hint_diagnostic_line_is_stable() {
        assert_eq!(
            lazy_smp_root_hint_diagnostic_line(LazySmpRootHintEffectiveness {
                publications: 7,
                eligible_reads: 5,
                applied: 3,
                ..LazySmpRootHintEffectiveness::default()
            }),
            "info string lazy_smp_root_hint publications 7 eligible_reads 5 applied 3 first_move_beta_cuts 0 researches 0 research_nodes 0 stale_research_nodes 0 stale_reads 0 depth_gap_sum 0 depth_gap_max 0 generation_gap_sum 0 generation_gap_max 0"
        );
    }

    #[test]
    fn backend_adjudicates_draw_and_both_perpetual_check_perspectives() {
        fn repeated_history(
            current: u64,
            black_checks: bool,
            white_checks: bool,
        ) -> PositionHistory {
            let mut history = PositionHistory::initial(current);
            for (hash, mover, gave_check) in [
                (11, Color::Black, black_checks),
                (12, Color::White, white_checks),
                (13, Color::Black, black_checks),
                (current, Color::White, white_checks),
            ]
            .into_iter()
            .cycle()
            .take(12)
            {
                history.push_after_move(hash, mover, gave_check);
            }
            history
        }

        let backend = SearchBackend::speculative(1, 0);
        let config = SearchConfig {
            max_depth: 1,
            time_limit: None,
            node_limit: Some(1),
            soft_limit: None,
            multi_pv: 1,
        };
        let mut board = Board::startpos();
        let current = board.hash();

        let draw = backend.search(&mut board, config, &repeated_history(current, false, false));
        assert_eq!(draw.score, 0);
        assert_eq!(draw.nodes, 0);
        assert!(draw.best_move.is_some());

        let losing = backend.search(&mut board, config, &repeated_history(current, true, false));
        assert_eq!(losing.score, -MATE_SCORE + 1);

        let winning = backend.search(&mut board, config, &repeated_history(current, false, true));
        assert_eq!(winning.score, MATE_SCORE - 1);
    }

    #[test]
    fn movetime_overhead_deducted() {
        // movetime 1000, overhead 50 → hard = 950
        let cfg = parse_go("movetime 1000", Color::Black, 50, false, 1);
        let hard = cfg.time_limit.unwrap().as_millis();
        assert!(hard <= 950, "hard={hard}");
        assert!(cfg.soft_limit.is_none());
    }

    #[test]
    fn oversized_clock_values_do_not_overflow_time_budget_arithmetic() {
        let cfg = parse_go(
            &format!("btime {} binc {} byoyomi {}", u64::MAX, u64::MAX, u64::MAX),
            Color::Black,
            0,
            false,
            1,
        );
        let hard = cfg.time_limit.expect("clock input should produce a limit");
        assert!(hard <= Duration::from_millis(u64::MAX));
    }

    #[test]
    fn spec_top_n_zero_uses_the_root_safety_enabled_usi_backend() {
        let backend = SearchBackend::speculative(1, 0);
        assert!(
            matches!(backend, SearchBackend::Sequential(_)),
            "SpecTopN=0 must not bypass the regular root-safety search path"
        );

        let mut board = Board::startpos();
        let history = PositionHistory::initial(board.hash());
        let result = backend.search(
            &mut board,
            SearchConfig {
                max_depth: 2,
                time_limit: None,
                // Leave enough budget for a completed depth-one root-safety
                // scan to be reused on the next iterative-deepening pass.
                node_limit: Some(20_000),
                soft_limit: None,
                multi_pv: 1,
            },
            &history,
        );
        let best = result
            .best_move
            .expect("root search must return a legal move");
        assert!(generate_legal_moves(&mut board).contains(&best));
        let root_safety = result
            .root_safety
            .expect("sequential USI route must expose root-safety metrics");
        assert!(root_safety.root_mate_in_one_cache_hits > 0);
        assert!(root_safety.root_mate_blunder_cache_hits > 0);
    }
}
