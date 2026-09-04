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
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, Ordering};
use std::sync::{Arc, OnceLock};
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

/// Exact cache of the existing floating-point LMR formula for all representable
/// TT depths and the practical maximum shogi move count. This removes two
/// transcendental `ln` calls from every late-move probe without changing the
/// reduction chosen by the search.
static LMR_REDUCTION_TABLE: OnceLock<Box<[[u8; 600]]>> = OnceLock::new();

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
        Self::with_abort_flag(tt, Arc::new(AtomicBool::new(false)))
    }

    /// Create a searcher using a caller-owned abort flag. This lets independent
    /// Lazy SMP workers stop as one group without sharing mutable search state.
    pub fn with_abort_flag(tt: Arc<Tt>, external_abort: Arc<AtomicBool>) -> Self {
        Searcher { tt, external_abort }
    }

    /// Returns an `Arc` to the abort flag; store `true` to stop the search early.
    pub fn abort_flag(&self) -> Arc<AtomicBool> {
        self.external_abort.clone()
    }

    /// Clear a previous stop signal before starting a new search.
    pub fn reset_abort_flag(&self) {
        self.external_abort.store(false, Ordering::Relaxed);
    }

    /// Run iterative-deepening search from the current position up to `config.max_depth`
    /// or until a time limit / abort signal fires, returning the best line found.
    ///
    /// Call [`Self::reset_abort_flag`] before reusing a searcher after an abort.
    pub fn search(&self, board: &mut Board, config: SearchConfig) -> SearchInfo {
        let state = Arc::new(SearchState {
            tt: self.tt.clone(),
            budget: Arc::new(Budget::new(
                config.time_limit,
                config.node_limit,
                self.external_abort.clone(),
            )),
            killers: KillerTable::new(),
            history: HistoryTable::new(),
            countermoves: CountermoveTable::new(),
        });

        let mut best_move = None;
        let mut best_score = NEG_INF;
        let mut done_depth = 0;
        let mut prev_best: Option<Move> = None;

        for depth in 1..=config.max_depth {
            let (m, score) = root_search(&state, board, depth, best_score, &[]);

            if state.budget.should_abort() {
                break;
            }

            best_move = m.or(best_move);
            best_score = score;
            done_depth = depth;

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
            best_move = generate_legal_moves(board).into_iter().next();
            if best_move.is_some() {
                best_score = evaluate(board);
            }
        }

        SearchInfo {
            best_move,
            score: best_score,
            depth: done_depth,
            nodes: state.budget.nodes(),
            elapsed: state.budget.elapsed(),
            hashfull: self.tt.hashfull(),
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
    let use_asp = depth >= 2 && prev_score.abs() < MATE_SCORE - 1000;
    let (mut lo, mut hi) = if use_asp {
        (prev_score - ASP_DELTA, prev_score + ASP_DELTA)
    } else {
        (NEG_INF, POS_INF)
    };

    loop {
        let (m, score) = root_search_inner(state, board, depth, &ordered, lo, hi);

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

fn root_search_inner(
    state: &Arc<SearchState>,
    board: &mut Board,
    depth: u32,
    ordered: &[Move],
    lo: i32,
    hi: i32,
) -> (Option<Move>, i32) {
    let mut best_move = None;
    let mut alpha = lo;

    for &m in ordered {
        let tok = board.do_move(m);
        let child_in_check = is_in_check(board, board.side_to_move);
        let score = -alpha_beta(
            state,
            board,
            -hi,
            -alpha,
            depth - 1,
            1,
            true,
            Some(m),
            None,
            Some(child_in_check),
        );
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
    known_in_check: Option<bool>, // supplied by a parent that already tested the moved position
) -> i32 {
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
        return quiescence(state, board, alpha, beta, ply, 0, known_in_check);
    }

    // TT probe
    let hash = board.hash();
    let orig_alpha = alpha;
    let mut tt_mv = None;
    let mut tt_se_score = None::<i32>; // TT score for singular extension (lower/exact bound only)
    let mut tt_se_depth = 0u8; // TT entry depth for SE eligibility check

    if let Some(entry) = state.tt.probe(hash) {
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
    let depth = if tt_mv.is_none() && depth >= 4 {
        depth - 1
    } else {
        depth
    };

    let stm = board.side_to_move;

    // Countermove: best quiet response to the opponent's previous move
    let countermove = prev_mv.and_then(|pm| state.countermoves.get(stm.flip(), pm));

    // Static eval — computed once per node for RFP and Futility Pruning.
    // Skipped when in check (position is not "quiet") or depth > 5 (overhead not justified).
    let in_check = known_in_check.unwrap_or_else(|| is_in_check(board, stm));
    let static_eval: Option<i32> = if !in_check && depth <= 5 {
        Some(evaluate(board))
    } else {
        None
    };

    // Reverse Futility Pruning: if a rough lower bound already beats beta, return early.
    if let Some(se) = static_eval
        && depth <= 3
        && beta.abs() < MATE_SCORE - 1000
        && se - RFP_MARGIN * depth as i32 >= beta
    {
        return se;
    }

    // ProbCut: if a shallow (depth-4) search with an inflated beta suggests this node
    // will fail high by more than PC_MARGIN, prune without a full search.
    // Only try captures with SEE >= PC_MARGIN (already winning material gain).
    if depth >= PC_MIN_DEPTH && !in_check && beta.abs() < MATE_SCORE - 1000 && skip_move.is_none()
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
            let tok = board.do_move(cap);
            let child_in_check = is_in_check(board, board.side_to_move);
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
            );
            board.undo_move(tok);
            if pc_score >= pc_beta {
                return pc_score;
            }
        }
    }

    // Null Move Pruning
    if can_null && depth > NMP_R && beta.abs() < MATE_SCORE - 1000 && !in_check
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
        );
        board.undo_null_move(null_tok);

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
                    Some(in_check),
                );
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
        skip_move.is_none()
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
            Some(in_check),
        );
        u32::from(sval < se_beta) // 1 if TT move is singular, else 0
    } else {
        0
    };

    // Quiet moves tried so far — used to apply history malus on beta cutoff.
    let enemy = board.occ_for(stm.flip());
    let mut tried_quiet: Vec<Move> = Vec::new();

    // ---------- First child: always sequential ----------
    let first_move = ordered[0];
    let tok = board.do_move(first_move);
    let child_in_check = is_in_check(board, board.side_to_move);
    let ext0 = check_ext(child_in_check, ply + 1);
    // Apply singular extension to the TT move (ordered[0] when tt_mv is set)
    let first_ext = ext0
        + if tt_mv.is_some_and(|t| t == first_move) {
            sing_ext
        } else {
            0
        };
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
    );
    board.undo_move(tok);

    if state.budget.should_abort() {
        return 0;
    }

    let mut best_score = score0;
    let mut best_move = Some(first_move);

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
        store_tt(
            state,
            hash,
            score0,
            depth,
            Bound::Lower,
            best_move,
            ply,
            skip_move,
        );
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
        store_tt(
            state, hash, best_score, depth, bound, best_move, ply, skip_move,
        );
        return best_score;
    }

    // ---------- Young brothers ----------
    // Returns the index in `rest` where sequential processing should begin:
    // ybw_end after the parallel YBW pass, or 0 at shallow depths (no YBW).
    let seq_start = if depth >= MIN_SPLIT_DEPTH {
        let nw_abort = AtomicBool::new(false);
        let alpha_for_nw = alpha;

        // ponytail: limit parallel siblings; tail searched sequentially after YBW pass
        const YBW_MAX_SIBLINGS: usize = 6;
        let ybw_end = rest.len().min(YBW_MAX_SIBLINGS);

        // Null-window parallel probe (with LMR for quiet late moves)
        // Rayon joins before returning, so the closure can borrow state and the
        // abort flag directly. Clone only the worker's private Board; the old
        // staging Vec also cloned every Arc and allocated once per split.
        let nw_results: Vec<(Move, i32, usize)> = rest[..ybw_end]
            .par_iter()
            .enumerate()
            .filter_map(|(i, &m)| {
                if nw_abort.load(Ordering::Relaxed) || state.budget.should_abort() {
                    return None;
                }
                let idx = i + 1;
                let mut b = board.clone();
                let reduce = lmr_reduce(&b, m, idx, depth, &killers, tt_mv, &state.history, stm);
                let tok = b.do_move(m);
                let child_in_check = is_in_check(&b, b.side_to_move);
                let ext = check_ext(child_in_check, ply + 1);
                let reduce = if ext > 0 { 0 } else { reduce }; // never reduce a checking move
                let probe_depth = depth.saturating_sub(1 + reduce) + ext;
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
                );
                b.undo_move(tok);
                Some((m, s, idx))
            })
            .collect();

        // Sequential pass: handle fail-highs, update heuristics, apply history malus
        for (m, nw_score, _idx) in nw_results {
            if state.budget.should_abort() {
                break;
            }

            let is_quiet_ybw = m.from.is_some() && !enemy.contains(m.to) && !m.promote;

            let s = if nw_score > alpha {
                // Fail-high: re-search at full depth with full window
                let tok = board.do_move(m);
                let child_in_check = is_in_check(board, board.side_to_move);
                let ext = check_ext(child_in_check, ply + 1);
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
                    Some(child_in_check),
                );
                board.undo_move(tok);
                full
            } else {
                nw_score
            };

            if s > best_score {
                best_score = s;
                best_move = Some(m);
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
                nw_abort.store(true, Ordering::Relaxed);
                store_tt(
                    state,
                    hash,
                    best_score,
                    depth,
                    Bound::Lower,
                    best_move,
                    ply,
                    skip_move,
                );
                return best_score;
            }
            if s > alpha {
                alpha = s;
            }
            if is_quiet_ybw {
                tried_quiet.push(m);
            }
        }
        ybw_end
    } else {
        0
    };

    // Sequential pass: remaining siblings (tail beyond YBW limit, or all at shallow depth).
    {
        let lmp_limit = if !in_check && depth <= 2 {
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

            let is_capture = m.from.is_some() && enemy.contains(m.to);
            let is_quiet = m.from.is_some() && !is_capture && !m.promote;

            // Futility Pruning: at depth 1, skip quiet moves that can't reach alpha
            if depth == 1
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

            let reduce = lmr_reduce(board, m, i + 1, depth, &killers, tt_mv, &state.history, stm);
            let tok = board.do_move(m);
            let child_in_check = is_in_check(board, board.side_to_move);
            let ext = check_ext(child_in_check, ply + 1);
            let reduce = if ext > 0 { 0 } else { reduce }; // never reduce a checking move

            // LMR probe
            let probe_depth = depth.saturating_sub(1 + reduce) + ext;
            let mut s = -alpha_beta(
                state,
                board,
                -beta,
                -alpha,
                probe_depth,
                ply + 1,
                true,
                Some(m),
                None,
                Some(child_in_check),
            );

            // Re-search at full depth if LMR probe fails high
            if reduce > 0 && s > alpha {
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
                    Some(child_in_check),
                );
            }
            board.undo_move(tok);

            if s > best_score {
                best_score = s;
                best_move = Some(m);
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
                store_tt(
                    state,
                    hash,
                    best_score,
                    depth,
                    Bound::Lower,
                    best_move,
                    ply,
                    skip_move,
                );
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
    store_tt(
        state, hash, best_score, depth, bound, best_move, ply, skip_move,
    );
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
    known_in_check: Option<bool>,
) -> i32 {
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
        return evaluate(board);
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
        && let Some(entry) = state.tt.probe(hash)
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
        let stand_pat = evaluate(board);
        if stand_pat >= beta {
            if qply == 0 && !state.budget.should_abort() {
                state.tt.store(
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
                state.tt.store(
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

    let moves = if in_check {
        generate_legal_moves(board) // must escape check; all legal moves required
    } else {
        generate_legal_captures(board)
    };

    if moves.is_empty() {
        let score = if in_check {
            -MATE_SCORE + ply as i32 // checkmate
        } else {
            alpha
        };
        if qply == 0 && !state.budget.should_abort() {
            state.tt.store(
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

    // Order by a cheap MVV-LVA-style key. Recursive see_score here is too costly
    // per node (qsearch is the hottest path); the coarse capture ordering is
    // plenty for quiescence and keeps each node fast enough to respect the clock.
    let mut ordered = moves;
    ordered.sort_by_cached_key(|&m| {
        (
            if Some(m) == tt_mv { 0 } else { 1 },
            -qsearch_order_key(board, m),
        )
    });

    let mut best_move = None;
    for m in ordered {
        let tok = board.do_move(m);
        let score = -quiescence(state, board, -beta, -alpha, ply + 1, qply + 1, None);
        board.undo_move(tok);

        if state.budget.should_abort() {
            return 0;
        }
        if score >= beta {
            if qply == 0 && !state.budget.should_abort() {
                state.tt.store(
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
    if !in_check && qply == 0 {
        const MAX_QCHECKS: usize = 4;
        let mut qcheck_count = 0;
        let mut qchecks = generate_legal_moves(board);
        qchecks.sort_by_cached_key(|&m| if Some(m) == tt_mv { 0 } else { 1 });
        for m in qchecks {
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
            let score = -quiescence(state, board, -beta, -alpha, ply + 1, qply + 1, None);
            board.undo_move(tok);

            if state.budget.should_abort() {
                return 0;
            }
            if score >= beta {
                if !state.budget.should_abort() {
                    state.tt.store(
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
        state.tt.store(
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
    /// Number of depths where bestmove changed (instability indicator).
    pub bestmove_changes: u32,
}

/// `SpeculativeSearcher` wraps iterative deepening with preemptive
/// parallel speculation driven by the policy function.
pub struct SpeculativeSearcher {
    tt: Arc<Tt>,
    top_n: usize,
    external_abort: Arc<AtomicBool>,
    // Dedicated pool for SpecGroup's background tasks, isolated from rayon's
    // global pool so they can never starve alpha_beta's own YBW dispatch
    // (`work.into_par_iter()...collect()`) of a worker. See SpecState::pool.
    spec_pool: Arc<rayon::ThreadPool>,
}

impl SpeculativeSearcher {
    /// Create a speculative searcher that considers the top `top_n` candidate
    /// replies for preemptive background search, backed by the given shared TT.
    pub fn new(tt: Arc<Tt>, top_n: usize) -> Self {
        let spec_pool = rayon::ThreadPoolBuilder::new()
            .num_threads(top_n.max(1))
            .build()
            .expect("failed to build dedicated speculative-search thread pool");
        SpeculativeSearcher {
            tt,
            top_n,
            external_abort: Arc::new(AtomicBool::new(false)),
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
    }

    /// Run iterative-deepening search with preemptive speculative parallelism on
    /// candidate replies, returning the best line plus speculation statistics.
    ///
    /// Call [`Self::reset_abort_flag`] before reusing a searcher after an abort.
    pub fn search(&self, board: &mut Board, config: SearchConfig) -> SpecSearchInfo {
        let state = Arc::new(SearchState {
            tt: self.tt.clone(),
            budget: Arc::new(Budget::new(
                config.time_limit,
                config.node_limit,
                self.external_abort.clone(),
            )),
            killers: KillerTable::new(),
            history: HistoryTable::new(),
            countermoves: CountermoveTable::new(),
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
                let (m, score) = root_search(&state, board, depth, best_score, &excluded);
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
            best_move = generate_legal_moves(board).into_iter().next();
            if best_move.is_some() {
                best_score = evaluate(board);
            }
        }

        state.budget.abort_now();

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
fn check_ext(in_check: bool, ply: u32) -> u32 {
    if ply < CHECK_EXT_MAX_PLY && in_check {
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
    let mut r = lmr_base_reduction(depth, move_idx);
    // History adjustment: well-tried quiet moves get less reduction; poorly-tried get more.
    let hist = history.get(stm, m.piece_kind, m.to);
    if hist > 3_000 {
        r = r.saturating_sub(1);
    } else if hist < -3_000 && depth >= 5 {
        r += 1;
    }
    r
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
                    std::array::from_fn(|index| (1.0 + depth_ln * move_lns[index] / 2.0) as u8)
                })
                .collect::<Vec<_>>()
                .into_boxed_slice()
        });
        table[depth as usize][move_idx] as u32
    } else {
        (1.0 + (depth as f32).ln() * (move_idx as f32).ln() / 2.0) as u32
    }
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

    #[test]
    fn lmr_table_is_bit_exact_with_previous_formula() {
        for depth in 3..128u32 {
            for move_idx in 2..600usize {
                let expected = (1.0 + (depth as f32).ln() * (move_idx as f32).ln() / 2.0) as u32;
                assert_eq!(lmr_base_reduction(depth, move_idx), expected);
            }
        }
    }

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
            history: HistoryTable::new(),
            countermoves: CountermoveTable::new(),
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

        root_search_inner(&state, &mut board, 1, &moves, NEG_INF, -500_000);

        let entry = tt
            .probe(hash)
            .expect("root_search_inner should have stored a TT entry");
        assert_eq!(entry.bound, Bound::Lower);
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
            history: HistoryTable::new(),
            countermoves: CountermoveTable::new(),
        });
        root_search_inner(&aborted_state, &mut board, 7, &moves, NEG_INF, POS_INF);

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
        let first = quiescence(&state, &mut board, NEG_INF, POS_INF, 3, 0, None);
        let entry = tt
            .probe(hash)
            .expect("top-level qsearch should store depth zero");
        assert_eq!(entry.depth, 0);
        assert_eq!(score_from_tt(entry.score, 3), first);

        // An exact depth-zero hit must avoid re-searching the same top-level
        // qsearch, while remaining valid at the original ply.
        let second = quiescence(&state, &mut board, first - 1, first + 1, 3, 0, None);
        assert_eq!(second, first);
    }

    #[test]
    fn qsearch_does_not_store_after_abort_or_overwrite_deeper_entry() {
        let mut board = Board::startpos();
        let hash = board.hash();
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
        let _ = quiescence(&state, &mut board, NEG_INF, POS_INF, 0, 0, None);
        assert_eq!(tt.probe(hash).expect("deeper entry must remain").depth, 4);

        let aborted_tt = Tt::new(1);
        let aborted_state = Arc::new(SearchState {
            tt: aborted_tt.clone(),
            budget: Arc::new(Budget::new(None, None, Arc::new(AtomicBool::new(true)))),
            killers: KillerTable::new(),
            history: HistoryTable::new(),
            countermoves: CountermoveTable::new(),
        });
        let _ = quiescence(&aborted_state, &mut board, NEG_INF, POS_INF, 0, 0, None);
        assert!(
            aborted_tt.probe(hash).is_none(),
            "aborted qsearch must not publish a TT entry"
        );
    }

    #[test]
    fn supplied_check_state_matches_recomputed_state() {
        for sfen in [crate::sfen::STARTPOS_SFEN, "4r3k/9/9/9/9/9/9/9/4K4 b - 1"] {
            let mut recomputed_board = Board::from_sfen(sfen).unwrap();
            let recomputed_state = fresh_state(Tt::new(1));
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
            );

            let mut supplied_board = Board::from_sfen(sfen).unwrap();
            let supplied_check = is_in_check(&supplied_board, supplied_board.side_to_move);
            let supplied_state = fresh_state(Tt::new(1));
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
