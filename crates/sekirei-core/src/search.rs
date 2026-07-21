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
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, AtomicU64, Ordering};
use std::time::Duration;

use crate::board::Board;
use crate::budget::{Budget, soft_limit_expired};
use crate::color::Color;
use crate::eval::{PIECE_VALUE, evaluate};
use crate::movegen::{generate_legal_captures, generate_legal_moves, is_in_check};
use crate::mv::Move;
use crate::piece::PieceKind;
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

/// Null Move Pruning reduction constant.
const NMP_R: u32 = 3;

/// Initial aspiration window half-width in centipawns.
const ASP_DELTA: i32 = 50;

/// Reverse Futility Pruning: margin per depth level in centipawns.
const RFP_MARGIN: i32 = 120;

/// Futility Pruning: margin for depth-1 quiet moves.
const FUTILITY_MARGIN: i32 = 300;

/// Late Move Pruning: base quiet-move count before pruning kicks in.
const LMP_BASE: usize = 5;

/// Check Extension: ply cap to prevent runaway check chains.
const CHECK_EXT_MAX_PLY: u32 = 30;

/// Singular Extension: minimum depth to consider extending the TT move.
const SE_MIN_DEPTH: u32 = 8;
/// Singular Extension: margin in centipawns (flat; empirically calibrated).
const SE_MARGIN: i32 = 64;

/// ProbCut: minimum depth to attempt a probabilistic shallow refutation search.
const PC_MIN_DEPTH: u32 = 8;
/// ProbCut: how far above beta a capture must score (shallow) to prune the node.
const PC_MARGIN: i32 = 200;

// ============================================================
// Killer Move Table
// ============================================================

const MAX_PLY: usize = 64;

/// Pack a Move into a u32 for atomic storage (19 bits used).
/// Sentinel value 0 means "no move" (square 0 with square 0 as from is an invalid board move).
#[inline]
fn pack_killer(m: Move) -> u32 {
    let from_val: u32 = match m.from {
        None => 81,
        Some(sq) => sq.index() as u32,
    };
    (m.to.index() as u32)
        | (from_val << 7)
        | ((m.promote as u32) << 14)
        | ((m.piece_kind.index() as u32) << 15)
}

#[inline]
fn unpack_killer(v: u32) -> Option<Move> {
    if v == 0 {
        return None;
    }
    let to_idx = (v & 0x7F) as u8;
    let from_val = ((v >> 7) & 0x7F) as u8;
    let promote = ((v >> 14) & 1) != 0;
    let kind_idx = ((v >> 15) & 0xF) as u8;
    let from = if from_val == 81 {
        None
    } else {
        Some(Square::from_index(from_val))
    };
    PieceKind::from_u8(kind_idx).map(|kind| Move {
        from,
        to: Square::from_index(to_idx),
        piece_kind: kind,
        promote,
    })
}

// Each ply's killer pair lives on its own cache line to prevent false sharing
// between threads searching different plies in parallel.
#[repr(align(64))]
struct KillerPair {
    k0: AtomicU32,
    k1: AtomicU32,
    _pad: [u8; 56],
}

struct KillerTable {
    slots: Vec<KillerPair>, // MAX_PLY entries, one cache line per ply
}

impl KillerTable {
    fn new() -> Self {
        KillerTable {
            slots: (0..MAX_PLY)
                .map(|_| KillerPair {
                    k0: AtomicU32::new(0),
                    k1: AtomicU32::new(0),
                    _pad: [0u8; 56],
                })
                .collect(),
        }
    }

    fn add(&self, ply: usize, m: Move) {
        if ply >= MAX_PLY {
            return;
        }
        let packed = pack_killer(m);
        let old_k0 = self.slots[ply].k0.swap(packed, Ordering::Relaxed);
        self.slots[ply].k1.store(old_k0, Ordering::Relaxed);
    }

    fn get(&self, ply: usize) -> [Option<Move>; 2] {
        if ply >= MAX_PLY {
            return [None, None];
        }
        [
            unpack_killer(self.slots[ply].k0.load(Ordering::Relaxed)),
            unpack_killer(self.slots[ply].k1.load(Ordering::Relaxed)),
        ]
    }
}

// ============================================================
// Countermove Heuristic Table
// ============================================================

/// For each opponent move (color × piece_kind × to), store the quiet move that
/// most recently caused a beta cutoff in response. Used to order quiet moves.
struct CountermoveTable {
    data: Vec<AtomicU32>, // 2 × PieceKind::COUNT × Square::NUM
}

impl CountermoveTable {
    fn new() -> Self {
        let len = 2 * PieceKind::COUNT * Square::NUM;
        CountermoveTable {
            data: (0..len).map(|_| AtomicU32::new(0)).collect(),
        }
    }

    #[inline]
    fn idx(color: Color, kind: PieceKind, to: Square) -> usize {
        color.index() * PieceKind::COUNT * Square::NUM
            + kind.index() * Square::NUM
            + to.index() as usize
    }

    fn update(&self, opp_color: Color, opp_mv: Move, response: Move) {
        let i = Self::idx(opp_color, opp_mv.piece_kind, opp_mv.to);
        self.data[i].store(pack_killer(response), Ordering::Relaxed);
    }

    fn get(&self, opp_color: Color, opp_mv: Move) -> Option<Move> {
        let i = Self::idx(opp_color, opp_mv.piece_kind, opp_mv.to);
        unpack_killer(self.data[i].load(Ordering::Relaxed))
    }
}

// ============================================================
// History Heuristic Table
// ============================================================

struct HistoryTable {
    // Indexed by color × PieceKind::COUNT × Square::NUM
    data: Vec<AtomicI32>,
}

impl HistoryTable {
    fn new() -> Self {
        let len = 2 * PieceKind::COUNT * Square::NUM;
        HistoryTable {
            data: (0..len).map(|_| AtomicI32::new(0)).collect(),
        }
    }

    #[inline]
    fn idx(color: Color, kind: PieceKind, to: Square) -> usize {
        color.index() * PieceKind::COUNT * Square::NUM
            + kind.index() * Square::NUM
            + to.index() as usize
    }

    /// Reward a move that caused a beta cutoff; bonus scales with depth².
    fn update(&self, color: Color, kind: PieceKind, to: Square, depth: u32) {
        let bonus = (depth * depth).min(400) as i32;
        let i = Self::idx(color, kind, to);
        let old = self.data[i].load(Ordering::Relaxed);
        // Clamp below captures (10_000) and promotions to keep band separation clean
        self.data[i].store((old + bonus).min(9_000), Ordering::Relaxed);
    }

    fn get(&self, color: Color, kind: PieceKind, to: Square) -> i32 {
        self.data[Self::idx(color, kind, to)].load(Ordering::Relaxed)
    }

    /// Penalise a quiet move that was tried but failed to produce a cutoff.
    fn malus(&self, color: Color, kind: PieceKind, to: Square, depth: u32) {
        let penalty = (depth * depth).min(400) as i32;
        let i = Self::idx(color, kind, to);
        let old = self.data[i].load(Ordering::Relaxed);
        self.data[i].store((old - penalty).max(-9_000), Ordering::Relaxed);
    }
}

// ============================================================
// Public API
// ============================================================

/// Iterative-deepening search parameters.
#[derive(Clone, Copy)]
pub struct SearchConfig {
    /// Maximum depth to search via iterative deepening.
    pub max_depth: u32,
    /// Hard time budget; the search aborts as soon as this elapses.
    pub time_limit: Option<Duration>,
    /// Soft limit: exit after completing a depth if elapsed >= soft_limit and bestmove is stable.
    pub soft_limit: Option<Duration>,
    /// Number of PV lines to return (1 = normal, >1 = MultiPV).
    pub multi_pv: u32,
    /// Enable YBW parallel search of young-brother siblings at `depth >=
    /// MIN_SPLIT_DEPTH` (default `true`, matching prior unconditional
    /// behavior). `false` forces every sibling to be searched sequentially.
    pub use_ybw: bool,
    /// Enable preemptive speculative search of the policy's top candidate
    /// replies (default `true`). Speculation only ever runs when this is
    /// true AND `multi_pv == 1` — the `multi_pv` requirement is a structural
    /// necessity (speculation predicts a single PV's reply), not a
    /// measurement toggle, so it stays a separate, non-overridable condition.
    pub use_speculation: bool,
    /// Number of top-policy-ranked candidates to speculatively search
    /// (default 3, matching the previous constructor-baked value).
    pub spec_top_n: usize,
    /// Max young-brother siblings dispatched in parallel per YBW split
    /// (default 6, matching the previous `YBW_MAX_SIBLINGS` constant).
    pub ybw_max_siblings: usize,
    /// Enable PVS (null-window-probe-then-full-window-research) at root and
    /// in the sequential tail, and make the YBW closure's own null-window
    /// probe conditional on this flag rather than unconditional (default
    /// `true`, matching the YBW block's prior unconditional PVS-probing
    /// behavior — root and the sequential tail gain real PVS for the first
    /// time under this default, which is the intended effect of this toggle
    /// existing at all).
    pub use_pvs: bool,
}

