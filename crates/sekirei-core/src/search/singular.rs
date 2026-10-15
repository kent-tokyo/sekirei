//! Singular-extension verification for the main alpha-beta search.

use std::sync::Arc;

use crate::board::Board;
use crate::mv::Move;

use super::{MATE_SCORE, SearchHistory, SearchState, alpha_beta, params as p};

/// Inputs captured at a node before its singular verification search.
#[derive(Clone, Copy)]
pub(super) struct SingularContext {
    pub(super) alpha: i32,
    pub(super) beta: i32,
    pub(super) depth: u32,
    pub(super) ply: u32,
    pub(super) prev_move: Option<Move>,
    pub(super) excluded_move: Option<Move>,
    pub(super) in_check: bool,
    pub(super) tt_move: Option<Move>,
    pub(super) tt_score: Option<i32>,
    pub(super) tt_depth: u8,
    pub(super) pv: bool,
    pub(super) cut: bool,
    pub(super) double_extensions: u8,
}

/// Either continue with the TT move adjusted by this many plies, or return a
/// MultiCut score without searching the node's normal children.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum SingularOutcome {
    Extension(i32),
    Cutoff(i32),
}

impl SingularContext {
    /// Search the node without its TT move to decide whether that move is
    /// singular. A verification search cannot recursively start another
    /// singular probe because it carries the excluded move.
    pub(super) fn probe(
        self,
        state: &Arc<SearchState>,
        board: &mut Board,
        history: &SearchHistory<'_>,
    ) -> SingularOutcome {
        let Some(tt_score) = self.tt_score.filter(|_| {
            self.excluded_move.is_none()
                && self.depth >= p::SE_MIN_DEPTH() as u32
                && (!self.in_check || p::SE_IN_CHECK() != 0)
                && self.tt_move.is_some()
                && u32::from(self.tt_depth) >= self.depth.saturating_sub(3)
        }) else {
            return SingularOutcome::Extension(0);
        };

        let se_beta = (tt_score - p::SE_MARGIN() - p::SE_MARGIN_PER_DEPTH() * self.depth as i32)
            .max(self.alpha);
        let score = alpha_beta(
            state,
            board,
            se_beta - 1,
            se_beta,
            self.depth / 2,
            self.ply,
            false,
            self.prev_move,
            self.tt_move,
            Some(self.in_check),
            history,
        );

        if !self.pv && p::MULTICUT() == 1 && score >= se_beta && se_beta >= self.beta {
            return SingularOutcome::Cutoff(se_beta);
        }
        if !self.pv
            && p::MULTICUT() == 2
            && score >= se_beta
            && score >= self.beta
            && score.abs() < MATE_SCORE - 1000
        {
            return SingularOutcome::Cutoff(score);
        }

        let max_extension = p::SE_EXT_MAX();
        let extension = if score < se_beta && max_extension == 0 {
            let double_margin = p::SE_DOUBLE_MARGIN();
            1 + i32::from(
                double_margin > 0
                    && !self.pv
                    && self.double_extensions < p::SE_MAX_DOUBLES() as u8
                    && score < se_beta - double_margin,
            )
        } else if score < se_beta {
            let quiet = self
                .tt_move
                .is_some_and(|m| !m.promote && board.piece_at(m.to).is_none());
            let margin = |base: i32, pv: i32, quiet_bonus: i32| {
                base + if self.pv { pv } else { 0 } - if quiet { quiet_bonus } else { 0 }
            };
            let margin_two = margin(p::SE_M2_BASE(), p::SE_M2_PV(), p::SE_M2_QUIET());
            let margin_three = margin(p::SE_M3_BASE(), p::SE_M3_PV(), p::SE_M3_QUIET());
            let can_extend_again = self.double_extensions < p::SE_MAX_DOUBLES() as u8;
            let mut extension = 1;
            if can_extend_again && max_extension >= 2 && score < se_beta - margin_two {
                extension += 1;
                if max_extension >= 3 && score < se_beta - margin_three {
                    extension += 1;
                }
            }
            extension
        } else if tt_score >= self.beta {
            -p::SE_NEG_TT()
        } else if self.cut {
            -p::SE_NEG_CUT()
        } else {
            0
        };
        SingularOutcome::Extension(extension)
    }
}
