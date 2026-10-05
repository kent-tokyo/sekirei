//! SEARCH_V2: the search below the root as one consistent rule set.
//!
//! The main search grew its pruning one switch at a time, and the
//! switches interact: aggressive late-move pruning, deep reverse futility
//! and strong extensions each lost strength there on their own. This module
//! searches the same tree with a single design instead:
//!
//! - its own per-thread move histories (butterfly from-to, continuation
//!   histories one to six plies back, pawn-structure and capture
//!   histories), updated with a larger penalty for failed moves than the
//!   bonus for the cutoff move, and quiet moves ordered by all of them with
//!   safe checks first;
//! - TT cutoffs that depend on the expected node type, a TT move that a
//!   node failing low does not overwrite, and small TT-based ProbCut;
//! - reverse futility to depth 13, null move only at expected cut nodes,
//!   razoring, ProbCut, internal iterative reduction;
//! - move-count, futility, continuation-history and exchange pruning over
//!   the reduced depth (move-count pruning only at non-PV nodes not in
//!   check unless `V2_LMP_NODES` says otherwise);
//! - singular, double and triple extensions, negative extensions and
//!   multi-cut;
//! - reductions in 1/1024 ply adjusted by node type, the TT move and the
//!   move's history, with deeper or shallower re-searches.
//!
//! The root, the quiescence search, the evaluation and its correction
//! history, the transposition table and the repetition rules are the main
//! search's. All constants here are this module's own.

use std::cell::RefCell;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use super::heuristics::{CorrKeys, HistoryTable};
use super::params as p;
use super::{
    MATE_SCORE, SearchHistory, SearchState, TtEntry, current_move_at, cut_at, evaluate_for_search,
    play, probe_tt_for_search_pv, quiescence, repetition_score, score_from_tt, score_to_tt,
    store_tt_for_search_pv, useless_non_promotion,
};
use crate::board::Board;
use crate::color::Color;
use crate::eval::PIECE_VALUE;
use crate::movegen::{CheckSquares, MoveBuffer, is_in_check, mate_in_one, see_exchange};
use crate::mv::Move;
use crate::piece::PieceKind;
use crate::tt::Bound;

/// Scores at or beyond this magnitude are mates.
const MATE_BOUND: i32 = MATE_SCORE - 1000;
const INF: i32 = MATE_SCORE + 1;
const NO_EVAL: i32 = i32::MIN;
const NO_PT: u32 = u32::MAX;
/// Frames below ply 0 that continuation histories may look at.
const OFF: usize = 8;
const MAX_PLY: usize = 240;

const SQ: usize = 81;
/// Piece-to keys: color × kind after the move (promoted kinds included) × to.
const PT_NB: usize = 2 * PieceKind::COUNT * SQ;
/// From-to keys: color × (from square, or the dropped kind) × to.
const FT_NB: usize = 2 * (SQ + 7) * SQ;
const PAWN_NB: usize = 512;
const LOW_PLIES: usize = 4;

const MAIN_LIM: i32 = 7000;
const CONT_LIM: i32 = 16000;
const CAPT_LIM: i32 = 10000;
const PAWN_LIM: i32 = 8000;

#[inline]
fn is_decisive(v: i32) -> bool {
    v.abs() >= MATE_BOUND
}

#[inline]
fn pt_key(color: Color, m: Move) -> u32 {
    let kind = if m.promote {
        m.piece_kind.promoted()
    } else {
        m.piece_kind
    };
    ((color.index() * PieceKind::COUNT + kind.index()) * SQ + m.to.index() as usize) as u32
}

#[inline]
fn ft_key(color: Color, m: Move) -> usize {
    let from = m
        .from
        .map_or(SQ + m.piece_kind.index(), |f| f.index() as usize);
    (color.index() * (SQ + 7) + from) * SQ + m.to.index() as usize
}

/// Tactical moves: captures and pawn promotions (ordered and pruned as
/// captures; everything else, drops included, is quiet).
#[inline]
fn is_tactical(board: &Board, m: Move) -> bool {
    (m.from.is_some() && board.piece_at(m.to).is_some())
        || (m.promote && m.piece_kind == PieceKind::Fu)
}

/// Captured kind index for the capture history (7: a pawn promotion).
#[inline]
fn victim_index(board: &Board, m: Move) -> usize {
    board
        .piece_at(m.to)
        .filter(|_| m.from.is_some())
        .map_or(7, |v| v.kind.unpromoted().index().min(6))
}

/// Material swing of capturing on `m.to`: the captured piece leaves the
/// board and its unpromoted kind goes to the capturer's hand.
#[inline]
fn victim_value(board: &Board, m: Move) -> i32 {
    board
        .piece_at(m.to)
        .filter(|_| m.from.is_some())
        .map_or(0, |v| {
            PIECE_VALUE[v.kind.index()] + PIECE_VALUE[v.kind.unpromoted().index()]
        })
}

/// Whether `m` captures a piece worth at least the capturing piece (its
/// exchange is then at least zero).
#[inline]
fn captures_up(board: &Board, m: Move) -> bool {
    m.from.is_some()
        && board
            .piece_at(m.to)
            .is_some_and(|v| PIECE_VALUE[v.kind.index()] >= PIECE_VALUE[m.piece_kind.index()])
}

/// Static exchange evaluation in evaluation units. `see_exchange` counts a
/// capture once (the piece leaving the board); the evaluation also sees it
/// arrive in the capturer's hand, about twice the swing.
#[inline]
fn see2(board: &Board, m: Move) -> i32 {
    2 * see_exchange(board, m)
}

#[inline]
fn gravity(e: &mut i16, bonus: i32, limit: i32) {
    let b = bonus.clamp(-limit, limit);
    let v = i32::from(*e);
    *e = (v + b - v * b.abs() / limit).clamp(-limit, limit) as i16;
}

#[inline]
fn stat_bonus(depth: i32) -> i32 {
    (p::V2_BONUS_LIN() * depth - 70).clamp(0, p::V2_BONUS_MAX())
}

#[inline]
fn stat_malus(depth: i32) -> i32 {
    (p::V2_MALUS_LIN() * depth - 120).clamp(0, p::V2_MALUS_MAX())
}

/// `ln(x)` for x below 256.
static LN_TABLE: std::sync::OnceLock<Vec<f32>> = std::sync::OnceLock::new();

/// `470 ln(d) ln(n)` for depths below 64 and move numbers below 256 (row
/// `d`, column `n`).
static RED_TABLE: std::sync::OnceLock<Vec<i32>> = std::sync::OnceLock::new();