impl Default for SearchConfig {
    fn default() -> Self {
        SearchConfig {
            max_depth: 6,
            time_limit: None,
            soft_limit: None,
            multi_pv: 1,
            use_ybw: true,
            use_speculation: true,
            spec_top_n: 3,
            ybw_max_siblings: 6,
            use_pvs: true,
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
    /// Number of depths where bestmove changed (instability indicator).
    /// Mirrors `SpecSearchInfo::bestmove_changes` so both result types share
    /// a uniform field set (e.g. for a benchmark comparing configurations
    /// across both `Searcher` and `SpeculativeSearcher`).
    pub bestmove_changes: u32,
    /// Always 0 here: this path never speculates. Present only so callers
    /// that compare `SearchInfo`/`SpecSearchInfo` uniformly don't need a
    /// separate case for "no speculation happened."
    pub spec_hits: u32,
    /// Always 0 here — see `spec_hits`.
    pub spec_total: u32,
    /// Principal variation from the last fully-completed depth, best move
    /// first. Built by tracking `alpha_beta`'s own PV-node updates as the
    /// search runs — never reconstructed from the TT (shared with
    /// speculative search, so a probed entry's `mv` can belong to a foreign
    /// line). May be shorter than `depth` if the line runs into a TT cutoff
    /// or terminal position before that ply.
    pub pv: Vec<Move>,
    /// YBW split/cancellation activity across this search. All zero when
    /// `use_ybw` is off or no node reached `MIN_SPLIT_DEPTH`.
    pub ybw: YbwSearchStats,
}

/// Snapshot of YBW split/cancellation activity for one search, separate from
/// `nodes` (which counts main-search nodes via `Budget`, unaffected by this).
/// See `SplitCancel`/`YbwProbeResult` in the search internals for exactly
/// what each count means.
#[derive(Debug, Default, Clone, Copy)]
pub struct YbwSearchStats {
    /// YBW splits entered (nodes where parallel sibling probing started).
    pub splits: u64,
    /// Sibling probes dispatched into the parallel pass.
    pub probes_started: u64,
    /// Probes that finished without being cancelled (`Completed` or `Cutoff`).
    pub probes_completed: u64,
    /// Probes whose split (or an ancestor split) was cancelled before or
    /// during the probe; their score was discarded, not just clamped.
    pub probes_cancelled: u64,
    /// Probes whose full-depth score already proved a beta cutoff by
    /// itself, skipping the usual full-window re-search.
    pub direct_cutoffs: u64,
    /// Full-window re-searches performed for non-cutoff fail-highs
    /// (sequential, post-collect -- unchanged from before this feature).
    pub full_researches: u64,
    /// Nodes visited under splits that ended up cancelled. Attributed per
    /// split (all descendants of a cancelled split), not per individual
    /// probe -- the shared `Budget` node counter can't isolate one
    /// concurrently-running sibling's own contribution, and a truly
    /// per-probe counter would need one atomic per dispatched sibling.
    pub cancelled_nodes: u64,
}

impl YbwStats {
    fn snapshot(&self) -> YbwSearchStats {
        YbwSearchStats {
            splits: self.splits.load(Ordering::Relaxed),
            probes_started: self.probes_started.load(Ordering::Relaxed),
            probes_completed: self.probes_completed.load(Ordering::Relaxed),
            probes_cancelled: self.probes_cancelled.load(Ordering::Relaxed),
            direct_cutoffs: self.direct_cutoffs.load(Ordering::Relaxed),
            full_researches: self.full_researches.load(Ordering::Relaxed),
            cancelled_nodes: self.cancelled_nodes.load(Ordering::Relaxed),
        }
    }
}

// ============================================================
// Internal search state (shared across threads via Arc)
// ============================================================

struct SearchState {
    tt: Arc<Tt>,
    budget: Arc<Budget>,
    killers: KillerTable,
    history: HistoryTable,
    countermoves: CountermoveTable,
    /// Copied in once per `search()` call (`SearchConfig` is `Copy`) so deep
    /// recursive calls (which only carry `&Arc<SearchState>`, not the config
    /// directly) can read toggles like `use_ybw`/`ybw_max_siblings` without a
    /// signature change to `alpha_beta` itself.
    config: SearchConfig,
    /// Which non-exact (selective) search techniques are active. Always
    /// `SearchFeatures::PRODUCTION` in every public `Searcher`/
    /// `SpeculativeSearcher` code path -- there is no USI option or public
    /// API to change it. Exists solely so `search.rs`'s own unit tests can
    /// run under `SearchFeatures::EXACT_REFERENCE[_WITH_TT]` and get a
    /// provably-exact (non-heuristic) alpha-beta tree to compare PVS against.
    /// See the module doc on `SearchFeatures` for why this can't just be a
    /// `SearchConfig` field.
    features: SearchFeatures,
    /// YBW split/cancellation instrumentation for this search, fresh per
    /// `search()`/`search_with_features()` call. See `YbwStats`.
    ybw_stats: Arc<YbwStats>,
}

/// Toggles for search techniques that are *not* exact minimax-preserving
/// optimizations -- each one trades some search precision for speed (LMR,
/// null-move pruning, etc.), so two searches that differ only in which of
/// these are active can legitimately reach different scores or moves at the
/// same (position, depth). This is deliberately separate from the public,
/// user-facing `SearchConfig` (whose toggles -- `use_ybw`, `use_speculation`,
/// `use_pvs` -- are all exact optimizations or structural choices, never
/// heuristics): mixing a test-only "disable everything" knob into the
/// production config struct would let it leak into the USI/CSA/train
/// surface, when its only real purpose is giving `search.rs`'s own tests a
/// non-selective reference tree to compare PVS's *value* correctness
/// against, independent of move-ordering/TT-timing effects. See
/// `exact_reference_tests` for what this is used for and why.
#[derive(Clone, Copy)]
struct SearchFeatures {
    /// Root aspiration windowing (narrow-then-widen around `prev_score`).
    /// Off means every root call searches the full `(NEG_INF, POS_INF)`
    /// window from the first iteration.
    aspiration: bool,
    /// Whether `alpha_beta`'s TT probe is consulted for cutoffs/move
    /// ordering at all. Off makes stores into that call's `Tt` inert (never
    /// read back), equivalent to searching with no transposition table.
    tt_cutoff: bool,
    lmr: bool,
    nmp: bool,
    rfp: bool,
    futility: bool,
    lmp: bool,
    probcut: bool,
    iir: bool,
    singular_extension: bool,
    check_extension: bool,
}

impl SearchFeatures {
    /// What every public search path uses today, unconditionally.
    const PRODUCTION: Self = SearchFeatures {
        aspiration: true,
        tt_cutoff: true,
        lmr: true,
        nmp: true,
        rfp: true,
        futility: true,
        lmp: true,
        probcut: true,
        iir: true,
        singular_extension: true,
        check_extension: true,
    };

    /// Every non-exact technique off, TT probing included. Under this
    /// profile `alpha_beta` is a textbook fail-soft alpha-beta search with no
    /// selective pruning of any kind, so PVS's null-window-probe-then-full-
    /// window-research is a *provably* exact optimization of it -- any score
    /// difference between sequential AB and sequential PVS here is a real
    /// bug, not expected heuristic noise.
    #[cfg(test)]
    const EXACT_REFERENCE: Self = SearchFeatures {
        aspiration: false,
        tt_cutoff: false,
        lmr: false,
        nmp: false,
        rfp: false,
        futility: false,
        lmp: false,
        probcut: false,
        iir: false,
        singular_extension: false,
        check_extension: false,
    };

    /// Same as `EXACT_REFERENCE` but with TT probing (and therefore its
    /// cutoffs and TT-move-driven ordering) left on -- isolates whether PVS's
    /// interaction with a *shared, actively-read* TT can corrupt a value,
    /// separately from the plain window/re-search logic `EXACT_REFERENCE`
    /// alone already covers.
    #[cfg(test)]
    const EXACT_REFERENCE_WITH_TT: Self = SearchFeatures {
        tt_cutoff: true,
        ..Self::EXACT_REFERENCE
    };
}

// ============================================================
// Searcher
// ============================================================

/// Sequential (with YBW-parallel helper threads) iterative-deepening alpha-beta searcher.
pub struct Searcher {
    tt: Arc<Tt>,
    /// Exposed for USI "stop" command — set to true to abort an in-progress search
    external_abort: Arc<AtomicBool>,
}

impl Searcher {
    /// Create a searcher backed by the given shared transposition table.
    pub fn new(tt: Arc<Tt>) -> Self {
        Searcher {
            tt,
            external_abort: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Returns an `Arc` to the abort flag; store `true` to stop the search early.
    pub fn abort_flag(&self) -> Arc<AtomicBool> {
        self.external_abort.clone()
    }

    /// Run iterative-deepening search from the current position up to `config.max_depth`
    /// or until a time limit / abort signal fires, returning the best line found.
    pub fn search(&self, board: &mut Board, config: SearchConfig) -> SearchInfo {
        self.search_with_features(board, config, SearchFeatures::PRODUCTION)
    }

    /// Same as `search`, but with an explicit `SearchFeatures` profile.
    /// Not exposed publicly -- every real caller goes through `search`, which
    /// always passes `SearchFeatures::PRODUCTION`. Exists so this module's
    /// own tests can request `SearchFeatures::EXACT_REFERENCE[_WITH_TT]`
    /// without a USI option or public API surface for it.
    fn search_with_features(
        &self,
        board: &mut Board,
        config: SearchConfig,
        features: SearchFeatures,
    ) -> SearchInfo {
        self.external_abort.store(false, Ordering::Relaxed);

        let state = Arc::new(SearchState {
            tt: self.tt.clone(),
            budget: Arc::new(Budget::new(config.time_limit, self.external_abort.clone())),
            killers: KillerTable::new(),
            history: HistoryTable::new(),
            countermoves: CountermoveTable::new(),
            config,
            features,
            ybw_stats: Arc::new(YbwStats::default()),
        });

        let mut best_move = None;
        let mut best_score = NEG_INF;
        let mut done_depth = 0;
        let mut prev_best: Option<Move> = None;
        let mut bestmove_changes = 0u32;
        let mut pv: Vec<Move> = Vec::new();

        for depth in 1..=config.max_depth {
            let mut pv_buf: Vec<Move> = Vec::new();
            let (m, score) = root_search(&state, board, depth, best_score, &[], Some(&mut pv_buf));

            if state.budget.should_abort() {
                break;
            }

            best_move = m.or(best_move);
            best_score = score;
            done_depth = depth;
            pv = pv_buf;

            if score.abs() >= MATE_SCORE - 1000 {
                break;
            }

            if best_move != prev_best && depth >= 3 {
                bestmove_changes += 1;
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

        SearchInfo {
            best_move,
            score: best_score,
            depth: done_depth,
            nodes: state.budget.nodes(),
            elapsed: state.budget.elapsed(),
            hashfull: self.tt.hashfull(),
            bestmove_changes,
            spec_hits: 0,
            spec_total: 0,
            pv,
            ybw: state.ybw_stats.snapshot(),
        }
    }
}

// ============================================================
// Root search with Aspiration Window
// ============================================================

fn root_search(
    state: &Arc<SearchState>,
    board: &mut Board,
    depth: u32,
    prev_score: i32,
    excluded: &[Move],
    mut pv: Option<&mut Vec<Move>>,
) -> (Option<Move>, i32) {
    let all_moves = generate_legal_moves(board);
    let moves: Vec<Move> = if excluded.is_empty() {
        all_moves
    } else {
        all_moves
            .into_iter()
            .filter(|m| !excluded.contains(m))
            .collect()
    };
    if moves.is_empty() {
        return (None, -MATE_SCORE);
    }

    // Single legal move: skip deep search but return an honest eval score.
    if moves.len() == 1 {
        let tok = board.do_move(moves[0]);
        let score = -evaluate(board);
        board.undo_move(tok);
        if let Some(p) = pv.as_deref_mut() {
            p.clear();
            p.push(moves[0]);
        }
        return (Some(moves[0]), score);
    }

    let tt_mv = state.tt.probe(board.hash()).and_then(|e| e.mv);
    let killers = state.killers.get(0);
    let ordered = order_moves(
        board,
        moves,
        tt_mv,
        killers,
        None,
        &state.history,
        board.side_to_move,
    );

    // Mate-in-1: check each root move for immediate checkmate before deep search
    for &m in &ordered {
        let tok = board.do_move(m);
        let mated =
            generate_legal_moves(board).is_empty() && is_in_check(board, board.side_to_move);
        board.undo_move(tok);
        if mated {
            // This shortcut bypasses root_search_inner/alpha_beta entirely,
            // so it must populate `pv` itself -- `[m]` is already the
            // complete line (the position after `m` has no legal replies).
            if let Some(p) = pv.as_deref_mut() {
                p.clear();
                p.push(m);
            }
            return (Some(m), MATE_SCORE - 1);
        }
    }

    // Opponent safety: at shallow depths, filter out root moves that immediately allow
    // opponent mate-in-1. Gated on depth <= 2 to bound the O(N×M²) cost.
    // At depth >= 3 the normal alpha-beta search catches these situations anyway.
    let ordered: Vec<Move> = if depth <= 2 {
        let mut safe_moves = Vec::new();
        let mut has_unsafe = false;
        for &m in &ordered {
            let tok = board.do_move(m);
            let mut opp_can_mate = false;
            'opp: for opp_m in generate_legal_moves(board) {
                let tok2 = board.do_move(opp_m);
                if generate_legal_moves(board).is_empty() && is_in_check(board, board.side_to_move)
                {
                    opp_can_mate = true;
                }
                board.undo_move(tok2);
                if opp_can_mate {
                    break 'opp;
                }
            }
            board.undo_move(tok);
            if opp_can_mate {
                has_unsafe = true;
            } else {
                safe_moves.push(m);
            }
        }
        if has_unsafe && !safe_moves.is_empty() {
            safe_moves
        } else {
            ordered
        }
    } else {
        ordered
    };

    // Aspiration window: start tight around prev_score; widen on fail
    let use_asp = state.features.aspiration && depth >= 2 && prev_score.abs() < MATE_SCORE - 1000;
    let (mut lo, mut hi) = if use_asp {
        (prev_score - ASP_DELTA, prev_score + ASP_DELTA)
    } else {
        (NEG_INF, POS_INF)
    };

    loop {
        let (m, score) =
            root_search_inner(state, board, depth, &ordered, lo, hi, pv.as_deref_mut());

        if state.budget.should_abort() {
            return (m, score);
        }

        if score <= lo {
            lo -= ASP_DELTA * 2;
            if lo < NEG_INF {
                lo = NEG_INF;
            }
        } else if score >= hi {
            hi += ASP_DELTA * 2;
            if hi > POS_INF {
                hi = POS_INF;
            }
        } else {
            return (m, score);
        }

        // Full window fallback
        if lo <= NEG_INF && hi >= POS_INF {
            return (m, score);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn root_search_inner(
    state: &Arc<SearchState>,
    board: &mut Board,
    depth: u32,
    ordered: &[Move],
    lo: i32,
    hi: i32,
    mut pv: Option<&mut Vec<Move>>,
) -> (Option<Move>, i32) {
    let mut best_move = None;
    let mut alpha = lo;

    // Defensive: `root_search` may call this repeatedly across aspiration
    // retries, reusing the same `pv` buffer each time. Clearing up front
    // means a retry that (in some pathological case) never finds a move
    // beating its own `lo` can't leave the *previous* attempt's stale line
    // behind -- an empty `pv` on such a failure is honest, a leftover one
    // from a different window would not be.
    if let Some(p) = pv.as_deref_mut() {
        p.clear();
    }

    for (i, &m) in ordered.iter().enumerate() {
        let tok = board.do_move(m);
        let want_pv = pv.is_some() && hi - alpha > 1;
        let mut child_pv: Vec<Move> = Vec::new();

        // First move (and every move when PVS is off) gets the ambient
        // full window; later moves under PVS get a null-window probe first,
        // full-window re-search only on fail-high.
        // Root has no ambient YBW split, so every call here passes `None`
        // for `local_abort` -- cancellation only exists below a split.
        let score = if i == 0 || !state.config.use_pvs {
            -alpha_beta(
                state,
                board,
                -hi,
                -alpha,
                depth - 1,
                1,
                true,
                Some(m),
                None,
                if want_pv { Some(&mut child_pv) } else { None },
                None,
            )
        } else {
            let probe = -alpha_beta(
                state,
                board,
                -alpha - 1,
                -alpha,
                depth - 1,
                1,
                true,
                Some(m),
                None,
                None,
                None,
            );
            if probe > alpha {
                child_pv.clear();
                -alpha_beta(
                    state,
                    board,
                    -hi,
                    -alpha,
                    depth - 1,
                    1,
                    true,
                    Some(m),
                    None,
                    if want_pv { Some(&mut child_pv) } else { None },
                    None,
                )
            } else {
                probe
            }
        };
        board.undo_move(tok);

        if state.budget.should_abort() {
            break;
        }

        if score > alpha {
            alpha = score;
            best_move = Some(m);
            if let Some(p) = pv.as_deref_mut() {
                p.clear();
                p.push(m);
                p.append(&mut child_pv);
            }
        }
        if alpha >= hi {
            break;
        }
    }

    if let Some(m) = best_move {
        let bound = if alpha >= hi {
            Bound::Lower // fail-high: true score ≥ alpha, exact unknown
        } else {
            Bound::Exact
        };
        state.tt.store(
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
// YBW branch-local cancellation
// ============================================================

/// Branch-local cancellation token for one YBW split. Cancelling a split
/// stops only its own still-running/not-yet-started sibling probes (and any
/// nested splits beneath them, via `parent`) -- it never reaches an ancestor
/// split or an unrelated sibling subtree, and it's entirely independent of
/// the global `Budget`/USI-stop mechanism (`Budget::should_abort`), which
/// continues to stop everything exactly as before, checked separately.
///
/// Threaded through `alpha_beta`/`quiescence` as `Option<&Arc<SplitCancel>>`:
/// ordinary recursive calls just propagate the same reference (no atomic
/// refcount traffic on the hot path); only the YBW dispatch itself clones the
/// `Arc` -- once per spawned sibling, the same cost class as the `state`
/// clone it already does today.
struct SplitCancel {
    cancelled: AtomicBool,
    parent: Option<Arc<SplitCancel>>,
    /// Nodes visited by calls carrying this split as their innermost
    /// `local_abort`. Used for `ybw_cancelled_nodes`: the shared `Budget`
    /// node counter can't isolate one concurrently-running sibling's own
    /// contribution, but this per-split counter can.
    nodes: AtomicU64,
}

impl SplitCancel {
    fn new(parent: Option<Arc<SplitCancel>>) -> Arc<Self> {
        Arc::new(SplitCancel {
            cancelled: AtomicBool::new(false),
            parent,
            nodes: AtomicU64::new(0),
        })
    }

    fn cancel(&self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }

    /// True if this split, or any ancestor split, has been cancelled.
    fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Relaxed)
            || self
                .parent
                .as_deref()
                .is_some_and(SplitCancel::is_cancelled)
    }
}

#[cfg(test)]
mod split_cancel_tests {
    use super::*;

    #[test]
    fn child_cancel_does_not_cancel_parent() {
        let parent = SplitCancel::new(None);
        let child = SplitCancel::new(Some(parent.clone()));
        child.cancel();
        assert!(child.is_cancelled());
        assert!(!parent.is_cancelled());
    }

    #[test]
    fn parent_cancel_is_observable_from_child() {
        let parent = SplitCancel::new(None);
        let child = SplitCancel::new(Some(parent.clone()));
        parent.cancel();
        assert!(parent.is_cancelled());
        assert!(child.is_cancelled());
    }

    #[test]
    fn sibling_tokens_are_independent() {
        let parent = SplitCancel::new(None);
        let a = SplitCancel::new(Some(parent.clone()));
        let b = SplitCancel::new(Some(parent.clone()));
        a.cancel();
        assert!(a.is_cancelled());
        assert!(!b.is_cancelled());
        assert!(!parent.is_cancelled());
    }

    #[test]
    fn nested_grandparent_cancel_is_observable_two_levels_down() {
        let root = SplitCancel::new(None);
        let mid = SplitCancel::new(Some(root.clone()));
        let leaf = SplitCancel::new(Some(mid.clone()));
        root.cancel();
        assert!(leaf.is_cancelled());
    }
}

/// Outcome of one YBW sibling's null-window probe.
enum YbwProbeResult {
    /// The probe completed without proving a cutoff by itself (a fail-low,
    /// or a PV candidate with `alpha < score < beta`). `reduce` is carried
    /// along so the post-collect sequential pass can decide whether a
    /// full-window re-search is still needed (an LMR-reduced fail-high must
    /// always be re-verified at full depth before it's trusted).
    Completed {
        mv: Move,
        score: i32,
        move_index: usize,
        reduce: u32,
    },
    /// The probe's own score already reached the ambient `beta` at full
    /// depth (`reduce == 0`) -- a valid proof of a beta cutoff without
    /// needing the usual full-window re-search. By the time this variant is
    /// constructed the split has already been cancelled (see the ordering
    /// note at the call site): the worker that discovers the cutoff must
    /// keep its own result, so cancellation happens *after* this value is
    /// built, never before.
    Cutoff {
        mv: Move,
        score: i32,
        move_index: usize,
    },
    /// This sibling's split (or an ancestor split) was cancelled before or
    /// during its probe. MUST NOT feed alpha/best_score/bestmove/PV updates,
    /// TT stores, or heuristic-table updates -- the score is discarded
    /// entirely, not just clamped.
    Cancelled { move_index: usize },
}

/// Per-search YBW instrumentation, separate from `Budget`'s node counter.
/// Fields are `AtomicU64` because they're written concurrently from every
/// YBW sibling's rayon task.
#[derive(Default)]
struct YbwStats {
    splits: AtomicU64,
    probes_started: AtomicU64,
    probes_completed: AtomicU64,
    probes_cancelled: AtomicU64,
    direct_cutoffs: AtomicU64,
    full_researches: AtomicU64,
    cancelled_nodes: AtomicU64,
}

// ============================================================
// Core Alpha-Beta with YBW parallelism
// ============================================================

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
    // Principal-variation output: `Some` only at PV-node calls (this node's
    // own `beta - alpha > 1`), populated with `[best_move, ...best child's
    // line]` whenever `best_move` updates. `None` everywhere else (null-
    // window probes, RFP/ProbCut/NMP/SE helper searches, quiescence) so the
    // allocation/bookkeeping cost is confined to the O(depth) nodes actually
    // on a PV, not every visited node. Deliberately NOT reconstructed from
    // the TT: the TT is shared with speculative search (see speculative.rs),
    // so a probed entry's `mv` can belong to a foreign line.
    mut pv: Option<&mut Vec<Move>>,
    // Branch-local YBW cancellation context (see `SplitCancel`). `None`
    // outside any YBW split (root-level calls, or a search with `use_ybw`
    // off). Propagated unchanged through every recursive call in this
    // function EXCEPT the YBW dispatch below, which creates a fresh child
    // split chained to this one as its parent.
    local_abort: Option<&Arc<SplitCancel>>,
) -> i32 {
    if state.budget.tick() {
        return 0;
    }
    if let Some(t) = local_abort {
        t.nodes.fetch_add(1, Ordering::Relaxed);
        if t.is_cancelled() {
            return 0;
        }
    }

    // Mate distance pruning: tighten window — we can't improve beyond the nearest mate
    alpha = alpha.max(-(MATE_SCORE - ply as i32));
    let beta = beta.min(MATE_SCORE - ply as i32);
    if alpha >= beta {
        return alpha;
    }

    if depth == 0 {
        return quiescence(state, board, alpha, beta, ply, 0, local_abort);
    }

    // TT probe
    let hash = board.hash();
    let orig_alpha = alpha;
    let mut tt_mv = None;
    let mut tt_se_score = None::<i32>; // TT score for singular extension (lower/exact bound only)
    let mut tt_se_depth = 0u8; // TT entry depth for SE eligibility check

    if state.features.tt_cutoff
        && let Some(entry) = state.tt.probe(hash)
    {
        let adj = score_from_tt(entry.score, ply);
        tt_mv = entry.mv;
        tt_se_depth = entry.depth;
        if !matches!(entry.bound, Bound::Upper) {
            tt_se_score = Some(adj); // lower or exact bound is usable for SE
        }
        if entry.depth >= depth as u8 {
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
    }

    // Internal Iterative Reduction: no TT move → move ordering is poor, search shallower
    let depth = if state.features.iir && tt_mv.is_none() && depth >= 4 {
        depth - 1
    } else {
        depth
    };

    let stm = board.side_to_move;

    // Countermove: best quiet response to the opponent's previous move
    let countermove = prev_mv.and_then(|pm| state.countermoves.get(stm.flip(), pm));

    // Static eval — computed once per node for RFP and Futility Pruning.
    // Skipped when in check (position is not "quiet") or depth > 5 (overhead not justified).
    let in_check = is_in_check(board, stm);
    let static_eval: Option<i32> = if !in_check && depth <= 5 {
        Some(evaluate(board))
    } else {
        None
    };

    // Reverse Futility Pruning: if a rough lower bound already beats beta, return early.
    if state.features.rfp
        && let Some(se) = static_eval
        && depth <= 3
        && beta.abs() < MATE_SCORE - 1000
        && se - RFP_MARGIN * depth as i32 >= beta
    {
        return se;
    }

    // ProbCut: if a shallow (depth-4) search with an inflated beta suggests this node
    // will fail high by more than PC_MARGIN, prune without a full search.
    // Only try captures with SEE >= PC_MARGIN (already winning material gain).
    if state.features.probcut
        && depth >= PC_MIN_DEPTH
        && !in_check
        && beta.abs() < MATE_SCORE - 1000
        && skip_move.is_none()
    // not inside a singular search
    {
        let pc_beta = beta + PC_MARGIN;
        let mut caps: Vec<Move> = generate_legal_captures(board)
            .into_iter()
            .filter(|&m| see_score(board, m) >= PC_MARGIN)
            .collect();
        caps.sort_by_cached_key(|&m| -see_score(board, m));
        let pc_depth = (depth - 4).min(3); // cap at 3 to keep the probe cheap
        for cap in caps {
            if state.budget.should_abort() {
                break;
            }
            if local_abort.is_some_and(|t| t.is_cancelled()) {
                return 0;
            }
            let tok = board.do_move(cap);
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
                None,
                local_abort,
            );
            board.undo_move(tok);
            if local_abort.is_some_and(|t| t.is_cancelled()) {
                return 0;
            }
            if pc_score >= pc_beta {
                return pc_score;
            }
        }
    }

    // Null Move Pruning
    if state.features.nmp
        && can_null
        && depth > NMP_R
        && beta.abs() < MATE_SCORE - 1000
        && !in_check
    // reuse the is_in_check result computed above
    {
        let null_tok = board.do_null_move();
        let null_score = -alpha_beta(
            state,
            board,
            -beta,
            -beta + 1,
            depth - 1 - NMP_R,
            ply + 1,
            false,
            None,
            None,
            None,
            local_abort,
        );
        board.undo_null_move(null_tok);
        if local_abort.is_some_and(|t| t.is_cancelled()) {
            return 0;
        }

        if null_score >= beta {
            if depth >= 6 {
                // Verification search: confirm with a real (non-null) shallow search.
                // Guards against zugzwang-like horizon effects common in shogi.
                let verify = alpha_beta(
                    state,
                    board,
                    beta - 1,
                    beta,
                    depth - 1 - NMP_R,
                    ply,
                    false,
                    prev_mv,
                    None,
                    None,
                    local_abort,
                );
                if local_abort.is_some_and(|t| t.is_cancelled()) {
                    return 0;
                }
                if verify >= beta {
                    return null_score;
                }
                // verification failed — fall through to normal search
            } else {
                return null_score;
            }
        }
    }

    let moves = generate_legal_moves(board);
    if moves.is_empty() {
        return -(MATE_SCORE - ply as i32); // shorter mate = higher score for the mating side
    }

    let killers = state.killers.get(ply as usize);
    let ordered = order_moves(
        board,
        moves,
        tt_mv,
        killers,
        countermove,
        &state.history,
        stm,
    );

    // For singular search: filter out the excluded move (rare, only at depth >= SE_MIN_DEPTH / 2)
    let ordered: Vec<Move> = if let Some(skip) = skip_move {
        ordered.into_iter().filter(|&m| m != skip).collect()
    } else {
        ordered
    };
    if ordered.is_empty() {
        return alpha;
    } // all moves excluded (shouldn't happen in practice)

    // Singular Extension: check whether the TT move is clearly the best in this position.
    // If all other moves fail below (tt_score - SE_MARGIN), the TT move is "singular" and
    // we extend its search by one ply.
    let sing_ext = if let Some(se_score) = tt_se_score.filter(|_| {
        state.features.singular_extension
            && skip_move.is_none()
            && depth >= SE_MIN_DEPTH
            && !in_check
            && tt_mv.is_some()
            && tt_se_depth >= (depth as u8).saturating_sub(3)
    }) {
        let se_beta = (se_score - SE_MARGIN).max(alpha);
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
            None,
            local_abort,
        );
        if local_abort.is_some_and(|t| t.is_cancelled()) {
            0 // discard a cancelled probe's result rather than trust a spurious extension
        } else {
            u32::from(sval < se_beta) // 1 if TT move is singular, else 0
        }
    } else {
        0
    };

    // Quiet moves tried so far — used to apply history malus on beta cutoff.
    let enemy = board.occ_for(stm.flip());
    let mut tried_quiet: Vec<Move> = Vec::new();

    // ---------- First child: always sequential ----------
    let first_move = ordered[0];
    let tok = board.do_move(first_move);
    let ext0 = if state.features.check_extension {
        check_ext(board, ply + 1)
    } else {
        0
    };
    // Apply singular extension to the TT move (ordered[0] when tt_mv is set)
    let first_ext = ext0
        + if tt_mv.is_some_and(|t| t == first_move) {
            sing_ext
        } else {
            0
        };
    let want_pv0 = pv.is_some() && beta - alpha > 1;
    let mut child_pv0: Vec<Move> = Vec::new();
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
        if want_pv0 { Some(&mut child_pv0) } else { None },
        local_abort,
    );
    board.undo_move(tok);

    if state.budget.should_abort() {
        return 0;
    }
    if local_abort.is_some_and(|t| t.is_cancelled()) {
        return 0;
    }

    let mut best_score = score0;
    let mut best_move = Some(first_move);
    if let Some(p) = pv.as_deref_mut() {
        p.clear();
        p.push(first_move);
        p.append(&mut child_pv0);
    }

    if score0 >= beta {
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
        store_tt(state, hash, score0, depth, Bound::Lower, best_move, ply);
        return score0;
    }
    if score0 > alpha {
        alpha = score0;
    }
    // Track first_move for malus if it didn't cut off
    if first_move.from.is_some() && !enemy.contains(first_move.to) && !first_move.promote {
        tried_quiet.push(first_move);
    }

    let rest = &ordered[1..];
    if rest.is_empty() {
        let bound = if best_score > orig_alpha {
            Bound::Exact
        } else {
            Bound::Upper
        };
        store_tt(state, hash, best_score, depth, bound, best_move, ply);
        return best_score;
    }

    // ---------- Young brothers ----------
    // Returns the index in `rest` where sequential processing should begin:
    // ybw_end after the parallel YBW pass, or 0 at shallow depths (no YBW) or
    // when `UseYBW` is toggled off (config.use_ybw).
    let seq_start = if state.config.use_ybw && depth >= MIN_SPLIT_DEPTH {
        let child_split = SplitCancel::new(local_abort.cloned());
        state.ybw_stats.splits.fetch_add(1, Ordering::Relaxed);
        let alpha_for_nw = alpha;

        let ybw_end = rest.len().min(state.config.ybw_max_siblings);

        #[allow(clippy::type_complexity)]
        let work: Vec<(Move, usize, Board, Arc<SearchState>, Arc<SplitCancel>)> = rest[..ybw_end]
            .iter()
            .enumerate()
            .map(|(i, &m)| (m, i + 1, board.clone(), state.clone(), child_split.clone()))
            .collect();
        state
            .ybw_stats
            .probes_started
            .fetch_add(work.len() as u64, Ordering::Relaxed);

        // Parallel probe (with LMR for quiet late moves). Window is a null
        // window when `use_pvs` (matching this block's long-standing default
        // behavior); otherwise the ambient full window, so disabling PVS
        // doesn't silently leave PVS running here — LMR's depth reduction
        // still applies either way, it's orthogonal to the window choice.
        let use_pvs = state.config.use_pvs;
        let nw_results: Vec<YbwProbeResult> = work
            .into_par_iter()
            .map(|(m, idx, mut b, ctx, split)| {
                if ctx.budget.should_abort() || split.is_cancelled() {
                    return YbwProbeResult::Cancelled { move_index: idx };
                }
                let reduce = if ctx.features.lmr {
                    lmr_reduce(&b, m, idx, depth, &killers, tt_mv, &ctx.history, stm)
                } else {
                    0
                };
                let tok = b.do_move(m);
                let ext = if ctx.features.check_extension {
                    check_ext(&b, ply + 1)
                } else {
                    0
                };
                let reduce = if ext > 0 { 0 } else { reduce }; // never reduce a checking move
                let probe_depth = depth.saturating_sub(1 + reduce) + ext;
                let (lo, hi) = if use_pvs {
                    (-alpha_for_nw - 1, -alpha_for_nw)
                } else {
                    (-beta, -alpha_for_nw)
                };
                let s = -alpha_beta(
                    &ctx,
                    &mut b,
                    lo,
                    hi,
                    probe_depth,
                    ply + 1,
                    true,
                    Some(m),
                    None,
                    None,
                    Some(&split),
                );
                b.undo_move(tok);

                if ctx.budget.should_abort() || split.is_cancelled() {
                    return YbwProbeResult::Cancelled { move_index: idx };
                }

                // A full-depth probe (`reduce == 0`) whose score already
                // reaches the ambient `beta` proves a cutoff outright: no
                // full-window re-search needed. A reduced-depth fail-high is
                // NOT trustworthy on its own (that's exactly why LMR
                // re-verifies at full depth), so it falls through to
                // `Completed` and lets the sequential pass decide.
                if reduce == 0 && s >= beta {
                    // Build the result from this worker's own valid score
                    // BEFORE cancelling the split -- the worker that
                    // discovers the cutoff must never treat itself as
                    // cancelled as a side effect of firing it.
                    let result = YbwProbeResult::Cutoff {
                        mv: m,
                        score: s,
                        move_index: idx,
                    };
                    split.cancel();
                    result
                } else {
                    YbwProbeResult::Completed {
                        mv: m,
                        score: s,
                        move_index: idx,
                        reduce,
                    }
                }
            })
            .collect();

        // Sequential pass: handle fail-highs, update heuristics, apply
        // history malus, in original move order -- unchanged from before
        // this commit except for the three-way `YbwProbeResult` split.
        // `Cancelled` results are skipped entirely: never touching alpha,
        // best_score, bestmove, PV, TT stores, or heuristic tables.
        for (expected_idx, result) in nw_results.into_iter().enumerate() {
            if state.budget.should_abort() {
                break;
            }
            if local_abort.is_some_and(|t| t.is_cancelled()) {
                return 0;
            }

            // `into_par_iter().map(...).collect::<Vec<_>>()` is an indexed
            // parallel iterator, so results come back in original dispatch
            // order regardless of completion timing -- this is a cheap
            // guard on that assumption, since the whole point of this loop
            // running sequentially in original move order depends on it.
            let (m, base_score, reduce, is_cutoff) = match result {
                YbwProbeResult::Cancelled { move_index } => {
                    debug_assert_eq!(move_index, expected_idx + 1);
                    state
                        .ybw_stats
                        .probes_cancelled
                        .fetch_add(1, Ordering::Relaxed);
                    continue;
                }
                YbwProbeResult::Cutoff {
                    mv,
                    score,
                    move_index,
                } => {
                    debug_assert_eq!(move_index, expected_idx + 1);
                    state
                        .ybw_stats
                        .probes_completed
                        .fetch_add(1, Ordering::Relaxed);
                    state
                        .ybw_stats
                        .direct_cutoffs
                        .fetch_add(1, Ordering::Relaxed);
                    (mv, score, 0u32, true)
                }
                YbwProbeResult::Completed {
                    mv,
                    score,
                    reduce,
                    move_index,
                } => {
                    debug_assert_eq!(move_index, expected_idx + 1);
                    state
                        .ybw_stats
                        .probes_completed
                        .fetch_add(1, Ordering::Relaxed);
                    (mv, score, reduce, false)
                }
            };

            let is_quiet_ybw = m.from.is_some() && !enemy.contains(m.to) && !m.promote;

            let needs_research = !is_cutoff
                && if use_pvs {
                    base_score > alpha
                } else {
                    reduce > 0 && base_score > alpha
                };
            let want_pv = pv.is_some() && beta - alpha > 1;
            let mut child_pv: Vec<Move> = Vec::new();
            let s = if needs_research {
                state
                    .ybw_stats
                    .full_researches
                    .fetch_add(1, Ordering::Relaxed);
                // Fail-high: re-search at full depth with full window
                let tok = board.do_move(m);
                let ext = if state.features.check_extension {
                    check_ext(board, ply + 1)
                } else {
                    0
                };
                let full = -alpha_beta(
                    state,
                    board,
                    -beta,
                    -alpha,
                    (depth - 1) + ext,
                    ply + 1,
                    true,
                    Some(m),
                    None,
                    if want_pv { Some(&mut child_pv) } else { None },
                    local_abort,
                );
                board.undo_move(tok);
                if local_abort.is_some_and(|t| t.is_cancelled()) {
                    return 0;
                }
                full
            } else {
                // A direct `Cutoff` is never re-searched, so it has no
                // continuation -- leave `child_pv` empty. The `p.push(m)`
                // below already accounts for this move; appending an empty
                // `child_pv` correctly yields a one-move PV `[m]` (pushing
                // `m` into `child_pv` too would duplicate it as `[m, m]`).
                base_score
            };

            if s > best_score {
                best_score = s;
                best_move = Some(m);
                if let Some(p) = pv.as_deref_mut() {
                    p.clear();
                    p.push(m);
                    p.append(&mut child_pv);
                }
            }
            if s >= beta {
                for &qm in &tried_quiet {
                    state.history.malus(stm, qm.piece_kind, qm.to, depth);
                }
                update_quiet_heuristics(
                    &state.killers,
                    &state.history,
                    &state.countermoves,
                    m,
                    stm,
                    ply,
                    depth,
                    board,
                    prev_mv,
                );
                store_tt(state, hash, best_score, depth, Bound::Lower, best_move, ply);
                return best_score;
            }
            if s > alpha {
                alpha = s;
            }
            if is_quiet_ybw {
                tried_quiet.push(m);
            }
        }
        // Approximate attribution: a split that never had a Cutoff/Cancelled
        // probe was never actually cancelled, so none of its nodes count as
        // "cancelled" work. `SplitCancel::nodes` sums all descendants of
        // this split rather than tracking each sibling probe individually.
        if child_split.is_cancelled() {
            state
                .ybw_stats
                .cancelled_nodes
                .fetch_add(child_split.nodes.load(Ordering::Relaxed), Ordering::Relaxed);
        }
        ybw_end
    } else {
        0
    };

    // Sequential pass: remaining siblings (tail beyond YBW limit, or all at shallow depth).
    {
        let lmp_limit = if state.features.lmp && !in_check && depth <= 2 {
            LMP_BASE + depth as usize * 3 // depth 1: 8 quiet moves, depth 2: 11 quiet moves
        } else {
            usize::MAX
        };

        let mut quiet_count = 0usize;

        for (j, &m) in rest[seq_start..].iter().enumerate() {
            let i = seq_start + j;
            if state.budget.should_abort() {
                break;
            }
            if local_abort.is_some_and(|t| t.is_cancelled()) {
                return 0;
            }

            let is_capture = m.from.is_some() && enemy.contains(m.to);
            let is_quiet = m.from.is_some() && !is_capture && !m.promote;

            // Futility Pruning: at depth 1, skip quiet moves that can't reach alpha
            if state.features.futility
                && depth == 1
                && let Some(se) = static_eval
                && is_quiet
                && se + FUTILITY_MARGIN < alpha
            {
                continue;
            }

            // Late Move Pruning: cut off remaining quiet moves beyond threshold
            if is_quiet {
                quiet_count += 1;
                if quiet_count > lmp_limit {
                    break;
                }
            }

            let reduce = if state.features.lmr {
                lmr_reduce(board, m, i + 1, depth, &killers, tt_mv, &state.history, stm)
            } else {
                0
            };
            let tok = board.do_move(m);
            let ext = if state.features.check_extension {
                check_ext(board, ply + 1)
            } else {
                0
            };
            let reduce = if ext > 0 { 0 } else { reduce }; // never reduce a checking move

            // LMR/PVS probe: under `use_pvs`, probe with a null window
            // instead of the ambient (-beta,-alpha); the re-search condition
            // then covers both LMR's depth reduction and PVS's window
            // narrowing in a single unified trigger.
            let probe_depth = depth.saturating_sub(1 + reduce) + ext;
            let (probe_lo, probe_hi) = if state.config.use_pvs {
                (-alpha - 1, -alpha)
            } else {
                (-beta, -alpha)
            };
            let want_pv = pv.is_some() && beta - alpha > 1;
            let mut child_pv: Vec<Move> = Vec::new();
            let mut s = -alpha_beta(
                state,
                board,
                probe_lo,
                probe_hi,
                probe_depth,
                ply + 1,
                true,
                Some(m),
                None,
                // Only a genuine full-window, full-depth probe can stand in
                // as the final line without a re-search below.
                if want_pv && !state.config.use_pvs && reduce == 0 {
                    Some(&mut child_pv)
                } else {
                    None
                },
                local_abort,
            );

            // Re-search at full window/depth if the probe failed high.
            let needs_research = if state.config.use_pvs {
                s > alpha
            } else {
                reduce > 0 && s > alpha
            };
            if needs_research {
                child_pv.clear();
                s = -alpha_beta(
                    state,
                    board,
                    -beta,
                    -alpha,
                    (depth - 1) + ext,
                    ply + 1,
                    true,
                    Some(m),
                    None,
                    if want_pv { Some(&mut child_pv) } else { None },
                    local_abort,
                );
            }
            board.undo_move(tok);
            if local_abort.is_some_and(|t| t.is_cancelled()) {
                return 0;
            }

            if s > best_score {
                best_score = s;
                best_move = Some(m);
                if let Some(p) = pv.as_deref_mut() {
                    p.clear();
                    p.push(m);
                    p.append(&mut child_pv);
                }
            }
            if s >= beta {
                for &qm in &tried_quiet {
                    state.history.malus(stm, qm.piece_kind, qm.to, depth);
                }
                update_quiet_heuristics(
                    &state.killers,
                    &state.history,
                    &state.countermoves,
                    m,
                    stm,
                    ply,
                    depth,
                    board,
                    prev_mv,
                );
                store_tt(state, hash, best_score, depth, Bound::Lower, best_move, ply);
                return best_score;
            }
            if s > alpha {
                alpha = s;
            }
            if is_quiet {
                tried_quiet.push(m);
            }
        }
    }

    let bound = if best_score > orig_alpha {
        Bound::Exact
    } else {
        Bound::Upper
    };
    store_tt(state, hash, best_score, depth, bound, best_move, ply);
    best_score
}

// ============================================================
// Quiescence Search
// ============================================================

/// Resolve a position to quiescence before calling evaluate. Searches captures
/// always, all legal replies while in check, and (only at qply 0) a few safe
/// quiet checks. Bounded by QSEARCH_MAX_PLY so forcing-check lines can't recurse
/// without end.
fn quiescence(
    state: &Arc<SearchState>,
    board: &mut Board,
    mut alpha: i32,
    beta: i32,
    ply: u32,
    qply: u32,
    local_abort: Option<&Arc<SplitCancel>>,
) -> i32 {
    // Enforce the hard time limit here too: a heavy qsearch subtree (quiet checks
    // + recursive SEE) can run for many seconds without returning to alpha_beta,
    // which is the only other place that ticks the budget.
    if state.budget.tick() {
        return 0;
    }
    if let Some(t) = local_abort {
        t.nodes.fetch_add(1, Ordering::Relaxed);
        if t.is_cancelled() {
            return 0;
        }
    }

    // Hard depth cap: terminate the quiescence even mid-check. Without this a
    // perpetual-check line recurses (in-check expands ALL legal replies below)
    // until the clock runs out — the move then blows past its byoyomi.
    const QSEARCH_MAX_PLY: u32 = 10;
    if qply >= QSEARCH_MAX_PLY {
        return evaluate(board);
    }

    let in_check = is_in_check(board, board.side_to_move);

    // Stand-pat and delta pruning only apply when not in check.
    // In check the side to move has no quiet option, so stand-pat is invalid.
    if !in_check {
        let stand_pat = evaluate(board);
        if stand_pat >= beta {
            return stand_pat;
        }
        if stand_pat > alpha {
            alpha = stand_pat;
        }
        // Delta Pruning: if even the best possible capture+promotion cannot improve alpha, skip.
        // Max gain = Ryu capture (1300) + Fu→Tokin promotion bonus (500) = 1800cp.
        const DELTA_MARGIN: i32 = 1_800;
        if stand_pat + DELTA_MARGIN < alpha {
            return alpha;
        }
    }

    let moves = if in_check {
        generate_legal_moves(board) // must escape check; all legal moves required
    } else {
        generate_legal_captures(board)
    };

    if moves.is_empty() {
        return if in_check {
            -MATE_SCORE + ply as i32 // checkmate
        } else {
            alpha
        };
    }

    // Order by a cheap MVV-LVA-style key. Recursive see_score here is too costly
    // per node (qsearch is the hottest path); the coarse capture ordering is
    // plenty for quiescence and keeps each node fast enough to respect the clock.
    let mut ordered = moves;
    ordered.sort_by_cached_key(|&m| -qsearch_order_key(board, m));

    for m in ordered {
        let tok = board.do_move(m);
        let score = -quiescence(state, board, -beta, -alpha, ply + 1, qply + 1, local_abort);
        board.undo_move(tok);

        if state.budget.should_abort() {
            return 0;
        }
        if local_abort.is_some_and(|t| t.is_cancelled()) {
            return 0;
        }
        if score >= beta {
            return score;
        }
        if score > alpha {
            alpha = score;
        }
    }

    // Quiet checks: at the shallowest qsearch level, search a handful of
    // non-capture moves that give check and have non-negative SEE.
    // Drops that give check (e.g. 飛打ち王手) are included naturally.
    if !in_check && qply == 0 {
        const MAX_QCHECKS: usize = 4;
        let mut qcheck_count = 0;
        for m in generate_legal_moves(board) {
            // Skip captures — already handled above
            if m.from.is_some() && board.piece_at(m.to).is_some() {
                continue;
            }
            // Test if this move gives check, then apply safety filter — combined in one do/undo
            let tok = board.do_move(m);
            let gives_check = is_in_check(board, board.side_to_move);
            if !gives_check {
                board.undo_move(tok);
                continue;
            }
            // Safety: skip if the checking piece can be immediately recaptured at a loss.
            // Promoting moves are exempt (promotion value offsets the risk).
            if !m.promote {
                let mover_val = PIECE_VALUE[m.piece_kind.index()];
                let unsafe_check = generate_legal_captures(board)
                    .iter()
                    .filter(|r| r.to == m.to)
                    .any(|r| PIECE_VALUE[r.piece_kind.index()] < mover_val);
                if unsafe_check {
                    board.undo_move(tok);
                    continue;
                }
            }
            let score = -quiescence(state, board, -beta, -alpha, ply + 1, qply + 1, local_abort);
            board.undo_move(tok);

            if state.budget.should_abort() {
                return 0;
            }
            if local_abort.is_some_and(|t| t.is_cancelled()) {
                return 0;
            }
            if score >= beta {
                return score;
            }
            if score > alpha {
                alpha = score;
            }
            qcheck_count += 1;
            if qcheck_count >= MAX_QCHECKS {
                break;
            }
        }
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
    /// Main-search nodes visited across all depths (does not include
    /// speculative-subtree nodes — see `spec_nodes`; this was previously the
    /// only node count and speculative work was invisible to it entirely).
    pub nodes: u64,
    /// Nodes visited by speculative search tasks, counted independently of
    /// `nodes`. Sum the two for a true total.
    pub spec_nodes: u64,
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
    /// Number of depths where bestmove changed (instability indicator).
    pub bestmove_changes: u32,
    /// Principal variation for the best line (`pv_list[0]`) from the last
    /// fully-completed depth, best move first. Other MultiPV lines only get
    /// their single move via `pv_list`, not a full line -- matches
    /// `SearchInfo::pv`'s tracking mechanism and its "not from the TT"
    /// rationale (see there).
    pub pv: Vec<Move>,
    /// YBW split/cancellation activity across this search. See
    /// `SearchInfo::ybw`.
    pub ybw: YbwSearchStats,
}

/// `SpeculativeSearcher` wraps iterative deepening with preemptive
/// parallel speculation driven by the policy function.
pub struct SpeculativeSearcher {
    tt: Arc<Tt>,
    external_abort: Arc<AtomicBool>,
}

impl SpeculativeSearcher {
    /// Create a speculative searcher backed by the given shared TT. The
    /// number of top-policy-ranked candidates to speculate on is a per-search
    /// setting (`SearchConfig::spec_top_n`), not fixed at construction time,
    /// since a USI `setoption` can arrive at any point relative to when this
    /// searcher itself was built.
    pub fn new(tt: Arc<Tt>) -> Self {
        SpeculativeSearcher {
            tt,
            external_abort: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Returns a clone of the abort flag; set to `true` to stop an in-progress search.
    pub fn abort_flag(&self) -> Arc<AtomicBool> {
        self.external_abort.clone()
    }

    /// Probe the TT for the best move stored at `hash` (used to extract ponder move).
    pub fn probe_tt(&self, hash: u64) -> Option<Move> {
        self.tt.probe(hash).and_then(|e| e.mv)
    }

    /// Reset the shared TT in place. Call on `usinewgame` so a new game never
    /// probes entries left behind by a previous, unrelated game.
    pub fn clear_tt(&self) {
        self.tt.clear();
    }

    /// Run iterative-deepening search with preemptive speculative parallelism on
    /// candidate replies, returning the best line plus speculation statistics.
    pub fn search(&self, board: &mut Board, config: SearchConfig) -> SpecSearchInfo {
        self.external_abort.store(false, Ordering::Relaxed);

        let state = Arc::new(SearchState {
            tt: self.tt.clone(),
            budget: Arc::new(Budget::new(config.time_limit, self.external_abort.clone())),
            killers: KillerTable::new(),
            history: HistoryTable::new(),
            countermoves: CountermoveTable::new(),
            config,
            features: SearchFeatures::PRODUCTION,
            ybw_stats: Arc::new(YbwStats::default()),
        });

        // Spec tasks share the *same* Budget as the main search (not an
        // independent copy) so a USI stop or the watchdog firing is visible
        // to both without hand-syncing a separate flag between them.
        let spec_state = Arc::new(SpecState {
            tt: self.tt.clone(),
            budget: state.budget.clone(),
            spec_nodes: AtomicU64::new(0),
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
        let mut pv: Vec<Move> = Vec::new();
        let mut bestmove_changes = 0u32;
        // Single-PV is a structural requirement (speculation predicts the
        // opponent's reply to PV[0], which doesn't exist as a concept under
        // MultiPV), not a measurement toggle -- `use_speculation` layers an
        // independent, explicit kill-switch on top of it, defaulting to
        // `true` so unset behavior matches the original `multi_pv == 1`-only
        // derivation exactly.
        let use_spec = config.use_speculation && config.multi_pv == 1;

        for depth in 1..=config.max_depth {
            // Speculative search only makes sense for single-PV (predicts opponent's reply to PV[0])
            let mut spec_group = if use_spec {
                spec_total += 1;
                Some(SpecGroup::spawn(
                    board,
                    &spec_state,
                    depth + 1,
                    config.spec_top_n,
                ))
            } else {
                None
            };

            // MultiPV: run N root searches per depth, excluding previously found moves.
            // Only the best (index 0) line's full move sequence is tracked into
            // `pv_buf` -- other MultiPV lines only ever get a single move via
            // `pv_list`, matching `SpecSearchInfo::pv`'s documented scope.
            let mut depth_pv: Vec<(Move, i32)> = Vec::new();
            let mut excluded: Vec<Move> = Vec::new();
            let mut pv_buf: Vec<Move> = Vec::new();
            for i in 0..config.multi_pv {
                let (m, score) = root_search(
                    &state,
                    board,
                    depth,
                    best_score,
                    &excluded,
                    if i == 0 { Some(&mut pv_buf) } else { None },
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
                pv = pv_buf;
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

        state.budget.abort_now();

        SpecSearchInfo {
            best_move,
            score: best_score,
            depth: done_depth,
            nodes: state.budget.nodes(),
            spec_nodes: spec_state.spec_nodes.load(Ordering::Relaxed),
            elapsed: state.budget.elapsed(),
            hashfull: self.tt.hashfull(),
            spec_hits,
            spec_total,
            pv_list,
            bestmove_changes,
            pv,
            ybw: state.ybw_stats.snapshot(),
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
/// `pub(crate)`: the speculative-search subsystem (`speculative.rs`) writes into
/// this SAME transposition table (`SpecState.tt` is literally `SearchState.tt`,
/// not an independent copy) and must apply the identical ply conversion on
/// store/probe — otherwise the main search misinterprets a raw, undeflated
/// mate score written by a speculative task as already having been deflated.
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
/// See `score_to_tt`'s doc comment for why this is `pub(crate)`.
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
fn store_tt(
    state: &SearchState,
    hash: u64,
    score: i32,
    depth: u32,
    bound: Bound,
    mv: Option<Move>,
    ply: u32,
) {
    state.tt.store(
        hash,
        TtEntry {
            score: score_to_tt(score, ply),
            depth: depth as u8,
            bound,
            mv,
        },
    );
}

/// Update killer, history, and countermove tables when a quiet move causes a beta cutoff.
/// Must be called with `board` in the state BEFORE `do_move(m)` (so side_to_move is correct).
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
    if m.from.is_some() && board.piece_at(m.to).is_none() && !m.promote {
        killers.add(ply as usize, m);
        history.update(stm, m.piece_kind, m.to, depth);
        if let Some(pm) = prev_mv {
            countermoves.update(stm.flip(), pm, m);
        }
    }
}

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

/// Static Exchange Evaluation — net material gain from a capture sequence on m.to.
///
/// Fast path: when the raw trade (victim − base attacker) is non-losing, the
/// capture is at worst an equal trade, so we return that lower bound without
/// touching the board — this keeps the hot move-ordering path cheap.
///
/// Slow path: only losing-looking captures (victim < attacker) run the full
/// recursive exchange. `do_move`/`undo_move` keep the board exact, so pins,
/// legality, and X-rays unblocked by a vacating piece are handled correctly.
/// The opponent may decline to recapture (modelled by `max(0, ..)` in
/// `see_recapture`); the initial move is never clamped so ordering still sees
/// that a sac is losing.
fn see_score(board: &mut Board, m: Move) -> i32 {
    // Only board moves can be captures; drops never are
    if m.from.is_none() {
        return 0;
    }
    let Some(cap) = board.piece_at(m.to) else {
        return 0;
    };

    let victim_val = PIECE_VALUE[cap.kind.index()];
    let base_attacker_val = PIECE_VALUE[m.piece_kind.index()];
    let promo_gain = if m.promote {
        PIECE_VALUE[m.piece_kind.promoted().index()] - base_attacker_val
    } else {
        0
    };

    // Fast path: if recaptured, the promotion increment cancels (we'd lose the
    // promoted piece), so the worst case is victim − base_attacker. When that is
    // ≥ 0 the capture cannot lose material, so skip the simulation.
    if victim_val >= base_attacker_val {
        return victim_val + promo_gain - base_attacker_val;
    }

    // Losing-looking: simulate the full exchange to see if it is actually losing.
    let tok = board.do_move(m);
    let score = victim_val + promo_gain - see_recapture(board, m.to, 0);
    board.undo_move(tok);
    score
}

/// Value of the best capture sequence on `sq` for the side to move (clamped at 0:
/// the side may decline to recapture). `depth` guards against pathological recursion.
fn see_recapture(board: &mut Board, sq: Square, depth: u32) -> i32 {
    if depth >= 32 {
        return 0;
    }
    // Least-valuable attacker that can capture on `sq`.
    let lva = generate_legal_captures(board)
        .into_iter()
        .filter(|c| c.to == sq)
        .min_by_key(|c| PIECE_VALUE[c.piece_kind.index()]);
    let Some(m) = lva else { return 0 };

    let victim_val = match board.piece_at(sq) {
        Some(p) => PIECE_VALUE[p.kind.index()],
        None => return 0, // sq empty: nothing to recapture
    };
    let tok = board.do_move(m);
    // Decline the recapture if it loses material (stand-pat option).
    let score = (victim_val - see_recapture(board, sq, depth + 1)).max(0);
    board.undo_move(tok);
    score
}

/// Returns 1 if the move just played (reflected in `board`) gives check, 0 otherwise.
/// Capped at `CHECK_EXT_MAX_PLY` to prevent infinite extension chains in perpetual check.
#[inline]
fn check_ext(board: &Board, ply: u32) -> u32 {
    if ply < CHECK_EXT_MAX_PLY && is_in_check(board, board.side_to_move) {
        1
    } else {
        0
    }
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
    // Depth × move-index scaling: conservative at shallow depth, more aggressive deeper.
    // Formula: floor(1 + ln(depth) * ln(move_idx) / 2)
    let r = 1.0 + (depth as f32).ln() * (move_idx as f32).ln() / 2.0;
    let mut r = r as u32;
    // History adjustment: well-tried quiet moves get less reduction; poorly-tried get more.
    let hist = history.get(stm, m.piece_kind, m.to);
    if hist > 3_000 {
        r = r.saturating_sub(1);
    } else if hist < -3_000 && depth >= 5 {
        r += 1;
    }
    r
}

fn order_moves(
    board: &mut Board,
    mut moves: Vec<Move>,
    tt_mv: Option<Move>,
    killers: [Option<Move>; 2],
    countermove: Option<Move>,
    history: &HistoryTable,
    stm: Color,
) -> Vec<Move> {
    // sort_by_cached_key computes the key exactly once per element, preventing
    // races where AtomicI32 history values change between comparisons in rayon threads.
    moves.sort_by_cached_key(|&m| {
        if tt_mv.is_some_and(|t| t == m) {
            return i32::MIN;
        } // 1. TT move first

        // 2. Captures ordered by SEE (2-ply Static Exchange Evaluation)
        //    Winning/equal (see >= 0): searched before killers
        //    Losing (see < 0): searched after quiet moves
        if m.from.is_some() && board.piece_at(m.to).is_some() {
            let see = see_score(board, m);
            return if see >= 0 {
                -(10_000 + see) // range: -11_300 to -10_000 (best captures first)
            } else {
                10_000 - see // range: 10_001 to 11_300 (losing captures last)
            };
        }

        if killers[0].is_some_and(|k| k == m) {
            return -9_100;
        } // 3. Killer 0
        if killers[1].is_some_and(|k| k == m) {
            return -9_050;
        } // 4. Killer 1
        if countermove.is_some_and(|cm| cm == m) {
            return -9_000;
        } // 5. Countermove

        // 6. Remaining quiet moves by history score
        -(-8_000 + history.get(stm, m.piece_kind, m.to))
    });
    moves
}

#[cfg(test)]
mod see_tests {
    use super::*;
    use crate::board::Board;
    use std::time::Instant;

    // Black rook on 5g captures a white pawn on 5e defended by a white pawn on 5d.
    // RxP wins a pawn (100) but loses the rook (1040) to PxR → SEE = 100 - 1040 = -940.
    #[test]
    fn see_losing_capture_defended() {
        let mut b = Board::from_sfen("k8/9/9/4p4/4p4/9/4R4/9/8K b - 1").unwrap();
        let target = Square::from_shogi(5, 5);
        let m = generate_legal_captures(&mut b)
            .into_iter()
            .find(|m| m.to == target)
            .expect("rook capture on 5e");
        assert_eq!(see_score(&mut b, m), -940);
    }

    // Same but no defender: the pawn is free → SEE = +100.
    #[test]
    fn see_free_capture_undefended() {
        let mut b = Board::from_sfen("k8/9/9/9/4p4/9/4R4/9/8K b - 1").unwrap();
        let target = Square::from_shogi(5, 5);
        let m = generate_legal_captures(&mut b)
            .into_iter()
            .find(|m| m.to == target)
            .expect("rook capture on 5e");
        assert_eq!(see_score(&mut b, m), 100);
    }

    // Regression: a search with a tiny hard time limit and a huge max_depth must
    // terminate near the limit, not run to depth 99. Before the watchdog fix the
    // speculative tasks could saturate the rayon pool and starve the time check,
    // hanging the move indefinitely — this test would then never return.
    #[test]
    fn search_respects_time_limit() {
        use crate::tt::Tt;
        let searcher = SpeculativeSearcher::new(Tt::new(8));
        let mut board = Board::startpos();
        let config = SearchConfig {
            max_depth: 99,
            // Generous enough for depth 1 to complete in the slow debug build even
            // under parallel-test rayon contention, so there is always a move;
            // tiny next to a depth-99 search, which would never finish unbounded.
            time_limit: Some(Duration::from_millis(1000)),
            soft_limit: None,
            multi_pv: 1,
            spec_top_n: 4,
            ..Default::default()
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
}

#[cfg(test)]
mod regression_tests {
    use super::*;
    use crate::board::Board;

    fn fresh_state(tt: Arc<Tt>) -> Arc<SearchState> {
        Arc::new(SearchState {
            tt,
            budget: Arc::new(Budget::new(None, Arc::new(AtomicBool::new(false)))),
            killers: KillerTable::new(),
            history: HistoryTable::new(),
            countermoves: CountermoveTable::new(),
            config: SearchConfig::default(),
            features: SearchFeatures::PRODUCTION,
            ybw_stats: Arc::new(YbwStats::default()),
        })
    }

    // Commit-3 requirement: a cancelled split must never leave an incomplete
    // result in the TT. Deterministic (no thread races): cancel the token
    // *before* calling `alpha_beta` at all, so the very first check inside
    // the function (before the TT probe, before any move is even generated)
    // must bail with the sentinel `0` -- and since that's strictly before
    // every `store_tt` call site in the function, the TT must stay empty for
    // this position. This is the structural guarantee the mid-flight
    // checkpoints throughout `alpha_beta`/`quiescence` all rely on; real
    // searches additionally exercise it under genuine concurrency (see
    // `exact_reference_tests::exact_reference_ybw_matches_sequential_pvs_at_threads_2_and_4`),
    // whose `ybw_direct_cutoffs`/`probes_cancelled` counters avoid asserting
    // on flaky OS-scheduling-dependent timing.
    #[test]
    fn cancelled_split_alpha_beta_call_never_stores_to_tt() {
        let mut board = Board::startpos();
        let hash = board.hash();
        let tt = Tt::new(4);
        let state = fresh_state(tt.clone());
        let already_cancelled = SplitCancel::new(None);
        already_cancelled.cancel();

        let score = alpha_beta(
            &state,
            &mut board,
            NEG_INF,
            POS_INF,
            5,
            0,
            true,
            None,
            None,
            None,
            Some(&already_cancelled),
        );

        assert_eq!(
            score, 0,
            "an already-cancelled split must bail immediately with the same sentinel \
             used for a global budget abort"
        );
        assert!(
            tt.probe(hash).is_none(),
            "a cancelled call must never store a TT entry for its own root position"
        );
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

        root_search_inner(&state, &mut board, 1, &moves, NEG_INF, -500_000, None);

        let entry = tt
            .probe(hash)
            .expect("root_search_inner should have stored a TT entry");
        assert_eq!(entry.bound, Bound::Lower);
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

        // A genuine, unaborted search populates a real TT entry.
        root_search_inner(
            &fresh_state(tt.clone()),
            &mut board,
            2,
            &moves,
            NEG_INF,
            POS_INF,
            None,
        );
        let genuine = tt
            .probe(hash)
            .expect("first call should have stored a genuine TT entry");

        // A second call, at a deeper depth, with external_abort already set —
        // simulating a USI "stop" that arrived before this root search began.
        let aborted_state = Arc::new(SearchState {
            tt: tt.clone(),
            budget: Arc::new(Budget::new(None, Arc::new(AtomicBool::new(true)))),
            killers: KillerTable::new(),
            history: HistoryTable::new(),
            countermoves: CountermoveTable::new(),
            config: SearchConfig::default(),
            features: SearchFeatures::PRODUCTION,
            ybw_stats: Arc::new(YbwStats::default()),
        });
        root_search_inner(
            &aborted_state,
            &mut board,
            7,
            &moves,
            NEG_INF,
            POS_INF,
            None,
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
            None,
        );

        let mut board_b = Board::from_sfen(MATE_IN_1_SFEN).unwrap();
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
            None,
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

/// PVS-vs-plain-alpha-beta correctness guard, at two different rigor levels.
///
/// PVS's null-window probe runs (and, on a fail-high, writes TT/history/
/// killer state) *before* the eventual full-window search of the same move --
/// under production `SearchFeatures` this can and does change which
/// heuristics fire downstream (LMR's reduction depends on move index, which
/// history/killer reordering changes; IIR depends on whether a TT move is
/// present; etc.). That makes sequential-AB-vs-sequential-PVS score/bestmove
/// equality *not* a valid correctness test under production features --
/// verified empirically: an earlier version of this suite asserted exactly
/// that and found real divergences (see git history for this comment). The
/// fix isn't to weaken the guard to "returns *a* legal move" and call it a
/// day -- it's to test the right thing at the right rigor level:
///
/// - `exact_reference_score_matches_with_and_without_pvs`: under
///   `SearchFeatures::EXACT_REFERENCE[_WITH_TT]` (every non-exact heuristic
///   off), PVS is a *provably* exact reformulation of alpha-beta, so score
///   equality is a real, permanent regression guard on the ~20 lines of PVS
///   window/re-search logic itself.
/// - `production_pvs_returns_a_sane_legal_result`: under real
///   `SearchFeatures::PRODUCTION`, only sanity (legal move, unmutated board,
///   score in range) is asserted -- production strength differences are an
///   Elo-gate question, not a unit-test question.
#[cfg(test)]
mod exact_reference_tests {
    use super::*;
    use crate::board::Board;
    use crate::movegen::generate_legal_moves;
    use crate::tt::Tt;

    // Chosen to collectively cover: an ordinary midgame position that also
    // happens to demonstrate a production-config bestmove tie, a position
    // with the side to move in check, a forced mate, a position with
    // several captures and pieces in hand to drop, and a position with a
    // promote/don't-promote choice.
    const POSITIONS: &[(&str, &str)] = &[
        (
            "midgame_tie",
            "lnsgkgsnl/6rb1/pppppp2p/9/6p2/7R1/PPPPPPP1P/1B7/LNSGKGSNL w Pp 10",
        ),
        ("in_check", "4k4/9/4R4/9/9/9/9/9/4K4 w - 1"),
        ("mate_in_1", "k8/2K6/9/9/4R4/9/9/9/9 b - 1"),
        (
            "multi_capture_and_drop",
            "lnsgkgsnl/7b1/p1pppp1pp/6p2/9/1r2P2P1/P1PP1PP1P/1SG2S1R1/LN2KG1NL w 2Pb 16",
        ),
        (
            "promotion_choice",
            "l2gk2nl/1rs2sgb1/p1n1pp1pp/2pp2p2/1p5P1/2P3P2/PPSPPP2P/1BG1GS1R1/LN1K3NL w - 20",
        ),
    ];

    fn run(sfen: &str, depth: u32, use_pvs: bool, features: SearchFeatures) -> SearchInfo {
        run_with_ybw(sfen, depth, use_pvs, false, features)
    }

    fn run_with_ybw(
        sfen: &str,
        depth: u32,
        use_pvs: bool,
        use_ybw: bool,
        features: SearchFeatures,
    ) -> SearchInfo {
        let mut board = Board::from_sfen(sfen).unwrap();
        let board_before = board.clone();
        let hash_before = board.hash();
        let acc_before = board.acc.clone();
        let info = Searcher::new(Tt::new(4)).search_with_features(
            &mut board,
            SearchConfig {
                max_depth: depth,
                time_limit: None,
                soft_limit: None,
                multi_pv: 1,
                use_ybw,
                use_speculation: false,
                use_pvs,
                ..Default::default()
            },
            features,
        );
        assert_eq!(
            board.hash(),
            hash_before,
            "search must leave the board exactly as it found it"
        );
        assert_eq!(
            board.acc, acc_before,
            "search must leave the NNUE accumulator exactly as it found it"
        );
        // Every caller gets a PV-legality check for free -- this is what
        // catches a YBW direct-cutoff PV node emitting a duplicated move
        // (`[m, m]`) instead of just `[m]`.
        assert_pv_matches_bestmove_and_replays_legally(
            &format!("{sfen} depth={depth} use_pvs={use_pvs} use_ybw={use_ybw}"),
            &board_before,
            info.best_move,
            &info.pv,
        );
        info
    }

    #[test]
    fn exact_reference_score_matches_with_and_without_pvs() {
        for &(label, sfen) in POSITIONS {
            for depth in 1..=4u32 {
                for features in [
                    SearchFeatures::EXACT_REFERENCE,
                    SearchFeatures::EXACT_REFERENCE_WITH_TT,
                ] {
                    let ab = run(sfen, depth, false, features);
                    let pvs = run(sfen, depth, true, features);
                    assert_eq!(
                        ab.score, pvs.score,
                        "{label} at depth {depth} (tt_cutoff={}): seq-AB and seq-PVS must \
                         score identically under a non-selective reference tree -- a \
                         difference here means PVS's window/re-search logic is wrong, not \
                         benign heuristic noise (bestmove is deliberately not compared: a \
                         tied position can correctly report either move)",
                        features.tt_cutoff,
                    );
                }
            }
        }
    }

    // Commit-3 correctness gate: YBW's true early-cancellation must not
    // change the *value* PVS+YBW computes relative to sequential PVS, under
    // the same non-selective `EXACT_REFERENCE[_WITH_TT]` reference tree used
    // above. Run under a local Rayon pool at Threads=2 and Threads=4 (never
    // the global pool, so this doesn't depend on how many cores the test
    // machine has) so the parallel dispatch, cancellation propagation, and
    // the cutoff-worker-keeps-its-own-result ordering are all genuinely
    // exercised, not just present in single-threaded code. Bestmove is
    // deliberately not compared, for the same tie-breaking reason as above.
    #[test]
    fn exact_reference_ybw_matches_sequential_pvs_at_threads_2_and_4() {
        let mut total_direct_cutoffs = 0u64;
        for &n_threads in &[2usize, 4usize] {
            let pool = rayon::ThreadPoolBuilder::new()
                .num_threads(n_threads)
                .build()
                .unwrap();
            pool.install(|| {
                for &(label, sfen) in POSITIONS {
                    for depth in 1..=4u32 {
                        for features in [
                            SearchFeatures::EXACT_REFERENCE,
                            SearchFeatures::EXACT_REFERENCE_WITH_TT,
                        ] {
                            let seq = run_with_ybw(sfen, depth, true, false, features);
                            let ybw = run_with_ybw(sfen, depth, true, true, features);
                            assert_eq!(
                                seq.score, ybw.score,
                                "{label} at depth {depth} threads={n_threads} \
                                 (tt_cutoff={}): seq-PVS and PVS+YBW must score \
                                 identically under a non-selective reference tree -- a \
                                 difference here means the cancellation feature changed \
                                 the computed value, not just which siblings ran \
                                 (bestmove deliberately not compared)",
                                features.tt_cutoff,
                            );
                            // Not a strict equality: when a *nested* split's own
                            // ambient `local_abort` (an ancestor split, cancelled by
                            // a sibling further up the tree) fires while this
                            // split's post-collect loop is partway through, already
                            // -collected results for the remaining siblings are
                            // abandoned wholesale rather than individually
                            // reclassified -- by construction, started can only be
                            // >= the sum actually classified, never less.
                            assert!(
                                ybw.ybw.probes_started
                                    >= ybw.ybw.probes_completed + ybw.ybw.probes_cancelled,
                                "{label} at depth {depth} threads={n_threads}: probes \
                                 classified as completed+cancelled ({} + {}) must never \
                                 exceed probes actually dispatched ({})",
                                ybw.ybw.probes_completed,
                                ybw.ybw.probes_cancelled,
                                ybw.ybw.probes_started,
                            );
                            total_direct_cutoffs += ybw.ybw.direct_cutoffs;
                        }
                    }
                }
            });
        }
        // Deliberately not asserting `probes_cancelled > 0` here: whether any
        // given sibling is fast enough to observe another's cancellation
        // before finishing on its own is real OS-scheduling-dependent
        // timing, not a value-correctness property -- exactly the kind of
        // flaky assertion to avoid. `direct_cutoffs` is the reliable signal
        // (a full-depth probe reaching beta is deterministic given a fixed
        // position/depth/features, independent of scheduling), which is why
        // it's the one asserted on.
        assert!(
            total_direct_cutoffs > 0,
            "this sweep should exercise at least one direct YBW cutoff across all \
             positions/depths/thread-counts -- if not, cancellation was never actually \
             tested and the equality assertions above are vacuous"
        );
    }

    #[test]
    fn production_pvs_returns_a_sane_legal_result() {
        for &(label, sfen) in POSITIONS {
            let mut board = Board::from_sfen(sfen).unwrap();
            let hash_before = board.hash();
            let acc_before = board.acc.clone();
            let legal = generate_legal_moves(&mut board);
            let config = SearchConfig {
                max_depth: 4,
                time_limit: None,
                soft_limit: None,
                multi_pv: 1,
                use_pvs: true,
                ..Default::default()
            };
            let info = Searcher::new(Tt::new(4)).search(&mut board, config);
            assert_eq!(
                board.hash(),
                hash_before,
                "{label}: board mutated by search"
            );
            assert_eq!(
                board.acc, acc_before,
                "{label}: NNUE accumulator mutated by search"
            );
            match info.best_move {
                Some(mv) => assert!(legal.contains(&mv), "{label}: bestmove {mv:?} not legal"),
                None => assert!(legal.is_empty(), "{label}: no bestmove despite legal moves"),
            }
            assert!(
                info.score.abs() <= MATE_SCORE,
                "{label}: score {} outside valid range",
                info.score
            );

            // Reproducibility: fresh state, same config, same result.
            let mut board2 = Board::from_sfen(sfen).unwrap();
            let info2 = Searcher::new(Tt::new(4)).search(&mut board2, config);
            assert_eq!(
                info.best_move, info2.best_move,
                "{label}: bestmove not reproducible across a fresh-state rerun"
            );
            assert_eq!(
                info.score, info2.score,
                "{label}: score not reproducible across a fresh-state rerun"
            );
        }
    }

    // pv[0] must always equal best_move, and the whole line must replay as
    // legal moves from the root position -- checked across sequential AB,
    // sequential PVS, and the real production defaults (UseYBW/UseSpeculation
    // on), since this is a structural property of the PV-tracking mechanism
    // itself, not something that depends on which heuristics are active.
    // Only the last fully-completed depth's PV is ever exposed (see
    // `Searcher::search`/`SpeculativeSearcher::search`'s `pv = pv_buf`
    // assignment, gated behind their own `should_abort`/`timed_out` checks,
    // plus `root_search_inner`'s defensive clear-on-entry) -- an aborted or
    // still-in-progress iteration's line can never leak into `info.pv`, and
    // a null-window probe's line can't either, since probes are always
    // passed `None` for `pv`.
    fn assert_pv_matches_bestmove_and_replays_legally(
        label: &str,
        board_before: &Board,
        best_move: Option<Move>,
        pv: &[Move],
    ) {
        let Some(bestmove) = best_move else {
            assert!(pv.is_empty(), "{label}: pv non-empty with no bestmove");
            return;
        };
        assert_eq!(
            pv.first().copied(),
            Some(bestmove),
            "{label}: pv[0] must equal best_move"
        );
        let mut replay = board_before.clone();
        for &mv in pv {
            let legal = generate_legal_moves(&mut replay);
            assert!(
                legal.contains(&mv),
                "{label}: pv move {mv:?} is not legal at its point in the replay"
            );
            replay.do_move(mv);
        }
    }

    #[test]
    fn pv_head_matches_bestmove_and_replays_legally() {
        for &(label, sfen) in POSITIONS {
            for depth in 1..=4u32 {
                for use_pvs in [false, true] {
                    let board = Board::from_sfen(sfen).unwrap();
                    let mut board_mut = board.clone();
                    let info = Searcher::new(Tt::new(4)).search(
                        &mut board_mut,
                        SearchConfig {
                            max_depth: depth,
                            time_limit: None,
                            soft_limit: None,
                            multi_pv: 1,
                            use_ybw: false,
                            use_speculation: false,
                            use_pvs,
                            ..Default::default()
                        },
                    );
                    assert_pv_matches_bestmove_and_replays_legally(
                        &format!("{label} depth={depth} use_pvs={use_pvs} (sequential)"),
                        &board,
                        info.best_move,
                        &info.pv,
                    );
                }
            }

            // Real production defaults: UseYBW + UseSpeculation + UsePVS all
            // on, exactly what a live game uses -- via SpeculativeSearcher,
            // since that's what sekirei-usi always constructs.
            let board = Board::from_sfen(sfen).unwrap();
            let mut board_mut = board.clone();
            let spec_info = SpeculativeSearcher::new(Tt::new(4)).search(
                &mut board_mut,
                SearchConfig {
                    max_depth: 4,
                    time_limit: None,
                    soft_limit: None,
                    multi_pv: 1,
                    ..Default::default()
                },
            );
            assert_pv_matches_bestmove_and_replays_legally(
                &format!("{label} (production SpeculativeSearcher)"),
                &board,
                spec_info.best_move,
                &spec_info.pv,
            );
        }
    }
}
