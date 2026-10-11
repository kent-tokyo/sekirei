//! Late-move pruning decisions for the main alpha-beta search.

use std::sync::OnceLock;

use crate::bitboard::Bitboard;
use crate::board::Board;
use crate::color::Color;
use crate::eval::PIECE_VALUE;
use crate::movegen::{move_gives_direct_check, see_exchange, see_swap};
use crate::mv::Move;
use crate::piece::PieceKind;

use super::heuristics::HistoryTable;
use super::path::ContMoves;
use super::{MATE_SCORE, params as p};

/// Exact cache of the existing floating-point LMR formula for all representable
/// TT depths and the practical maximum shogi move count. This removes two
/// transcendental `ln` calls from every late-move probe without changing the
/// reduction chosen by the search.
static LMR_REDUCTION_TABLE: OnceLock<Box<[[u8; 600]]>> = OnceLock::new();

/// The late move reduction cap of a move that turns out to give check,
/// decided on the board before the move: `CHECK_R_MAX`, or
/// `CHECK_R_MAX_BAD` when the move hangs the moved piece.
#[inline]
pub(super) fn check_reduction_cap(board: &Board, m: Move, reduce: u32) -> u32 {
    let (good, bad) = (p::CHECK_R_MAX() as u32, p::CHECK_R_MAX_BAD() as u32);
    if reduce <= good.min(bad) || good == bad {
        return good;
    }
    if see_exchange(board, m) < 0 {
        bad
    } else {
        good
    }
}

/// Whether a non-capture promotion is reduced and pruned like a quiet move
/// (`QUIET_PROMO`).
#[inline]
pub(super) fn promotion_counts_as_quiet(m: Move) -> bool {
    match p::QUIET_PROMO() {
        0 => false,
        1 => m.piece_kind != PieceKind::Fu,
        _ => true,
    }
}

/// Compute Late Move Reduction amount for a move.
/// Returns 0 if the move should not be reduced.
#[inline]
#[allow(clippy::too_many_arguments)]
pub(super) fn lmr_reduce(
    board: &Board,
    m: Move,
    move_idx: usize,
    depth: u32,
    killers: &[Option<Move>; 2],
    tt_mv: Option<Move>,
    history: &HistoryTable,
    stm: Color,
) -> u32 {
    if depth < p::LMR_MIN_DEPTH() as u32 {
        return 0;
    }
    if move_idx < 2 {
        return 0;
    }
    if tt_mv.is_some_and(|t| t == m) {
        return 0;
    }
    let mut less = 0;
    if m.from.is_some_and(|_| board.piece_at(m.to).is_some()) {
        match p::CAPTURE_LMR() {
            0 => return 0,
            1 if see_swap(board, m) >= 0 => return 0,
            1 => {}
            _ => {
                if see_swap(board, m) >= 0 {
                    less = 1;
                }
            }
        }
    } else if m.promote && !promotion_counts_as_quiet(m) {
        return 0;
    }
    if killers.contains(&Some(m)) {
        match p::KILLER_LMR() {
            0 => return 0,
            1 => less = 1,
            _ => {}
        }
    }
    let mut r = lmr_base_reduction(depth, move_idx).saturating_sub(less);
    if board.piece_at(m.to).is_some() {
        return r;
    }
    let hist = history.get(stm, m);
    if hist > p::LMR_HIST() {
        r = r.saturating_sub(1);
    } else if hist < -p::LMR_HIST() && depth >= 5 {
        r += 1;
    }
    r
}

/// What late move reductions know about the node beyond the move itself.
#[derive(Clone, Copy)]
pub(super) struct LmrContext {
    pub(super) pv: bool,
    /// Expected cut node.
    pub(super) cut: bool,
    pub(super) improving: bool,
    pub(super) cont: ContMoves,
    /// Reduce less for a PV flag (`TT_PV_MODE`, `TTPV_LMR16`).
    pub(super) tt_pv: bool,
    /// The node's TT move is a capture (`LMR_TTCAP16`).
    pub(super) tt_capture: bool,
    /// Opponent pieces, to tell quiet moves.
    pub(super) enemy: Bitboard,
}

impl LmrContext {
    /// History of `m` at this node: butterfly + continuation + two-ply
    /// follow-up.
    #[inline]
    pub(super) fn history_of(&self, history: &HistoryTable, stm: Color, m: Move) -> i32 {
        self.cont.primary_score(history, stm, m)
    }
}

