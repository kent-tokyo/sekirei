//! Legal-move generation and staged ordering for one alpha-beta node.

use crate::board::Board;
use crate::color::Color;
use crate::movegen::MoveBuffer;
use crate::mv::Move;

use super::move_order::{order_moves_head, order_moves_in_place};
use super::path::ContMoves;
use super::{MIN_SPLIT_DEPTH, SearchState, params as p, useless_non_promotion};

/// Inputs that determine the initial ordering of one node's legal moves.
#[derive(Clone, Copy)]
pub(super) struct MoveListContext {
    pub(super) depth: u32,
    pub(super) ply: u32,
    pub(super) previous_move: Option<Move>,
    pub(super) excluded_move: Option<Move>,
    pub(super) in_check: bool,
    pub(super) tt_move: Option<Move>,
    pub(super) killers: [Option<Move>; 2],
    pub(super) countermove: Option<Move>,
    pub(super) side_to_move: Color,
}

/// A terminal result found while preparing a node, or its ordered move list.
pub(super) enum MovePreparation {
    Mated,
    ExcludedAll,
    Ready(NodeMoveList),
}

/// An owned legal-move buffer plus the state of deferred quiet-move ordering.
pub(super) struct NodeMoveList {
    buffer: MoveBuffer,
    context: MoveListContext,
    deferred_order: bool,
    lazy_order: bool,
    lazy_tail: Option<usize>,
}

impl NodeMoveList {
    #[inline]
    pub(super) fn prepare(
        state: &SearchState,
        board: &mut Board,
        context: MoveListContext,
    ) -> MovePreparation {
        let _timer = state.timer(|d| &d.movegen_order_ns);
        let mut buffer = {
            let _generate_timer = state.timer(|d| &d.movegen_generate_ns);
            MoveBuffer::legal_with_in_check(board, context.in_check)
        };
        if buffer.is_empty() {
            return MovePreparation::Mated;
        }
        if p::SKIP_NONPROMO() != 0 {
            buffer
                .as_mut_list()
                .retain(|m| !useless_non_promotion(*m, context.side_to_move));
        }

        let lazy_order = p::ORDER_LAZY() != 0
            && !(state.pruning.ybw_split
                && context.depth >= MIN_SPLIT_DEPTH
                && rayon::current_num_threads() > 1);
        let mut deferred_order = false;
        let mut lazy_tail = None;
        {
            let _order_timer = state.timer(|d| &d.move_order_ns);
            let list = buffer.as_mut_list().as_mut_slice();
            let tt_position = context
                .tt_move
                .filter(|_| context.excluded_move.is_none())
                .and_then(|tt_move| list.iter().position(|&mv| mv == tt_move));
            if let Some(position) = tt_position {
                list[..=position].rotate_right(1);
                deferred_order = true;
            } else if lazy_order {
                let split = order_moves_head(
                    board,
                    list,
                    context.tt_move,
                    context.killers,
                    context.countermove,
                    &state.history,
                    context.side_to_move,
                );
                if split == 0 {
                    Self::order_all(state, board, list, context, context.tt_move);
                } else if split < list.len() {
                    lazy_tail = Some(split);
                }
            } else {
                Self::order_all(state, board, list, context, context.tt_move);
            }
        }

        if let Some(excluded_move) = context.excluded_move {
            buffer.as_mut_list().retain(|m| *m != excluded_move);
            if let Some(split) = lazy_tail.take() {
                let list = buffer.as_mut_list().as_mut_slice();
                let from = split.saturating_sub(1).min(list.len());
                Self::order_all(state, board, &mut list[from..], context, context.tt_move);
            }
        }
        if buffer.is_empty() {
            return MovePreparation::ExcludedAll;
        }

        MovePreparation::Ready(Self {
            buffer,
            context,
            deferred_order,
            lazy_order,
            lazy_tail,
        })
    }

    #[inline]
    pub(super) fn as_slice(&self) -> &[Move] {
        self.buffer.as_slice()
    }

    /// Whether a TT move was promoted to the front while its tail stayed
    /// unordered. Diagnostics use this fact after the tail has been ordered.
    #[inline]
    pub(super) fn deferred_order(&self) -> bool {
        self.deferred_order
    }

    /// Order the tail only after the TT move has failed to cut the node.
    #[inline]
    pub(super) fn order_after_first(&mut self, state: &SearchState, board: &mut Board) {
        if !self.deferred_order {
            return;
        }
        let tail = &mut self.buffer.as_mut_list().as_mut_slice()[1..];
        let split = if self.lazy_order {
            order_moves_head(
                board,
                tail,
                None,
                self.context.killers,
                self.context.countermove,
                &state.history,
                self.context.side_to_move,
            )
        } else {
            0
        };
        if split == 0 {
            Self::order_all(state, board, tail, self.context, None);
        } else if split < tail.len() {
            self.lazy_tail = Some(1 + split);
        }
    }

    /// Finish ordering an untouched quiet tail when sequential search reaches it.
    #[inline]
    pub(super) fn order_lazy_tail_at(
        &mut self,
        state: &SearchState,
        board: &mut Board,
        index: usize,
    ) {
        if self.lazy_tail != Some(index) {
            return;
        }
        self.lazy_tail = None;
        let tail = &mut self.buffer.as_mut_list().as_mut_slice()[index..];
        Self::order_all(state, board, tail, self.context, None);
    }

    #[inline]
    fn order_all(
        state: &SearchState,
        board: &mut Board,
        moves: &mut [Move],
        context: MoveListContext,
        tt_move: Option<Move>,
    ) {
        order_moves_in_place(
            board,
            moves,
            tt_move,
            context.killers,
            context.countermove,
            ContMoves::at(context.ply, context.previous_move),
            &state.history,
            context.side_to_move,
            state.counters(),
        );
    }
}
