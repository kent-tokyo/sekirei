//! Identity, storage, and learning rules for one main-search node.

use crate::bitboard::Bitboard;
use crate::board::Board;
use crate::color::Color;
use crate::mv::Move;
use crate::tt::Bound;

use super::{
    ContMoves, CorrKeys, HistoryTable, MATE_SCORE, SearchState, history_malus, params as p,
    reward_capture, store_tt, update_quiet_heuristics,
};

/// What identifies a main-search node in the transposition table: its hash,
/// the (possibly reduced) depth it is searched to, the ply, and the move a
/// singular verification search excludes.
#[derive(Clone, Copy)]
pub(super) struct NodeKey {
    hash: u64,
    depth: u32,
    ply: u32,
    skip_move: Option<Move>,
    /// The node's correction keys and corrected static evaluation, when it
    /// has one (not in check, shallow enough to evaluate).
    corr: Option<(CorrKeys, i32)>,
    /// Opponent pieces at the node, to tell captures apart.
    enemy: Bitboard,
    /// The node is a PV node or its TT entry says it was on a PV (stored as
    /// the entry's PV flag).
    tt_pv: bool,
}

impl NodeKey {
    #[inline]
    pub(super) fn new(
        board: &Board,
        hash: u64,
        depth: u32,
        ply: u32,
        skip_move: Option<Move>,
        tt_pv: bool,
    ) -> Self {
        let enemy = board.occ_for(board.side_to_move.flip());
        Self {
            hash,
            depth,
            ply,
            skip_move,
            corr: None,
            enemy,
            tt_pv,
        }
    }

    #[inline]
    pub(super) fn set_correction(&mut self, keys: CorrKeys, corrected_eval: i32) {
        self.corr = Some((keys, corrected_eval));
    }

    #[inline]
    pub(super) fn store(&self, state: &SearchState, score: i32, bound: Bound, mv: Option<Move>) {
        self.store_entry(state, score, bound, mv);
        self.learn_correction(state, score, bound, mv);
    }

    /// Store a fail-low without replacing the move already in the table, but
    /// still learn from the best move searched at this node.
    #[inline]
    pub(super) fn store_without_move(
        &self,
        state: &SearchState,
        score: i32,
        bound: Bound,
        best_move: Option<Move>,
    ) {
        self.store_entry(state, score, bound, None);
        self.learn_correction(state, score, bound, best_move);
    }

    #[inline]
    fn store_entry(&self, state: &SearchState, score: i32, bound: Bound, mv: Option<Move>) {
        store_tt(
            state,
            self.hash,
            score,
            self.depth,
            bound,
            mv,
            self.ply,
            self.skip_move,
            self.tt_pv,
        );
    }

    /// Teach the static-eval correction the result of this node when it
    /// says something about the evaluation: not for captures or promotions
    /// (the evaluation of the position does not see them), mate scores,
    /// singular verification searches, or bounds on the wrong side of the
    /// corrected evaluation.
    #[inline]
    fn learn_correction(&self, state: &SearchState, score: i32, bound: Bound, mv: Option<Move>) {
        let Some((keys, eval)) = self.corr else {
            return;
        };
        if self.skip_move.is_some()
            || state.budget.should_abort()
            || score.abs() >= MATE_SCORE - 1000
            || mv.is_some_and(|m| m.promote || self.enemy.contains(m.to))
            || (bound == Bound::Lower && score <= eval)
            || (bound == Bound::Upper && score >= eval)
        {
            return;
        }
        state
            .history
            .learn_correction(keys, score - eval, self.depth);
    }

    /// A beta cutoff on `cutoff_move`: penalise the quiet moves tried before
    /// it, reward it, store the lower bound and return the score.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn beta_cutoff(
        &self,
        state: &SearchState,
        best_score: i32,
        best_move: Option<Move>,
        tried_quiet: &[Move],
        tried_captures: &[Move],
        cutoff_move: Move,
        stm: Color,
        board: &Board,
        prev_mv: Option<Move>,
    ) -> i32 {
        let depth = self.depth;
        let cont = ContMoves::at(self.ply, prev_mv);
        let pawn_bucket = (p::PAWN_HIST_WEIGHT() > 0).then(|| HistoryTable::pawn_bucket(board));
        for &qm in tried_quiet {
            state.history.malus(stm, qm, depth);
            cont.update(&state.history, stm, qm, -history_malus(depth));
            if let Some(b) = pawn_bucket {
                state.history.pawnh_add(b, stm, qm, -history_malus(depth));
            }
        }
        let capture_malus = -history_malus(depth) * p::CAPT_UPDATE() / 16;
        for &cm in tried_captures {
            if let Some(victim) = board.piece_at(cm.to) {
                state
                    .history
                    .capture_add(stm, cm, victim.kind, capture_malus);
            }
        }
        reward_capture(&state.history, board, stm, cutoff_move, depth);
        update_quiet_heuristics(
            &state.killers,
            &state.history,
            &state.countermoves,
            cutoff_move,
            stm,
            self.ply,
            depth,
            board,
            prev_mv,
        );
        self.store(state, best_score, Bound::Lower, best_move);
        best_score
    }
}