/// Adjust the base reduction `r` of a reducible move (0 stays 0). The
/// adjustments are summed in sixteenths of a ply and rounded, so that the
/// tuner can move them in small steps.
#[inline]
pub(super) fn lmr_adjust(
    r: u32,
    ctx: &LmrContext,
    history: &HistoryTable,
    stm: Color,
    m: Move,
) -> u32 {
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
    if ctx.cut {
        r16 += p::CUT_LMR16();
    }
    if ctx.tt_pv {
        r16 -= p::TTPV_LMR16();
    }
    if ctx.tt_capture && !ctx.enemy.contains(m.to) {
        r16 += p::LMR_TTCAP16();
    }
    if !ctx.improving {
        r16 += p::LMR_NOT_IMPROVING16();
    }
    ((r16 + 8) / 16).max(0) as u32
}

#[inline]
pub(super) fn lmr_base_reduction(depth: u32, move_idx: usize) -> u32 {
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

/// The parent's state that decides whether a late move is skipped.
#[derive(Clone, Copy)]
pub(super) struct LateMoveNode {
    pub(super) depth: u32,
    pub(super) alpha: i32,
    pub(super) beta: i32,
    pub(super) in_check: bool,
    pub(super) static_eval: Option<i32>,
    pub(super) best_score: i32,
    pub(super) improving: bool,
    /// History of the move (butterfly + continuation + follow-up), computed
    /// only when history pruning is enabled.
    pub(super) history: i32,
    /// Depth the move would be searched to after its late move reduction.
    pub(super) lmr_depth: u32,
    /// PRUNE_STYLE 1 inputs: continuation + follow-up history and butterfly
    /// history of a quiet move (or the capture history of a capture),
    /// whether a move has already raised alpha, and whether the node is a
    /// PV node.
    pub(super) cont_hist: i32,
    pub(super) main_hist: i32,
    pub(super) alpha_raised: bool,
    pub(super) pv: bool,
}

impl LateMoveNode {
    /// PRUNE_STYLE 1: one rule set over the reduced depth at every node
    /// depth. Once a move has been searched without a lost score:
    /// - move-count pruning of quiet moves and drops (checks too unless
    ///   `PS_LMP_CHECKS` is 0) from `(PS_LMP_BASE + d^2) / (2 - improving)`;
    /// - captures, promotions and checks: futility of captures by the
    ///   captured piece and capture history while the reduced depth is
    ///   below 7, and static-exchange pruning below `-(PS_SEE_TACT * d)`;
    /// - other quiet moves: pruned when the continuation history is below
    ///   `-PS_CONT_K * d`; the reduced depth moves by the histories
    ///   (`/ PS_HIST_DIV`); futility on that depth below 13
    ///   (`PS_FUT_BASE + PS_FUT_NOBEST + PS_FUT_PER * d`); static-exchange
    ///   pruning below `-PS_SEE_Q * d^2` of that depth.
    fn prunes_by_lmr_depth(
        &self,
        board: &Board,
        m: Move,
        is_quiet: bool,
        move_number: usize,
    ) -> bool {
        if self.best_score <= -(MATE_SCORE - 1000) {
            return false;
        }
        let depth = self.depth as i32;
        let parts = p::PS_PARTS();
        let gives_check = move_gives_direct_check(board, m);
        let lmp_nodes = p::PS_LMP_NODES();
        if parts & 1 != 0
            && is_quiet
            && (!self.pv || lmp_nodes & 1 != 0)
            && (!self.in_check || lmp_nodes & 2 != 0)
            && (!gives_check || p::PS_LMP_CHECKS() != 0)
            && (move_number as i32)
                >= (p::PS_LMP_BASE() + depth * depth) / (2 - i32::from(self.improving))
        {
            return true;
        }
        let lmr_depth = self.lmr_depth as i32;
        if !is_quiet || gives_check {
            if parts & 2 == 0 {
                return false;
            }
            let victim = board.piece_at(m.to).filter(|_| m.from.is_some());
            if let Some(v) = victim
                && !gives_check
                && lmr_depth < 7
                && let Some(se) = self.static_eval
                && se
                    + 218
                    + 223 * lmr_depth
                    + PIECE_VALUE[v.kind.index()]
                    + 131 * self.main_hist / 1024
                    <= self.alpha
            {
                return true;
            }
            let margin = (p::PS_SEE_TACT() * depth + self.main_hist * 34 / 1024).max(0);
            return self.alpha >= 0 && see_exchange(board, m) < -margin;
        }
        if self.pv && p::PS_PV_QUIET() == 0 {
            return false;
        }
        if parts & 4 != 0 && self.cont_hist < -p::PS_CONT_K() * depth {
            return true;
        }
        let hist = self.cont_hist + 71 * self.main_hist / 32;
        let d = lmr_depth + hist / p::PS_HIST_DIV().max(1);
        if parts & 8 != 0
            && !self.in_check
            && d < 13
            && let Some(se) = self.static_eval
            && se
                + p::PS_FUT_BASE()
                + if self.alpha_raised {
                    0
                } else {
                    p::PS_FUT_NOBEST()
                }
                + p::PS_FUT_PER() * d
                + if se > self.alpha { 86 } else { 0 }
                <= self.alpha
        {
            return true;
        }
        let d = d.max(0);
        parts & 16 != 0 && see_exchange(board, m) < -p::PS_SEE_Q() * d * d
    }

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
    pub(super) fn prunes(
        &self,
        board: &Board,
        m: Move,
        is_quiet: bool,
        move_number: usize,
    ) -> bool {
        if p::PRUNE_STYLE() == 1 {
            return self.prunes_by_lmr_depth(board, m, is_quiet, move_number);
        }
        let depth = self.depth;
        if depth == 1
            && let Some(se) = self.static_eval
            && is_quiet
            && se + p::FUTILITY_MARGIN() < self.alpha
        {
            return true;
        }
        let non_pv = (self.beta - self.alpha == 1 || p::PRUNE_PV() == 1)
            && !self.in_check
            && self.best_score > -(MATE_SCORE - 1000);
        let fut_node = non_pv
            || (p::PRUNE_PV() == 2 && !self.in_check && self.best_score > -(MATE_SCORE - 1000));
        let exempt_check = || check_exempt_from_pruning(board, m);
        if non_pv
            && !is_quiet
            && p::CAPT_FUT_MAX_DEPTH() > 0
            && self.lmr_depth <= p::CAPT_FUT_MAX_DEPTH() as u32
            && let Some(se) = self.static_eval
        {
            let victim = board
                .piece_at(m.to)
                .map_or(0, |v| PIECE_VALUE[v.kind.index()]);
            let promo = if m.promote {
                PIECE_VALUE[m.piece_kind.promoted().index()] - PIECE_VALUE[m.piece_kind.index()]
            } else {
                0
            };
            if se
                + p::CAPT_FUT_BASE()
                + p::CAPT_FUT_PER_DEPTH() * self.lmr_depth as i32
                + victim
                + promo
                <= self.alpha
                && !move_gives_direct_check(board, m)
            {
                return true;
            }
        }
        let (lmr_depth, fut_depth) = if p::PRUNE_HIST_DIV() > 0 && is_quiet {
            let d = (self.lmr_depth as i32 + self.history / p::PRUNE_HIST_DIV()).max(0) as u32;
            (d.max(1), d)
        } else {
            (self.lmr_depth.max(1), self.lmr_depth)
        };
        if fut_node
            && is_quiet
            && p::LMR_FUT_MAX_DEPTH() > 0
            && fut_depth <= p::LMR_FUT_MAX_DEPTH() as u32
            && self.static_eval.is_some_and(|se| {
                se + p::SHALLOW_FUTILITY_BASE() + p::SHALLOW_FUTILITY_PER_DEPTH() * lmr_depth as i32
                    <= self.alpha
            })
            && !exempt_check()
        {
            return true;
        }
        let lmp_checks = p::LMP_CHECKS() != 0;
        let lmp_node = non_pv
            || (p::LMP_PV() != 0
                && depth <= p::LMP_PV_MAX_DEPTH() as u32
                && !self.in_check
                && self.best_score > -(MATE_SCORE - 1000));
        if lmp_node
            && is_quiet
            && depth <= p::LMP_MAX_DEPTH().max(p::SHALLOW_PRUNE_MAX_DEPTH()) as u32
            && move_number as u32 >= lmp_limit(depth, self.improving)
            && (lmp_checks || !exempt_check())
        {
            return true;
        }
        if non_pv
            && depth > p::SHALLOW_PRUNE_MAX_DEPTH() as u32
            && depth <= p::LMP_MAX_DEPTH() as u32
        {
            return false;
        }
        let shallow_non_pv = non_pv && depth <= p::SHALLOW_PRUNE_MAX_DEPTH() as u32;
        if !shallow_non_pv {
            return false;
        }
        if is_quiet {
            (lmp_checks && move_number as u32 >= lmp_limit(depth, self.improving))
                || (move_number as u32 >= lmp_limit(depth, self.improving)
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
                || (p::QSEE_MARGIN() > 0
                    && see_exchange(board, m) < -p::QSEE_MARGIN() * (depth * depth) as i32
                    && !exempt_check())
        } else {
            m.from.is_some()
                && board.piece_at(m.to).is_some()
                && see_swap(board, m) < -120 * depth as i32
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
        1 => move_gives_direct_check(board, m) && see_exchange(board, m) >= 0,
        _ => false,
    }
}

/// Shallow move-count pruning: quiet moves from this number on are pruned.
#[inline]
fn lmp_limit(depth: u32, improving: bool) -> u32 {
    let base = p::LMP_BASE() as u32 + depth * depth;
    if improving {
        base * p::LMP_IMPROVING_MUL() as u32 / 16
    } else {
        base * p::LMP_NONIMP_MUL() as u32 / 16
    }
}
