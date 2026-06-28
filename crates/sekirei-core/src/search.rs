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
use std::time::{Duration, Instant};

use crate::board::Board;
use crate::color::Color;
use crate::eval::{PIECE_VALUE, evaluate};
use crate::movegen::{generate_legal_captures, generate_legal_moves, is_in_check};
use crate::mv::Move;
use crate::piece::PieceKind;
use crate::speculative::{SpecGroup, SpecState};
use crate::square::Square;
use crate::tt::{Bound, Tt, TtEntry};

pub const MATE_SCORE: i32 = 900_000;
pub const NEG_INF: i32 = -1_000_000;
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

pub struct SearchConfig {
    pub max_depth: u32,
    pub time_limit: Option<Duration>,
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
            soft_limit: None,
            multi_pv: 1,
        }
    }
}

pub struct SearchInfo {
    pub best_move: Option<Move>,
    pub score: i32,
    pub depth: u32,
    pub nodes: u64,
    pub elapsed: Duration,
    pub hashfull: u32,
}

// ============================================================
// Internal search state (shared across threads via Arc)
// ============================================================

struct SearchState {
    tt: Arc<Tt>,
    nodes: AtomicU64,
    /// Set by the time-check inside alpha_beta
    abort: AtomicBool,
    /// External stop signal (e.g. USI "stop" command)
    external_abort: Arc<AtomicBool>,
    start: Instant,
    time_limit: Option<Duration>,
    killers: KillerTable,
    history: HistoryTable,
    countermoves: CountermoveTable,
}

// SAFETY: All fields are either atomic types (Sync), Arc (Sync), or Instant/Option<Duration> (Sync).

// ============================================================
// Searcher
// ============================================================

pub struct Searcher {
    tt: Arc<Tt>,
    /// Exposed for USI "stop" command — set to true to abort an in-progress search
    external_abort: Arc<AtomicBool>,
}

impl Searcher {
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

