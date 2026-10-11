//! Static-evaluation preparation and pruning decisions for alpha-beta nodes.

use crate::board::Board;
use crate::color::Color;
use crate::mv::Move;
use crate::tt::Bound;

use super::heuristics::CorrKeys;
use super::node::NodeKey;
use super::path::record_static_eval;
use super::{MATE_SCORE, SearchState, evaluate_for_search, params as p};

/// Static evaluation facts shared by forward-pruning rules at one node.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct NodeEvaluation {
    /// Corrected and optionally TT-refined evaluation, absent in check or
    /// beyond the configured static-evaluation depth.
    pub(super) value: Option<i32>,
    /// Whether the position improved over the same side's evaluation two
    /// plies earlier. Missing evaluations count as improving.
    pub(super) improving: bool,
}

/// Evaluate one quiet node, register its correction-learning baseline, and
/// refine the pruning value with a compatible TT bound when enabled.
#[allow(clippy::too_many_arguments)]
pub(super) fn prepare_node_evaluation(
    state: &SearchState,
    board: &Board,
    stm: Color,
    depth: u32,
    ply: u32,
    in_check: bool,
    tt_bound_score: Option<(Bound, i32)>,
    node: &mut NodeKey,
) -> NodeEvaluation {
    let corrected = if !in_check && depth <= p::STATIC_EVAL_MAX_DEPTH() as u32 {
        let raw = evaluate_for_search(state, board);
        let keys = CorrKeys::of(board, stm);
        let value =
            (raw + state.history.correction(keys)).clamp(-(MATE_SCORE - 2000), MATE_SCORE - 2000);
        node.set_correction(keys, value);
        Some(value)
    } else {
        None
    };
    let improving = record_static_eval(ply, corrected);
    let value = if p::EVAL_TT() != 0 {
        corrected.map(|eval| match tt_bound_score {
            Some((bound, score))
                if score.abs() < MATE_SCORE - 2000
                    && match bound {
                        Bound::Exact => true,
                        Bound::Lower => score > eval,
                        Bound::Upper => score < eval,
                    } =>
            {
                score
            }
            _ => eval,
        })
    } else {
        corrected
    };
    NodeEvaluation { value, improving }
}

/// Reverse futility pruning result, if the node's rough lower bound already
/// reaches beta. `RFP_GUARD` restricts this to null-window nodes whose TT move
/// is absent or a capture and damps the returned score toward beta.
#[allow(clippy::too_many_arguments)]
pub(super) fn reverse_futility_score(
    board: &Board,
    static_eval: Option<i32>,
    improving: bool,
    depth: u32,
    alpha: i32,
    beta: i32,
    tt_pv: bool,
    tt_mv: Option<Move>,
) -> Option<i32> {
    let eval = static_eval?;
    let guarded = p::RFP_GUARD() != 0;
    if depth > p::RFP_MAX_DEPTH() as u32
        || beta.abs() >= MATE_SCORE - 1000
        || (tt_pv && p::TT_PV_MODE() & 2 != 0)
        || (guarded
            && (beta - alpha != 1
                || tt_mv.is_some_and(|m| m.from.is_none() || board.piece_at(m.to).is_none())))
        || (eval
            - (p::RFP_MARGIN()
                - if improving {
                    p::RFP_IMPROVING_BONUS()
                } else {
                    0
                })
                * depth as i32)
            < beta
    {
        return None;
    }
    Some(if guarded {
        (2 * beta + eval) / 3
    } else {
        match p::RFP_DAMP() {
            0 => eval,
            1 => (eval + beta) / 2,
            _ => (eval + 2 * beta) / 3,
        }
    })
}

/// At a shallow null-window node far below alpha, ask quiescence whether the
/// position still fails low. The callback is evaluated only when the razor
/// margin is crossed.
pub(super) fn razoring_score(
    static_eval: Option<i32>,
    depth: u32,
    alpha: i32,
    beta: i32,
    mut quiescence_probe: impl FnMut(i32, i32) -> i32,
) -> Option<i32> {
    let eval = static_eval?;
    if depth > 2
        || beta - alpha != 1
        || alpha.abs() >= MATE_SCORE - 1000
        || eval + p::RAZOR_MARGIN_BASE() + p::RAZOR_MARGIN_PER_DEPTH() * depth as i32 > alpha
    {
        return None;
    }
    let score = quiescence_probe(alpha, alpha + 1);
    (score <= alpha).then_some(score)
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use crate::board::Board;

    use super::{razoring_score, reverse_futility_score};

    #[test]
    fn razoring_calls_quiescence_only_after_crossing_the_margin() {
        let calls = Cell::new(0);
        let skipped = razoring_score(Some(-100_000), 3, 0, 1, |_, _| {
            calls.set(calls.get() + 1);
            -1
        });
        assert_eq!(skipped, None);
        assert_eq!(calls.get(), 0);

        let accepted = razoring_score(Some(-100_000), 2, 0, 1, |lo, hi| {
            calls.set(calls.get() + 1);
            assert_eq!((lo, hi), (0, 1));
            -4
        });
        assert_eq!(accepted, Some(-4));
        assert_eq!(calls.get(), 1);
    }

    #[test]
    fn razoring_keeps_searching_when_quiescence_raises_alpha() {
        assert_eq!(razoring_score(Some(-100_000), 2, 0, 1, |_, _| 1), None);
    }

    #[test]
    fn reverse_futility_obeys_depth_mate_pv_and_margin_guards() {
        let board = Board::startpos();
        assert_eq!(
            reverse_futility_score(&board, None, false, 1, 0, 1, false, None),
            None
        );
        assert_eq!(
            reverse_futility_score(&board, Some(10_000), false, 99, 0, 1, false, None),
            None
        );
        assert_eq!(
            reverse_futility_score(
                &board,
                Some(10_000),
                false,
                1,
                0,
                super::MATE_SCORE,
                false,
                None,
            ),
            None
        );
        assert_eq!(
            reverse_futility_score(&board, Some(0), false, 1, 0, 500, false, None),
            None
        );
        let score = reverse_futility_score(&board, Some(1_000), true, 1, 0, 100, false, None);
        assert_eq!(score, Some(1_000));
    }
}