#[inline]
fn ln_table() -> &'static [f32] {
    LN_TABLE.get_or_init(|| (0..256).map(|x: i32| (x.max(1) as f32).ln()).collect())
}

/// Base reduction of the `n`-th move at `depth`, in 1/1024 ply.
#[inline]
fn base_reduction(improving: bool, depth: i32, n: u32) -> i32 {
    let d = depth.clamp(1, 255) as usize;
    let n = (n as usize).clamp(1, 255);
    let scale = if d < 64 {
        RED_TABLE.get_or_init(|| {
            let ln = ln_table();
            (0..64 * 256)
                .map(|k| (470.0 * ln[(k / 256).max(1)] * ln[(k % 256).max(1)]) as i32)
                .collect()
        })[d * 256 + n]
    } else {
        let ln = ln_table();
        (470.0 * ln[d] * ln[n]) as i32
    };
    scale + 1090 + if improving { 0 } else { scale * 216 / 512 }
}

/// Move-count pruning limit at `depth`.
#[inline]
fn lmp_limit(depth: i32, improving: bool) -> u32 {
    ((p::V2_LMP_BASE() + depth * depth) / (2 - i32::from(improving))) as u32
}

/// A move-list entry: ordering key, move, flags (bits 0-1: gives check, 0
/// unknown, 1 no, 2 yes; bit 2: tactical) and cached exchange value
/// (`i32::MIN`: not computed).
type Entry = (i32, Move, u8, i32);

/// A node's move lists: ordered moves, unscored quiet moves, losing
/// tactical moves.
type Lists = (Vec<Entry>, Vec<Move>, Vec<Entry>);

#[derive(Clone, Copy)]
struct Frame {
    /// Corrected static evaluation (NO_EVAL in check or unknown).
    eval: i32,
    /// The move searched from this ply (None: a null move or nothing).
    mv: Option<Move>,
    /// Its piece-to key (NO_PT for a null move or nothing).
    pt: u32,
    capture: bool,
    in_check: bool,
    tt_pv: bool,
    cutoff_cnt: u32,
    move_count: u32,
    excluded: Option<Move>,
    /// Plies by which the move searched from this ply is being reduced
    /// (V2_HINDSIGHT; 0 outside a reduced search).
    reduction: i32,
}

impl Frame {
    const EMPTY: Frame = Frame {
        eval: NO_EVAL,
        mv: None,
        pt: NO_PT,
        capture: false,
        in_check: false,
        tt_pv: false,
        cutoff_cnt: 0,
        move_count: 0,
        excluded: None,
        reduction: 0,
    };
}

struct Data {
    main: Vec<i16>,
    cont: Vec<i16>,
    capt: Vec<i16>,
    pawn: Vec<i16>,
    low: Vec<i16>,
    ss: Vec<Frame>,
    lists: Vec<Vec<Entry>>,
    /// Per ply: quiet moves not yet scored, and losing tactical moves.
    quiet_lists: Vec<Vec<Move>>,
    bad_lists: Vec<Vec<Entry>>,
    nmp_min_ply: u32,
    epoch: u64,
    /// The TT's search count when these tables last saw a search.
    search: u64,
}

impl Data {
    fn new() -> Self {
        Data {
            main: vec![0; FT_NB],
            cont: vec![0; PT_NB * PT_NB],
            capt: vec![0; PT_NB * 8],
            pawn: vec![0; PAWN_NB * PT_NB],
            low: vec![0; LOW_PLIES * FT_NB],
            ss: vec![Frame::EMPTY; MAX_PLY + OFF + 8],
            lists: (0..MAX_PLY + 8).map(|_| Vec::with_capacity(128)).collect(),
            quiet_lists: (0..MAX_PLY + 8).map(|_| Vec::with_capacity(128)).collect(),
            bad_lists: (0..MAX_PLY + 8).map(|_| Vec::with_capacity(16)).collect(),
            nmp_min_ply: 0,
            epoch: u64::MAX,
            search: u64::MAX,
        }
    }

    /// Scale every history by `num / 16` (a new search, V2_KEEP 2).
    fn age(&mut self, num: i32) {
        for t in [
            &mut self.main,
            &mut self.cont,
            &mut self.capt,
            &mut self.pawn,
            &mut self.low,
        ] {
            for e in t.iter_mut() {
                *e = (i32::from(*e) * num / 16) as i16;
            }
        }
    }

    fn clear(&mut self) {
        for t in [
            &mut self.main,
            &mut self.cont,
            &mut self.capt,
            &mut self.pawn,
            &mut self.low,
        ] {
            t.fill(0);
        }
    }

    /// Continuation-history sum of `pt` against the moves 1, 2, 3, 4 and 6
    /// plies before frame `i`.
    #[inline]
    fn cont_score(&self, i: usize, pt: u32) -> i32 {
        let mut s = 0;
        for k in [1usize, 2, 3, 4, 6] {
            let prev = self.ss[i - k].pt;
            if prev != NO_PT {
                s += i32::from(self.cont[prev as usize * PT_NB + pt as usize]);
            }
        }
        s
    }

    #[inline]
    fn cont_at(&self, i: usize, k: usize, pt: u32) -> i32 {
        let prev = self.ss[i - k].pt;
        if prev == NO_PT {
            0
        } else {
            i32::from(self.cont[prev as usize * PT_NB + pt as usize])
        }
    }

    /// Update the continuation histories of `pt` played at frame `i`.
    fn update_cont(&mut self, i: usize, pt: u32, bonus: i32) {
        let in_check = self.ss[i].in_check;
        for (k, w) in [(1usize, 1024), (2, 1024), (3, 512), (4, 512), (6, 512)] {
            if in_check && k > 2 {
                break;
            }
            let prev = self.ss[i - k].pt;
            if prev != NO_PT {
                gravity(
                    &mut self.cont[prev as usize * PT_NB + pt as usize],
                    bonus * w / 1024,
                    CONT_LIM,
                );
            }
        }
    }

    fn update_quiet(
        &mut self,
        i: usize,
        ply: usize,
        stm: Color,
        m: Move,
        pawn_bucket: usize,
        bonus: i32,
    ) {
        let ft = ft_key(stm, m);
        gravity(&mut self.main[ft], bonus, MAIN_LIM);
        if ply < LOW_PLIES {
            gravity(&mut self.low[ply * FT_NB + ft], bonus, MAIN_LIM);
        }
        let pt = pt_key(stm, m);
        self.update_cont(i, pt, bonus);
        gravity(
            &mut self.pawn[pawn_bucket * PT_NB + pt as usize],
            bonus / 2,
            PAWN_LIM,
        );
    }