    pub fn search(&self, board: &mut Board, config: SearchConfig) -> SearchInfo {
        self.external_abort.store(false, Ordering::Relaxed);

        let state = Arc::new(SearchState {
            tt: self.tt.clone(),
            nodes: AtomicU64::new(0),
            abort: AtomicBool::new(false),
            external_abort: self.external_abort.clone(),
            start: Instant::now(),
            time_limit: config.time_limit,
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

            if state.abort.load(Ordering::Relaxed) {
                break;
            }

            best_move = m.or(best_move);
            best_score = score;
            done_depth = depth;

            if score.abs() >= MATE_SCORE - 1000 {
                break;
            }

            if depth >= 2
                && config
                    .soft_limit
                    .is_some_and(|soft| state.start.elapsed() >= soft)
                && best_move == prev_best
            {
                break;
            }
            prev_best = best_move;
        }

        SearchInfo {
            best_move,
            score: best_score,
            depth: done_depth,
            nodes: state.nodes.load(Ordering::Relaxed),
            elapsed: state.start.elapsed(),
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
        all_moves.into_iter().filter(|m| !excluded.contains(m)).collect()
    };
    if moves.is_empty() {
        return (None, -MATE_SCORE);
    }

    // Single legal move: return immediately without searching
    if moves.len() == 1 {
        return (Some(moves[0]), 0);
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
        let mated = generate_legal_moves(board).is_empty()
            && is_in_check(board, board.side_to_move);
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
                if generate_legal_moves(board).is_empty()
                    && is_in_check(board, board.side_to_move)
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

        if state.abort.load(Ordering::Relaxed) {
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
        let score = -alpha_beta(state, board, -hi, -alpha, depth - 1, 1, true, Some(m), None);
        board.undo_move(tok);

        if state.abort.load(Ordering::Relaxed) {
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
) -> i32 {
    state.nodes.fetch_add(1, Ordering::Relaxed);

    if state.nodes.load(Ordering::Relaxed) & 0xFFF == 0
        && let Some(lim) = state.time_limit
        && state.start.elapsed() >= lim
    {
        state.abort.store(true, Ordering::Relaxed);
    }
    if state.abort.load(Ordering::Relaxed) || state.external_abort.load(Ordering::Relaxed) {
        return 0;
    }

    // Mate distance pruning: tighten window — we can't improve beyond the nearest mate
    alpha = alpha.max(-(MATE_SCORE - ply as i32));
    let beta = beta.min(MATE_SCORE - ply as i32);
    if alpha >= beta {
        return alpha;
    }

    if depth == 0 {
        return quiescence(state, board, alpha, beta, ply);
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
    let in_check = is_in_check(board, stm);
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
            if state.abort.load(Ordering::Relaxed) {
                break;
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
        );
        board.undo_null_move(null_tok);

        if null_score >= beta {
            return null_score; // fail-soft: tighter lower bound than beta
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
    let ext0 = check_ext(board, ply + 1);
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
    );
    board.undo_move(tok);

    if state.abort.load(Ordering::Relaxed) {
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
    if depth >= MIN_SPLIT_DEPTH {
        let nw_abort = Arc::new(AtomicBool::new(false));
        let alpha_for_nw = alpha;

        #[allow(clippy::type_complexity)]
        let work: Vec<(Move, usize, Board, Arc<SearchState>, Arc<AtomicBool>)> = rest
            .iter()
            .enumerate()
            .map(|(i, &m)| (m, i + 1, board.clone(), state.clone(), nw_abort.clone()))
            .collect();

        // Null-window parallel probe (with LMR for quiet late moves)
        let nw_results: Vec<(Move, i32, usize)> = work
            .into_par_iter()
            .filter_map(|(m, idx, mut b, ctx, lab)| {
                if lab.load(Ordering::Relaxed) || ctx.abort.load(Ordering::Relaxed) {
                    return None;
                }
                let reduce = lmr_reduce(&b, m, idx, depth, &killers, tt_mv);
                let tok = b.do_move(m);
                let ext = check_ext(&b, ply + 1);
                let reduce = if ext > 0 { 0 } else { reduce }; // never reduce a checking move
                let probe_depth = depth.saturating_sub(1 + reduce) + ext;
                let s = -alpha_beta(
                    &ctx,
                    &mut b,
                    -alpha_for_nw - 1,
                    -alpha_for_nw,
                    probe_depth,
                    ply + 1,
                    true,
                    Some(m),
                    None,
                );
                b.undo_move(tok);
                Some((m, s, idx))
            })
            .collect();

        // Sequential pass: handle fail-highs, update heuristics, apply history malus
        for (m, nw_score, _idx) in nw_results {
            if state.abort.load(Ordering::Relaxed) {
                break;
            }

            let is_quiet_ybw = m.from.is_some() && !enemy.contains(m.to) && !m.promote;

            let s = if nw_score > alpha {
                // Fail-high: re-search at full depth with full window
                let tok = board.do_move(m);
                let ext = check_ext(board, ply + 1);
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
    } else {
        // Sequential fallback for shallow depths
        let lmp_limit = if !in_check && depth <= 2 {
            LMP_BASE + depth as usize * 3 // depth 1: 8 quiet moves, depth 2: 11 quiet moves
        } else {
            usize::MAX
        };

        let mut quiet_count = 0usize;

        for (i, &m) in rest.iter().enumerate() {
            if state.abort.load(Ordering::Relaxed) {
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

            let reduce = lmr_reduce(board, m, i + 1, depth, &killers, tt_mv);
            let tok = board.do_move(m);
            let ext = check_ext(board, ply + 1);
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

/// Resolve captures until the position is "quiet" before calling evaluate.
/// Uses stand-pat as a lower bound; only searches captures (board moves to enemy squares).
#[allow(clippy::only_used_in_recursion)] // ply passed through for future extensions
fn quiescence(
    state: &Arc<SearchState>,
    board: &mut Board,
    mut alpha: i32,
    beta: i32,
    ply: u32,
) -> i32 {
    state.nodes.fetch_add(1, Ordering::Relaxed);

    if state.abort.load(Ordering::Relaxed) || state.external_abort.load(Ordering::Relaxed) {
        return 0;
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
        // Delta Pruning: if even capturing the highest-value piece cannot improve alpha, skip.
        // Ryu (promoted rook) ≈ 1300cp is the maximum gain from a single capture.
        const DELTA_MARGIN: i32 = 1_300;
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

    // Sort by SEE: biggest gain first
    let mut ordered = moves;
    ordered.sort_by_cached_key(|&m| -see_score(board, m));

    for m in ordered {
        let tok = board.do_move(m);
        let score = -quiescence(state, board, -beta, -alpha, ply + 1);
        board.undo_move(tok);

        if state.abort.load(Ordering::Relaxed) {
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
    if !in_check && ply == 0 {
        const MAX_QCHECKS: usize = 4;
        let mut qcheck_count = 0;
        for m in generate_legal_moves(board) {
            // Skip captures — already handled above
            if m.from.is_some() && board.piece_at(m.to).is_some() {
                continue;
            }
            // Skip moves with negative SEE (losing check attempts)
            if see_score(board, m) < 0 {
                continue;
            }
            // Test if this move gives check
            let tok = board.do_move(m);
            let gives_check = is_in_check(board, board.side_to_move);
            if !gives_check {
                board.undo_move(tok);
                continue;
            }
            let score = -quiescence(state, board, -beta, -alpha, ply + 1);
            board.undo_move(tok);

            if state.abort.load(Ordering::Relaxed) {
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
    pub best_move: Option<Move>,
    pub score: i32,
    pub depth: u32,
    pub nodes: u64,
    pub elapsed: Duration,
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
}

impl SpeculativeSearcher {
    pub fn new(tt: Arc<Tt>, top_n: usize) -> Self {
        SpeculativeSearcher {
            tt,
            top_n,
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

    pub fn search(&self, board: &mut Board, config: SearchConfig) -> SpecSearchInfo {
        self.external_abort.store(false, Ordering::Relaxed);
        let global_abort = Arc::new(AtomicBool::new(false));

        let state = Arc::new(SearchState {
            tt: self.tt.clone(),
            nodes: AtomicU64::new(0),
            abort: AtomicBool::new(false),
            external_abort: self.external_abort.clone(),
            start: Instant::now(),
            time_limit: config.time_limit,
            killers: KillerTable::new(),
            history: HistoryTable::new(),
            countermoves: CountermoveTable::new(),
        });

        let spec_state = Arc::new(SpecState {
            tt: self.tt.clone(),
            abort: global_abort.clone(),
        });

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
                if state.abort.load(Ordering::Relaxed) {
                    break;
                }
                match m {
                    Some(mv) => { depth_pv.push((mv, score)); excluded.push(mv); }
                    None => break,
                }
            }
            let m = depth_pv.first().map(|&(mv, _)| mv);
            let score = depth_pv.first().map(|&(_, s)| s).unwrap_or(NEG_INF);

            let timed_out = state.abort.load(Ordering::Relaxed);

            if timed_out {
                global_abort.store(true, Ordering::Relaxed);
            }

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
            // ponytail: 1.5x extension on instability; tune if LOS drops
            let effective_soft = if best_move != prev_best && depth >= 3 {
                config.soft_limit.map(|s| s.mul_f32(1.5))
            } else {
                config.soft_limit
            };
            if depth >= 2
                && effective_soft.is_some_and(|soft| state.start.elapsed() >= soft)
                && best_move == prev_best
            {
                break;
            }
            prev_best = best_move;
        }

        global_abort.store(true, Ordering::Relaxed);

        SpecSearchInfo {
            best_move,
            score: best_score,
            depth: done_depth,
            nodes: state.nodes.load(Ordering::Relaxed),
            elapsed: state.start.elapsed(),
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
#[inline]
fn score_to_tt(score: i32, ply: u32) -> i32 {
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
#[inline]
fn score_from_tt(stored: i32, ply: u32) -> i32 {
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

/// Static Exchange Evaluation — estimates net material gain from a capture sequence.
///
/// Algorithm (2-ply lookahead):
///   gain_1 = victim_value − attacker_value  (immediate capture)
///   if gain_1 >= 0: safe regardless of recapture → return gain_1
///   else: check if opponent can recapture on the target square
///     no recapture → return victim_value  (free piece!)
///     recapture    → return gain_1        (losing capture, goes last)
///
/// This correctly handles the common "undefended piece" case where a heavy piece
/// captures a lighter one that is actually free (no defender).
#[inline]
fn see_score(board: &mut Board, m: Move) -> i32 {
    // Only board moves can be captures; drops never are
    if m.from.is_none() {
        return 0;
    }

    let Some(cap) = board.piece_at(m.to) else { return 0 };

    let victim_val = PIECE_VALUE[cap.kind.index()];
    // Use the post-move piece value: attacker may promote when landing on m.to.
    let attacker_kind = if m.promote { m.piece_kind.promoted() } else { m.piece_kind };
    let attacker_val = PIECE_VALUE[attacker_kind.index()];
    let gain_1 = victim_val - attacker_val;

    if gain_1 >= 0 {
        // Winning or equal trade — safe even after recapture.
        return gain_1;
    }

    // Potentially losing: only truly losing if opponent can recapture.
    // Check POST-move so that X-ray attackers unblocked by our piece moving are visible.
    let tok = board.do_move(m);
    let opp_can_recapture = generate_legal_captures(board)
        .iter()
        .any(|r| r.to == m.to);
    board.undo_move(tok);

    if opp_can_recapture {
        gain_1
    } else {
        victim_val // No recapture available: free piece
    }
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
fn lmr_reduce(
    board: &Board,
    m: Move,
    move_idx: usize,
    depth: u32,
    killers: &[Option<Move>; 2],
    tt_mv: Option<Move>,
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
    r as u32
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
