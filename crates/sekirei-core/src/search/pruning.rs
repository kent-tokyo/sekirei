//! Forward-pruning probes used by the main alpha-beta search.

use std::sync::Arc;

use crate::board::Board;
use crate::movegen::{MoveBuffer, see_swap};
use crate::mv::Move;
use crate::tt::{Bound, TtEntry};

use super::{
    SearchHistory, SearchState, alpha_beta, params as p, play, score_to_tt, set_current_move,
    set_cut, sort_moves_by_cached_key, store_tt_for_search, useless_non_promotion,
};

/// ProbCut: when a shallow search of a clearly winning capture (SEE at least
/// `PC_MARGIN`) beats `beta + PC_MARGIN`, the node is assumed to fail high.
/// Returns that score.
#[allow(clippy::too_many_arguments)]
pub(super) fn probcut(
    state: &Arc<SearchState>,
    board: &mut Board,
    beta: i32,
    depth: u32,
    ply: u32,
    cut_node: bool,
    history: &SearchHistory<'_>,
) -> Option<i32> {
    let pc_beta = beta + p::PC_MARGIN();
    let mut caps = MoveBuffer::captures(board);
    let stm = board.side_to_move;
    caps.as_mut_list().retain(|m| {
        see_swap(board, *m) >= p::PC_MARGIN()
            && (p::SKIP_NONPROMO() == 0 || !useless_non_promotion(*m, stm))
    });
    let cap_list = caps.as_mut_list().as_mut_slice();
    let mut cap_key = |m: &Move| -see_swap(board, *m);
    sort_moves_by_cached_key(cap_list, &mut cap_key);
    let pc_depth = (depth - 4).min(3); // cap at 3 to keep the probe cheap
    for &cap in caps.as_slice() {
        if state.budget.should_abort() {
            break;
        }
        set_current_move(ply, Some(cap));
        let (tok, child_in_check, child_history) = play(board, cap, history);
        set_cut(ply + 1, !cut_node);
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
            if p::PC_STORE() != 0 && !state.budget.should_abort() {
                store_tt_for_search(
                    state,
                    board.hash(),
                    TtEntry {
                        score: score_to_tt(pc_score, ply),
                        depth: (pc_depth + 1) as u8,
                        bound: Bound::Lower,
                        mv: Some(cap),
                    },
                );
            }
            return Some(pc_score);
        }
    }
    None
}

/// Null move pruning: a reduced search after passing the move. A fail-high
/// at depth 6 or more is confirmed by a real shallow search first, against
/// zugzwang-like horizon effects common in shogi. Returns the null-move score
/// when the node is cut.
#[allow(clippy::too_many_arguments)]
pub(super) fn null_move_pruning(
    state: &Arc<SearchState>,
    board: &mut Board,
    beta: i32,
    depth: u32,
    ply: u32,
    prev_mv: Option<Move>,
    cut_node: bool,
    history: &SearchHistory<'_>,
    static_eval: Option<i32>,
) -> Option<i32> {
    // NMP_EVAL_DIV: a static eval far above beta reduces the null move more.
    let eval_r = match static_eval {
        Some(se) if p::NMP_EVAL_DIV() > 0 && se > beta => {
            ((se - beta) / p::NMP_EVAL_DIV()).min(p::NMP_EVAL_RMAX()) as u32
        }
        _ => 0,
    };
    let r = nmp_reduction(depth) + eval_r;
    set_current_move(ply, None);
    let null_tok = board.do_null_move();
    set_cut(ply + 1, !cut_node);
    let null_score = -alpha_beta(
        state,
        board,
        -beta,
        -beta + 1,
        depth.saturating_sub(1 + r),
        ply + 1,
        false,
        None,
        None,
        None,
        history,
    );
    board.undo_null_move(null_tok);
    if null_score < beta {
        if let Some(d) = state.counters() {
            d.exit(7);
        }
        return None;
    }
    if depth < p::NMP_VERIFY_DEPTH() as u32 {
        return Some(null_score);
    }
    let verify = alpha_beta(
        state,
        board,
        beta - 1,
        beta,
        depth.saturating_sub(1 + r),
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
pub(super) fn nmp_eval_allows(static_eval: Option<i32>, beta: i32) -> bool {
    let margin = p::NMP_EVAL_MARGIN();
    margin >= 5000 || static_eval.is_none_or(|se| se >= beta - margin)
}

/// Null move depth reduction at `depth`.
#[inline]
fn nmp_reduction(depth: u32) -> u32 {
    p::NMP_R() as u32 + depth * p::NMP_R_PER_DEPTH() as u32 / 64
}