    #[inline]
    fn capt_score(&self, board: &Board, stm: Color, m: Move) -> i32 {
        i32::from(self.capt[pt_key(stm, m) as usize * 8 + victim_index(board, m)])
    }

    fn update_capture(&mut self, board: &Board, stm: Color, m: Move, bonus: i32) {
        let idx = pt_key(stm, m) as usize * 8 + victim_index(board, m);
        gravity(&mut self.capt[idx], bonus, CAPT_LIM);
    }
}

/// Tables of finished threads. A USI search runs on a new thread for every
/// move; the pool hands its tables (and what they learned) to the next one.
/// The lock is taken only when a thread starts or ends, never in the search.
#[allow(clippy::vec_box)]
static POOL: std::sync::Mutex<Vec<Box<Data>>> = std::sync::Mutex::new(Vec::new());

/// A thread's tables, returned to `POOL` when the thread ends.
struct Slot(Option<Box<Data>>);

impl Drop for Slot {
    fn drop(&mut self) {
        if let Some(d) = self.0.take()
            && let Ok(mut pool) = POOL.lock()
        {
            pool.push(d);
        }
    }
}

thread_local! {
    static DATA: RefCell<Slot> = const { RefCell::new(Slot(None)) };
}

#[inline]
fn diag(state: &SearchState, depth: i32, k: usize) {
    if let Some(d) = state.counters() {
        d.at_depth(depth.max(0) as u32, k);
    }
}

/// Run `f` on this thread's tables, taking them from the pool on first use
/// and clearing or aging them when a new game or a new search starts.
fn with_data<R>(state: &SearchState, f: impl FnOnce(&mut Data) -> R) -> R {
    DATA.with(|cell| {
        let mut slot = cell.borrow_mut();
        let d = slot.0.get_or_insert_with(|| {
            POOL.lock()
                .ok()
                .and_then(|mut pool| pool.pop())
                .unwrap_or_else(|| Box::new(Data::new()))
        });
        let epoch = state.history.epoch();
        if d.epoch != epoch {
            d.clear();
            d.epoch = epoch;
        }
        let search_no = state.tt.search_count();
        if d.search != search_no {
            d.search = search_no;
            match p::V2_KEEP() {
                0 => d.clear(),
                1 => {}
                k => d.age(16 / k),
            }
            for t in d.low.iter_mut() {
                *t = 0;
            }
        }
        f(d)
    })
}

/// Root move order from these tables (V2_ROOT): the TT move, winning
/// tactical moves, quiet moves by history (root-ply history weighted
/// most; safe checks first), losing tactical moves.
pub(super) fn order_root(state: &SearchState, board: &Board, moves: &mut [Move], tt: Option<Move>) {
    with_data(state, |d| {
        let stm = board.side_to_move;
        let bucket = HistoryTable::pawn_bucket(board);
        let checks = CheckSquares::new(board);
        let mut keyed: Vec<(i32, Move)> = moves
            .iter()
            .map(|&m| {
                let k = if Some(m) == tt {
                    i32::MAX
                } else if is_tactical(board, m) {
                    let s = p::V2_MVV_W() * victim_value(board, m) + d.capt_score(board, stm, m);
                    if see2(board, m) >= -s / p::V2_GOODCAP_DIV().max(1) {
                        1_000_000_000 + s
                    } else {
                        -1_000_000_000 + s
                    }
                } else {
                    let ft = ft_key(stm, m);
                    let pt = pt_key(stm, m) as usize;
                    let mut s = 2 * i32::from(d.main[ft])
                        + 2 * i32::from(d.pawn[bucket * PT_NB + pt])
                        + p::V2_LOWPLY_W() * i32::from(d.low[ft]);
                    if checks.gives_direct_check(m)
                        && p::V2_CHECK_BONUS() > 0
                        && see2(board, m) >= -p::V2_CHECK_SEE()
                    {
                        s += p::V2_CHECK_BONUS();
                    }
                    s
                };
                (k, m)
            })
            .collect();
        keyed.sort_by_key(|&(k, _)| std::cmp::Reverse(k));
        for (slot, (_, m)) in moves.iter_mut().zip(keyed) {
            *slot = m;
        }
    });
}

/// Root learning (V2_ROOT): the move that raised alpha last gets the
/// history bonus of `depth`, the other quiet moves searched the malus.
pub(super) fn root_learn(
    state: &SearchState,
    board: &Board,
    best: Move,
    searched: &[Move],
    depth: u32,
) {
    with_data(state, |d| {
        let stm = board.side_to_move;
        let bucket = HistoryTable::pawn_bucket(board);
        let i = OFF;
        for k in 1..=OFF {
            d.ss[i - k] = Frame::EMPTY;
        }
        d.ss[i] = Frame::EMPTY;
        let depth = depth as i32;
        let (bonus, malus) = (stat_bonus(depth), stat_malus(depth));
        if is_tactical(board, best) {
            d.update_capture(board, stm, best, bonus);
        } else {
            d.update_quiet(i, 0, stm, best, bucket, bonus);
        }
        for &m in searched.iter().filter(|&&m| m != best).take(32) {
            if is_tactical(board, m) {
                d.update_capture(board, stm, m, -malus);
            } else if !is_tactical(board, best) {
                d.update_quiet(i, 0, stm, m, bucket, -malus);
            }
        }
    });
}

/// Entry from the main search (the children of the root and below).
#[allow(clippy::too_many_arguments)]
pub(super) fn entry(
    state: &Arc<SearchState>,
    board: &mut Board,
    alpha: i32,
    beta: i32,
    depth: u32,
    ply: u32,
    known_in_check: Option<bool>,
    history: &SearchHistory<'_>,
) -> i32 {
    with_data(state, |d| {
        let stm = board.side_to_move;
        let i = ply as usize + OFF;
        // The path above the entry ply as the main search's stack records it.
        for k in 1..=OFF {
            let f = &mut d.ss[i - k];
            *f = Frame::EMPTY;
            if (k as u32) <= ply
                && let Some(m) = current_move_at(ply - k as u32)
            {
                let mover = if k % 2 == 1 { stm.flip() } else { stm };
                f.mv = Some(m);
                f.pt = pt_key(mover, m);
                f.move_count = 2;
            }
        }
        d.nmp_min_ply = 0;
        let in_check = known_in_check.unwrap_or_else(|| is_in_check(board, stm));
        d.ss[i] = Frame {
            in_check,
            ..Frame::EMPTY
        };
        let pv = beta - alpha > 1;
        let cut_node = !pv && cut_at(ply);
        search(
            d,
            state,
            board,
            alpha,
            beta,
            depth as i32,
            ply,
            cut_node,
            pv,
            history,
        )
    })
}

#[allow(clippy::too_many_arguments)]
fn qs(
    state: &Arc<SearchState>,
    board: &mut Board,
    alpha: i32,
    beta: i32,
    ply: u32,
    in_check: bool,
    history: &SearchHistory<'_>,
) -> i32 {
    if state.budget.should_abort() {
        return 0;
    }
    let alpha = alpha.max(-(MATE_SCORE - ply as i32));
    let beta = beta.min(MATE_SCORE - ply as i32);
    if alpha >= beta {
        return alpha;
    }
    quiescence(state, board, alpha, beta, ply, 0, Some(in_check), history)
}

#[allow(clippy::too_many_arguments)]
fn search(
    d: &mut Data,
    state: &Arc<SearchState>,
    board: &mut Board,
    alpha: i32,
    beta: i32,
    depth: i32,
    ply: u32,
    cut_node: bool,
    pv: bool,
    history: &SearchHistory<'_>,
) -> i32 {
    let i = ply as usize + OFF;
    if depth <= 0 {
        if let Some(c) = state.counters() {
            c.exit(0);
        }
        let in_check = d.ss[i].in_check;
        return qs(state, board, alpha, beta, ply, in_check, history);
    }
    let p = ply as usize;
    let mut lists = (
        std::mem::take(&mut d.lists[p]),
        std::mem::take(&mut d.quiet_lists[p]),
        std::mem::take(&mut d.bad_lists[p]),
    );
    lists.0.clear();
    lists.1.clear();
    lists.2.clear();
    let v = node(
        d, &mut lists, state, board, alpha, beta, depth, ply, cut_node, pv, history,
    );
    d.lists[p] = lists.0;
    d.quiet_lists[p] = lists.1;
    d.bad_lists[p] = lists.2;
    v
}

#[allow(clippy::too_many_arguments)]
fn node(
    d: &mut Data,
    lists: &mut Lists,
    state: &Arc<SearchState>,
    board: &mut Board,
    mut alpha: i32,
    mut beta: i32,
    mut depth: i32,
    ply: u32,
    cut_node: bool,
    pv: bool,
    history: &SearchHistory<'_>,
) -> i32 {
    let i = ply as usize + OFF;
    if let Some(c) = state.counters() {
        c.alpha_beta_calls.fetch_add(1, Ordering::Relaxed);
        if pv {
            c.pv_calls.fetch_add(1, Ordering::Relaxed);
        }
    }
    if let Some(outcome) = history.outcome_at_current_position() {
        return repetition_score(outcome, board.side_to_move, ply);
    }
    if state.budget.tick() {
        return 0;
    }
    let entry_depth = depth;
    diag(state, entry_depth, 0);
    let in_check = d.ss[i].in_check;
    let excluded = d.ss[i].excluded;
    let stm = board.side_to_move;
    if ply as usize >= MAX_PLY {
        return if in_check {
            0
        } else {
            evaluate_for_search(state, board)
        };
    }

    // Mate distance pruning.
    alpha = alpha.max(-(MATE_SCORE - ply as i32));
    beta = beta.min(MATE_SCORE - ply as i32);
    if alpha >= beta {
        return alpha;
    }

    d.ss[i + 1].excluded = None;
    d.ss[i + 2].cutoff_cnt = 0;
    d.ss[i].move_count = 0;
    let prior = d.ss[i - 1];
    let hash = board.hash();

    // Transposition table.
    let entry = if excluded.is_none() {
        probe_tt_for_search_pv(state, hash)
    } else {
        None
    };
    let tt_hit = entry.is_some();
    if let Some(c) = state.counters() {
        c.record_tt_probe(
            depth.max(0) as u32,
            tt_hit,
            entry.map_or(0, |(e, _)| e.depth),
        );
    }
    let (tt_move, tt_score, tt_depth, tt_bound, tt_pv_flag) = match entry {
        Some((e, pvf)) => (
            e.mv,
            score_from_tt(e.score, ply),
            i32::from(e.depth),
            Some(e.bound),
            pvf,
        ),
        None => (None, 0, -1, None, false),
    };
    let has_lower = matches!(tt_bound, Some(Bound::Lower | Bound::Exact));
    let has_upper = matches!(tt_bound, Some(Bound::Upper | Bound::Exact));
    let tt_pv = if excluded.is_some() {
        d.ss[i].tt_pv
    } else {
        pv || (tt_hit && tt_pv_flag)
    };
    d.ss[i].tt_pv = tt_pv;
    let tt_capture = tt_move.is_some_and(|m| is_tactical(board, m));

    if !pv
        && excluded.is_none()
        && tt_hit
        && tt_depth > depth - i32::from(tt_score <= beta)
        && (if tt_score >= beta {
            has_lower
        } else {
            has_upper
        })
        && (cut_node == (tt_score >= beta) || depth > 5)
    {
        // A quiet TT move that cuts teaches the histories.
        if tt_score >= beta
            && let Some(m) = tt_move
            && !is_tactical(board, m)
            && board.piece_at(m.to).is_none()
        {
            let bucket = HistoryTable::pawn_bucket(board);
            d.update_quiet(i, ply as usize, stm, m, bucket, stat_bonus(depth) * 3 / 4);
            if prior.mv.is_some() && prior.move_count <= 2 && !prior.capture {
                let pt = prior.pt;
                d.update_cont(i - 1, pt, -stat_malus(depth + 1) / 2);
            }
        }
        if let Some(c) = state.counters() {
            c.exit(1);
        }
        diag(state, entry_depth, 1);
        return tt_score;
    }

    // A mate in one ends the node.
    if !in_check && excluded.is_none() && !tt_hit && mate_in_one(board).is_some() {
        diag(state, entry_depth, 6);
        return MATE_SCORE - (ply as i32 + 1);
    }

    // Static evaluation.
    let mut corr: Option<(CorrKeys, i32)> = None;
    let (static_eval, eval) = if in_check {
        (NO_EVAL, NO_EVAL)
    } else {
        let raw = evaluate_for_search(state, board);
        let keys = CorrKeys::of(board, stm);
        let se =
            (raw + state.history.correction(keys)).clamp(-(MATE_BOUND - 1000), MATE_BOUND - 1000);
        corr = Some((keys, se));
        let mut ev = se;
        if tt_hit && !is_decisive(tt_score) && (if tt_score > se { has_lower } else { has_upper }) {
            ev = tt_score;
        }
        (se, ev)
    };
    d.ss[i].eval = static_eval;
    if let Some(c) = state.counters()
        && excluded.is_none()
    {
        c.record_v2_node(
            depth.max(0) as u32,
            tt_pv,
            static_eval != NO_EVAL && eval >= beta,
            in_check,
        );
    }
    let prev2 = d.ss[i - 2].eval;
    let improving = static_eval != NO_EVAL && prev2 != NO_EVAL && static_eval > prev2;
    let opp_worsening =
        static_eval != NO_EVAL && prior.eval != NO_EVAL && static_eval > -prior.eval;

    // The opponent's quiet move learns from how the evaluation moved.
    if static_eval != NO_EVAL
        && prior.eval != NO_EVAL
        && let Some(pm) = prior.mv
        && !prior.capture
        && !prior.in_check
        && excluded.is_none()
    {
        let bonus = (-8 * (prior.eval + static_eval)).clamp(-1500, 1300);
        gravity(&mut d.main[ft_key(stm.flip(), pm)], bonus, MAIN_LIM);
    }

    // V2_HINDSIGHT: a node reached by a strongly reduced move goes one ply
    // deeper unless the opponent's position got worse; one reduced while both
    // sides' evaluations agree it is quiet goes one ply shallower.
    if p::V2_HINDSIGHT() != 0 && static_eval != NO_EVAL && excluded.is_none() {
        if prior.reduction >= 3 && !opp_worsening {
            depth += 1;
        }
        if prior.reduction >= 2
            && depth >= 2
            && prior.eval != NO_EVAL
            && static_eval + prior.eval > p::V2_HS_MARGIN()
        {
            depth -= 1;
        }
    }

    // Internal iterative reduction.
    if tt_move.is_none() && ((pv && depth >= 4) || (cut_node && depth >= 7)) {
        depth -= 1;
    }

    if !in_check && excluded.is_none() {
        // Razoring.
        if !pv && eval < alpha - 450 - 280 * depth * depth {
            let v = qs(state, board, alpha - 1, alpha, ply, false, history);
            if v < alpha && !is_decisive(v) {
                if let Some(c) = state.counters() {
                    c.exit(4);
                }
                diag(state, entry_depth, 2);
                return v;
            }
        }

        // Reverse futility pruning.
        let mult = p::V2_RFP_MULT() - 20 * i32::from(cut_node && !tt_hit);
        let margin = mult * depth
            - if improving { 2 * mult } else { 0 }
            - if opp_worsening { mult / 3 } else { 0 };
        if !tt_pv
            && depth <= p::V2_RFP_DEPTH()
            && eval - margin >= beta
            && eval >= beta
            && !is_decisive(eval)
            && !is_decisive(beta)
            && (tt_move.is_none() || tt_capture)
        {
            if let Some(c) = state.counters() {
                c.exit(3);
            }
            diag(state, entry_depth, 3);
            return beta + (eval - beta) / 3;
        }

        // Null move at expected cut nodes (V2_NMP 1: at every non-PV node).
        if (cut_node || (p::V2_NMP() == 1 && !pv))
            && prior.mv.is_some()
            && eval >= beta
            && static_eval >= beta - 20 * depth + 380
            && ply >= d.nmp_min_ply
            && !is_decisive(beta)
        {
            let r = if p::V2_NMP() == 1 {
                3 + depth / 4
            } else {
                ((eval - beta) / p::V2_NMP_DIV().max(1)).min(6) + depth / 3 + 5
            };
            d.ss[i].mv = None;
            d.ss[i].pt = NO_PT;
            d.ss[i].capture = false;
            d.ss[i + 1].in_check = false;
            let tok = board.do_null_move();
            let null = -search(
                d,
                state,
                board,
                -beta,
                -beta + 1,
                depth - r,
                ply + 1,
                false,
                false,
                history,
            );
            board.undo_null_move(tok);
            if state.budget.should_abort() {
                return 0;
            }
            if null >= beta && !is_decisive(null) {
                let verify_from = if p::V2_NMP() == 1 { 6 } else { 16 };
                if d.nmp_min_ply != 0 || depth < verify_from {
                    if let Some(c) = state.counters() {
                        c.exit(6);
                    }
                    diag(state, entry_depth, 4);
                    return null;
                }
                d.nmp_min_ply = ply + 3 * (depth - r).max(0) as u32 / 4;
                let v = search(
                    d,
                    state,
                    board,
                    beta - 1,
                    beta,
                    depth - r,
                    ply,
                    false,
                    false,
                    history,
                );
                d.nmp_min_ply = 0;
                if v >= beta {
                    diag(state, entry_depth, 4);
                    return null;
                }
            }
        }
    }

    // Move generation (needed for ProbCut as well).
    let mut buf = MoveBuffer::legal_with_in_check(board, in_check);
    if buf.is_empty() {
        return if excluded.is_some() {
            alpha
        } else {
            -(MATE_SCORE - ply as i32)
        };
    }
    if p::SKIP_NONPROMO() != 0 {
        buf.as_mut_list()
            .retain(|m| !useless_non_promotion(*m, stm));
    }
    let moves = buf.as_slice();
    let tt_move = tt_move.filter(|t| moves.contains(t));

    // ProbCut.
    let pc_beta = beta + 190 - 60 * i32::from(improving);
    if !pv
        && !in_check
        && excluded.is_none()
        && depth >= 5
        && !is_decisive(beta)
        && !(tt_hit && tt_depth >= depth - 3 && tt_score < pc_beta)
        && static_eval != NO_EVAL
    {
        // The losing-tactical list is free until the move loop: use it here.
        let caps = &mut lists.2;
        caps.clear();
        for &m in moves {
            if is_tactical(board, m) {
                let k = p::V2_MVV_W() * victim_value(board, m) + d.capt_score(board, stm, m);
                caps.push((k, m, 0, i32::MIN));
            }
        }
        caps.sort_unstable_by_key(|&(k, _, _, _)| std::cmp::Reverse(k));
        for &(_, m, _, _) in caps.iter() {
            if see2(board, m) < pc_beta - static_eval {
                continue;
            }
            d.ss[i].mv = Some(m);
            d.ss[i].pt = pt_key(stm, m);
            d.ss[i].capture = true;
            let (tok, child_check, child_history) = play(board, m, history);
            d.ss[i + 1].in_check = child_check;
            let mut v = -qs(
                state,
                board,
                -pc_beta,
                -pc_beta + 1,
                ply + 1,
                child_check,
                &child_history,
            );
            if v >= pc_beta {
                v = -search(
                    d,
                    state,
                    board,
                    -pc_beta,
                    -pc_beta + 1,
                    depth - 4,
                    ply + 1,
                    !cut_node,
                    false,
                    &child_history,
                );
            }
            board.undo_move_for_search(tok);
            if state.budget.should_abort() {
                return 0;
            }
            if v >= pc_beta {
                store_tt_for_search_pv(
                    state,
                    hash,
                    TtEntry {
                        score: score_to_tt(v, ply),
                        depth: (depth - 3).clamp(0, 255) as u8,
                        bound: Bound::Lower,
                        mv: Some(m),
                    },
                    tt_pv,
                );
                if let Some(c) = state.counters() {
                    c.exit(5);
                }
                diag(state, entry_depth, 5);
                return if is_decisive(v) {
                    v
                } else {
                    v - (pc_beta - beta)
                };
            }
        }
        lists.2.clear();
    }

    // Small ProbCut on a TT lower bound well above beta.
    let pc2 = beta + 400;
    if excluded.is_none()
        && has_lower
        && tt_depth >= depth - 4
        && tt_score >= pc2
        && !is_decisive(beta)
        && !is_decisive(tt_score)
    {
        diag(state, entry_depth, 5);
        return pc2;
    }

    let checks = CheckSquares::new(board);

    // Order in stages: the TT move, winning tactical moves, quiet moves
    // (scored only when the loop reaches them), losing tactical moves.
    let (list, quiet_list, bad_list) = lists;
    let pawn_bucket = HistoryTable::pawn_bucket(board);
    let low_ply = (ply as usize) < LOW_PLIES;
    let mut first_sorted = 0;
    if let Some(t) = tt_move {
        list.push((
            i32::MAX,
            t,
            if is_tactical(board, t) { 4 } else { 0 },
            i32::MIN,
        ));
        first_sorted = 1;
    }
    for &m in moves {
        if Some(m) == tt_move {
            continue;
        }
        if is_tactical(board, m) {
            let s = p::V2_MVV_W() * victim_value(board, m) + d.capt_score(board, stm, m);
            // A capture of a piece worth at least the capturer cannot lose
            // the exchange: with a non-negative key it is a winning move
            // without computing the exchange (computed later only if needed).
            let se = if s >= 0 && captures_up(board, m) {
                i32::MIN
            } else {
                see2(board, m)
            };
            if se == i32::MIN || se >= -s / p::V2_GOODCAP_DIV().max(1) {
                list.push((s, m, 4, se));
            } else {
                bad_list.push((s, m, 4, se));
            }
        } else {
            quiet_list.push(m);
        }
    }
    drop(buf);
    list[first_sorted..].sort_unstable_by_key(|&(k, _, _, _)| std::cmp::Reverse(k));
    bad_list.sort_unstable_by_key(|&(k, _, _, _)| std::cmp::Reverse(k));
    diag(state, entry_depth, 7);

    let mut best_value = -INF;
    let mut best_move: Option<Move> = None;
    let mut move_count: u32 = 0;
    let mut quiets: [Option<Move>; 32] = [None; 32];
    let mut nq = 0usize;
    let mut caps_tried: [Option<Move>; 32] = [None; 32];
    let mut nc = 0usize;
    let mut skip_quiets = false;
    let lmp_nodes = p::V2_LMP_NODES();
    let lmp_allowed = (!pv || lmp_nodes & 1 != 0) && (!in_check || lmp_nodes & 2 != 0);

    let mut stage = 0;
    let mut idx = 0;
    // The quiet moves occupy list[quiet_start..quiet_end] once scored.
    let (mut quiet_start, mut quiet_end) = (0, 0);
    // V2_SORT 1: the first quiet moves are picked one at a time (best
    // first, earlier on ties), the rest sorted stably when more are needed.
    let lazy_sort = p::V2_SORT() != 0;
    let (mut picks, mut tail_sorted) = (0u32, false);
    loop {
        if idx < quiet_end {
            if skip_quiets {
                // Every remaining quiet move would be counted and skipped.
                let n = if excluded.is_some() {
                    list[idx..quiet_end]
                        .iter()
                        .filter(|e| Some(e.1) != excluded)
                        .count()
                } else {
                    quiet_end - idx
                };
                move_count += n as u32;
                d.ss[i].move_count = move_count;
                idx = quiet_end;
                continue;
            }
            if lazy_sort && idx >= quiet_start && !tail_sorted {
                if picks < 4 {
                    let mut j = idx;
                    for k in idx + 1..quiet_end {
                        if list[k].0 > list[j].0 {
                            j = k;
                        }
                    }
                    list[idx..=j].rotate_right(1);
                    picks += 1;
                } else {
                    list[idx..quiet_end].sort_by_key(|&(k, _, _, _)| std::cmp::Reverse(k));
                    tail_sorted = true;
                }
            }
        }
        if idx == list.len() {
            match stage {
                0 => {
                    stage = 1;
                    if !skip_quiets {
                        let start = list.len();
                        for &m in quiet_list.iter() {
                            let ft = ft_key(stm, m);
                            let pt = pt_key(stm, m);
                            let mut s = 2 * i32::from(d.main[ft])
                                + 2 * i32::from(d.pawn[pawn_bucket * PT_NB + pt as usize])
                                + d.cont_score(i, pt);
                            if low_ply {
                                s += p::V2_LOWPLY_W() * i32::from(d.low[ply as usize * FT_NB + ft])
                                    / (1 + ply as i32);
                            }
                            let check = checks.gives_direct_check(m);
                            let se = if check { see2(board, m) } else { i32::MIN };
                            if check && p::V2_CHECK_BONUS() > 0 && se >= -p::V2_CHECK_SEE() {
                                s += p::V2_CHECK_BONUS();
                            }
                            list.push((s, m, 1 + u8::from(check), se));
                        }
                        if !lazy_sort {
                            list[start..]
                                .sort_unstable_by_key(|&(k, _, _, _)| std::cmp::Reverse(k));
                        }
                        quiet_start = start;
                        quiet_end = list.len();
                    }
                    continue;
                }
                1 => {
                    stage = 2;
                    list.extend_from_slice(bad_list);
                    continue;
                }
                _ => break,
            }
        }
        let (_, m, check_flag, see_cached) = list[idx];
        let see_of = |board: &Board| {
            if see_cached == i32::MIN {
                see2(board, m)
            } else {
                see_cached
            }
        };
        idx += 1;
        if Some(m) == excluded {
            continue;
        }
        move_count += 1;
        d.ss[i].move_count = move_count;
        let tactical = check_flag & 4 != 0;
        // Move-count pruning first: skipped quiet moves (checks included)
        // need nothing else.
        if best_value > -MATE_BOUND {
            if lmp_allowed && move_count >= lmp_limit(depth, improving) {
                skip_quiets = true;
            }
            if !tactical && skip_quiets {
                continue;
            }
        }
        let capture = m.from.is_some() && board.piece_at(m.to).is_some();
        let pt = pt_key(stm, m);
        let ft = ft_key(stm, m);
        let gives_check = match check_flag & 3 {
            0 => checks.gives_direct_check(m),
            f => f == 2,
        };
        let mut new_depth = depth - 1;
        let mut r = base_reduction(improving, depth, move_count);

        // Pruning at shallow depth.
        if best_value > -MATE_BOUND {
            // V2_SHAPE 1: PV-like nodes prune over a more reduced depth.
            let prune_r = r + if tt_pv && p::V2_SHAPE() & 1 != 0 {
                1000
            } else {
                0
            };
            let mut lmr_depth = new_depth - prune_r / 1024;
            if tactical || gives_check {
                let capt_hist = if tactical {
                    d.capt_score(board, stm, m)
                } else {
                    0
                };
                if !gives_check && lmr_depth < 7 && !in_check && static_eval != NO_EVAL {
                    let fut = static_eval
                        + 250
                        + 230 * lmr_depth
                        + victim_value(board, m)
                        + 100 * capt_hist / 1024;
                    if fut <= alpha {
                        continue;
                    }
                }
                let see_hist = (capt_hist / 32).clamp(-150 * depth, 150 * depth);
                if (!gives_check || p::V2_CHK() & 1 == 0)
                    && (p::V2_SHAPE() & 2 == 0 || alpha >= 0)
                    && see_of(board) < -p::V2_SEE_T() * depth - see_hist
                {
                    continue;
                }
            } else {
                let mut h = d.cont_at(i, 1, pt)
                    + d.cont_at(i, 2, pt)
                    + i32::from(d.pawn[pawn_bucket * PT_NB + pt as usize]);
                if h < -p::V2_HIST_PRUNE() * depth {
                    continue;
                }
                h += 2 * i32::from(d.main[ft]);
                lmr_depth += h / p::V2_HIST_DIV().max(1);
                if !in_check && static_eval != NO_EVAL && lmr_depth < 9 {
                    let fut = static_eval
                        + if best_value < static_eval - 50 {
                            p::V2_FUT_BASE() + p::V2_FUT_NOBEST()
                        } else {
                            p::V2_FUT_BASE()
                        }
                        + p::V2_FUT_PER() * lmr_depth;
                    if fut <= alpha {
                        if best_value <= fut && !is_decisive(best_value) && !is_decisive(fut) {
                            best_value = fut;
                        }
                        continue;
                    }
                }
                let ld = lmr_depth.max(0);
                if see_of(board) < -p::V2_SEE_Q() * ld * ld {
                    continue;
                }
            }
        }

        // Extensions.
        let mut ext = 0;
        if Some(m) == tt_move
            && excluded.is_none()
            && depth >= 6
            && (ply as i32) < 2 * (depth + ply as i32).min(64)
            && has_lower
            && !is_decisive(tt_score)
            && tt_depth >= depth - 3
        {
            let sbeta = tt_score - (p::V2_SE_MARGIN() + 75 * i32::from(tt_pv && !pv)) * depth / 60;
            let sdepth = new_depth / 2;
            let saved = d.ss[i];
            d.ss[i].excluded = Some(m);
            let v = search(
                d,
                state,
                board,
                sbeta - 1,
                sbeta,
                sdepth,
                ply,
                cut_node,
                false,
                history,
            );
            d.ss[i] = saved;
            if state.budget.should_abort() {
                return 0;
            }
            if v < sbeta {
                let double_margin = 250 * i32::from(pv) - 200 * i32::from(!tt_capture);
                let triple_margin = 90 + 280 * i32::from(pv) - 230 * i32::from(!tt_capture)
                    + 100 * i32::from(tt_pv);
                ext =
                    1 + i32::from(v < sbeta - double_margin) + i32::from(v < sbeta - triple_margin);
                // The other moves of a node with a singular move go deeper too.
                if !pv && depth < 14 {
                    depth += 1;
                }
            } else if v >= beta && !is_decisive(v) {
                // Multi-cut: without the TT move the node still fails high.
                return v;
            } else if tt_score >= beta {
                ext = -3;
            } else if cut_node {
                ext = -2;
            }
        }
        new_depth += ext;

        // History score of the move for its reduction (before it is played).
        let stat = if tactical {
            p::V2_MVV_W() * victim_value(board, m) + d.capt_score(board, stm, m) - 4500
        } else {
            2 * i32::from(d.main[ft]) + d.cont_at(i, 1, pt) + d.cont_at(i, 2, pt) - 3600
        };

        // Play the move.
        if let Some(c) = state.counters() {
            c.searched_by_window[usize::from(pv)].fetch_add(1, Ordering::Relaxed);
            c.at_depth(entry_depth.max(0) as u32, 8);
        }
        d.ss[i].mv = Some(m);
        d.ss[i].pt = pt;
        d.ss[i].capture = capture;
        let (tok, child_check, child_history) = play(board, m, history);
        if let Some(c) = state.counters() {
            let kind = if move_count == 1 {
                0
            } else if capture {
                1
            } else if m.promote {
                2
            } else if child_check {
                if m.from.is_none() { 3 } else { 4 }
            } else if m.from.is_none() {
                5
            } else {
                6
            };
            c.record_move_kind(depth.max(0) as u32, in_check, kind);
        }
        d.ss[i + 1].in_check = child_check;
        d.ss[i + 1].excluded = None;

        let mut value;
        if depth >= 2 && move_count > 1 {
            let all_node = !pv && !cut_node;
            // Nodes on (or once on) a principal variation are reduced much
            // less, expected cut nodes much more.
            if tt_pv {
                r -= p::V2_TTPV_R()
                    + if pv { 1000 } else { 0 }
                    + if tt_hit && tt_score > alpha { 900 } else { 0 }
                    + if tt_depth >= depth {
                        980 + if cut_node { 1050 } else { 0 }
                    } else {
                        0
                    };
            }
            r += 540 - 66 * move_count as i32;
            if cut_node {
                r += p::V2_CUT_R() + if tt_move.is_none() { 1050 } else { 0 };
            }
            let shape = p::V2_SHAPE();
            if tt_capture && (!tactical || shape & 4 != 0) {
                r += if shape & 4 != 0 { 1050 } else { 1400 };
            }
            if shape & 4 != 0 {
                // V2_SHAPE 4: more reduction after several cutoffs among the
                // children, more at all-nodes in proportion.
                if d.ss[i + 1].cutoff_cnt > 1 {
                    r += 250
                        + if d.ss[i + 1].cutoff_cnt > 2 { 1120 } else { 0 }
                        + if all_node { 1040 } else { 0 };
                }
            } else if d.ss[i + 1].cutoff_cnt > 2 {
                r += 1050 + if all_node { 800 } else { 0 };
            }
            if Some(m) == tt_move {
                r -= 2000;
            }
            r -= stat * p::V2_STAT_R() / 8192;
            if shape & 4 != 0 && all_node {
                r += r * 270 / (256 * depth + 260);
            }
            // V2_SHAPE 8: the reduced search of a PV node goes one ply deeper.
            let (lo, hi) = if shape & 8 != 0 {
                (1 + i32::from(pv), new_depth + 2 + i32::from(pv))
            } else {
                (
                    1,
                    new_depth + i32::from(!all_node) + i32::from(pv && best_move.is_none()),
                )
            };
            let mut dd = (new_depth - r / 1024 + if shape & 8 != 0 { i32::from(pv) } else { 0 })
                .clamp(lo.min(hi), hi.max(1));
            // V2_CHECK_R: a move that gives check is reduced at most this
            // many plies; V2_CAPT_LMR 0: tactical moves are not reduced.
            if child_check {
                dd = dd.max(new_depth - p::V2_CHECK_R());
            }
            if tactical && p::V2_CAPT_LMR() == 0 {
                dd = dd.max(new_depth);
            }
            d.ss[i].reduction = new_depth - dd;
            value = -search(
                d,
                state,
                board,
                -(alpha + 1),
                -alpha,
                dd,
                ply + 1,
                true,
                false,
                &child_history,
            );
            d.ss[i].reduction = 0;
            if value > alpha && dd < new_depth {
                let deeper = value > best_value + 45 + 2 * new_depth;
                let shallower = value < best_value + 10;
                new_depth += i32::from(deeper) - i32::from(shallower);
                if new_depth > dd {
                    value = -search(
                        d,
                        state,
                        board,
                        -(alpha + 1),
                        -alpha,
                        new_depth,
                        ply + 1,
                        !cut_node,
                        false,
                        &child_history,
                    );
                }
                if !tactical {
                    let b = if value >= beta { 1600 } else { -400 };
                    d.update_cont(i, pt, b);
                }
            }
        } else if !pv || move_count > 1 {
            if tt_move.is_none() {
                r += 1100;
            }
            let nd = new_depth - i32::from(r > 3500) - i32::from(r > 4800 && new_depth > 2);
            value = -search(
                d,
                state,
                board,
                -(alpha + 1),
                -alpha,
                nd,
                ply + 1,
                !cut_node,
                false,
                &child_history,
            );
        } else {
            value = -INF;
        }
        if pv && (move_count == 1 || value > alpha) {
            let nd = if Some(m) == tt_move && tt_depth > 1 {
                new_depth.max(1)
            } else {
                new_depth
            };
            value = -search(
                d,
                state,
                board,
                -beta,
                -alpha,
                nd,
                ply + 1,
                false,
                true,
                &child_history,
            );
        }
        board.undo_move_for_search(tok);
        if state.budget.should_abort() {
            return 0;
        }

        if value > best_value {
            best_value = value;
            if value > alpha {
                best_move = Some(m);
                if value >= beta {
                    d.ss[i].cutoff_cnt += 1 + u32::from(ext < 2 || pv);
                    if let Some(c) = state.counters() {
                        c.record_cut(move_count as usize, !tactical, tt_move.is_some());
                    }
                    break;
                }
                alpha = value;
                if depth > 2 && depth < 14 && !is_decisive(value) {
                    depth -= 1;
                }
            }
        }
        if Some(m) != best_move {
            if tactical {
                if nc < 32 {
                    caps_tried[nc] = Some(m);
                    nc += 1;
                }
            } else if nq < 32 {
                quiets[nq] = Some(m);
                nq += 1;
            }
        }
    }

    if move_count == 0 {
        // Only the excluded move was legal.
        return alpha;
    }

    // Learn from the result.
    if let Some(bm) = best_move {
        let bonus = stat_bonus(depth);
        let malus = stat_malus(depth);
        if !is_tactical(board, bm) {
            d.update_quiet(i, ply as usize, stm, bm, pawn_bucket, bonus);
            for q in quiets[..nq].iter().flatten() {
                d.update_quiet(i, ply as usize, stm, *q, pawn_bucket, -malus);
            }
        } else {
            d.update_capture(board, stm, bm, bonus);
        }
        for c in caps_tried[..nc].iter().flatten() {
            d.update_capture(board, stm, *c, -malus);
        }
        // The opponent's quiet move that this one refuted early is penalised.
        if prior.mv.is_some() && prior.move_count <= 2 && !prior.capture && best_value >= beta {
            d.update_cont(i - 1, prior.pt, -malus / 2);
        }
    } else if let Some(pm) = prior.mv
        && !prior.capture
        && prior.pt != NO_PT
    {
        // The opponent's move that made this node fail low was good.
        let bonus = stat_bonus(depth) * if prior.move_count > 8 { 2 } else { 1 } / 2;
        d.update_cont(i - 1, prior.pt, bonus);
        gravity(&mut d.main[ft_key(stm.flip(), pm)], bonus / 2, MAIN_LIM);
    }

    // V2_FH_BLEND: a fail-high score is pulled toward beta, less so deeper.
    if p::V2_FH_BLEND() != 0
        && best_value >= beta
        && !is_decisive(best_value)
        && !is_decisive(beta)
        && !is_decisive(alpha)
    {
        best_value = (best_value * depth.max(1) + beta) / (depth.max(1) + 1);
    }

    // A node failing low under a PV-like parent counts as PV-like.
    if best_value <= alpha && excluded.is_none() {
        d.ss[i].tt_pv = d.ss[i].tt_pv || (d.ss[i - 1].tt_pv && depth > 3);
    }
    let tt_pv = d.ss[i].tt_pv;
    if excluded.is_none() {
        let bound = if best_value >= beta {
            Bound::Lower
        } else if pv && best_move.is_some() {
            Bound::Exact
        } else {
            Bound::Upper
        };
        store_tt_for_search_pv(
            state,
            hash,
            TtEntry {
                score: score_to_tt(best_value, ply),
                depth: depth.clamp(0, 255) as u8,
                bound,
                mv: best_move,
            },
            tt_pv,
        );
        if let Some((keys, se)) = corr
            && !best_move.is_some_and(|m| is_tactical(board, m))
            && !is_decisive(best_value)
            && !(bound == Bound::Lower && best_value <= se)
            && !(bound == Bound::Upper && best_value >= se)
        {
            state
                .history
                .learn_correction(keys, best_value - se, depth.max(1) as u32);
        }
    }
    best_value
}
